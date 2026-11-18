//! Connects the TUI to workspace config, the dispatcher and the product map.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use mwa_core::Dispatcher;
use mwa_host::{Header, Host, OnDelta, SourceBadge, Status, Who};
use mwa_productmap::map::ProductMap;
use mwa_protocol::{Config, Role};

/// `{product}` is replaced with `[product] name` (or a neutral phrase).
const CHAT_SYSTEM: &str = "당신은 mwa(Manual Write Anything) 안의 도우미입니다. mwa는 {product}의 \
한국어 사용자 매뉴얼을 코드와 실제 화면에서 만들고 갱신하는 도구입니다. 합니다체로 간결하게 답하세요. \
아래 제품 지도 정보에 근거해 답하고, 근거가 없는 내용은 모른다고 말하세요. \
아직 구현되지 않은 기능(대화로 매뉴얼 편집, PDF·Confluence 내보내기, 자동 갱신)을 요청받으면 아직 구현 전이라고 알려 주세요. \
지금 쓸 수 있는 명령은 /map(제품 지도 생성), /shot(메뉴 화면 촬영), /gen(선택한 기능의 매뉴얼 생성), \
/doctor(점검), /auth(로그인 프로파일), /help 입니다.";

pub struct CliHost {
    ws: PathBuf,
    cfg: Config,
    dispatcher: Arc<Dispatcher>,
    /// One capture at a time: each one logs in, and parallel runs would double the load.
    capturing: std::sync::atomic::AtomicBool,
    /// One manual generation at a time (model quota and cost).
    generating: std::sync::atomic::AtomicBool,
}

