//! 자동 작업 (D-050): rules that keep manuals up to date on a schedule. A rule says when, which
//! features (those whose code changed, or chosen ones), what to do (read the code and propose
//! changes, or write again; re-shoot pictures) and how far (stop for review, or accept and
//! publish). Rules live in the project (`automation.json`); runs in `.mwa/automation-runs.json`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const RULES_FILE: &str = "automation.json";
const RUNS_FILE: &str = ".mwa/automation-runs.json";
const KEEP_RUNS: usize = 50;

/// The engine's port while `mwa serve` runs: publishing opens its own page.
pub static SERVE_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
/// Where a run says what it is doing.
pub type Progress = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// One run at a time, scheduled or started by hand.
static RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// `daily`, `weekly` or `hours`.
    pub every: String,
    /// Local time `HH:MM` for daily and weekly.
    #[serde(default = "nine")]
    pub at: String,
    /// 1 = Monday … 7 = Sunday, for weekly.
    #[serde(default = "one")]
    pub weekday: u32,
    /// For `hours`.
    #[serde(default = "one")]
    pub hours: u32,
    /// `changed` (features whose code changed) or `features` (the chosen ones).
    pub scope: String,
    #[serde(default)]
    pub features: Vec<String>,
    /// `propose` (read the code, propose changes) or `rewrite` (write the manual again).
    pub action: String,
    /// Re-shoot the pictures too.
    #[serde(default)]
    pub reshoot: bool,
    /// `review` (stop for a person) or `publish` (accept and publish to Confluence).
    pub until: String,
    /// Last start (epoch seconds); the schedule counts from it.
    #[serde(default)]
    pub last_run: Option<u64>,
    #[serde(default)]
    pub created_at: u64,
}

