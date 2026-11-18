//! Routes a role to its configured backend with budget, concurrency limits,
//! one retry for malformed output, short rate-limit waits and fallback.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mwa_backends::{Backend, BackendError, DeltaFn};
use mwa_protocol::{Config, Quota, Request, Response, Role};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::runlog::{CallRecord, LogError, RunLog};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("역할 '{0}'이 mwa.toml에 설정되지 않았습니다")]
    RoleNotConfigured(Role),
    #[error("실행 예산 {cap}달러에 도달해 멈췄습니다 (사용 {spent:.4}달러)")]
    BudgetExceeded { spent: f64, cap: f64 },
    // Not `#[source]`: the message already includes the backend error, and a source
    // would make anyhow's `{:#}` print it twice.
    #[error("[{backend}] {error}")]
    Backend {
        backend: String,
        error: BackendError,
    },
    #[error(transparent)]
    Log(#[from] LogError),
}

#[derive(Debug)]
pub struct CallOutcome {
    pub response: Response,
    pub backend: String,
    pub model: String,
    pub cost_usd: Option<f64>,
}

struct Slot {
    backend: Box<dyn Backend>,
    permits: Semaphore,
}

pub struct Dispatcher {
    cfg: Config,
    slots: BTreeMap<String, Slot>,
    log: Mutex<RunLog>,
    run_id: String,
    spent: Mutex<f64>,
    quota: Mutex<Option<Quota>>,
    /// Runtime model choice per role (e.g. picked in the UI); not written back to mwa.toml.
    model_overrides: Mutex<BTreeMap<Role, String>>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Dispatcher {
    pub fn new(cfg: Config, log: RunLog) -> Self {
        let backends = cfg
            .backends
            .iter()
            .map(|(name, b)| (name.clone(), mwa_backends::build(b)))
            .collect();
        Self::with_backends(cfg, log, backends)
    }

    /// Like `new`, with caller-supplied backend instances (used by tests).
    pub fn with_backends(
        cfg: Config,
        log: RunLog,
        backends: BTreeMap<String, Box<dyn Backend>>,
    ) -> Self {
        let slots = backends
            .into_iter()
            .map(|(name, backend)| {
                let limit = cfg.backends.get(&name).map_or(1, |b| b.max_concurrency());
                (
                    name,
                    Slot {
                        backend,
                        permits: Semaphore::new(limit),
                    },
                )
            })
            .collect();
        let run_id = format!("{}-{}", now_ms(), std::process::id());
        Self {
            cfg,
            slots,
            log: Mutex::new(log),
            run_id,
            spent: Mutex::new(0.0),
            quota: Mutex::new(None),
            model_overrides: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn spent_usd(&self) -> f64 {
        *self.spent.lock().unwrap()
    }

    /// Most recent subscription usage window reported by any backend.
    pub fn last_quota(&self) -> Option<Quota> {
        self.quota.lock().unwrap().clone()
    }

    pub fn budget_usd(&self) -> f64 {
        self.cfg.budget.max_usd_per_run
    }

    /// (backend, model) configured for a role, if any.
    pub fn role_model(&self, role: Role) -> Option<(String, String)> {
        let rc = self.cfg.role(role)?;
        Some((
            rc.backend.get_ref().clone(),
            self.model_for(role, &rc.model),
        ))
    }

    /// Uses `model` for `role` from now on in this process (the role's backend is unchanged).
    pub fn set_role_model(&self, role: Role, model: &str) -> Result<(), CoreError> {
        self.cfg
            .role(role)
            .ok_or(CoreError::RoleNotConfigured(role))?;
        self.model_overrides
            .lock()
            .unwrap()
            .insert(role, model.to_string());
        Ok(())
    }

    fn model_for(&self, role: Role, configured: &str) -> String {
        self.model_overrides
            .lock()
            .unwrap()
            .get(&role)
            .cloned()
            .unwrap_or_else(|| configured.to_string())
    }

    pub fn backend(&self, name: &str) -> Option<&dyn Backend> {
        self.slots.get(name).map(|s| s.backend.as_ref())
    }

    /// Removes run-log entries older than the configured retention.
    pub fn prune_log(&self) -> Result<usize, LogError> {
        self.log
            .lock()
            .unwrap()
            .prune(self.cfg.limits.log_retention_days, now_ms())
    }

    pub async fn call(
        &self,
        role: Role,
        system: &str,
        prompt: &str,
        images: &[PathBuf],
        schema: Option<&Value>,
    ) -> Result<CallOutcome, CoreError> {
        self.call_inner(role, system, prompt, images, schema, None)
            .await
    }

    /// Like `call`, reporting generated text as it arrives (chat UI).
    pub async fn call_streaming(
        &self,
        role: Role,
        system: &str,
        prompt: &str,
        on_delta: DeltaFn<'_>,
    ) -> Result<CallOutcome, CoreError> {
        self.call_inner(role, system, prompt, &[], None, Some(on_delta))
            .await
    }

    async fn call_inner(
        &self,
        role: Role,
        system: &str,
        prompt: &str,
        images: &[PathBuf],
        schema: Option<&Value>,
        on_delta: Option<DeltaFn<'_>>,
    ) -> Result<CallOutcome, CoreError> {
        let rc = self
            .cfg
            .role(role)
            .ok_or(CoreError::RoleNotConfigured(role))?;
        let model = self.model_for(role, &rc.model);
        let req = Request {
            system: system.into(),
            prompt: prompt.into(),
            images: images.to_vec(),
            schema: schema.cloned(),
            model: model.clone(),
        };
        match self
            .attempt(role, rc.backend.get_ref(), &model, &req, on_delta)
            .await
        {
            Err(CoreError::Backend {
                error: BackendError::RateLimited { .. },
                ..
            }) if rc.fallback.is_some() => {
                let fb = rc.fallback.as_ref().expect("checked");
                self.attempt(role, fb.backend.get_ref(), &fb.model, &req, on_delta)
                    .await
            }
            other => other,
        }
    }

    /// Calls one specific backend/model directly (used by `mwa doctor`), still logged and budgeted.
    pub async fn call_backend(
        &self,
        label: &str,
        backend: &str,
        req: &Request,
    ) -> Result<CallOutcome, CoreError> {
        self.attempt_labeled(label, backend, &req.model, req, None)
            .await
    }

    async fn attempt(
        &self,
        role: Role,
        backend: &str,
        model: &str,
        base: &Request,
        on_delta: Option<DeltaFn<'_>>,
    ) -> Result<CallOutcome, CoreError> {
        self.attempt_labeled(role.as_str(), backend, model, base, on_delta)
            .await
    }

    async fn attempt_labeled(
        &self,
        label: &str,
        backend: &str,
        model: &str,
        base: &Request,
        on_delta: Option<DeltaFn<'_>>,
    ) -> Result<CallOutcome, CoreError> {
        let slot = self
            .slots
            .get(backend)
            .expect("config validation guarantees role backends exist");
        let mut req = base.clone();
        req.model = model.to_string();

        let mut retried_invalid = false;
        let mut waited = false;
        loop {
            let spent = self.spent_usd();
            let cap = self.cfg.budget.max_usd_per_run;
            if spent >= cap {
                return Err(CoreError::BudgetExceeded { spent, cap });
            }

            let started_at_ms = now_ms();
            let start = Instant::now();
            let result = {
                let _permit = slot
                    .permits
                    .acquire()
                    .await
                    .expect("semaphore never closed");
                match on_delta {
                    Some(cb) => slot.backend.complete_streaming(&req, cb).await,
                    None => slot.backend.complete(&req).await,
                }
            };
            let mut record = CallRecord {
                run_id: self.run_id.clone(),
                started_at_ms,
                role: label.to_string(),
                backend: backend.to_string(),
                kind: slot.backend.kind().to_string(),
                model: model.to_string(),
                duration_ms: start.elapsed().as_millis() as i64,
                ..Default::default()
            };

            match result {
                Ok(resp) => {
                    let resolved = resp.model.clone().unwrap_or_else(|| model.to_string());
                    let cost = resp.usage.cost_usd.or_else(|| {
                        let (i, o) = (resp.usage.input_tokens?, resp.usage.output_tokens?);
                        self.cfg
                            .price(&resolved, i, o)
                            .or_else(|| self.cfg.price(model, i, o))
                    });
                    record.model = resolved.clone();
                    record.input_tokens = resp.usage.input_tokens;
                    record.output_tokens = resp.usage.output_tokens;
                    record.cost_usd = cost;
                    record.status = "ok".into();
                    record.raw = resp.raw.clone();
                    self.log.lock().unwrap().record(&record)?;
                    *self.spent.lock().unwrap() += cost.unwrap_or(0.0);
                    if resp.quota.is_some() {
                        *self.quota.lock().unwrap() = resp.quota.clone();
                    }
                    return Ok(CallOutcome {
                        response: resp,
                        backend: backend.to_string(),
                        model: resolved,
                        cost_usd: cost,
                    });
                }
                Err(err) => {
                    record.status = "error".into();
                    record.error = Some(err.to_string());
                    self.log.lock().unwrap().record(&record)?;
                    match err {
                        BackendError::InvalidOutput(_) if !retried_invalid => {
                            retried_invalid = true;
                        }
                        BackendError::RateLimited {
                            resets_at: Some(at),
                        } if !waited => {
                            let now = (now_ms() / 1000) as u64;
                            let wait = at.saturating_sub(now);
                            if wait > self.cfg.limits.rate_limit_max_wait_secs {
                                return Err(CoreError::Backend {
                                    backend: backend.to_string(),
                                    error: err,
                                });
                            }
                            waited = true;
                            tokio::time::sleep(Duration::from_secs(wait)).await;
                        }
                        other => {
                            return Err(CoreError::Backend {
                                backend: backend.to_string(),
                                error: other,
                            });
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mwa_backends::Probe;
    use mwa_protocol::Usage;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type Scripted = Result<Response, BackendError>;

    #[derive(Clone, Default)]
    struct Counters {
        calls: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
    }

    struct Mock {
        script: Mutex<VecDeque<Scripted>>,
        delay: Duration,
        c: Counters,
    }

    fn ok(cost: Option<f64>, tokens: Option<(u64, u64)>) -> Scripted {
        Ok(Response {
            text: "ok".into(),
            json: None,
            model: Some("m-resolved".into()),
            usage: Usage {
                input_tokens: tokens.map(|t| t.0),
                output_tokens: tokens.map(|t| t.1),
                cost_usd: cost,
            },
            raw: "raw".into(),
            quota: None,
        })
    }

    fn mock(script: Vec<Scripted>, delay_ms: u64) -> (Box<dyn Backend>, Counters) {
        let c = Counters::default();
        let m = Mock {
            script: Mutex::new(script.into()),
            delay: Duration::from_millis(delay_ms),
            c: c.clone(),
        };
        (Box::new(m), c)
    }

    #[async_trait]
    impl Backend for Mock {
        fn kind(&self) -> &'static str {
            "mock"
        }
        async fn complete(&self, _req: &Request) -> Result<Response, BackendError> {
            self.c.calls.fetch_add(1, Ordering::SeqCst);
            let now = self.c.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.c.max_active.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            self.c.active.fetch_sub(1, Ordering::SeqCst);
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| ok(Some(0.0), None))
        }
        async fn probe(&self) -> Probe {
            Probe {
                available: true,
                version: None,
                detail: String::new(),
            }
        }
    }

    const CFG: &str = r#"
[backends.a]
kind = "ollama"
max_concurrency = 2
[backends.b]
kind = "ollama"
[roles.worker]
backend = "a"
model = "m"
fallback = { backend = "b", model = "fb-model" }
[roles.writer]
backend = "a"
model = "m"
[budget]
max_usd_per_run = 1.0
[limits]
rate_limit_max_wait_secs = 2
[prices]
"m-resolved" = [1000000.0, 0.0]
"#;

    fn dispatcher(a: Box<dyn Backend>, b: Box<dyn Backend>) -> (Dispatcher, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, _) = Config::parse(CFG).unwrap();
        let log = RunLog::open(dir.path()).unwrap();
        let backends = BTreeMap::from([("a".to_string(), a), ("b".to_string(), b)]);
        (Dispatcher::with_backends(cfg, log, backends), dir)
    }

    async fn call(d: &Dispatcher, role: Role) -> Result<CallOutcome, CoreError> {
        d.call(role, "s", "p", &[], None).await
    }

    fn rows(d: &Dispatcher) -> i64 {
        d.log.lock().unwrap().count_for_run(d.run_id()).unwrap()
    }

    #[tokio::test]
    async fn success_is_logged_and_costed() {
        let (a, _) = mock(vec![ok(Some(0.25), None)], 0);
        let (b, _) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        let out = call(&d, Role::Worker).await.unwrap();
        assert_eq!(out.backend, "a");
        assert_eq!(out.model, "m-resolved");
        assert_eq!(d.spent_usd(), 0.25);
        assert_eq!(rows(&d), 1);
    }

    #[tokio::test]
    async fn price_table_fills_missing_cost() {
        // 1 input token at 1e6 USD/Mtok = 1.0 USD
        let (a, _) = mock(vec![ok(None, Some((1, 0)))], 0);
        let (b, _) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        assert_eq!(call(&d, Role::Worker).await.unwrap().cost_usd, Some(1.0));
    }

    #[tokio::test]
    async fn budget_stops_before_next_call() {
        let (a, ca) = mock(vec![ok(Some(1.0), None)], 0);
        let (b, _) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        call(&d, Role::Worker).await.unwrap();
        let err = call(&d, Role::Worker).await.unwrap_err();
        assert!(matches!(err, CoreError::BudgetExceeded { .. }), "{err:?}");
        assert_eq!(ca.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn invalid_output_retried_once() {
        let bad = || Err(BackendError::InvalidOutput("x".into()));
        let (a, ca) = mock(vec![bad(), ok(Some(0.0), None)], 0);
        let (b, _) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        assert!(call(&d, Role::Worker).await.is_ok());
        assert_eq!(ca.calls.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&d), 2, "failed attempt is logged too");

        let (a, ca) = mock(vec![bad(), bad()], 0);
        let (b, _) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        assert!(call(&d, Role::Worker).await.is_err());
        assert_eq!(ca.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn far_rate_limit_falls_back() {
        let far = Err(BackendError::RateLimited {
            resets_at: Some(u64::MAX / 2),
        });
        let (a, _) = mock(vec![far], 0);
        let (b, cb) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        let out = call(&d, Role::Worker).await.unwrap();
        assert_eq!(out.backend, "b");
        assert_eq!(cb.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rate_limit_without_fallback_is_error() {
        let far = Err(BackendError::RateLimited { resets_at: None });
        let (a, _) = mock(vec![far], 0);
        let (b, _) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        let err = call(&d, Role::Writer).await.unwrap_err();
        assert!(matches!(
            err,
            CoreError::Backend {
                error: BackendError::RateLimited { .. },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn near_rate_limit_waits_then_retries_same_backend() {
        let soon = (now_ms() / 1000) as u64 + 1;
        let (a, ca) = mock(
            vec![Err(BackendError::RateLimited {
                resets_at: Some(soon),
            })],
            0,
        );
        let (b, cb) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        let out = call(&d, Role::Worker).await.unwrap();
        assert_eq!(out.backend, "a");
        assert_eq!(ca.calls.load(Ordering::SeqCst), 2);
        assert_eq!(cb.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrency_is_capped_per_backend() {
        let (a, ca) = mock(vec![], 50);
        let (b, _) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        let d = Arc::new(d);
        let tasks: Vec<_> = (0..6)
            .map(|_| {
                let d = d.clone();
                tokio::spawn(async move { call(&d, Role::Worker).await.unwrap() })
            })
            .collect();
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(ca.calls.load(Ordering::SeqCst), 6);
        assert_eq!(ca.max_active.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn streaming_call_delivers_text_and_keeps_quota() {
        let mut r = ok(Some(0.0), None).unwrap();
        r.quota = Some(Quota {
            window: "seven_day".into(),
            status: "allowed".into(),
            utilization: Some(0.1),
            resets_at: None,
        });
        let (a, _) = mock(vec![Ok(r)], 0);
        let (b, _) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        let got = Mutex::new(String::new());
        let cb = |t: &str| got.lock().unwrap().push_str(t);
        d.call_streaming(Role::Worker, "s", "p", &cb).await.unwrap();
        assert_eq!(
            *got.lock().unwrap(),
            "ok",
            "default streaming delivers full text"
        );
        assert_eq!(d.last_quota().unwrap().utilization, Some(0.1));
        assert_eq!(d.role_model(Role::Worker), Some(("a".into(), "m".into())));
        d.set_role_model(Role::Worker, "m2").unwrap();
        assert_eq!(d.role_model(Role::Worker), Some(("a".into(), "m2".into())));
        assert!(
            d.set_role_model(Role::Verifier, "x").is_err(),
            "unconfigured role"
        );
    }

    #[tokio::test]
    async fn unconfigured_role_is_error() {
        let (a, _) = mock(vec![], 0);
        let (b, _) = mock(vec![], 0);
        let (d, _dir) = dispatcher(a, b);
        assert!(matches!(
            call(&d, Role::Verifier).await,
            Err(CoreError::RoleNotConfigured(Role::Verifier))
        ));
    }
}
