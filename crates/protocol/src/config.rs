use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use toml::Spanned;

/// Contents of `mwa.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, rename = "source")]
    pub sources: Vec<SourceCfg>,
    #[serde(default)]
    pub backends: BTreeMap<String, BackendCfg>,
    #[serde(default)]
    pub roles: BTreeMap<Role, RoleCfg>,
    #[serde(default)]
    pub budget: BudgetCfg,
    #[serde(default)]
    pub limits: LimitsCfg,
    /// USD per million tokens as `[input, output]`, keyed by model id or alias.
    /// Used when a backend reports tokens but no cost (HTTP APIs).
    #[serde(default)]
    pub prices: BTreeMap<String, [f64; 2]>,
    #[serde(default)]
    pub product: Option<ProductCfg>,
    /// Named login profiles for the product UI. Passwords live in the user's credential file.
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileCfg>,
    #[serde(default)]
    pub capture: Option<CaptureCfg>,
    #[serde(default)]
    pub manual: ManualCfg,
    /// Where manuals are published. The API token lives in the user's credential file.
    #[serde(default)]
    pub confluence: Option<ConfluenceCfg>,
    /// Optional add-ons outside the core work (version, update, write manuals).
    #[serde(default)]
    pub extensions: ExtensionsCfg,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionsCfg {
    /// Publish to Confluence Cloud from the editor.
    #[serde(default)]
    pub confluence: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfluenceCfg {
    /// `https://<site>.atlassian.net`
    pub base_url: String,
    /// Atlassian account e-mail (with the API token, the login for the REST API).
    pub email: String,
    /// Space key.
    pub space: String,
    /// Page id that feature pages go under.
    pub parent: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualCfg {
    /// Existing manual (Markdown) per feature key, compared after generation.
    #[serde(default)]
    pub compare: BTreeMap<String, String>,
    /// Sample manual (Markdown) to learn the format from when the project has no learned
    /// format yet (`template/schema.json`).
    #[serde(default)]
    pub template: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureCfg {
    /// Login profile used for screenshots; must have purpose = "capture".
    pub profile: String,
    /// Product domain (`?db=`) to show, e.g. "tenant-a".
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default = "default_viewport")]
    pub viewport: [u32; 2],
    /// Device pixel ratio of saved images.
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default = "default_node")]
    pub node: String,
    /// Do not start a capture with less available memory than this (MB).
    #[serde(default = "default_min_start_mb")]
    pub min_start_mb: u64,
    /// Stop a running capture when available memory drops below this (MB).
    #[serde(default = "default_abort_below_mb")]
    pub abort_below_mb: u64,
}

fn default_min_start_mb() -> u64 {
    2500
}
fn default_abort_below_mb() -> u64 {
    1000
}

