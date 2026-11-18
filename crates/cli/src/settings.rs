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
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleView {
    pub role: String,
    pub backend: String,
    pub model: String,
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
        }),
        roles: Role::ALL
            .iter()
            .filter_map(|r| {
                let rc = cfg.roles.get(r)?;
                Some(RoleView {
                    role: r.as_str().to_string(),
                    backend: rc.backend.get_ref().clone(),
                    model: rc.model.clone(),
                    uses: role_uses(*r).to_string(),
                })
            })
            .collect(),
        backends: cfg.backends.keys().cloned().collect(),
        max_usd_per_run: cfg.budget.max_usd_per_run,
    })
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
    }

    let budget = doc
        .entry("budget")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("[budget]가 표가 아닙니다")?;
    budget["max_usd_per_run"] = value(s.max_usd_per_run);

    Ok(doc.to_string())
}

/// Validates and writes. On any error the file is left unchanged.
pub fn save(ws: &Path, s: &Settings) -> Result<(), String> {
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
        });
        s.roles[1].model = "opus".into();
        s.max_usd_per_run = 3.0;
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
        assert_eq!(back.max_usd_per_run, 3.0);

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
