//! Runs the Playwright sidecar (`sidecar/capture/index.mjs`): one job JSON in, one event per shot out.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub base_url: String,
    pub cookie: Cookie,
    pub domain: Option<String>,
    pub viewport: [u32; 2],
    pub scale: f64,
    /// Page clock (Unix ms); the same for every run so relative times match.
    pub fixed_time: u64,
    /// Non-GET `/api/*` requests allowed through on a shared server: session keep-alive only
    /// (adapter `[auth] keepalive_paths`). Logout is never allowed so a stray click cannot end
    /// our session.
    pub allow_non_get: Vec<String>,
    /// `sessionStorage` key that selects `domain` (adapter `[ui] domain_storage_key`).
    pub domain_storage_key: Option<String>,
    /// Toast notices to record (adapter `[ui] toast_selector`, `toast_type_attr`).
    pub toast_selector: Option<String>,
    pub toast_type_attr: Option<String>,
    /// Accessible names of buttons to mask (e.g. the header user menu).
    pub mask_buttons: Vec<String>,
    pub settle_ms: u64,
    pub timeout_ms: u64,
    /// Demo responses served instead of the server (D-028). Never forwarded.
    pub mocks: Vec<Mock>,
    pub shots: Vec<JobShot>,
}

/// One demo response. `path` is an `/api/...` template with `{}` segments; empty `params`
/// matches any values, and every `query` entry must match the request's query string.
#[derive(Debug, Clone, Serialize)]
pub struct Mock {
    pub method: String,
    pub path: String,
    pub params: Vec<String>,
    pub query: std::collections::BTreeMap<String, String>,
    pub body: serde_json::Value,
}

#[derive(Clone, Serialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
}

impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cookie({}=<redacted>)", self.name)
    }
}