fn nine() -> String {
    "09:00".into()
}
fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rules {
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub rule: String,
    pub name: String,
    pub started: u64,
    #[serde(default)]
    pub finished: Option<u64>,
    /// `running`, `ok`, `none` (nothing to do) or `failed`.
    pub status: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub lines: Vec<String>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn load(ws: &Path) -> Rules {
    std::fs::read_to_string(ws.join(RULES_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Saves the rules a person edited; a new rule's schedule starts now (no run right away).
pub fn save(ws: &Path, mut rules: Rules) -> Result<Rules> {
    let old = load(ws);
    let t = now();
    for r in &mut rules.rules {
        if r.id.trim().is_empty() {
            r.id = format!("r{t}{}", rules_len_hint(&old));
        }
        if r.created_at == 0 {
            r.created_at = t;
        }
        // The last run belongs to the engine, not to the form.
        r.last_run = old
            .rules
            .iter()
            .find(|o| o.id == r.id)
            .and_then(|o| o.last_run)
            .or(r.last_run);
        anyhow::ensure!(
            ["daily", "weekly", "hours"].contains(&r.every.as_str()),
            "주기가 올바르지 않습니다"
        );
        anyhow::ensure!(parse_hm(&r.at).is_some(), "시각은 09:00처럼 적습니다");
        anyhow::ensure!(
            r.scope != "features" || !r.features.is_empty(),
            "'{}': 고른 메뉴가 없습니다",
            r.name
        );
    }
    write_json(&ws.join(RULES_FILE), &rules)?;
    Ok(rules)
}

fn rules_len_hint(r: &Rules) -> usize {
    r.rules.len()
}

pub fn runs(ws: &Path) -> Vec<Run> {
    std::fs::read_to_string(ws.join(RUNS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_json(path: &Path, v: &impl Serialize) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(v)? + "\n")?;
    Ok(())
}

fn record(ws: &Path, run: &Run) -> Result<()> {
    let mut all = runs(ws);
    match all
        .iter_mut()
        .find(|r| r.rule == run.rule && r.started == run.started)
    {
        Some(r) => *r = run.clone(),
        None => all.insert(0, run.clone()),
    }
    all.truncate(KEEP_RUNS);
    write_json(&ws.join(RUNS_FILE), &all)
}

fn parse_hm(s: &str) -> Option<(u64, u64)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u64, u64) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// This machine's offset from UTC in seconds (`date +%z`), 0 when unknown.
fn utc_offset() -> i64 {
    std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let sign = if s.starts_with('-') { -1 } else { 1 };
            let d = s.trim_start_matches(['+', '-']);
            let (h, m): (i64, i64) = (d.get(..2)?.parse().ok()?, d.get(2..4)?.parse().ok()?);
            Some(sign * (h * 3600 + m * 60))
        })
        .unwrap_or(0)
}

/// When the rule runs next (epoch seconds), counting from its last run or creation.
pub fn next_run(r: &Rule, offset: i64) -> Option<u64> {
    if !r.enabled {
        return None;
    }
    let from = r.last_run.unwrap_or(r.created_at);
    if r.every == "hours" {
        return Some(from + u64::from(r.hours.max(1)) * 3600);
    }
    let (h, m) = parse_hm(&r.at)?;
    let local = |t: u64| t as i64 + offset;
    // The first matching local time after `from`.
    let day = 86_400i64;
    let start_day = local(from).div_euclid(day);
    (0..=8).find_map(|k| {
        let d = start_day + k;
        let t_local = d * day + (h * 3600 + m * 60) as i64;
        let t = (t_local - offset) as u64;
        // 1970-01-01 was a Thursday (4).
        let weekday = ((d + 3).rem_euclid(7) + 1) as u32;
        (t > from && (r.every == "daily" || weekday == r.weekday)).then_some(t)
    })
}

/// The rules with when each runs next, and the latest runs, for the screen.
pub fn view(ws: &Path) -> serde_json::Value {
    let offset = utc_offset();
    let rules = load(ws);
    serde_json::json!({
        "rules": rules.rules.iter().map(|r| {
            let mut v = serde_json::to_value(r).unwrap_or_default();
            v["next_run"] = serde_json::json!(next_run(r, offset));
            v
        }).collect::<Vec<_>>(),
        "runs": runs(ws),
        "running": RUNNING.load(Ordering::SeqCst),
    })
}

/// Features whose code changed since their facts were read: a file a fact quotes, or a screen
/// file, differs between then and now (git). Features never read are left out.
fn changed_features(ws: &Path, cfg: &mwa_protocol::Config) -> Result<Vec<String>> {
    let map: mwa_productmap::map::ProductMap = serde_json::from_str(
        &std::fs::read_to_string(ws.join("map/product-map.json"))
            .context("제품 지도가 없습니다")?,
    )?;
    let product = cfg.product.as_ref().context("제품 설정이 없습니다")?;
    let root = |name: &str| -> Option<PathBuf> {
        cfg.sources
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.path.clone())
    };
    let (front, back) = (root(&product.frontend), root(&product.backend));
    let head = |dir: &Path| -> Option<String> {
        let o = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let diff = |dir: &Path, from: &str, to: &str| -> Vec<String> {
        std::process::Command::new("git")
            .args(["diff", "--name-only", from, to])
            .current_dir(dir)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for f in &map.features {
        let Some(run) = mwa_manual::pipeline::load_run(ws, &f.key) else {
            continue;
        };
        let Some(then) = mwa_manual::versions::list(ws, &f.key)
            .last()
            .map(|m| m.sources.clone())
        else {
            continue;
        };
        let mut touched: Vec<String> = Vec::new();
        for (name, dir, repo) in [
            (&product.frontend, &front, mwa_manual::model::Repo::Frontend),
            (&product.backend, &back, mwa_manual::model::Repo::Backend),
        ] {
            let (Some(dir), Some(was)) = (dir, then.iter().find(|s| &s.name == name)) else {
                continue;
            };
            let Some(now) = head(dir) else { continue };
            if now.starts_with(&was.sha) || was.sha.starts_with(&now) {
                continue;
            }
            let changed = diff(dir, &was.sha, &now);
            let mut mine: Vec<String> = run
                .facts
                .iter()
                .filter_map(|x| x.source.as_ref())
                .filter(|s| s.repo == repo)
                .map(|s| s.file.clone())
                .collect();
            if repo == mwa_manual::model::Repo::Frontend {
                mine.extend(f.screens.iter().map(|s| s.file.clone()));
            }
            touched.extend(changed.into_iter().filter(|c| mine.contains(c)));
        }
        if !touched.is_empty() {
            out.push(f.key.clone());
        }
    }
    Ok(out)
}

/// Publishes a feature's saved document the way the editor's button does: the engine opens its
/// own page in a headless browser.
async fn publish(feature: &str, group: Option<&str>) -> Result<String> {
    let port = SERVE_PORT
        .get()
        .context("자동 게시는 mwa serve가 띄워져 있을 때만 됩니다")?;
    let mut url = format!("http://127.0.0.1:{port}/#autopublish={feature}");
    if let Some(g) = group {
        url.push_str(&format!(
            "&group={}",
            g.bytes()
                .map(|b| if b.is_ascii_alphanumeric() {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                })
                .collect::<String>()
        ));
    }
    let script = mwa_capture::shoot::default_script();
    let dir = script.parent().unwrap_or(Path::new(".")).to_path_buf();
    const JS: &str = "const { chromium } = require('playwright');\
(async () => { const b = await chromium.launch(); const p = await b.newPage();\
await p.goto(process.argv[1]);\
await p.waitForFunction(() => document.title.startsWith('mwa-'), null, { timeout: 300000 });\
console.log(await p.title()); await b.close(); })()\
.catch((e) => { console.log('mwa-error ' + e.message); process.exit(1); });";
    let out = tokio::process::Command::new("node")
        .args(["-e", JS, &url])
        .current_dir(&dir)
        .output()
        .await
        .context("node를 실행할 수 없습니다")?;
    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match said.strip_prefix("mwa-done ") {
        Some(link) => Ok(link.to_string()),
        None => anyhow::bail!("{}", said.strip_prefix("mwa-error ").unwrap_or(&said)),
    }
}

/// Runs one rule now. `progress` shows on the screen when a person started it.
pub async fn run(
    host: &crate::tui_host::CliHost,
    ws: &Path,
    id: &str,
    progress: Progress,
) -> Result<Vec<String>> {
    anyhow::ensure!(
        !RUNNING.swap(true, Ordering::SeqCst),
        "다른 자동 작업이 돌고 있습니다"
    );
    let result = async {
        let mut rules = load(ws);
        let rule = rules
            .rules
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .with_context(|| format!("없는 규칙입니다: {id}"))?;
        let started = now();
        if let Some(r) = rules.rules.iter_mut().find(|r| r.id == id) {
            r.last_run = Some(started);
        }
        write_json(&ws.join(RULES_FILE), &rules)?;
        let mut run = Run {
            rule: rule.id.clone(),
            name: rule.name.clone(),
            started,
            finished: None,
            status: "running".into(),
            features: Vec::new(),
            lines: Vec::new(),
        };
        record(ws, &run)?;
        let lines = steps(host, ws, &rule, &mut run, &progress).await;
        run.finished = Some(now());
        match lines {
            Ok(()) => {
                if run.status == "running" {
                    run.status = if run.features.is_empty() {
                        "none"
                    } else {
                        "ok"
                    }
                    .into();
                }
            }
            Err(e) => {
                run.status = "failed".into();
                run.lines.push(format!("실패: {e:#}"));
            }
        }
        record(ws, &run)?;
        Ok(run.lines)
    }
    .await;
    RUNNING.store(false, Ordering::SeqCst);
    result
}

async fn steps(
    host: &crate::tui_host::CliHost,
    ws: &Path,
    rule: &Rule,
    run: &mut Run,
    progress: &Progress,
) -> Result<()> {
    use mwa_host::Host;
    let relay = |p: &Progress| -> mwa_host::OnDelta {
        let p = p.clone();
        Box::new(move |t: &str| p(t))
    };
    let cfg = crate::load_config(ws)?;
    // The code as it is now.
    if cfg.sources.iter().any(|s| s.git.is_some()) {
        progress("Git 소스 받는 중");
        host.sync_sources(relay(progress))
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
    }
    progress("제품 지도 만드는 중");
    host.build_map().await.map_err(|e| anyhow::anyhow!(e))?;
    let targets = if rule.scope == "features" {
        rule.features.clone()
    } else {
        changed_features(ws, &cfg)?
    };
    if targets.is_empty() {
        run.lines.push("코드가 바뀐 기능이 없습니다".into());
        return Ok(());
    }
    run.features = targets.clone();
    let map = host.load_map();
    for (i, f) in targets.iter().enumerate() {
        let label = map
            .as_ref()
            .and_then(|m| m.features.iter().find(|x| &x.key == f))
            .map_or(f.clone(), |x| x.label.clone());
        let say = |t: &str| progress(&format!("{label} ({}/{}): {t}", i + 1, targets.len()));
        let mut note = |t: String| run.lines.push(format!("{label}: {t}"));
        say("코드 읽는 중");
        if let Err(e) = host.generate_manual(f, relay(progress)).await {
            note(format!("코드를 읽지 못했습니다: {e}"));
            continue;
        }
        let has_doc = mwa_manual::document::load(ws, f).is_some();
        if rule.action == "rewrite" || !has_doc {
            say("매뉴얼 쓰는 중");
            match host.make_document(f, relay(progress)).await {
                Ok(l) => note(l.last().cloned().unwrap_or_default()),
                Err(e) => {
                    note(format!("매뉴얼을 쓰지 못했습니다: {e}"));
                    continue;
                }
            }
        } else {
            say("바뀐 곳 제안 받는 중");
            match host.propose_document(f, relay(progress)).await {
                Ok(l) => note(l.last().cloned().unwrap_or_default()),
                Err(e) => {
                    note(format!("제안을 받지 못했습니다: {e}"));
                    continue;
                }
            }
        }
        if rule.reshoot && cfg.capture.is_some() {
            say("그림 다시 찍는 중");
            if mwa_manual::reference::for_feature(ws, f).is_some() {
                // Saved plans first (no model). Plan again when the code changed, a replay
                // failed, or a person gave a scene a new instruction.
                let full = rule.scope == "changed"
                    || host
                        .capture_reference(&format!("{f}?quick"), relay(progress))
                        .await
                        .is_err()
                    || mwa_manual::reference::load_scene_shots(ws, f).is_none_or(|s| {
                        s.scenes.iter().any(|x| {
                            (x.plan.is_none() && x.instruction.is_some())
                                || x.differences
                                    .first()
                                    .is_some_and(|d| d.starts_with("빠른 다시 찍기 실패"))
                        })
                    });
                if full {
                    if let Err(e) = host.capture_reference(f, relay(progress)).await {
                        note(format!("장면을 다시 찍지 못했습니다: {e}"));
                    }
                } else {
                    note("그림: 저장된 계획으로 다시 찍음 (모델 호출 없음)".into());
                }
            }
            let slots: Vec<String> = mwa_manual::document::load(ws, f)
                .map(|d| mwa_manual::document::picture_slots(&d.html))
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.bid)
                .collect();
            if !slots.is_empty()
                && let Err(e) = host
                    .capture_document(&format!("{f}|{}", slots.join(",")), relay(progress))
                    .await
            {
                note(format!("그림 자리를 찍지 못했습니다: {e}"));
            }
        }
        if rule.until == "publish" {
            if let Some(d) = mwa_manual::document::accept_proposals(ws, f)? {
                note(format!(
                    "제안을 모두 받아 버전 {}으로 저장했습니다",
                    d.version
                ));
            }
            if !cfg.extensions.confluence {
                note("게시: 컨플루언스 확장이 꺼져 있어 저장까지만 했습니다".into());
                continue;
            }
            say("컨플루언스에 게시하는 중");
            let group = map
                .as_ref()
                .and_then(|m| m.features.iter().find(|x| &x.key == f))
                .and_then(|x| x.group.clone());
            match publish(f, group.as_deref()).await {
                Ok(link) => note(format!("게시했습니다: {link}")),
                Err(e) => note(format!("게시하지 못했습니다: {e:#}")),
            }
        } else {
            note("검토 대기: ④에서 확인하세요".into());
        }
    }
    Ok(())
}

/// Runs every rule that is due (called by `mwa serve` once a minute).
pub async fn tick(host: &crate::tui_host::CliHost, ws: &Path) {
    let offset = utc_offset();
    let t = now();
    let due: Vec<String> = load(ws)
        .rules
        .iter()
        .filter(|r| next_run(r, offset).is_some_and(|n| n <= t))
        .map(|r| r.id.clone())
        .collect();
    for id in due {
        let _ = run(host, ws, &id, std::sync::Arc::new(|_: &str| {})).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(every: &str, at: &str, weekday: u32, from: u64) -> Rule {
        Rule {
            id: "a".into(),
            name: "매일 갱신".into(),
            enabled: true,
            every: every.into(),
            at: at.into(),
            weekday,
            hours: 6,
            scope: "changed".into(),
            features: Vec::new(),
            action: "propose".into(),
            reshoot: true,
            until: "review".into(),
            last_run: Some(from),
            created_at: 0,
        }
    }

    #[test]
    fn next_runs() {
        // 2026-11-19 (Thursday) 10:00 UTC.
        let t = 1_795_082_400;
        assert_eq!(t % 86_400, 36_000);
        // Daily 09:00 UTC: tomorrow; in UTC+9 09:00 is 00:00 UTC, also tomorrow.
        assert_eq!(
            next_run(&rule("daily", "09:00", 1, t), 0),
            Some(t + 23 * 3600)
        );
        assert_eq!(
            next_run(&rule("daily", "09:00", 1, t), 9 * 3600),
            Some(t + 14 * 3600)
        );
        // Weekly Monday 07:00 UTC: four days on.
        assert_eq!(
            next_run(&rule("weekly", "07:00", 1, t), 0),
            Some(t - 3 * 3600 + 4 * 86_400)
        );
        assert_eq!(
            next_run(&rule("hours", "09:00", 1, t), 0),
            Some(t + 6 * 3600)
        );
        let mut off = rule("daily", "09:00", 1, t);
        off.enabled = false;
        assert_eq!(next_run(&off, 0), None);
    }
}
