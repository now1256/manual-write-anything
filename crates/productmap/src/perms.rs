//! Permission codes and where each is enforced: backend dependency (server), menu visibility
//! and in-screen checks (UI only), and license module.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::MapError;
use crate::fsutil::{rel, walk_files};
use crate::openapi::Operation;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerSite {
    pub file: String,
    pub line: usize,
    /// Route decorator above the handler, e.g. `POST /license/apply` (router prefix not applied).
    pub route: Option<String>,
    /// Full backend path when the route matches exactly one OpenAPI operation by suffix.
    pub operation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Permission {
    pub code: String,
    pub label: String,
    /// First label segment; the module licenses are sold by.
    pub license_menu: String,
    pub licensable: bool,
    pub server: Vec<ServerSite>,
    /// Menu keys gated by this code.
    pub menus: Vec<String>,
    /// Screen routes whose reachable code checks this code.
    pub ui_pages: Vec<String>,
}

/// Parses `NAME: dict[str, str] = { "CODE": "label", ... }` line by line.
pub fn parse_labels(text: &str, dict_name: &str) -> Result<BTreeMap<String, String>, MapError> {
    let mut lines = text
        .lines()
        .skip_while(|l| !l.trim_start().starts_with(dict_name));
    let header = lines
        .next()
        .ok_or_else(|| MapError::Invalid(format!("{dict_name} 정의를 찾지 못했습니다")))?;
    if !header.trim_end().ends_with('{') {
        return Err(MapError::Invalid(format!(
            "{dict_name}이 여러 줄 딕셔너리 형태가 아닙니다"
        )));
    }
    let mut out = BTreeMap::new();
    for line in lines {
        let t = line.trim();
        if t.starts_with('}') {
            return Ok(out);
        }
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        let quoted = quoted_strings(t);
        if let [code, label] = quoted.as_slice() {
            out.insert(code.clone(), label.clone());
        }
    }
    Err(MapError::Invalid(format!(
        "{dict_name} 닫는 괄호가 없습니다"
    )))
}

/// Strings in `NAME ... = frozenset({"a", "b"})`-style single statement (may span lines).
pub fn parse_string_set(text: &str, name: &str) -> Vec<String> {
    let Some(start) = text
        .lines()
        .position(|l| l.trim_start().starts_with(name) && l.contains('='))
    else {
        return Vec::new();
    };
    let mut stmt = String::new();
    let mut depth = 0i32;
    for line in text.lines().skip(start) {
        stmt.push_str(line);
        depth += line.matches(['(', '{', '[']).count() as i32
            - line.matches([')', '}', ']']).count() as i32;
        if depth <= 0 {
            break;
        }
    }
    let rhs = stmt.split_once('=').map(|(_, r)| r).unwrap_or("");
    quoted_strings(rhs)
}

fn quoted_strings(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut v = String::new();
            for d in chars.by_ref() {
                if d == '"' {
                    break;
                }
                v.push(d);
            }
            out.push(v);
        } else if c == '#' {
            break;
        }
    }
    out
}

/// `Depends(<enforcement_fn>("CODE"))` call sites under `code_dir`.
pub fn server_sites(
    backend_root: &Path,
    code_dir: &str,
    enforcement_fn: &str,
    ops: &[Operation],
) -> BTreeMap<String, Vec<ServerSite>> {
    let needle = format!("Depends({enforcement_fn}(\"");
    let mut out: BTreeMap<String, Vec<ServerSite>> = BTreeMap::new();
    let files = walk_files(&backend_root.join(code_dir), &|p| {
        p.extension().is_some_and(|e| e == "py")
    });
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let Some(pos) = line.find(&needle) else {
                continue;
            };
            if line.trim_start().starts_with('#') {
                continue;
            }
            let rest = &line[pos + needle.len()..];
            let Some(code) = rest.split('"').next() else {
                continue;
            };
            let route = decorator_above(&lines, i);
            let operation = route.as_ref().and_then(|r| unique_suffix_op(ops, r));
            out.entry(code.to_string()).or_default().push(ServerSite {
                file: rel(backend_root, &file),
                line: i + 1,
                route,
                operation,
            });
        }
    }
    out
}

/// Nearest `@x.get("/path"` style decorator above line `i` (within 40 lines), as `METHOD /path`.
fn decorator_above(lines: &[&str], i: usize) -> Option<String> {
    for j in (i.saturating_sub(40)..i).rev() {
        let t = lines[j].trim_start();
        if !t.starts_with('@') {
            continue;
        }
        // Decorators may wrap: `@router.post(\n    "/path", ...)`.
        let joined: String = lines[j..(j + 4).min(i)].join(" ");
        for m in ["get", "post", "put", "patch", "delete"] {
            let marker = format!(".{m}(");
            if let Some(p) = joined.find(&marker) {
                let path = quoted_strings(&joined[p + marker.len()..])
                    .into_iter()
                    .next()?;
                return Some(format!("{} {}", m.to_uppercase(), path));
            }
        }
    }
    None
}

fn unique_suffix_op(ops: &[Operation], route: &str) -> Option<String> {
    let (method, path) = route.split_once(' ')?;
    let mut hits = ops
        .iter()
        .filter(|o| o.method == method && o.raw_path.ends_with(path));
    let first = hits.next()?;
    hits.next()
        .is_none()
        .then(|| format!("{} {}", first.method, first.raw_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PY: &str = r#"
PERMISSION_NAMES: dict[str, str] = {
    # comment with "quotes": "ignored"
    "AUDIT_ACCESS": "설정 > 감사 로그 > 접근",
    "ORD_VIEW": "영업 > 주문 관리 > 주문 > 목록 조회",
}

FREE_MENUS: frozenset[str] = frozenset({"설정", "모니터링"})
"#;

    #[test]
    fn labels_and_sets() {
        let labels = parse_labels(PY, "PERMISSION_NAMES").unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels["AUDIT_ACCESS"], "설정 > 감사 로그 > 접근");
        assert_eq!(parse_string_set(PY, "FREE_MENUS"), vec!["설정", "모니터링"]);
        assert!(parse_labels(PY, "NOPE").is_err());
    }

    #[test]
    fn server_sites_skip_docstrings_and_find_routes() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(
            lib.join("apiLicense.py"),
            r#""""needs_permission("CFG_ACCESS") is not used here."""
@router.post("/license/apply")
async def apply(body: X,
                user: dict = Depends(needs_permission("OPS_ACCESS"))):
    pass
"#,
        )
        .unwrap();
        let ops = vec![Operation {
            method: "POST".into(),
            path: "/license/apply".into(),
            raw_path: "/license/apply".into(),
            operation_id: None,
            summary: None,
            tags: vec![],
        }];
        let sites = server_sites(dir.path(), "lib", "needs_permission", &ops);
        assert_eq!(sites.keys().collect::<Vec<_>>(), vec!["OPS_ACCESS"]);
        let s = &sites["OPS_ACCESS"][0];
        assert_eq!((s.file.as_str(), s.line), ("lib/apiLicense.py", 4));
        assert_eq!(s.route.as_deref(), Some("POST /license/apply"));
        assert_eq!(s.operation.as_deref(), Some("POST /license/apply"));
    }

    #[test]
    fn wrapped_decorator() {
        let lines = [
            "@router.post(",
            "    \"/sessions/{id}/share\", response_model=X,",
            ")",
            "async def f(",
            "    user: dict = Depends(needs_permission(\"A\")),",
        ];
        assert_eq!(
            decorator_above(&lines, 4).as_deref(),
            Some("POST /sessions/{id}/share")
        );
    }
}
