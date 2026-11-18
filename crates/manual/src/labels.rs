//! Visible UI text read straight from the frontend code (no model): JSX text, label-like
//! attributes and props, and toast messages. Used as the glossary and to check bold labels.

use std::collections::BTreeMap;
use std::path::Path;

use mwa_productmap::ts::{Part, parse_str};
use serde::{Deserialize, Serialize};

use crate::model::Repo;
use crate::slice::Slice;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub text: String,
    /// Where it comes from: `jsx`, `attr:placeholder`, `prop:label`, `toast.error`, ...
    pub origin: String,
    pub file: String,
    pub line: usize,
}

const ATTRS: &[&str] = &[
    "placeholder",
    "title",
    "aria-label",
    "label",
    "description",
    "desc",
    "emptyText",
    "confirmText",
    "cancelText",
];
const PROPS: &[&str] = &[
    "label",
    "title",
    "placeholder",
    "desc",
    "description",
    "header",
    "message",
    "text",
    "emptyTitle",
    "emptyDesc",
];

fn has_korean_or_word(s: &str) -> bool {
    s.chars().any(|c| ('가'..='힣').contains(&c))
        || (s.len() > 2 && s.chars().any(|c| c.is_alphabetic()))
}

fn literal_text(parts: &[Part]) -> Option<String> {
    // Keep literal text; substitutions become "{}" so templates like `${n}명 삭제` stay readable.
    let s: String = parts
        .iter()
        .map(|p| match p {
            Part::Lit(t) => t.clone(),
            Part::Sub(_) => "{}".into(),
        })
        .collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn extract(slice: &Slice) -> Vec<Label> {
    let mut out = Vec::new();
    for f in slice.files.iter().filter(|f| f.repo == Repo::Frontend) {
        if !(f.path.ends_with(".tsx") || f.path.ends_with(".ts")) {
            continue;
        }
        let text = f.lines.join("\n");
        let Ok(src) = parse_str(Path::new(&f.path), text) else {
            continue;
        };
        let in_range = |line: usize| f.ranges.iter().any(|(a, b)| (*a..=*b).contains(&line));
        for n in src.nodes() {
            let line = src.line(n);
            if !in_range(line) {
                continue;
            }
            let found: Option<(String, String)> = match n.kind() {
                "jsx_text" => {
                    let t = src
                        .text_of(n)
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    (!t.is_empty()).then(|| (t, "jsx".to_string()))
                }
                "jsx_attribute" => {
                    let mut c = n.walk();
                    let kids: Vec<_> = n.named_children(&mut c).collect();
                    let name = kids
                        .first()
                        .map(|k| src.text_of(*k).to_string())
                        .unwrap_or_default();
                    let value = kids.get(1).copied();
                    match value {
                        Some(v) if ATTRS.contains(&name.as_str()) => src
                            .parts(v)
                            .and_then(|p| literal_text(&p))
                            .map(|t| (t, format!("attr:{name}"))),
                        _ => None,
                    }
                }
                "pair" => {
                    let key = n.child_by_field_name("key").map(|k| {
                        src.string_value(k)
                            .unwrap_or_else(|| src.text_of(k).to_string())
                    });
                    match (key, n.child_by_field_name("value")) {
                        (Some(k), Some(v)) if PROPS.contains(&k.as_str()) => src
                            .parts(v)
                            .and_then(|p| literal_text(&p))
                            .map(|t| (t, format!("prop:{k}"))),
                        _ => None,
                    }
                }
                "call_expression" => {
                    let f = n
                        .child_by_field_name("function")
                        .map(|x| src.text_of(x).to_string());
                    match f.as_deref() {
                        Some(name) if name.starts_with("toast") => src
                            .args(n)
                            .first()
                            .and_then(|a| src.parts(*a))
                            .and_then(|p| literal_text(&p))
                            .map(|t| (t, name.to_string())),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some((t, origin)) = found
                && has_korean_or_word(&t)
            {
                out.push(Label {
                    text: t,
                    origin,
                    file: f.path.clone(),
                    line,
                });
            }
        }
    }
    // One entry per (text, origin), first occurrence wins.
    let mut seen: BTreeMap<(String, String), ()> = BTreeMap::new();
    out.retain(|l| {
        seen.insert((l.text.clone(), l.origin.clone()), ())
            .is_none()
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slice::SliceFile;

    #[test]
    fn extracts_visible_text_only_in_ranges() {
        let code = r#"export function A() {
  toast.success(`${n}명을 삭제했습니다.`);
  const cols = [{ key: "name", header: "이름" }];
  return (<Modal title="사용자 등록">
    <Input placeholder="아이디를 입력하세요" />
    <Button>  사용자   등록 </Button>
    <span>{count}</span>
  </Modal>);
}
const unused = <p>범위 밖</p>;"#;
        let s = Slice {
            files: vec![SliceFile {
                repo: Repo::Frontend,
                path: "a.tsx".into(),
                lines: code.lines().map(String::from).collect(),
                ranges: vec![(1, 9)],
            }],
        };
        let got: Vec<(String, String)> = extract(&s)
            .into_iter()
            .map(|l| (l.text, l.origin))
            .collect();
        for want in [
            ("{}명을 삭제했습니다.", "toast.success"),
            ("이름", "prop:header"),
            ("사용자 등록", "attr:title"),
            ("아이디를 입력하세요", "attr:placeholder"),
            ("사용자 등록", "jsx"),
        ] {
            assert!(
                got.contains(&(want.0.to_string(), want.1.to_string())),
                "missing {want:?} in {got:?}"
            );
        }
        assert!(!got.iter().any(|(t, _)| t == "범위 밖"));
    }
}