impl CliHost {
    pub fn open(ws: &Path) -> Result<Self> {
        let (cfg, _warnings) = crate::load_config_quiet(ws)?;
        let dispatcher = Arc::new(crate::open_dispatcher(ws, cfg.clone())?);
        Ok(Self {
            ws: ws.to_path_buf(),
            cfg,
            dispatcher,
            capturing: std::sync::atomic::AtomicBool::new(false),
            generating: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn map_overview(&self) -> Option<String> {
        let map = self.load_map()?;
        let mut out = format!("제품 지도: 기능 {}개\n", map.features.len());
        for f in &map.features {
            out.push_str(&format!(
                "- {} > {} ({})\n",
                f.group.as_deref().unwrap_or(&f.license_menu),
                f.label,
                f.href
            ));
        }
        Some(out)
    }
}

#[async_trait]
impl Host for CliHost {
    fn header(&self) -> Header {
        let ws = std::fs::canonicalize(&self.ws).unwrap_or_else(|_| self.ws.clone());
        Header {
            workspace: ws.display().to_string(),
            sources: crate::snapshots(&self.cfg)
                .into_iter()
                .filter_map(|(name, s)| {
                    let s = s.ok()?;
                    Some(SourceBadge {
                        name,
                        preview: s.is_preview(),
                        sha: s.sha,
                    })
                })
                .collect(),
        }
    }

    fn load_map(&self) -> Option<ProductMap> {
        let text = std::fs::read_to_string(self.ws.join("map/product-map.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn status(&self) -> Status {
        let model = self
            .dispatcher
            .role_model(Role::Chat)
            .map(|(b, m)| format!("{b}·{m}"))
            .unwrap_or_else(|| "chat 역할 없음".into());
        Status {
            model,
            spent_usd: self.dispatcher.spent_usd(),
            budget_usd: self.dispatcher.budget_usd(),
            quota: self.dispatcher.last_quota(),
        }
    }

    async fn chat(
        &self,
        transcript: Vec<(Who, String)>,
        context: Option<String>,
        on_delta: OnDelta,
    ) -> Result<(), String> {
        let product = self
            .cfg
            .product
            .as_ref()
            .and_then(|p| p.name.clone())
            .unwrap_or_else(|| "연결된 제품".into());
        let mut system = CHAT_SYSTEM.replace("{product}", &product);
        if let Some(o) = self.map_overview() {
            system.push_str("\n\n");
            system.push_str(&o);
        }
        if let Some(c) = context {
            system.push_str("\n\n");
            system.push_str(&c);
        }
        let mut prompt = String::from("[대화]\n");
        for (who, text) in &transcript {
            let label = if *who == Who::User {
                "사용자"
            } else {
                "도우미"
            };
            prompt.push_str(&format!("{label}: {text}\n"));
        }
        prompt.push_str("\n위 대화의 마지막 사용자 메시지에 답하세요.");
        self.dispatcher
            .call_streaming(Role::Chat, &system, &prompt, &*on_delta)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn build_map(&self) -> Result<(ProductMap, Vec<String>), String> {
        let (map, warnings, path) = crate::build_map(&self.ws, &self.cfg, None, false)
            .await
            .map_err(|e| format!("{e:#}"))?;
        let mut lines: Vec<String> = warnings.into_iter().map(|w| format!("경고: {w}")).collect();
        lines.extend(crate::map_summary_lines(&map));
        lines.push(format!("저장: {}", path.display()));
        Ok((map, lines))
    }

    async fn sync_sources(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        let cfg = self.cfg.clone();
        tokio::task::spawn_blocking(move || crate::sync_sources(&cfg, |l| progress(&l)))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("{e:#}"))
    }

    async fn doctor(&self) -> Vec<String> {
        match crate::doctor_report(&self.ws, self.cfg.clone(), false).await {
            Ok((lines, _)) => lines,
            Err(e) => vec![format!("점검 실패: {e:#}")],
        }
    }

    fn chat_models(&self) -> Vec<String> {
        let Some((backend, current)) = self.dispatcher.role_model(Role::Chat) else {
            return Vec::new();
        };
        match self.cfg.backends.get(&backend) {
            // Aliases accepted by `claude --model` (docs/cli-flags.md).
            Some(mwa_protocol::BackendCfg::ClaudeCode { .. }) => {
                ["haiku", "sonnet", "opus", "fable"]
                    .map(String::from)
                    .to_vec()
            }
            _ => vec![current],
        }
    }

    fn set_chat_model(&self, model: &str) -> Result<(), String> {
        if !self.chat_models().iter().any(|m| m == model) {
            return Err(format!("쓸 수 없는 모델입니다: {model}"));
        }
        self.dispatcher
            .set_role_model(Role::Chat, model)
            .map_err(|e| e.to_string())
    }

    async fn capture_menus(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        use std::sync::atomic::Ordering;
        if self.capturing.swap(true, Ordering::SeqCst) {
            return Err("이미 촬영 중입니다".into());
        }
        let result =
            crate::capture_menus(&self.ws, &self.cfg, 2, &[], false, |line| progress(&line)).await;
        self.capturing.store(false, Ordering::SeqCst);
        result.map(|m| m.summary()).map_err(|e| format!("{e:#}"))
    }

    async fn generate_manual(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        use std::sync::atomic::Ordering;
        if self.generating.swap(true, Ordering::SeqCst) {
            return Err("이미 매뉴얼을 만들고 있습니다".into());
        }
        let compare = self.cfg.manual.compare.get(feature).and_then(|p| {
            Some((
                p.rsplit('/').next()?.to_string(),
                std::fs::read_to_string(p).ok()?,
            ))
        });
        let result = crate::generate_manual(&self.ws, &self.cfg, feature, None, compare, |l| {
            progress(&l)
        })
        .await;
        self.generating.store(false, Ordering::SeqCst);
        result
            .map(|run| mwa_manual::pipeline::summary(&run))
            .map_err(|e| format!("{e:#}"))
    }

    fn manual_run(&self, feature: &str) -> Option<serde_json::Value> {
        serde_json::to_value(mwa_manual::pipeline::load_run(&self.ws, feature)?).ok()
    }

    fn shots_manifest(&self) -> Option<serde_json::Value> {
        serde_json::to_value(mwa_capture::menus::load_manifest(&self.ws)?).ok()
    }

    fn shot_file(&self, rel: &str) -> Option<PathBuf> {
        mwa_capture::menus::shot_file(&self.ws, rel)
    }

    fn busy(&self) -> bool {
        self.capturing.load(std::sync::atomic::Ordering::SeqCst)
            || self.generating.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn auth_lines(&self) -> Vec<String> {
        match mwa_capture::creds::CredStore::default_path()
            .and_then(|p| mwa_capture::creds::CredStore::open(&p))
        {
            Ok(store) => crate::auth_list_lines(&self.cfg, &store),
            Err(e) => vec![format!("자격 파일 오류: {e}")],
        }
    }
}
