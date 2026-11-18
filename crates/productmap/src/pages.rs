//! Screen discovery: Next.js app-router `page.tsx` files and their link to menus.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil::{rel, walk_files};
use crate::nav::Menu;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    /// The menu's own screen.
    Menu,
    /// A deeper route under a menu (detail pages, tabs).
    MenuDetail,
    /// Not reachable from the side menu (login, dashboard, ...).
    Outside,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub route: String,
    /// Relative to the frontend root.
    pub file: String,
    pub menu_key: Option<String>,
    pub kind: PageKind,
}

/// `src/app/(app)/data/[id]/page.tsx` → `/data/{id}`; route groups are dropped.
pub fn route_of(app_dir: &Path, page: &Path) -> String {
    let dir = page.parent().unwrap_or(app_dir);
    let segs: Vec<String> = dir
        .strip_prefix(app_dir)
        .unwrap_or(dir)
        .components()
        .filter_map(|c| {
            let s = c.as_os_str().to_string_lossy();
            if s.starts_with('(') && s.ends_with(')') {
                None
            } else if s.starts_with('[') && s.ends_with(']') {
                Some(format!("{{{}}}", &s[1..s.len() - 1]))
            } else {
                Some(s.into_owned())
            }
        })
        .collect();
    format!("/{}", segs.join("/"))
}

/// Pages sorted by route.
pub fn discover(root: &Path, app_dir: &str, api_routes_dir: &str, menus: &[Menu]) -> Vec<Page> {
    let app = root.join(app_dir);
    let api = root.join(api_routes_dir);
    let mut pages: Vec<Page> = walk_files(&app, &|p| {
        !p.starts_with(&api)
            && matches!(
                p.file_name().and_then(|n| n.to_str()),
                Some("page.tsx" | "page.ts")
            )
    })
    .into_iter()
    .map(|file| {
        let route = route_of(&app, &file);
        let menu = menus
            .iter()
            .filter(|m| route == m.prefix() || route.starts_with(&format!("{}/", m.prefix())))
            .max_by_key(|m| m.prefix().len());
        let kind = match menu {
            Some(m) if route == m.prefix() || route == m.href => PageKind::Menu,
            Some(_) => PageKind::MenuDetail,
            None => PageKind::Outside,
        };
        Page {
            route,
            file: rel(root, &file),
            menu_key: menu.map(|m| m.key.clone()),
            kind,
        }
    })
    .collect();
    pages.sort_by(|a, b| a.route.cmp(&b.route));
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(key: &str, href: &str) -> Menu {
        Menu {
            section: "S".into(),
            group_key: None,
            group_label: None,
            key: key.into(),
            label: key.into(),
            href: href.into(),
            match_prefix: None,
            permissions: vec![],
            permission_hrefs: vec![],
            license_menu: String::new(),
            line: 1,
        }
    }

    #[test]
    fn routes_and_menu_linking() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for p in [
            "src/app/(app)/data/studio/page.tsx",
            "src/app/(app)/data/studio/[id]/page.tsx",
            "src/app/(auth)/login/page.tsx",
            "src/app/(app)/page.tsx",
            "src/app/api/x/page.tsx",
        ] {
            let f = root.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, "").unwrap();
        }
        let menus = [menu("studio", "/data/studio"), menu("data", "/data")];
        let pages = discover(root, "src/app", "src/app/api", &menus);
        let got: Vec<(&str, Option<&str>, PageKind)> = pages
            .iter()
            .map(|p| (p.route.as_str(), p.menu_key.as_deref(), p.kind))
            .collect();
        assert_eq!(
            got,
            vec![
                ("/", None, PageKind::Outside),
                ("/data/studio", Some("studio"), PageKind::Menu),
                ("/data/studio/{id}", Some("studio"), PageKind::MenuDetail),
                ("/login", None, PageKind::Outside),
            ]
        );
    }
}
