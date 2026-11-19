//! Menu extraction from a nav config written as exported arrays of item objects
//! (`{key, label, href?, permissions?, children?: [{key, label, href, match?, permissions?, permissionHrefs?}]}`).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::MapError;
use crate::ts::{Src, parse_file};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Menu {
    /// Exported array the item came from (e.g. `MAIN_NAV`).
    pub section: String,
    pub group_key: Option<String>,
    pub group_label: Option<String>,
    pub key: String,
    pub label: String,
    pub href: String,
    /// Route prefix used for guards when it differs from `href`.
    pub match_prefix: Option<String>,
    /// Any-of permissions that make the menu visible (`permissionHrefs` win over `permissions`).
    pub permissions: Vec<String>,
    pub permission_hrefs: Vec<(String, String)>,
    /// License module the menu belongs to.
    pub license_menu: String,
    pub line: usize,
}

impl Menu {
    pub fn prefix(&self) -> &str {
        self.match_prefix.as_deref().unwrap_or(&self.href)
    }
}

/// `sections`: (exported const name, license menu override for every item in it).
pub fn parse_nav(path: &Path, sections: &[(&str, Option<&str>)]) -> Result<Vec<Menu>, MapError> {
    let src = parse_file(path)?;
    let consts = src.string_consts();
    let mut out = Vec::new();
    for (section, license_override) in sections {
        let arr = src
            .top_level_value(section)
            .filter(|n| n.kind() == "array")
            .ok_or_else(|| {
                MapError::Invalid(format!(
                    "{}: 배열 {section}을 찾지 못했습니다",
                    path.display()
                ))
            })?;
        let mut c = arr.walk();
        for item in arr.named_children(&mut c).filter(|n| n.kind() == "object") {
            let r = Reader {
                src: &src,
                consts: &consts,
            };
            let key = r.req_str(item, "key")?;
            let label = r.req_str(item, "label")?;
            let license_menu = license_override
                .map(String::from)
                .or_else(|| r.str(item, "licenseMenu"))
                .unwrap_or_else(|| label.clone());
            let children = src.prop(item, "children").filter(|n| n.kind() == "array");
            match children {
                Some(children) => {
                    let mut cc = children.walk();
                    for child in children
                        .named_children(&mut cc)
                        .filter(|n| n.kind() == "object")
                    {
                        let mut m = r.menu(child, section)?;
                        m.group_key = Some(key.clone());
                        m.group_label = Some(label.clone());
                        m.license_menu = license_menu.clone();
                        out.push(m);
                    }
                }
                None => {
                    let mut m = r.menu(item, section)?;
                    m.license_menu = license_menu;
                    out.push(m);
                }
            }
        }
    }
    Ok(out)
}

struct Reader<'a> {
    src: &'a Src,
    consts: &'a HashMap<String, String>,
}

impl Reader<'_> {
    fn value(&self, n: Node) -> Option<String> {
        self.src.string_value(n).or_else(|| {
            (n.kind() == "identifier")
                .then(|| self.consts.get(self.src.text_of(n)).cloned())
                .flatten()
        })
    }

    fn str(&self, obj: Node, key: &str) -> Option<String> {
        self.src.prop(obj, key).and_then(|v| self.value(v))
    }

    fn req_str(&self, obj: Node, key: &str) -> Result<String, MapError> {
        self.str(obj, key).ok_or_else(|| {
            MapError::Invalid(format!(
                "{}:{}: '{key}' 값을 읽지 못했습니다",
                self.src.path.display(),
                self.src.line(obj)
            ))
        })
    }

    fn menu(&self, obj: Node, section: &str) -> Result<Menu, MapError> {
        let permission_hrefs: Vec<(String, String)> = self
            .src
            .prop(obj, "permissionHrefs")
            .filter(|n| n.kind() == "array")
            .map(|arr| {
                let mut c = arr.walk();
                arr.named_children(&mut c)
                    .filter_map(|o| Some((self.str(o, "permission")?, self.str(o, "href")?)))
                    .collect()
            })
            .unwrap_or_default();
        let permissions = if permission_hrefs.is_empty() {
            self.src
                .prop(obj, "permissions")
                .map(|a| self.src.string_array(a))
                .unwrap_or_default()
        } else {
            permission_hrefs.iter().map(|(p, _)| p.clone()).collect()
        };
        Ok(Menu {
            section: section.to_string(),
            group_key: None,
            group_label: None,
            key: self.req_str(obj, "key")?,
            label: self.req_str(obj, "label")?,
            href: self.req_str(obj, "href")?,
            match_prefix: self.str(obj, "match"),
            permissions,
            permission_hrefs,
            license_menu: String::new(),
            line: self.src.line(obj),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAV: &str = r#"
import { faX } from "x";
export const LICENSE_HREF = "/settings/license";
export const MAIN_NAV: NavItem[] = [
  {
    key: "sales",
    label: "영업",
    icon: faX,
    children: [
      { key: "conn", label: "주문 관리", href: "/sales/orders", permissions: ["ORD_VIEW"] },
      { key: "studio", label: "배송 관리", href: "/sales/shipping", match: "/sales/shipping",
        permissionHrefs: [{ permission: "A_VIEW", href: "/sales/shipping/a" }, { permission: "B_VIEW", href: "/sales/shipping/b" }] },
    ],
  },
  { key: "bi", label: "BI", licenseMenu: "분석", icon: faX, children: [{ key: "bi-a", label: "A", href: "/bi/a" }] },
];
export const SYS_NAV: NavItem[] = [
  { key: "sys", label: "시스템", icon: faX, children: [{ key: "license", label: "제품 정보", href: LICENSE_HREF, permissions: ["OPS_ACCESS"] }] },
];
export const FLAT_NAV: NavItem[] = [
  { key: "pending", label: "결재 대기", href: "/approval/pending", icon: faX },
];
"#;

    #[test]
    fn parses_groups_flat_items_consts_and_license() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("menu.ts");
        std::fs::write(&path, NAV).unwrap();
        let menus = parse_nav(
            &path,
            &[
                ("MAIN_NAV", None),
                ("SYS_NAV", Some("설정")),
                ("FLAT_NAV", Some("결재")),
            ],
        )
        .unwrap();
        assert_eq!(menus.len(), 5);

        let conn = &menus[0];
        assert_eq!(conn.group_label.as_deref(), Some("영업"));
        assert_eq!(conn.permissions, vec!["ORD_VIEW"]);
        assert_eq!(conn.license_menu, "영업");

        let studio = &menus[1];
        assert_eq!(
            studio.permissions,
            vec!["A_VIEW", "B_VIEW"],
            "permissionHrefs win"
        );
        assert_eq!(studio.prefix(), "/sales/shipping");

        assert_eq!(menus[2].license_menu, "분석");
        assert!(menus[2].permissions.is_empty());

        assert_eq!(
            menus[3].href, "/settings/license",
            "identifier resolved via const"
        );
        assert_eq!(menus[3].license_menu, "설정");

        let flat = &menus[4];
        assert_eq!(flat.group_key, None);
        assert_eq!(flat.license_menu, "결재");
    }

    #[test]
    fn missing_section_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("menu.ts");
        std::fs::write(&path, NAV).unwrap();
        assert!(parse_nav(&path, &[("NOPE_NAV", None)]).is_err());
    }
}