fn default_viewport() -> [u32; 2] {
    [1440, 900]
}
fn default_scale() -> f64 {
    2.0
}
fn default_node() -> String {
    "node".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileCfg {
    /// Product UI origin, e.g. `http://localhost:3001`.
    pub base_url: String,
    pub user_id: String,
    #[serde(default)]
    pub purpose: ProfilePurpose,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfilePurpose {
    /// Day-to-day capture and QA.
    #[default]
    Capture,
    /// Account setup only (e.g. the seeded admin); never used for capture.
    Bootstrap,
}

/// Which product adapter reads which sources.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductCfg {
    /// Adapter file (product conventions), relative to the workspace, e.g. `adapter.toml`.
    pub adapter: String,
    /// Source names from `[[source]]`.
    pub frontend: String,
    pub backend: String,
    /// OpenAPI document URL or file path. Optional; without it backend operations are not matched.
    #[serde(default)]
    pub openapi: Option<String>,
    /// Product name shown to the chat model and in manuals, e.g. "Acme Console".
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCfg {
    pub name: String,
    /// Local checkout. Relative paths are relative to the project folder. With `git` and no
    /// path, the clone lives in `.mwa/sources/<name>`.
    #[serde(default)]
    pub path: PathBuf,
    /// Repository to clone and update (`mwa sources sync`), e.g. `git@github.com:acme/web.git`.
    #[serde(default)]
    pub git: Option<String>,
    /// Branch to follow (default: the remote's default branch).
    #[serde(default)]
    pub branch: Option<String>,
    /// Code on another machine over SSH (D-054): `user@host`. The copy lives in
    /// `.mwa/sources/<name>` unless `path` says otherwise.
    #[serde(default)]
    pub ssh: Option<String>,
    /// Jump hosts on the way, in order (`ssh -J`).
    #[serde(default)]
    pub jump: Vec<String>,
    /// Container on that host holding the code (`docker exec`).
    #[serde(default)]
    pub docker: Option<String>,
    /// The code's folder on the host, or inside the container.
    #[serde(default)]
    pub remote_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BackendCfg {
    #[serde(rename = "cli:claude-code")]
    ClaudeCode {
        #[serde(default = "default_claude_command")]
        command: String,
        #[serde(default = "default_concurrency")]
        max_concurrency: usize,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
    #[serde(rename = "api:anthropic")]
    Anthropic {
        #[serde(default = "default_anthropic_url")]
        base_url: String,
        /// Name of the environment variable holding the key; the key itself never goes in the file.
        api_key_env: String,
        #[serde(default = "default_max_tokens")]
        max_tokens: u32,
        #[serde(default = "default_concurrency")]
        max_concurrency: usize,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
    #[serde(rename = "api:openai-compat")]
    OpenAiCompat {
        base_url: String,
        #[serde(default)]
        api_key_env: Option<String>,
        #[serde(default = "default_concurrency")]
        max_concurrency: usize,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
    #[serde(rename = "ollama")]
    Ollama {
        #[serde(default = "default_ollama_url")]
        base_url: String,
        #[serde(default = "default_concurrency")]
        max_concurrency: usize,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
}

impl BackendCfg {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ClaudeCode { .. } => "cli:claude-code",
            Self::Anthropic { .. } => "api:anthropic",
            Self::OpenAiCompat { .. } => "api:openai-compat",
            Self::Ollama { .. } => "ollama",
        }
    }

    pub fn max_concurrency(&self) -> usize {
        match self {
            Self::ClaudeCode {
                max_concurrency, ..
            }
            | Self::Anthropic {
                max_concurrency, ..
            }
            | Self::OpenAiCompat {
                max_concurrency, ..
            }
            | Self::Ollama {
                max_concurrency, ..
            } => *max_concurrency,
        }
    }

    pub fn timeout_secs(&self) -> u64 {
        match self {
            Self::ClaudeCode { timeout_secs, .. }
            | Self::Anthropic { timeout_secs, .. }
            | Self::OpenAiCompat { timeout_secs, .. }
            | Self::Ollama { timeout_secs, .. } => *timeout_secs,
        }
    }

    /// Model family used to warn when writer and verifier would share blind spots.
    /// Generic endpoints are keyed by model name since any model can sit behind them.
    pub fn family(&self, model: &str) -> String {
        match self {
            Self::ClaudeCode { .. } | Self::Anthropic { .. } => "anthropic".into(),
            Self::OpenAiCompat { .. } | Self::Ollama { .. } => format!("model:{model}"),
        }
    }
}

fn default_claude_command() -> String {
    "claude".into()
}
fn default_anthropic_url() -> String {
    "https://api.anthropic.com".into()
}
fn default_ollama_url() -> String {
    "http://localhost:11434/v1".into()
}
fn default_concurrency() -> usize {
    2
}
fn default_timeout() -> u64 {
    300
}
fn default_max_tokens() -> u32 {
    4096
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Worker,
    Writer,
    Verifier,
    Chat,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::Worker, Role::Writer, Role::Verifier, Role::Chat];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Worker => "worker",
            Role::Writer => "writer",
            Role::Verifier => "verifier",
            Role::Chat => "chat",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Role::ALL
            .into_iter()
            .find(|r| r.as_str() == s)
            .ok_or_else(|| {
                format!("알 수 없는 역할 '{s}' (worker, writer, verifier, chat 중 하나)")
            })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleCfg {
    pub backend: Spanned<String>,
    pub model: String,
    #[serde(default)]
    pub fallback: Option<FallbackCfg>,
    /// A second model that checks the same work (verifier: cross-check, D-051).
    #[serde(default)]
    pub cross_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackCfg {
    pub backend: Spanned<String>,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetCfg {
    /// Stop a run once the summed list-price estimate reaches this amount.
    #[serde(default = "default_budget")]
    pub max_usd_per_run: f64,
}

impl Default for BudgetCfg {
    fn default() -> Self {
        Self {
            max_usd_per_run: default_budget(),
        }
    }
}

fn default_budget() -> f64 {
    5.0
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsCfg {
    /// Wait for a rate-limit reset only if it is at most this far away; otherwise fail over or stop.
    #[serde(default = "default_rate_wait")]
    pub rate_limit_max_wait_secs: u64,
    /// Days to keep run-log rows and raw outputs (F13).
    #[serde(default = "default_retention")]
    pub log_retention_days: u32,
}

impl Default for LimitsCfg {
    fn default() -> Self {
        Self {
            rate_limit_max_wait_secs: default_rate_wait(),
            log_retention_days: default_retention(),
        }
    }
}

fn default_rate_wait() -> u64 {
    60
}
fn default_retention() -> u32 {
    14
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}")]
    Parse(String),
    #[error("{line}행: {msg}")]
    Invalid { line: usize, msg: String },
}

impl Config {
    /// Parses and validates. Returns the config plus non-fatal warnings.
    /// Makes source paths absolute against the project folder `ws`.
    pub fn resolve_paths(&mut self, ws: &std::path::Path) {
        for s in &mut self.sources {
            if s.path.as_os_str().is_empty() && (s.git.is_some() || s.ssh.is_some()) {
                s.path = std::path::Path::new(".mwa/sources").join(&s.name);
            }
            if s.path.is_relative() {
                s.path = ws.join(&s.path);
            }
        }
    }

    pub fn parse(text: &str) -> Result<(Config, Vec<String>), ConfigError> {
        let cfg: Config = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        let warnings = cfg.validate(text)?;
        Ok((cfg, warnings))
    }

    fn validate(&self, text: &str) -> Result<Vec<String>, ConfigError> {
        let line_of = |span: std::ops::Range<usize>| text[..span.start].matches('\n').count() + 1;
        let check_ref = |name: &Spanned<String>| {
            if self.backends.contains_key(name.get_ref()) {
                Ok(())
            } else {
                Err(ConfigError::Invalid {
                    line: line_of(name.span()),
                    msg: format!("정의되지 않은 백엔드 '{}'", name.get_ref()),
                })
            }
        };
        for role in self.roles.values() {
            check_ref(&role.backend)?;
            if let Some(fb) = &role.fallback {
                check_ref(&fb.backend)?;
            }
        }

        let mut seen = std::collections::BTreeSet::new();
        for s in &self.sources {
            if s.ssh.is_some() && s.remote_path.as_deref().is_none_or(|p| p.trim().is_empty()) {
                return Err(ConfigError::Parse(format!(
                    "소스 '{}'에 SSH 서버의 폴더(remote_path)가 필요합니다",
                    s.name
                )));
            }
            if s.path.as_os_str().is_empty() && s.git.is_none() && s.ssh.is_none() {
                return Err(ConfigError::Parse(format!(
                    "소스 '{}'에 path, git, ssh 중 하나가 필요합니다",
                    s.name
                )));
            }
            if !seen.insert(s.name.as_str()) {
                return Err(ConfigError::Parse(format!(
                    "소스 이름 '{}'이 중복됩니다",
                    s.name
                )));
            }
        }
        if let Some(p) = &self.product {
            for name in [&p.frontend, &p.backend] {
                if !seen.contains(name.as_str()) {
                    return Err(ConfigError::Parse(format!(
                        "product가 가리키는 소스 '{name}'이 [[source]]에 없습니다"
                    )));
                }
            }
        }
        if let Some(c) = &self.capture {
            match self.profiles.get(&c.profile) {
                None => {
                    return Err(ConfigError::Parse(format!(
                        "capture.profile '{}'이 [profiles]에 없습니다",
                        c.profile
                    )));
                }
                Some(p) if p.purpose != ProfilePurpose::Capture => {
                    return Err(ConfigError::Parse(format!(
                        "capture.profile '{}'은 purpose가 capture가 아닙니다(계정 준비 전용)",
                        c.profile
                    )));
                }
                Some(_) => {}
            }
        }
        for (name, b) in &self.backends {
            if b.max_concurrency() == 0 {
                return Err(ConfigError::Parse(format!(
                    "백엔드 '{name}'의 max_concurrency는 1 이상이어야 합니다"
                )));
            }
        }
        let cap = self.budget.max_usd_per_run;
        if cap.is_nan() || cap <= 0.0 {
            return Err(ConfigError::Parse(
                "budget.max_usd_per_run은 0보다 커야 합니다".into(),
            ));
        }

        let mut warnings = Vec::new();
        if let (Some(w), Some(v)) = (
            self.roles.get(&Role::Writer),
            self.roles.get(&Role::Verifier),
        ) {
            let fam = |r: &RoleCfg| self.backends[r.backend.get_ref()].family(&r.model);
            if fam(w) == fam(v) {
                warnings.push(format!(
                    "writer와 verifier가 같은 모델 계열({})입니다. 검증은 다른 계열을 권장합니다",
                    fam(w)
                ));
            }
        }
        Ok(warnings)
    }

    pub fn role(&self, role: Role) -> Option<&RoleCfg> {
        self.roles.get(&role)
    }

    /// List-price estimate for a call, if the model has a configured price.
    pub fn price(&self, model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
        let [inp, out] = self.prices.get(model)?;
        Some((input_tokens as f64 * inp + output_tokens as f64 * out) / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r#"
[[source]]
name = "backend"
path = "/tmp/backend"

[backends.claude]
kind = "cli:claude-code"

[backends.local]
kind = "ollama"
max_concurrency = 1

[roles.worker]
backend = "claude"
model = "haiku"
fallback = { backend = "local", model = "qwen3" }

[roles.writer]
backend = "claude"
model = "sonnet"

[roles.verifier]
backend = "claude"
model = "opus"
"#;

    #[test]
    fn parses_valid_config_with_defaults() {
        let (cfg, warnings) = Config::parse(OK).unwrap();
        assert_eq!(cfg.sources[0].name, "backend");
        let claude = &cfg.backends["claude"];
        assert_eq!(claude.kind(), "cli:claude-code");
        assert_eq!(claude.max_concurrency(), 2);
        assert_eq!(claude.timeout_secs(), 300);
        assert_eq!(cfg.backends["local"].max_concurrency(), 1);
        let worker = cfg.role(Role::Worker).unwrap();
        assert_eq!(worker.fallback.as_ref().unwrap().model, "qwen3");
        assert_eq!(cfg.budget.max_usd_per_run, 5.0);
        assert_eq!(cfg.limits.log_retention_days, 14);
        assert_eq!(warnings.len(), 1, "same-family writer/verifier must warn");
    }

    #[test]
    fn unknown_backend_reference_reports_line() {
        let text = "[backends.claude]\nkind = \"cli:claude-code\"\n\n[roles.worker]\nbackend = \"nope\"\nmodel = \"x\"\n";
        match Config::parse(text) {
            Err(ConfigError::Invalid { line, msg }) => {
                assert_eq!(line, 5);
                assert!(msg.contains("nope"));
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn typo_in_field_is_rejected_with_location() {
        // Tagged enums are buffered by serde, so toml points at the table header, not the field.
        let text =
            "[sources]\n\n[backends.claude]\nkind = \"cli:claude-code\"\nmax_concurency = 3\n";
        assert!(Config::parse(text).is_err());
        let text = "[backends.claude]\nkind = \"cli:claude-code\"\nmax_concurency = 3\n";
        let err = Config::parse(text).unwrap_err().to_string();
        assert!(
            err.contains("line 1") && err.contains("max_concurency"),
            "{err}"
        );
    }

    #[test]
    fn unknown_kind_and_role_are_rejected() {
        assert!(Config::parse("[backends.x]\nkind = \"cli:nope\"\n").is_err());
        let text = "[backends.c]\nkind = \"cli:claude-code\"\n[roles.boss]\nbackend = \"c\"\nmodel = \"m\"\n";
        assert!(Config::parse(text).is_err());
    }

    #[test]
    fn api_key_value_is_never_a_field() {
        let text = "[backends.a]\nkind = \"api:anthropic\"\napi_key = \"sk-xxx\"\n";
        assert!(Config::parse(text).is_err());
    }

    #[test]
    fn profiles_default_to_capture() {
        let text = "[profiles.a]\nbase_url = \"http://x\"\nuser_id = \"u\"\n[profiles.b]\nbase_url = \"http://x\"\nuser_id = \"v\"\npurpose = \"bootstrap\"\n";
        let (cfg, _) = Config::parse(text).unwrap();
        assert_eq!(cfg.profiles["a"].purpose, ProfilePurpose::Capture);
        assert_eq!(cfg.profiles["b"].purpose, ProfilePurpose::Bootstrap);
        assert!(
            Config::parse(
                "[profiles.a]\nbase_url = \"http://x\"\nuser_id = \"u\"\npassword = \"p\"\n"
            )
            .is_err()
        );
    }

    #[test]
    fn capture_profile_rules() {
        let base = "[profiles.a]\nbase_url = \"http://x\"\nuser_id = \"u\"\n";
        let (cfg, _) = Config::parse(&format!(
            "{base}[capture]\nprofile = \"a\"\ndomain = \"tenant-a\"\n"
        ))
        .unwrap();
        let c = cfg.capture.unwrap();
        assert_eq!(
            (c.viewport, c.scale, c.domain.as_deref()),
            ([1440, 900], 2.0, Some("tenant-a"))
        );
        assert!(Config::parse(&format!("{base}[capture]\nprofile = \"nope\"\n")).is_err());
        let boot = base.replace(
            "user_id = \"u\"\n",
            "user_id = \"u\"\npurpose = \"bootstrap\"\n",
        );
        assert!(Config::parse(&format!("{boot}[capture]\nprofile = \"a\"\n")).is_err());
    }

    #[test]
    fn product_must_reference_sources() {
        let ok = "[[source]]\nname = \"f\"\npath = \"/f\"\n[[source]]\nname = \"b\"\npath = \"/b\"\n[product]\nadapter = \"adapter.toml\"\nfrontend = \"f\"\nbackend = \"b\"\n";
        let (cfg, _) = Config::parse(ok).unwrap();
        assert_eq!(cfg.product.unwrap().openapi, None);
        assert!(Config::parse(&ok.replace("backend = \"b\"", "backend = \"x\"")).is_err());
    }

    #[test]
    fn price_is_per_million_tokens() {
        let (cfg, _) = Config::parse("[prices]\n\"claude-sonnet-5-5\" = [2.0, 10.0]\n").unwrap();
        assert_eq!(
            cfg.price("claude-sonnet-5-5", 1_000_000, 100_000),
            Some(3.0)
        );
        assert_eq!(cfg.price("other", 1, 1), None);
    }

    #[test]
    fn zero_concurrency_and_budget_rejected() {
        assert!(Config::parse("[backends.c]\nkind = \"ollama\"\nmax_concurrency = 0\n").is_err());
        assert!(Config::parse("[budget]\nmax_usd_per_run = 0\n").is_err());
        assert!(Config::parse("[budget]\nmax_usd_per_run = nan\n").is_err());
    }
}
