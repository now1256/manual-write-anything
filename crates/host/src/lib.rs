//! Screen-neutral interface between mwa's engine and its front ends (TUI, `mwa serve`).
//! Implemented by the CLI; front ends only call it.

use async_trait::async_trait;
use mwa_productmap::map::{Feature, ProductMap};
use mwa_protocol::Quota;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Header {
    pub workspace: String,
    pub sources: Vec<SourceBadge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceBadge {
    pub name: String,
    pub sha: String,
    /// Uncommitted changes present.
    pub preview: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    /// e.g. `claude·sonnet`.
    pub model: String,
    pub spent_usd: f64,
    pub budget_usd: f64,
    pub quota: Option<Quota>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Who {
    User,
    Assistant,
    System,
    Error,
}

pub type OnDelta = Box<dyn Fn(&str) + Send + Sync>;

/// One project the user has created or opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    /// Project folder (holds `mwa.toml`).
    pub path: String,
    /// The folder still has a `mwa.toml`.
    pub ready: bool,
    /// Unix milliseconds.
    pub last_opened: Option<u64>,
}

/// The list of projects and how to open one; implemented by the CLI.
pub trait Projects: Send + Sync + 'static {
    fn list(&self) -> Vec<ProjectEntry>;
    /// Creates a project folder (default location when `path` is `None`) and registers it.
    fn create(&self, name: &str, path: Option<&str>) -> Result<ProjectEntry, String>;
    /// Opens an existing project folder and registers it as most recently used.
    fn open(&self, path: &str) -> Result<std::sync::Arc<dyn Host>, String>;
}

#[async_trait]
pub trait Host: Send + Sync + 'static {
    fn header(&self) -> Header;
    /// Previously built `map/product-map.json`, if any.
    fn load_map(&self) -> Option<ProductMap>;
    fn status(&self) -> Status;
    /// One chat turn. `transcript` ends with the new user message; `context` describes the
    /// feature the user is looking at.
    async fn chat(
        &self,
        transcript: Vec<(Who, String)>,
        context: Option<String>,
        on_delta: OnDelta,
    ) -> Result<(), String>;
    /// Rebuilds the product map; returns it with summary lines.
    async fn build_map(&self) -> Result<(ProductMap, Vec<String>), String>;
    async fn doctor(&self) -> Vec<String>;
    fn auth_lines(&self) -> Vec<String>;
    /// A capture or generation is running; switching projects must wait.
    fn busy(&self) -> bool {
        false
    }
    /// Models the chat role can switch to (empty = no choice).
    fn chat_models(&self) -> Vec<String> {
        Vec::new()
    }
    /// Switches the chat model for this session.
    fn set_chat_model(&self, model: &str) -> Result<(), String> {
        Err(format!("모델을 바꿀 수 없습니다: {model}"))
    }
    /// Captures every menu screen; reports progress lines, returns the summary.
    async fn capture_menus(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        drop(progress);
        Err("이 화면에서는 촬영을 지원하지 않습니다".into())
    }
    /// Generates one feature's manual; reports progress lines, returns the summary.
    async fn generate_manual(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        drop(progress);
        Err(format!(
            "이 화면에서는 매뉴얼 생성을 지원하지 않습니다: {feature}"
        ))
    }
    /// Saved `manual/<feature>.json`, if any.
    fn manual_run(&self, feature: &str) -> Option<serde_json::Value> {
        let _ = feature;
        None
    }
    /// Latest `shots/menus/manifest.json`, if any.
    fn shots_manifest(&self) -> Option<serde_json::Value> {
        None
    }
    /// A captured file under `shots/` (plain relative path only).
    fn shot_file(&self, rel: &str) -> Option<std::path::PathBuf> {
        let _ = rel;
        None
    }
}

pub fn feature_context(f: &Feature) -> String {
    let mut out = format!(
        "보고 있는 기능: {} (경로 {}, 대메뉴 {}, 노출 권한 {})\n",
        f.label,
        f.href,
        f.license_menu,
        if f.permissions.is_empty() {
            "없음(항상 노출)".into()
        } else {
            f.permissions.join(", ")
        }
    );
    for s in &f.screens {
        out.push_str(&format!("화면 {} ({}):\n", s.route, s.file));
        if !s.ui_permissions.is_empty() {
            out.push_str(&format!(
                "  화면 안 권한 검사: {}\n",
                s.ui_permissions.join(", ")
            ));
        }
        for a in &s.apis {
            out.push_str(&format!(
                "  {} {} → {}\n",
                a.method,
                a.path,
                a.backend.as_deref().unwrap_or("백엔드 미확인")
            ));
        }
    }
    out
}

/// Model context for a chat turn: the feature the user has open plus features whose
/// label appears in the latest user message (at most 3 in total).
pub fn chat_context(
    map: &ProductMap,
    selected_key: Option<&str>,
    last_user: &str,
) -> Option<String> {
    let mut picked: Vec<usize> = Vec::new();
    if let Some(k) = selected_key
        && let Some(i) = map.features.iter().position(|f| f.key == k)
    {
        picked.push(i);
    }
    let compact: String = last_user.split_whitespace().collect();
    for (i, f) in map.features.iter().enumerate() {
        let label: String = f.label.split_whitespace().collect();
        if picked.len() < 3 && !picked.contains(&i) && !label.is_empty() && compact.contains(&label)
        {
            picked.push(i);
        }
    }
    if picked.is_empty() {
        return None;
    }
    let parts: Vec<String> = picked
        .iter()
        .map(|i| feature_context(&map.features[*i]))
        .collect();
    Some(parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mini() -> ProductMap {
        serde_json::from_str(include_str!("../../productmap/tests/snapshots/mini.json")).unwrap()
    }

    #[test]
    fn context_from_selection_and_mentions() {
        let m = mini();
        assert_eq!(chat_context(&m, None, "안녕하세요"), None);
        let by_name = chat_context(&m, None, "연결관리 화면 설명해줘").unwrap();
        assert!(by_name.contains("연결 관리") && by_name.contains("/api/source/systems"));
        let by_sel = chat_context(&m, Some("license"), "이거 뭐야").unwrap();
        assert!(by_sel.contains("제품 정보"));
        assert!(chat_context(&m, Some("nope"), "").is_none());
    }

    #[test]
    fn who_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&Who::Assistant).unwrap(),
            "\"assistant\""
        );
    }
}
