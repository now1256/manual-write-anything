//! Assembles the product map: features (menus) → screens → APIs → backend operations,
//! plus permissions and the gaps the extractors could not close.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::MapError;
use crate::adapters::Adapter;
use crate::calls::{ApiCall, Unresolved, client_calls};
use crate::graph::{Coverage, Project};
use crate::nav::{Menu, parse_nav};
use crate::openapi::{Operation, find, operations};
use crate::pages::{PageKind, discover};
use crate::perms::{Permission, parse_labels, parse_string_set, server_sites};
use crate::routes::{ProxyRoute, proxy_routes};
use crate::urlpat::match_score;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductMap {
    pub schema_version: u32,
    pub adapter: String,
    pub sources: MapSources,
    pub features: Vec<Feature>,
    /// Screens not reachable from the side menu.
    pub other_screens: Vec<Screen>,
    pub permissions: Vec<Permission>,
    pub gaps: Gaps,
    pub coverage: CoverageStats,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MapSources {
    pub frontend_sha: Option<String>,
    pub backend_sha: Option<String>,
    pub openapi: Option<OpenApiSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenApiSource {
    /// URL or file it was read from. A running server's commit is unknown (no build-info endpoint).
    pub origin: String,
    pub sha256: String,
    pub operations: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub key: String,
    pub section: String,
    pub group: Option<String>,
    pub label: String,
    pub href: String,
    /// Any-of permissions that make the menu visible; empty = always visible.
    pub permissions: Vec<String>,
    pub license_menu: String,
    pub nav_line: usize,
    pub screens: Vec<Screen>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub route: String,
    pub file: String,
    pub kind: PageKind,
    pub reached_files: usize,
    /// Known permission codes checked somewhere in the code this screen reaches.
    pub ui_permissions: Vec<String>,
    pub apis: Vec<ApiLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ApiLink {
    pub method: String,
    pub path: String,
    /// `file:line` of the client call (first one found).
    pub client: String,
    /// `file:line` of the Next.js proxy handler.
    pub proxy: Option<String>,
    /// `METHOD /backend/path` the proxy calls.
    pub backend: Option<String>,
    /// Matching OpenAPI operation as `METHOD raw_path`.
    pub operation: Option<String>,
    pub operation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Gaps {
    /// `apiFetch(...)` calls whose URL could not be evaluated.
    pub unresolved_client_calls: Vec<Unresolved>,
    /// Client calls with no matching proxy route.
    pub unlinked_client_calls: Vec<ApiCall>,
    /// Proxy handlers whose backend call was not recognized.
    pub unresolved_routes: Vec<String>,
    /// Backend calls with no operation in the OpenAPI document (`backend <- proxy`).
    pub missing_backend_operations: Vec<String>,
    /// Codes used by menus or screens but absent from the permission label table.
    pub unknown_permission_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CoverageStats {
    pub menus: usize,
    pub menus_with_screen: usize,
    pub screens: usize,
    pub screens_with_api: usize,
    pub client_calls: usize,
    pub client_calls_linked: usize,
    pub proxy_handlers: usize,
    pub proxy_resolved: usize,
    pub upstream_calls: usize,
    pub upstream_with_operation: usize,
}

pub struct BuildInput<'a> {
    pub frontend: &'a Path,
    pub backend: &'a Path,
    pub frontend_sha: Option<String>,
    pub backend_sha: Option<String>,
    /// (origin, raw JSON text)
    pub openapi: Option<(String, String)>,
}

pub fn build(a: &Adapter, input: &BuildInput) -> Result<ProductMap, MapError> {
    let (ops, openapi_source) = match &input.openapi {
        Some((origin, raw)) => {
            let doc: serde_json::Value = serde_json::from_str(raw)
                .map_err(|e| MapError::Invalid(format!("OpenAPI JSON 오류: {e}")))?;
            let ops = operations(&doc);
            let sha: String = Sha256::digest(raw.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let src = OpenApiSource {
                origin: origin.clone(),
                sha256: sha,
                operations: ops.len(),
            };
            (ops, Some(src))
        }
        None => (Vec::new(), None),
    };

    let menus = parse_nav(&input.frontend.join(a.nav_file), a.nav_sections)?;
    let pages = discover(input.frontend, a.app_dir, a.api_routes_dir, &menus);
    let mut project = Project::new(input.frontend, a.alias_prefix);
    let routes = proxy_routes(&mut project, a.app_dir, a.api_routes_dir, a.proxy_rules)?;

    let perm_file = input.backend.join(a.permissions_file);
    let perm_text =
        std::fs::read_to_string(&perm_file).map_err(|e| MapError::Io(perm_file.clone(), e))?;
    let labels = parse_labels(&perm_text, a.permission_labels_dict)?;
    let non_licensable = parse_string_set(&perm_text, a.non_licensable_set);
    let sites = server_sites(input.backend, a.backend_code_dir, a.enforcement_fn, &ops);
    let codes: BTreeSet<String> = labels.keys().cloned().collect();

    let mut gaps = Gaps::default();
    let mut stats = CoverageStats {
        menus: menus.len(),
        proxy_handlers: routes.len(),
        proxy_resolved: routes.iter().filter(|r| r.upstream.is_some()).count(),
        ..Default::default()
    };
    gaps.unresolved_routes = routes
        .iter()
        .filter(|r| r.upstream.is_none())
        .map(|r| format!("{} {} ({}:{})", r.method, r.path, r.file, r.line))
        .collect();
    if openapi_source.is_some() {
        for r in &routes {
            let Some(u) = &r.upstream else { continue };
            stats.upstream_calls += 1;
            if find(&ops, &u.method, &u.path).is_some() {
                stats.upstream_with_operation += 1;
            } else {
                gaps.missing_backend_operations.push(format!(
                    "{} {} <- {} {} ({}:{})",
                    u.method, u.path, r.method, r.path, r.file, u.line
                ));
            }
        }
    }

    let mut screens: Vec<(Option<String>, Screen)> = Vec::new();
    let mut unresolved_all = BTreeSet::new();
    let mut unlinked_all = BTreeSet::new();
    for page in &pages {
        let entry = input.frontend.join(&page.file);
        let reach = project.reach(&entry, a.stop_dirs)?;
        let (calls, unresolved) = client_calls(&mut project, &reach, a.client_call_fns)?;
        unresolved_all.extend(unresolved);
        let ui_permissions = ui_codes(&mut project, &reach, &codes)?;
        let mut apis = BTreeMap::<(String, String), ApiLink>::new();
        for call in &calls {
            stats.client_calls += 1;
            let links = link(call, &routes, &ops);
            if links.is_empty() {
                unlinked_all.insert(call.clone());
            } else {
                stats.client_calls_linked += 1;
            }
            for l in links {
                apis.entry((l.method.clone(), l.path.clone())).or_insert(l);
            }
        }
        let screen = Screen {
            route: page.route.clone(),
            file: page.file.clone(),
            kind: page.kind,
            reached_files: reach.len(),
            ui_permissions,
            apis: apis.into_values().collect(),
        };
        stats.screens += 1;
        if !screen.apis.is_empty() {
            stats.screens_with_api += 1;
        }
        screens.push((page.menu_key.clone(), screen));
    }
    gaps.unresolved_client_calls = unresolved_all.into_iter().collect();
    gaps.unlinked_client_calls = unlinked_all.into_iter().collect();

    let mut features: Vec<Feature> = menus.iter().map(feature_of).collect();
    let mut other_screens = Vec::new();
    for (menu_key, screen) in screens {
        match menu_key.and_then(|k| features.iter_mut().find(|f| f.key == k)) {
            Some(f) => f.screens.push(screen),
            None => other_screens.push(screen),
        }
    }
    stats.menus_with_screen = features
        .iter()
        .filter(|f| f.screens.iter().any(|s| s.kind == PageKind::Menu))
        .count();

    let mut permissions: Vec<Permission> = labels
        .iter()
        .map(|(code, label)| {
            let license_menu = label.split('>').next().unwrap_or("").trim().to_string();
            Permission {
                code: code.clone(),
                label: label.clone(),
                licensable: !non_licensable.contains(&license_menu),
                license_menu,
                server: sites.get(code).cloned().unwrap_or_default(),
                menus: menus
                    .iter()
                    .filter(|m| m.permissions.contains(code))
                    .map(|m| m.key.clone())
                    .collect(),
                ui_pages: features
                    .iter()
                    .flat_map(|f| f.screens.iter())
                    .chain(other_screens.iter())
                    .filter(|s| s.ui_permissions.contains(code))
                    .map(|s| s.route.clone())
                    .collect(),
            }
        })
        .collect();
    permissions.sort_by(|a, b| a.code.cmp(&b.code));

    let used: BTreeSet<&String> = menus
        .iter()
        .flat_map(|m| m.permissions.iter())
        .chain(sites.keys())
        .collect();
    gaps.unknown_permission_codes = used
        .into_iter()
        .filter(|c| !codes.contains(*c))
        .cloned()
        .collect();

    Ok(ProductMap {
        schema_version: SCHEMA_VERSION,
        adapter: a.name.to_string(),
        sources: MapSources {
            frontend_sha: input.frontend_sha.clone(),
            backend_sha: input.backend_sha.clone(),
            openapi: openapi_source,
        },
        features,
        other_screens,
        permissions,
        gaps,
        coverage: stats,
    })
}

fn feature_of(m: &Menu) -> Feature {
    Feature {
        key: m.key.clone(),
        section: m.section.clone(),
        group: m.group_label.clone(),
        label: m.label.clone(),
        href: m.href.clone(),
        permissions: m.permissions.clone(),
        license_menu: m.license_menu.clone(),
        nav_line: m.line,
        screens: Vec::new(),
    }
}

/// Proxy routes a client call reaches. A bare reference (`?`) links to every method of the path.
fn link(call: &ApiCall, routes: &[ProxyRoute], ops: &[Operation]) -> Vec<ApiLink> {
    let scored: Vec<(usize, &ProxyRoute)> = routes
        .iter()
        .filter(|r| call.method == "?" || r.method == call.method)
        .filter_map(|r| match_score(&r.path, &call.path).map(|s| (s, r)))
        .collect();
    let Some(best) = scored.iter().map(|(s, _)| *s).max() else {
        return Vec::new();
    };
    scored
        .into_iter()
        .filter(|(s, _)| *s == best)
        .map(|(_, r)| {
            let op = r
                .upstream
                .as_ref()
                .and_then(|u| find(ops, &u.method, &u.path));
            ApiLink {
                method: r.method.clone(),
                path: r.path.clone(),
                client: format!("{}:{}", call.file, call.line),
                proxy: Some(format!("{}:{}", r.file, r.line)),
                backend: r
                    .upstream
                    .as_ref()
                    .map(|u| format!("{} {}", u.method, u.path)),
                operation: op.map(|o| format!("{} {}", o.method, o.raw_path)),
                operation_id: op.and_then(|o| o.operation_id.clone()),
            }
        })
        .collect()
}

/// Known permission codes appearing as string literals in the reached code.
fn ui_codes(
    project: &mut Project,
    reach: &BTreeMap<PathBuf, Coverage>,
    codes: &BTreeSet<String>,
) -> Result<Vec<String>, MapError> {
    let mut found = BTreeSet::new();
    for (file, cov) in reach {
        let src = project.src(file)?;
        for n in src.nodes() {
            if n.kind() == "string"
                && cov.contains(n.start_byte())
                && let Some(v) = src.string_value(n)
                && codes.contains(&v)
            {
                found.insert(v);
            }
        }
    }
    Ok(found.into_iter().collect())
}
