//! Settings page (M-App T3): a JSON view of the editable parts of `mwa.toml`, and saving it
//! back with `toml_edit` so comments and unrelated keys survive. A change is written only if
//! the resulting file still passes `Config::parse`.

use std::path::Path;

use mwa_protocol::{Config, Role};
use serde::{Deserialize, Serialize};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

use crate::CONFIG_FILE;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub sources: Vec<SourceView>,
    pub product: Option<ProductView>,
    pub profiles: Vec<ProfileView>,
    pub capture: Option<CaptureView>,
    pub roles: Vec<RoleView>,
    /// Backend names defined in `[backends.*]` (read only here).
    #[serde(default)]
    pub backends: Vec<String>,
    pub max_usd_per_run: f64,
    /// Sample manual to learn the format from (`[manual] template`).
    #[serde(default)]
    pub template: String,
    /// Agent CLI path per `cli:claude-code` backend (empty = find automatically).
    #[serde(default)]
    pub agents: Vec<AgentView>,
    /// Adapter files found in the user's adapter folder (read only): `[name, path]`.
    #[serde(default)]
    pub known_adapters: Vec<[String; 2]>,
    /// Add-on: publish to Confluence (`[extensions] confluence`).
    #[serde(default)]
    pub confluence: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentView {
    pub backend: String,
    #[serde(default)]
    pub command: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceView {
    pub name: String,
    /// As written in the file (may be relative or empty for git sources).
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub git: String,
    #[serde(default)]
    pub branch: String,
    /// SSH server holding the code (`user@host`); empty for folders and Git.
    #[serde(default)]
    pub ssh: String,
    /// Jump hosts, comma-separated in the form.
    #[serde(default)]
    pub jump: String,
    #[serde(default)]
    pub docker: String,
    #[serde(default)]
    pub remote_path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductView {
    #[serde(default)]
    pub name: String,
    pub adapter: String,
    pub frontend: String,
    pub backend: String,
    #[serde(default)]
    pub openapi: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileView {
    pub name: String,
    pub base_url: String,
    pub user_id: String,
    /// `capture` or `bootstrap`.
    pub purpose: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CaptureView {
    pub profile: String,
    #[serde(default)]
    pub domain: String,
    pub viewport: [u32; 2],
    pub scale: f64,
    /// Node.js for the capture runner (empty = find automatically).
    #[serde(default)]
    pub node: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleView {
    pub role: String,
    pub backend: String,
    pub model: String,
    /// Second model checking the same work (verifier only); empty = none.
    #[serde(default)]
    pub cross_model: String,
    /// What the pipeline uses this role for (read only).
    #[serde(default)]
    pub uses: String,
}

fn role_uses(r: Role) -> &'static str {
    match r {
        Role::Worker => "코드에서 사실 추출",
        Role::Writer => "양식 학습 · 매뉴얼 작성 · 데모 데이터 · 촬영 계획",
        Role::Verifier => "근거 검증 · 기존 매뉴얼 비교",
        Role::Chat => "대화",
    }
}

pub fn read(ws: &Path) -> Result<Settings, String> {
    let text = std::fs::read_to_string(ws.join(CONFIG_FILE)).map_err(|e| e.to_string())?;
    let (cfg, _) = Config::parse(&text).map_err(|e| e.to_string())?;
    Ok(Settings {
        sources: cfg
            .sources
            .iter()
            .map(|s| SourceView {
                name: s.name.clone(),
                path: s.path.display().to_string(),
                git: s.git.clone().unwrap_or_default(),
                branch: s.branch.clone().unwrap_or_default(),
                ssh: s.ssh.clone().unwrap_or_default(),
                jump: s.jump.join(", "),
                docker: s.docker.clone().unwrap_or_default(),
                remote_path: s.remote_path.clone().unwrap_or_default(),
            })
            .collect(),
        product: cfg.product.as_ref().map(|p| ProductView {
            name: p.name.clone().unwrap_or_default(),
            adapter: p.adapter.clone(),
            frontend: p.frontend.clone(),
            backend: p.backend.clone(),
            openapi: p.openapi.clone().unwrap_or_default(),
        }),
        profiles: cfg
            .profiles
            .iter()
            .map(|(name, p)| ProfileView {
                name: name.clone(),
                base_url: p.base_url.clone(),
                user_id: p.user_id.clone(),
                purpose: format!("{:?}", p.purpose).to_lowercase(),
            })
            .collect(),
        capture: cfg.capture.as_ref().map(|c| CaptureView {
            profile: c.profile.clone(),
            domain: c.domain.clone().unwrap_or_default(),
            viewport: c.viewport,
            scale: c.scale,
            node: if c.node == "node" {
                String::new()
            } else {
                c.node.clone()
            },
        }),
        roles: Role::ALL
            .iter()
            .filter_map(|r| {
                let rc = cfg.roles.get(r)?;
                Some(RoleView {
                    role: r.as_str().to_string(),
                    backend: rc.backend.get_ref().clone(),
                    model: rc.model.clone(),
                    cross_model: rc.cross_model.clone().unwrap_or_default(),
                    uses: role_uses(*r).to_string(),
                })
            })
            .collect(),
        backends: cfg.backends.keys().cloned().collect(),
        max_usd_per_run: cfg.budget.max_usd_per_run,
        confluence: cfg.extensions.confluence,
        template: cfg.manual.template.clone().unwrap_or_default(),
        known_adapters: known_adapters(),
        agents: cfg
            .backends
            .iter()
            .filter_map(|(name, b)| match b {
                mwa_protocol::config::BackendCfg::ClaudeCode { command, .. } => Some(AgentView {
                    backend: name.clone(),
                    command: if command == "claude" {
                        String::new()
                    } else {
                        command.clone()
                    },
                }),
                _ => None,
            })
            .collect(),
    })
}

/// `<data dir>/adapters/*.toml`, the place adapters are kept for reuse across projects.
pub fn known_adapters() -> Vec<[String; 2]> {
    let dir = crate::projects::data_dir().join("adapters");
    let mut out: Vec<[String; 2]> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .filter_map(|p| {
                    let text = std::fs::read_to_string(&p).ok()?;
                    let name = mwa_productmap::adapters::from_str(&text).ok()?.name;
                    Some([name.to_string(), p.display().to_string()])
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn set_or_remove(t: &mut Table, key: &str, v: &str) {
    if v.trim().is_empty() {
        t.remove(key);
    } else {
        t[key] = value(v.trim());
    }
}

/// Applies `s` to the file text; returns the new text (not yet validated).
pub fn apply(text: &str, s: &Settings) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{CONFIG_FILE} 형식 오류: {e}"))?;

    let mut sources = ArrayOfTables::new();
    for src in &s.sources {
        let mut t = Table::new();
        t["name"] = value(src.name.trim());
        set_or_remove(&mut t, "path", &src.path);
        set_or_remove(&mut t, "git", &src.git);
        set_or_remove(&mut t, "branch", &src.branch);
        set_or_remove(&mut t, "ssh", &src.ssh);
        set_or_remove(&mut t, "docker", &src.docker);
        set_or_remove(&mut t, "remote_path", &src.remote_path);
        let hops: Vec<&str> = src
            .jump
            .split(',')
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .collect();
        if hops.is_empty() || src.ssh.trim().is_empty() {
            t.remove("jump");
        } else {
            t["jump"] = value(hops.into_iter().collect::<Array>());
        }
        sources.push(t);
    }
    if sources.is_empty() {
        doc.remove("source");
    } else {
        doc["source"] = Item::ArrayOfTables(sources);
    }

    match &s.product {
        None => {
            doc.remove("product");
        }
        Some(p) => {
            let t = doc
                .entry("product")
                .or_insert(Item::Table(Table::new()))
                .as_table_mut()
                .ok_or("[product]가 표가 아닙니다")?;
            set_or_remove(t, "name", &p.name);
            set_or_remove(t, "adapter", &p.adapter);
            set_or_remove(t, "frontend", &p.frontend);
            set_or_remove(t, "backend", &p.backend);
            set_or_remove(t, "openapi", &p.openapi);
        }
    }

    let mut profiles = Table::new();
    profiles.set_implicit(true);
    for p in &s.profiles {
        let mut t = Table::new();
        t["base_url"] = value(p.base_url.trim());
        t["user_id"] = value(p.user_id.trim());
        t["purpose"] = value(p.purpose.trim());
        profiles[p.name.trim()] = Item::Table(t);
    }
    if s.profiles.is_empty() {
        doc.remove("profiles");
    } else {
        doc["profiles"] = Item::Table(profiles);
    }

    match &s.capture {
        None => {
            doc.remove("capture");
        }
        Some(c) => {
            let t = doc
                .entry("capture")
                .or_insert(Item::Table(Table::new()))
                .as_table_mut()
                .ok_or("[capture]가 표가 아닙니다")?;
            t["profile"] = value(c.profile.trim());
            set_or_remove(t, "domain", &c.domain);
            let mut vp = Array::new();
            vp.push(c.viewport[0] as i64);
            vp.push(c.viewport[1] as i64);
            t["viewport"] = value(vp);
            t["scale"] = value(c.scale);
            set_or_remove(t, "node", &c.node);
        }
    }

    for r in &s.roles {
        let roles = doc
            .entry("roles")
            .or_insert({
                let mut t = Table::new();
                t.set_implicit(true);
                Item::Table(t)
            })
            .as_table_mut()
            .ok_or("[roles]가 표가 아닙니다")?;
        let t = roles
            .entry(&r.role)
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or("역할 설정이 표가 아닙니다")?;
        t["backend"] = value(r.backend.trim());
        t["model"] = value(r.model.trim());
        if r.cross_model.trim().is_empty() {
            t.remove("cross_model");
        } else {
            t["cross_model"] = value(r.cross_model.trim());
        }
    }

    let budget = doc
        .entry("budget")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("[budget]가 표가 아닙니다")?;
    budget["max_usd_per_run"] = value(s.max_usd_per_run);

    let ext = doc
        .entry("extensions")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("[extensions]가 표가 아닙니다")?;
    ext["confluence"] = value(s.confluence);

    for a in &s.agents {
        if let Some(t) = doc
            .get_mut("backends")
            .and_then(|b| b.get_mut(&a.backend))
            .and_then(Item::as_table_mut)
        {
            set_or_remove(t, "command", &a.command);
        }
    }

    if s.template.trim().is_empty() {
        if let Some(t) = doc.get_mut("manual").and_then(Item::as_table_mut) {
            t.remove("template");
        }
    } else {
        let t = doc
            .entry("manual")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or("[manual]가 표가 아닙니다")?;
        t["template"] = value(s.template.trim());
    }

    Ok(doc.to_string())
}

/// Missing values, in words a user understands (before the strict config check).
fn missing(s: &Settings) -> Option<String> {
    for (i, src) in s.sources.iter().enumerate() {
        if src.name.trim().is_empty() {
            return Some(format!("{}번째 코드 소스의 이름을 적으세요", i + 1));
        }
        if src.path.trim().is_empty() && src.git.trim().is_empty() && src.ssh.trim().is_empty() {
            return Some(format!(
                "코드 소스 '{}'의 폴더, Git 주소, SSH 서버 중 하나를 적으세요",
                src.name
            ));
        }
        if !src.ssh.trim().is_empty() && src.remote_path.trim().is_empty() {
            return Some(format!(
                "코드 소스 '{}'의 SSH 서버 폴더를 적으세요",
                src.name
            ));
        }
    }
    if let Some(p) = &s.product {
        if p.frontend.trim().is_empty() {
            return Some("제품의 프론트 소스를 고르세요".into());
        }
        if p.backend.trim().is_empty() {
            return Some("제품의 백엔드 소스를 고르세요".into());
        }
        if p.adapter.trim().is_empty() {
            return Some("제품의 어댑터 파일을 고르세요".into());
        }
    }
    // A site may be saved before its login (the address found while connecting the code);
    // the id comes in "사이트와 계정", which stays unfinished until then.
    for p in &s.profiles {
        if p.name.trim().is_empty() || p.base_url.trim().is_empty() {
            return Some("사이트의 이름과 주소를 적으세요".into());
        }
    }
    if let Some(c) = &s.capture
        && !s.profiles.iter().any(|p| p.name.trim() == c.profile.trim())
    {
        return Some("촬영에 쓸 계정을 고르세요".into());
    }
    None
}

/// Validates and writes. On any error the file is left unchanged.
pub fn save(ws: &Path, s: &Settings) -> Result<(), String> {
    if let Some(m) = missing(s) {
        return Err(format!("저장하지 않았습니다: {m}"));
    }
    let path = ws.join(CONFIG_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let new = apply(&text, s)?;
    Config::parse(&new).map_err(|e| format!("저장하지 않았습니다: {e}"))?;
    std::fs::write(&path, new).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_comments_and_rejects_invalid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(CONFIG_FILE), crate::TEMPLATE).unwrap();
        let mut s = read(dir.path()).unwrap();
        assert!(s.sources.is_empty() && s.product.is_none());
        assert_eq!(s.roles.len(), 4);

        s.sources = vec![
            SourceView {
                name: "frontend".into(),
                git: "git@example.com:acme/web.git".into(),
                branch: "main".into(),
                ..Default::default()
            },
            SourceView {
                name: "backend".into(),
                path: "/srv/acme/api".into(),
                ..Default::default()
            },
        ];
        s.product = Some(ProductView {
            name: "Acme".into(),
            adapter: "adapter.toml".into(),
            frontend: "frontend".into(),
            backend: "backend".into(),
            openapi: String::new(),
        });
        s.profiles = vec![ProfileView {
            name: "manual".into(),
            base_url: "http://localhost:3000".into(),
            user_id: "writer".into(),
            purpose: "capture".into(),
        }];
        s.capture = Some(CaptureView {
            profile: "manual".into(),
            domain: "tenant-a".into(),
            viewport: [1440, 900],
            scale: 2.0,
            node: String::new(),
        });
        s.agents[0].command = "/opt/claude/bin/claude".into();
        s.roles[1].model = "opus".into();
        s.roles[2].cross_model = "fable".into();
        s.max_usd_per_run = 3.0;
        s.template = "/docs/sample-manual.md".into();
        save(dir.path(), &s).unwrap();

        let text = std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(
            text.contains("# 구독 로그인된 Claude Code CLI"),
            "comments survive"
        );
        let back = read(dir.path()).unwrap();
        assert_eq!(back.sources, s.sources);
        assert_eq!(back.product, s.product);
        assert_eq!(back.profiles, s.profiles);
        assert_eq!(back.capture, s.capture);
        assert_eq!(back.roles[1].model, "opus");
        assert_eq!(back.roles[2].cross_model, "fable");
        assert_eq!(back.max_usd_per_run, 3.0);
        assert_eq!(back.template, "/docs/sample-manual.md");
        assert_eq!(back.agents[0].command, "/opt/claude/bin/claude");

        // Product pointing at a missing source: rejected, file unchanged.
        let mut bad = back.clone();
        bad.product.as_mut().unwrap().frontend = "nope".into();
        assert!(
            save(dir.path(), &bad)
                .unwrap_err()
                .contains("저장하지 않았습니다")
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap(),
            text
        );
    }
}
