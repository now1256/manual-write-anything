//! Product map (F1): deterministic extraction of menus, screens, APIs and permissions
//! from the product source. No LLM. Product-specific knowledge lives in `adapters/`.

pub mod adapters;
pub mod calls;
pub mod expand;
pub mod fsutil;
pub mod graph;
pub mod map;
pub mod nav;
pub mod openapi;
pub mod pages;
pub mod perms;
pub mod routes;
pub mod shapes;
pub mod ts;
pub mod urlpat;

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    #[error("{0} 파싱 실패: {1}")]
    Parse(PathBuf, String),
    #[error("{0}")]
    Invalid(String),
}

/// One-line, length-limited excerpt of source text for reports.
pub(crate) fn short(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 120 {
        format!("{}…", one.chars().take(120).collect::<String>())
    } else {
        one
    }
}