impl Cookie {
    /// From a `name=value` pair as returned by the login check.
    pub fn from_pair(pair: &str) -> Option<Self> {
        let (name, value) = pair.split_once('=')?;
        Some(Self {
            name: name.to_string(),
            value: value.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JobShot {
    pub id: String,
    /// Path on the product UI, e.g. `/sales/orders`.
    pub path: String,
    pub out: PathBuf,
    /// Interactions after the page settles (detail shots); empty for a plain page.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// `page` (viewport), `full` (whole page) or `dialog` (the open dialog only).
    pub capture: String,
    /// Cut the picture to the smallest element holding all these texts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub region: Vec<String>,
    /// Widen the cut to this width / height ratio.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    /// Return the accessibility snapshot of the final screen.
    pub aria: bool,
}

/// One planned interaction, located by accessible role and name (F6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// `click`, `fill`, `select`, `hover` or `wait`.
    pub action: String,
    pub role: String,
    pub name: String,
    #[serde(default)]
    pub exact: bool,
    /// `dialog` to search only inside the open dialog.
    #[serde(default)]
    pub within: String,
    /// Text of the table row to search in.
    #[serde(default)]
    pub row: String,
    /// Text inside the smallest block (card, section) to search in.
    #[serde(default)]
    pub near: String,
    /// Value for `fill`/`select`.
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepLog {
    pub ok: bool,
    pub ms: u64,
    /// Plain-language reason when the step failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiSample {
    pub path: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blocked {
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotEvent {
    pub id: String,
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub blocked: Vec<Blocked>,
    #[serde(default)]
    pub console_errors: Vec<String>,
    #[serde(default)]
    pub final_path: Option<String>,
    #[serde(default)]
    pub idle_timed_out: bool,
    /// Toast notices visible when the picture was taken (`type: text`).
    #[serde(default)]
    pub toasts: Vec<String>,
    /// Requests answered from demo data (`METHOD /path`).
    #[serde(default)]
    pub mocked: Vec<String>,
    /// Accessibility snapshot (YAML) when requested or when a step failed.
    #[serde(default)]
    pub aria: Option<String>,
    /// Index of the step that failed.
    #[serde(default)]
    pub failed_step: Option<usize>,
    /// One entry per executed step (a failed step is the last one).
    #[serde(default)]
    pub steps_log: Vec<StepLog>,
    /// Screenshot taken when a step failed.
    #[serde(default)]
    pub fail_shot: Option<String>,
    /// The `region` texts were not found together; the picture is uncut.
    #[serde(default)]
    pub region_missed: bool,
    /// GET `/api/*` JSON bodies seen while probing (truncated).
    #[serde(default)]
    pub api_data: Vec<ApiSample>,
    #[serde(default)]
    pub ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ShootError {
    #[error("촬영 사이드카를 실행할 수 없습니다 ({0}): {1}")]
    Spawn(String, std::io::Error),
    #[error("촬영 사이드카 오류: {0}")]
    Fatal(String),
    #[error("촬영 사이드카 입출력 오류: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "여유 메모리가 {available}MB로 기준 {limit}MB보다 적어 촬영을 멈췄습니다 (공유 서버 보호)"
    )]
    LowMemory { available: u64, limit: u64 },
}

const SCRIPT: &str = include_str!("../../../sidecar/capture/index.mjs");
const PACKAGE: &str = include_str!("../../../sidecar/capture/package.json");

/// Default sidecar script: `$MWA_SIDECAR`, else the repository copy (development), else the copy
/// built into this binary, unpacked into mwa's data folder (`<data>/capture/`) so an installed
/// `mwa` works on any computer. Its Playwright package is installed there by "브라우저 설치".
pub fn default_script() -> PathBuf {
    if let Some(p) = std::env::var_os("MWA_SIDECAR") {
        return PathBuf::from(p);
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecar/capture/index.mjs");
    if repo.is_file() {
        return repo;
    }
    unpack(&data_dir().join("capture"))
}

/// Writes the built-in script and its package.json into `dir` (only when they differ, so an
/// upgraded mwa refreshes them) and returns the script path.
fn unpack(dir: &Path) -> PathBuf {
    let stale = |p: &Path, want: &str| std::fs::read_to_string(p).map_or(true, |t| t != want);
    let _ = std::fs::create_dir_all(dir);
    let script = dir.join("index.mjs");
    if stale(&script, SCRIPT) {
        let _ = std::fs::write(&script, SCRIPT);
    }
    let package = dir.join("package.json");
    if stale(&package, PACKAGE) {
        let _ = std::fs::write(&package, PACKAGE);
    }
    script
}

/// mwa's data folder: `$MWA_HOME`, else `~/Library/Application Support/mwa` (macOS), else
/// `$XDG_DATA_HOME/mwa` or `~/.local/share/mwa`.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("MWA_HOME") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        return home.join("Library/Application Support/mwa");
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("mwa")
}

pub async fn run_sidecar(
    node: &str,
    script: &Path,
    job: &Job,
    guard: Option<crate::mem::MemGuard>,
    mut on_shot: impl FnMut(&ShotEvent),
) -> Result<Vec<ShotEvent>, ShootError> {
    let mut child = Command::new(node)
        .arg(script)
        .current_dir(script.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| ShootError::Spawn(format!("{node} {}", script.display()), e))?;
    let mut stdin = child.stdin.take().expect("piped");
    stdin
        .write_all(
            serde_json::to_string(job)
                .expect("job serializes")
                .as_bytes(),
        )
        .await?;
    drop(stdin);
    let mut stderr = child.stderr.take().expect("piped");
    let stderr_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s).await;
        s
    });

    let mut events = Vec::new();
    let mut fatal = None;
    let mut done = false;
    let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match v["type"].as_str() {
            Some("shot") => {
                if let Ok(ev) = serde_json::from_value::<ShotEvent>(v) {
                    on_shot(&ev);
                    events.push(ev);
                }
                // Checked between shots; dropping `child` (kill_on_drop) stops the browser.
                if let Some(g) = guard
                    && let Some(available) = g.short()
                {
                    let _ = child.kill().await;
                    return Err(ShootError::LowMemory {
                        available,
                        limit: g.abort_below_mb,
                    });
                }
            }
            Some("fatal") => fatal = v["error"].as_str().map(String::from),
            Some("done") => done = true,
            _ => {}
        }
    }
    let status = child.wait().await?;
    let stderr = stderr_task.await.unwrap_or_default();
    if let Some(e) = fatal {
        return Err(ShootError::Fatal(e));
    }
    if !done || !status.success() {
        let tail: String = stderr.lines().rev().take(5).collect::<Vec<_>>().join(" / ");
        return Err(ShootError::Fatal(format!(
            "비정상 종료 ({:?}): {tail}",
            status.code()
        )));
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_script_unpacks() {
        let dir = tempfile::tempdir().unwrap();
        let script = unpack(&dir.path().join("capture"));
        assert_eq!(std::fs::read_to_string(&script).unwrap(), SCRIPT);
        assert!(
            std::fs::read_to_string(dir.path().join("capture/package.json"))
                .unwrap()
                .contains("playwright")
        );
        std::fs::write(&script, "old").unwrap();
        unpack(&dir.path().join("capture"));
        assert_eq!(
            std::fs::read_to_string(&script).unwrap(),
            SCRIPT,
            "refreshed after an upgrade"
        );
    }

    fn job() -> Job {
        Job {
            base_url: "http://h:3001".into(),
            cookie: Cookie::from_pair("acme_session=abc").unwrap(),
            domain: Some("tenant-a".into()),
            viewport: [1440, 900],
            scale: 2.0,
            fixed_time: 1,
            allow_non_get: vec!["/api/auth/session/touch".into()],
            domain_storage_key: Some("acme:tenant".into()),
            toast_selector: None,
            toast_type_attr: None,
            mask_buttons: vec!["사용자 메뉴".into()],
            settle_ms: 800,
            timeout_ms: 20000,
            mocks: vec![],
            shots: vec![],
        }
    }

    /// `/bin/sh <script>` stands in for `node index.mjs`.
    async fn run_fake(body: &str) -> (Result<Vec<ShotEvent>, ShootError>, String) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake.sh");
        let stdin_copy = dir.path().join("stdin.json");
        std::fs::write(
            &script,
            format!("cat > '{}'\n{body}\n", stdin_copy.display()),
        )
        .unwrap();
        let mut seen = 0;
        let r = run_sidecar("/bin/sh", &script, &job(), None, |_| seen += 1).await;
        let sent = std::fs::read_to_string(&stdin_copy).unwrap_or_default();
        (r, sent)
    }

    #[tokio::test]
    async fn parses_events_and_sends_camel_case_job() {
        let (r, sent) = run_fake(
            r#"echo '{"type":"shot","id":"a","ok":true,"blocked":[{"method":"POST","url":"/api/x"}],"consoleErrors":[],"ms":12}'
echo 'noise'
echo '{"type":"shot","id":"b","ok":false,"error":"로그인 화면으로 이동됨"}'
echo '{"type":"done"}'"#,
        )
        .await;
        let evs = r.unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].blocked[0].method, "POST");
        assert!(!evs[1].ok);
        let v: serde_json::Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(v["baseUrl"], "http://h:3001");
        assert_eq!(v["allowNonGet"][0], "/api/auth/session/touch");
        assert_eq!(v["cookie"]["name"], "acme_session");
        assert_eq!(v["domainStorageKey"], "acme:tenant");
    }

    #[tokio::test]
    async fn fatal_and_abnormal_exit_are_errors() {
        let (r, _) = run_fake(r#"echo '{"type":"fatal","error":"browser crashed"}'; exit 1"#).await;
        assert!(matches!(r, Err(ShootError::Fatal(ref m)) if m.contains("browser crashed")));
        let (r, _) = run_fake(r#"echo 'boom' >&2; exit 3"#).await;
        assert!(matches!(r, Err(ShootError::Fatal(ref m)) if m.contains("boom")));
    }

    #[tokio::test]
    async fn low_memory_stops_between_shots() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake.sh");
        std::fs::write(
            &script,
            "cat > /dev/null
echo '{\"type\":\"shot\",\"id\":\"a\",\"ok\":true}'
sleep 5
echo '{\"type\":\"done\"}'
",
        )
        .unwrap();
        let guard = crate::mem::MemGuard {
            abort_below_mb: 1000,
            probe: || Some(300),
        };
        let start = std::time::Instant::now();
        let r = run_sidecar("/bin/sh", &script, &job(), Some(guard), |_| {}).await;
        assert!(
            matches!(
                r,
                Err(ShootError::LowMemory {
                    available: 300,
                    limit: 1000
                })
            ),
            "{r:?}"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(4),
            "child killed promptly"
        );
    }

    #[test]
    fn cookie_debug_is_redacted() {
        assert!(
            !format!("{:?}", Cookie::from_pair("acme_session=secret").unwrap()).contains("secret")
        );
    }
}
