//! Product adapters: file locations and naming conventions of one product, loaded from the
//! project's `adapter.toml`. Everything else in this crate is product-agnostic, and no
//! product's conventions are compiled into the binary.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use crate::MapError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterFile {
    name: String,
    nav_file: String,
    nav_sections: Vec<NavSectionFile>,
    app_dir: String,
    api_routes_dir: String,
    alias_prefix: String,
    #[serde(default)]
    stop_dirs: Vec<String>,
    client_call_fns: Vec<String>,
    #[serde(default)]
    proxy_rules: Vec<ProxyRuleFile>,
    permissions_file: String,
    permission_labels_dict: String,
    non_licensable_set: String,
    enforcement_fn: String,
    backend_code_dir: String,
    /// Login and screen conventions, read by the capture crate (`mwa_capture::spec`).
    #[serde(default, rename = "auth")]
    _auth: Option<toml::Value>,
    #[serde(default, rename = "ui")]
    _ui: Option<toml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NavSectionFile {
    export: String,
    #[serde(default)]
    license_menu: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProxyRuleFile {
    Sink {
        func: String,
        path_arg: usize,
        init_arg: usize,
    },
    Factory {
        factory: String,
        opts_arg: usize,
        path_prop: String,
    },
    PrefixedWrapper {
        inner: String,
        path_arg: usize,
        opts_arg: usize,
    },
    DirectFetch {
        func: String,
        base_const: String,
    },
}

/// Loads an adapter file. Each distinct file content is parsed once per process and kept
/// for its lifetime (the map code borrows adapter strings as `'static`).
pub fn load(path: &Path) -> Result<&'static Adapter, MapError> {
    let text = std::fs::read_to_string(path).map_err(|e| MapError::Io(path.to_path_buf(), e))?;
    from_str(&text).map_err(|e| MapError::Invalid(format!("{}: {e}", path.display())))
}

pub fn from_str(text: &str) -> Result<&'static Adapter, String> {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static Adapter>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(a) = cache.lock().expect("adapter cache").get(text) {
        return Ok(a);
    }
    let f: AdapterFile = toml::from_str(text).map_err(|e| e.to_string())?;
    let a: &'static Adapter = Box::leak(Box::new(f.into_adapter()));
    cache
        .lock()
        .expect("adapter cache")
        .insert(text.to_string(), a);
    Ok(a)
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_all(v: Vec<String>) -> &'static [&'static str] {
    Box::leak(
        v.into_iter()
            .map(leak)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

impl AdapterFile {
    fn into_adapter(self) -> Adapter {
        let nav_sections: Vec<(&'static str, Option<&'static str>)> = self
            .nav_sections
            .into_iter()
            .map(|s| (leak(s.export), s.license_menu.map(leak)))
            .collect();
        let proxy_rules: Vec<ProxyRule> = self
            .proxy_rules
            .into_iter()
            .map(|r| match r {
                ProxyRuleFile::Sink {
                    func,
                    path_arg,
                    init_arg,
                } => ProxyRule::Sink {
                    func: leak(func),
                    path_arg,
                    init_arg,
                },
                ProxyRuleFile::Factory {
                    factory,
                    opts_arg,
                    path_prop,
                } => ProxyRule::Factory {
                    factory: leak(factory),
                    opts_arg,
                    path_prop: leak(path_prop),
                },
                ProxyRuleFile::PrefixedWrapper {
                    inner,
                    path_arg,
                    opts_arg,
                } => ProxyRule::PrefixedWrapper {
                    inner: leak(inner),
                    path_arg,
                    opts_arg,
                },
                ProxyRuleFile::DirectFetch { func, base_const } => ProxyRule::DirectFetch {
                    func: leak(func),
                    base_const: leak(base_const),
                },
            })
            .collect();
        Adapter {
            name: leak(self.name),
            nav_file: leak(self.nav_file),
            nav_sections: Box::leak(nav_sections.into_boxed_slice()),
            app_dir: leak(self.app_dir),
            api_routes_dir: leak(self.api_routes_dir),
            alias_prefix: leak(self.alias_prefix),
            stop_dirs: leak_all(self.stop_dirs),
            client_call_fns: leak_all(self.client_call_fns),
            proxy_rules: Box::leak(proxy_rules.into_boxed_slice()),
            permissions_file: leak(self.permissions_file),
            permission_labels_dict: leak(self.permission_labels_dict),
            non_licensable_set: leak(self.non_licensable_set),
            enforcement_fn: leak(self.enforcement_fn),
            backend_code_dir: leak(self.backend_code_dir),
        }
    }
}

/// How a proxy route reaches the backend.
#[derive(Debug, Clone, Copy)]
pub enum ProxyRule {
    /// `fn(req, "/backend/path", init?)`: backend path at `path_arg`, `method` in the object at `init_arg`.
    Sink {
        func: &'static str,
        path_arg: usize,
        init_arg: usize,
    },
    /// `const forward = factory("/prefix")` in a helper module, then `forward(req, {path, method})`.
    Factory {
        factory: &'static str,
        opts_arg: usize,
        path_prop: &'static str,
    },
    /// `const helper = (req, path, opts) => inner("/prefix", req, path, opts)`; route calls `helper(req, path, opts)`.
    PrefixedWrapper {
        inner: &'static str,
        path_arg: usize,
        opts_arg: usize,
    },
    /// `fetch(\`${BASE}/backend/path\`, init)` where `BASE` names the backend origin.
    DirectFetch {
        func: &'static str,
        base_const: &'static str,
    },
}

#[derive(Debug, Clone)]
pub struct Adapter {
    pub name: &'static str,
    /// Frontend files, relative to the frontend repository root.
    pub nav_file: &'static str,
    /// (exported nav array, license menu override for all its items)
    pub nav_sections: &'static [(&'static str, Option<&'static str>)],
    pub app_dir: &'static str,
    pub api_routes_dir: &'static str,
    /// Import alias prefix mapped to the repository root (`@/src/x` → `<root>/src/x`).
    pub alias_prefix: &'static str,
    /// Imports into these directories are not followed when building a screen's file set:
    /// shared shells and contexts would attach the same APIs to every screen.
    pub stop_dirs: &'static [&'static str],
    /// Client-side calls whose first argument is a `/api/...` URL.
    pub client_call_fns: &'static [&'static str],
    pub proxy_rules: &'static [ProxyRule],
    /// Backend files, relative to the backend repository root.
    pub permissions_file: &'static str,
    pub permission_labels_dict: &'static str,
    pub non_licensable_set: &'static str,
    pub enforcement_fn: &'static str,
    pub backend_code_dir: &'static str,
}
