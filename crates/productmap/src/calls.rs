//! Client-side API calls (`apiFetch("/api/...", {method})`) in the code a screen reaches.
//! Small same-file URL helpers (`const url = (db, path) => \`${BASE}${path}?...\``) are inlined.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::MapError;
use crate::expand::{Env, Expander, imported_consts, local_var_literal};
use crate::fsutil::rel;
use crate::graph::{Coverage, Project};
use crate::urlpat::template;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ApiCall {
    /// HTTP method, or `?` when only a reference to the URL was found.
    pub method: String,
    /// Normalized `/api/...` template.
    pub path: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Unresolved {
    pub file: String,
    pub line: usize,
    pub text: String,
}

pub fn client_calls(
    project: &mut Project,
    reach: &BTreeMap<PathBuf, Coverage>,
    call_fns: &[&str],
) -> Result<(Vec<ApiCall>, Vec<Unresolved>), MapError> {
    let mut calls = BTreeSet::new();
    let mut unresolved = BTreeSet::new();
    for (file, coverage) in reach {
        let imported = imported_consts(project, file)?;
        let file_rel = rel(&project.root, file);
        let src = project.src(file)?;
        let x = Expander::new(src, imported);
        let mut consumed = BTreeSet::new();
        let const_values: BTreeSet<usize> = src
            .top_level_declarators()
            .into_iter()
            .filter_map(|d| d.child_by_field_name("value"))
            .map(|v| v.id())
            .collect();
        let nodes: Vec<Node> = src
            .nodes()
            .into_iter()
            .filter(|n| coverage.contains(n.start_byte()))
            .collect();

        for &node in &nodes {
            let name = match node.kind() {
                "call_expression" => src.callee_name(node),
                "new_expression" => node
                    .child_by_field_name("constructor")
                    .map(|c| src.text_of(c).to_string()),
                _ => None,
            };
            let Some(name) = name.filter(|n| call_fns.contains(&n.as_str())) else {
                continue;
            };
            let args = src.args(node);
            let Some(first) = args.first().copied() else {
                continue;
            };
            mark_literals(first, &mut consumed);
            let arg = local_var_literal(src, first, node).unwrap_or(first);
            mark_literals(arg, &mut consumed);
            let path = x
                .expand(arg, &Env::new(), 0)
                .and_then(|parts| template(&parts, &mut |_| None));
            match path {
                Some(path) if path.starts_with("/api/") => {
                    let method = if name == "EventSource" {
                        "GET".to_string()
                    } else {
                        x.method(args.get(1).copied())
                    };
                    calls.insert(ApiCall {
                        method,
                        path,
                        file: file_rel.clone(),
                        line: src.line(node),
                    });
                }
                Some(_) => {}
                None if name == "apiFetch" => {
                    unresolved.insert(Unresolved {
                        file: file_rel.clone(),
                        line: src.line(node),
                        text: crate::short(src.text_of(node)),
                    });
                }
                None => {}
            }
        }

        // `/api/...` URLs handed to other helpers (SSE hooks, URL builders) count as references:
        // literals, plus identifiers naming `/api/` constants passed as call arguments.
        // Constant initializers and the bodies of inlined URL helpers are already accounted for.
        let inlined = x.inlined.borrow().clone();
        for &node in &nodes {
            let is_literal = matches!(node.kind(), "string" | "template_string");
            let is_const_arg = node.kind() == "identifier"
                && node.parent().is_some_and(|p| p.kind() == "arguments")
                && (x.consts.contains_key(src.text_of(node))
                    || x.imported.contains_key(src.text_of(node)));
            if !(is_literal || is_const_arg)
                || consumed.contains(&node.id())
                || const_values.contains(&node.id())
            {
                continue;
            }
            if has_ancestor(node, "template_substitution")
                || inlined.iter().any(|r| r.contains(&node.start_byte()))
            {
                continue;
            }
            if let Some(path) = x
                .expand(node, &Env::new(), 0)
                .and_then(|p| template(&p, &mut |_| None))
                && path.starts_with("/api/")
            {
                calls.insert(ApiCall {
                    method: "?".into(),
                    path,
                    file: file_rel.clone(),
                    line: src.line(node),
                });
            }
        }
    }
    Ok((
        calls.into_iter().collect(),
        unresolved.into_iter().collect(),
    ))
}

fn has_ancestor(n: Node, kind: &str) -> bool {
    let mut p = n.parent();
    while let Some(x) = p {
        if x.kind() == kind {
            return true;
        }
        p = x.parent();
    }
    false
}

fn mark_literals(n: Node, consumed: &mut BTreeSet<usize>) {
    if matches!(n.kind(), "string" | "template_string") {
        consumed.insert(n.id());
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        mark_literals(ch, consumed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn whole(r: &Path, f: &str) -> BTreeMap<PathBuf, Coverage> {
        BTreeMap::from([(r.join(f), Coverage::Whole)])
    }

    fn paths(calls: &[ApiCall]) -> Vec<(String, String)> {
        calls
            .iter()
            .map(|c| (c.method.clone(), c.path.clone()))
            .collect()
    }

    #[test]
    fn extracts_methods_paths_consts_and_refs() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            r,
            "src/lib/api/base.ts",
            r#"export const SYS = "/api/source/systems";"#,
        );
        write(
            r,
            "src/lib/api/f.ts",
            r#"import { SYS } from "./base";
const LOCAL = "/api/local";
export const list = (db) => apiFetch(`/api/source/systems?db=${db}`);
export const del = (id, db) => apiFetch(`/api/source/systems/${id}?db=${db}`, { method: "delete" });
export const sum = () => apiFetch(`${SYS}/summary`);
export const loc = () => apiFetch(LOCAL, { method: "POST" });
export const viaVar = (id) => { const url = `/api/x/${id}/test`; return apiFetch(url, { method: "POST" }); };
export const dyn = (u) => apiFetch(u);
export const sse = () => new EventSource("/api/stream");
export const ext = () => fetch("https://example.com/x");
export const hook = () => useStream(`/api/board/${id}/events`);
"#,
        );
        let mut p = Project::new(r, "@/");
        let (calls, unresolved) = client_calls(
            &mut p,
            &whole(r, "src/lib/api/f.ts"),
            &["apiFetch", "fetch", "EventSource"],
        )
        .unwrap();
        let got = paths(&calls);
        for want in [
            ("GET", "/api/source/systems"),
            ("DELETE", "/api/source/systems/{}"),
            ("GET", "/api/source/systems/summary"),
            ("POST", "/api/local"),
            ("POST", "/api/x/{}/test"),
            ("GET", "/api/stream"),
            ("?", "/api/board/{}/events"),
        ] {
            assert!(
                got.contains(&(want.0.to_string(), want.1.to_string())),
                "missing {want:?} in {got:?}"
            );
        }
        assert!(!got.iter().any(|(_, p)| p.contains("example.com")));
        assert!(
            !got.contains(&("?".into(), "/api/x/{}/test".into())),
            "consumed literal not double-counted"
        );
        assert_eq!(unresolved.len(), 1, "{unresolved:?}");
        assert!(unresolved[0].text.contains("apiFetch(u)"));
    }

    #[test]
    fn inlines_url_and_init_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            r,
            "src/lib/api/nlq.ts",
            r#"const BASE = "/api/nlq/semantic-models";
const url = (db: string, path = "", params?: Record<string, string>) => {
  const qs = new URLSearchParams({ db });
  return `${BASE}${path}?${qs}`;
};
const json = (method: string, body?: unknown): RequestInit => ({ method, body: JSON.stringify(body) });
const dbQuery = (db: string) => `db=${encodeURIComponent(db)}`;
const jobPath = (jobId: string) => `${BASE}/jobs/${encodeURIComponent(jobId)}`;
export const a = (db) => apiFetch(url(db, "", { page: "1" }));
export const b = (db) => apiFetch(url(db, "/summary"));
export const c = (db, id) => apiFetch(url(db, `/${encodeURIComponent(id)}`), json("PATCH", {}));
export const d = (db, id) => apiFetch(`${jobPath(id)}/train?${dbQuery(db)}`, { method: "POST" });
"#,
        );
        let mut p = Project::new(r, "@/");
        let (calls, unresolved) =
            client_calls(&mut p, &whole(r, "src/lib/api/nlq.ts"), &["apiFetch"]).unwrap();
        assert!(unresolved.is_empty(), "{unresolved:?}");
        let got = paths(&calls);
        for want in [
            ("GET", "/api/nlq/semantic-models"),
            ("GET", "/api/nlq/semantic-models/summary"),
            ("PATCH", "/api/nlq/semantic-models/{}"),
            ("POST", "/api/nlq/semantic-models/jobs/{}/train"),
        ] {
            assert!(
                got.contains(&(want.0.to_string(), want.1.to_string())),
                "missing {want:?} in {got:?}"
            );
        }
        assert!(
            !got.iter().any(|(m, _)| m == "?"),
            "helper bodies and consts are not references: {got:?}"
        );
    }

    #[test]
    fn only_covered_ranges_are_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let text = r#"export const a = () => apiFetch("/api/a");
export const z = () => apiFetch("/api/z");"#;
        write(r, "f.ts", text);
        let end_of_a = text.find('\n').unwrap();
        let reach = BTreeMap::from([(
            r.join("f.ts"),
            Coverage::Ranges(std::iter::once(0..end_of_a).collect()),
        )]);
        let mut p = Project::new(r, "@/");
        let (calls, _) = client_calls(&mut p, &reach, &["apiFetch"]).unwrap();
        assert_eq!(
            paths(&calls),
            vec![("GET".to_string(), "/api/a".to_string())]
        );
    }
}
