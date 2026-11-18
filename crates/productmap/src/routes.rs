//! Next.js proxy routes (`src/app/api/**/route.ts`) and the backend call each handler makes.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::MapError;
use crate::adapters::ProxyRule;
use crate::expand::{
    Env, Expander, function_def, imported_consts, local_var_literal, unwrap_parens,
};
use crate::fsutil::{rel, walk_files};
use crate::graph::Project;
use crate::ts::{Part, Src};
use crate::urlpat::template;

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Upstream {
    pub method: String,
    pub path: String,
    /// Which proxy rule matched.
    pub rule: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProxyRoute {
    pub method: String,
    /// `/api/...` template.
    pub path: String,
    pub file: String,
    pub line: usize,
    pub upstream: Option<Upstream>,
}

/// How a callable name in a route file reaches the backend.
#[derive(Debug, Clone)]
enum Binding {
    /// Factory-made forwarder with a fixed prefix.
    Forward {
        prefix: String,
        opts_arg: usize,
        path_prop: &'static str,
    },
    /// Wrapper that prepends a prefix to its `path` argument.
    Prefixed {
        prefix: String,
        path_arg: usize,
        opts_arg: usize,
    },
}

/// `src/app/api/a/[id]/route.ts` → `/api/a/{}`.
pub fn route_path(app_dir: &Path, file: &Path) -> String {
    let dir = file.parent().unwrap_or(app_dir);
    let segs: Vec<String> = dir
        .strip_prefix(app_dir)
        .unwrap_or(dir)
        .components()
        .filter_map(|c| {
            let s = c.as_os_str().to_string_lossy();
            if s.starts_with('(') && s.ends_with(')') {
                None
            } else if s.starts_with('[') && s.ends_with(']') {
                Some("{}".to_string())
            } else {
                Some(s.into_owned())
            }
        })
        .collect();
    format!("/{}", segs.join("/"))
}

pub fn proxy_routes(
    project: &mut Project,
    app_dir: &str,
    api_routes_dir: &str,
    rules: &[ProxyRule],
) -> Result<Vec<ProxyRoute>, MapError> {
    let root = project.root.clone();
    let files = walk_files(&root.join(api_routes_dir), &|p| {
        matches!(
            p.file_name().and_then(|n| n.to_str()),
            Some("route.ts" | "route.tsx")
        )
    });
    let mut out = Vec::new();
    for file in files {
        let bindings = bindings_for(project, &file, rules)?;
        let imported = imported_consts(project, &file)?;
        let path = route_path(&root.join(app_dir), &file);
        let file_rel = rel(&root, &file);
        let src = project.src(&file)?;
        let x = Expander::new(src, imported);
        for (method, node) in handlers(src) {
            let upstream = find_upstream(src, &x, node, &method, rules, &bindings, 0);
            out.push(ProxyRoute {
                method,
                path: path.clone(),
                file: file_rel.clone(),
                line: src.line(node),
                upstream,
            });
        }
    }
    out.sort();
    Ok(out)
}

/// Exported HTTP handlers: `export async function GET`, `export const GET = ...`,
/// `export { handler as GET }`.
fn handlers(src: &Src) -> Vec<(String, Node<'_>)> {
    let mut out = Vec::new();
    let root = src.root();
    let mut c = root.walk();
    for stmt in root
        .children(&mut c)
        .filter(|s| s.kind() == "export_statement")
    {
        if let Some(decl) = stmt.child_by_field_name("declaration") {
            if decl.kind() == "function_declaration"
                && let Some(name) = decl.child_by_field_name("name").map(|n| src.text_of(n))
                && METHODS.contains(&name)
            {
                out.push((name.to_string(), decl));
            }
            if decl.kind() == "lexical_declaration" {
                let mut dc = decl.walk();
                for d in decl
                    .named_children(&mut dc)
                    .filter(|d| d.kind() == "variable_declarator")
                {
                    if let (Some(n), Some(v)) = (
                        d.child_by_field_name("name"),
                        d.child_by_field_name("value"),
                    ) && METHODS.contains(&src.text_of(n))
                    {
                        out.push((src.text_of(n).to_string(), v));
                    }
                }
            }
        } else {
            let mut sc = stmt.walk();
            for clause in stmt
                .named_children(&mut sc)
                .filter(|n| n.kind() == "export_clause")
            {
                let mut cc = clause.walk();
                for spec in clause
                    .named_children(&mut cc)
                    .filter(|n| n.kind() == "export_specifier")
                {
                    let name = spec
                        .child_by_field_name("name")
                        .map(|n| src.text_of(n).to_string());
                    let alias = spec
                        .child_by_field_name("alias")
                        .map(|n| src.text_of(n).to_string());
                    let exported = alias.clone().or(name.clone()).unwrap_or_default();
                    if METHODS.contains(&exported.as_str())
                        && let Some(def) = name.and_then(|n| function_def(src, &n))
                    {
                        out.push((exported, def));
                    }
                }
            }
        }
    }
    out
}

/// Factory forwarders and prefixed wrappers visible in `file` (own top level or imported).
fn bindings_for(
    project: &mut Project,
    file: &Path,
    rules: &[ProxyRule],
) -> Result<HashMap<String, Binding>, MapError> {
    let mut candidates: Vec<(String, std::path::PathBuf, String)> = Vec::new();
    for imp in project.src(file)?.imports() {
        if let Some(target) = project.resolve(file, &imp.source) {
            for (imported, local) in imp.names {
                candidates.push((local, target.clone(), imported));
            }
        }
    }
    let own: Vec<String> = {
        let src = project.src(file)?;
        src.top_level_declarators()
            .into_iter()
            .filter_map(|d| {
                d.child_by_field_name("name")
                    .map(|n| src.text_of(n).to_string())
            })
            .collect()
    };
    for name in own {
        candidates.push((name.clone(), file.to_path_buf(), name));
    }

    let mut out = HashMap::new();
    for (local, target, name) in candidates {
        let src = project.src(&target)?;
        let Some(value) = src.top_level_value(&name) else {
            continue;
        };
        for rule in rules {
            match *rule {
                ProxyRule::Factory {
                    factory,
                    opts_arg,
                    path_prop,
                } => {
                    if value.kind() == "call_expression"
                        && src.callee_name(value).as_deref() == Some(factory)
                        && let Some(prefix) =
                            src.args(value).first().and_then(|a| src.string_value(*a))
                    {
                        out.insert(
                            local.clone(),
                            Binding::Forward {
                                prefix,
                                opts_arg,
                                path_prop,
                            },
                        );
                    }
                }
                ProxyRule::PrefixedWrapper {
                    inner,
                    path_arg,
                    opts_arg,
                } => {
                    if value.kind() == "arrow_function"
                        && let Some(body) = value.child_by_field_name("body").map(unwrap_parens)
                        && body.kind() == "call_expression"
                        && src.callee_name(body).as_deref() == Some(inner)
                        && let Some(prefix) =
                            src.args(body).first().and_then(|a| src.string_value(*a))
                    {
                        out.insert(
                            local.clone(),
                            Binding::Prefixed {
                                prefix,
                                path_arg,
                                opts_arg,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// First backend call inside `scope`, following same-file helper functions up to depth 2.
fn find_upstream(
    src: &Src,
    x: &Expander,
    scope: Node,
    handler_method: &str,
    rules: &[ProxyRule],
    bindings: &HashMap<String, Binding>,
    depth: usize,
) -> Option<Upstream> {
    let calls: Vec<Node> = src
        .nodes()
        .into_iter()
        .filter(|n| {
            n.kind() == "call_expression"
                && n.start_byte() >= scope.start_byte()
                && n.end_byte() <= scope.end_byte()
        })
        .collect();
    for &call in &calls {
        if let Some(u) = match_call(src, x, call, handler_method, rules, bindings) {
            return Some(u);
        }
    }
    if depth < 2 {
        for &call in &calls {
            if let Some(name) = src.callee_name(call)
                && let Some(def) = function_def(src, &name)
                && !(def.start_byte() <= scope.start_byte() && scope.end_byte() <= def.end_byte())
                && let Some(u) =
                    find_upstream(src, x, def, handler_method, rules, bindings, depth + 1)
            {
                return Some(u);
            }
        }
    }
    None
}

fn match_call(
    src: &Src,
    x: &Expander,
    call: Node,
    handler_method: &str,
    rules: &[ProxyRule],
    bindings: &HashMap<String, Binding>,
) -> Option<Upstream> {
    let name = src.callee_name(call)?;
    let args = src.args(call);
    let line = src.line(call);
    let path_of = |n: Option<Node>| -> Option<String> {
        let n = n?;
        let n = local_var_literal(src, n, call).unwrap_or(n);
        let parts = x.expand(n, &Env::new(), 0)?;
        // A wrapper called with `""` targets the prefix itself.
        if parts
            .iter()
            .all(|p| matches!(p, Part::Lit(s) if s.is_empty()))
        {
            return Some(String::new());
        }
        template(&parts, &mut |_| None)
    };
    let method_of = |init: Option<Node>| -> String {
        let init = init.map(|n| local_var_literal(src, n, call).unwrap_or(n));
        match x.method(init) {
            m if m == "?" => handler_method.to_string(),
            m => m,
        }
    };

    if let Some(binding) = bindings.get(&name) {
        let (prefix, path, opts) = match binding {
            Binding::Forward {
                prefix,
                opts_arg,
                path_prop,
            } => {
                let opts = args.get(*opts_arg).copied();
                let path = match opts.and_then(|o| src.prop(o, path_prop)) {
                    Some(p) => path_of(Some(p))?,
                    None => String::new(),
                };
                (prefix, path, opts)
            }
            Binding::Prefixed {
                prefix,
                path_arg,
                opts_arg,
            } => {
                let path = path_of(args.get(*path_arg).copied())?;
                (prefix, path, args.get(*opts_arg).copied())
            }
        };
        let rule = match binding {
            Binding::Forward { .. } => "factory",
            Binding::Prefixed { .. } => "prefixed-wrapper",
        };
        let full =
            crate::urlpat::normalize(&format!("{prefix}{}", if path == "/" { "" } else { &path }))?;
        return Some(Upstream {
            method: method_of(opts),
            path: full,
            rule: format!("{rule}:{name}"),
            line,
        });
    }

    for rule in rules {
        match *rule {
            ProxyRule::Sink {
                func,
                path_arg,
                init_arg,
            } if name == func => {
                return Some(Upstream {
                    method: method_of(args.get(init_arg).copied()),
                    path: path_of(args.get(path_arg).copied())?,
                    rule: format!("sink:{func}"),
                    line,
                });
            }
            ProxyRule::DirectFetch { func, base_const } if name == func => {
                let first = *args.first()?;
                let parts = src.parts(first)?;
                let rest = match parts.split_first() {
                    Some((Part::Sub(b), rest)) if b == base_const => rest.to_vec(),
                    _ => continue,
                };
                return Some(Upstream {
                    method: method_of(args.get(1).copied()),
                    path: template(&rest, &mut |_| None)?,
                    rule: format!("direct:{base_const}"),
                    line,
                });
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_rules() -> &'static [ProxyRule] {
        crate::adapters::from_str(include_str!("../tests/fixtures/mini/adapter.toml"))
            .unwrap()
            .proxy_rules
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn summary(routes: &[ProxyRoute]) -> Vec<String> {
        routes
            .iter()
            .map(|r| match &r.upstream {
                Some(u) => format!(
                    "{} {} -> {} {} [{}]",
                    r.method,
                    r.path,
                    u.method,
                    u.path,
                    u.rule.split(':').next().unwrap()
                ),
                None => format!("{} {} -> ?", r.method, r.path),
            })
            .collect()
    }

    #[test]
    fn all_proxy_patterns() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            r,
            "src/app/api/constant.ts",
            r#"const BACKEND_ORIGIN = process.env.BACKEND_ORIGIN ?? "http://x"; export { BACKEND_ORIGIN };"#,
        );
        write(
            r,
            "src/lib/api/forward.ts",
            r#"export function forwarder(upstream: string) { return async () => {}; }"#,
        );
        write(
            r,
            "src/lib/api/nlq/proxy.ts",
            r#"async function proxyQuery(base, req, path, opts) { return callUpstream(req, `${base}${path}?x`, {}); }
export const proxyQueryFlows = (req, path, opts) => proxyQuery("/nlq/query-flows", req, path, opts);"#,
        );
        write(
            r,
            "src/app/api/emb/_forward.ts",
            r#"import { forwarder } from "@/src/lib/api/forward"; export const forward = forwarder("/embedding-training");"#,
        );
        write(
            r,
            "src/app/api/sys/[id]/route.ts",
            r#"export async function GET(req) { const res = await callUpstream(req, `/systems/${id}?db=${db}`); return res; }
export async function PATCH(req) { return callUpstream(req, `/systems/${id}?db=${db}`, { method: "PATCH", body }); }
export async function POST(req) { const init: RequestInit = { method: "POST" }; return callUpstream(req, "/systems/test", init); }"#,
        );
        write(
            r,
            "src/app/api/emb/jobs/[id]/route.ts",
            r#"import { forward } from "../../_forward";
export async function GET(req) { return forward(req, { path: `/jobs/${id}`, failMessage: "x" }); }
export async function DELETE(req) { return forward(req, { path: `/jobs/${id}`, method: "DELETE", failMessage: "x" }); }"#,
        );
        write(
            r,
            "src/app/api/nlq/flows/[id]/route.ts",
            r#"import { proxyQueryFlows } from "@/src/lib/api/nlq/proxy";
export const PATCH = (req, ctx) => proxyQueryFlows(req, `/${encodeURIComponent(id)}`, { method: "PATCH", fallback: "x" });
async function handler(req) { return proxyQueryFlows(req, "", { fallback: "x" }); }
export { handler as GET };"#,
        );
        write(
            r,
            "src/app/api/auth/login/route.ts",
            r#"import { BACKEND_ORIGIN } from "@/src/app/api/constant";
export async function POST(req) { const upstream = await fetch(`${BACKEND_ORIGIN}/auth/login`, { method: "POST" }); return upstream; }"#,
        );
        write(
            r,
            "src/app/api/auth/me/route.ts",
            r#"export async function GET(req) { const auth = await requireApiAuth(req); return ok(auth.user); }"#,
        );
        write(
            r,
            "src/app/api/helper/route.ts",
            r#"async function load(req, path) { return callUpstream(req, `/helper${path}`); }
export async function GET(req) { return load(req, "/x"); }"#,
        );

        let mut p = Project::new(r, "@/");
        let routes = proxy_routes(&mut p, "src/app", "src/app/api", fixture_rules()).unwrap();
        let got = summary(&routes);
        for want in [
            "GET /api/sys/{} -> GET /systems/{} [sink]",
            "PATCH /api/sys/{} -> PATCH /systems/{} [sink]",
            "POST /api/sys/{} -> POST /systems/test [sink]",
            "GET /api/emb/jobs/{} -> GET /embedding-training/jobs/{} [factory]",
            "DELETE /api/emb/jobs/{} -> DELETE /embedding-training/jobs/{} [factory]",
            "PATCH /api/nlq/flows/{} -> PATCH /nlq/query-flows/{} [prefixed-wrapper]",
            "GET /api/nlq/flows/{} -> GET /nlq/query-flows [prefixed-wrapper]",
            "POST /api/auth/login -> POST /auth/login [direct]",
            "GET /api/auth/me -> ?",
        ] {
            assert!(got.contains(&want.to_string()), "missing {want}\n{got:#?}");
        }
        // Helper-defined path: `/helper${path}` keeps the dynamic suffix as a query-like tail.
        assert!(
            got.iter()
                .any(|g| g.starts_with("GET /api/helper -> GET /helper")),
            "{got:#?}"
        );
    }
}
