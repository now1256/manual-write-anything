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

/// Result of checking one settings field (folder, git URL, adapter, site, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub ok: bool,
    pub detail: String,
}

/// One item of the "시작하기" checklist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStep {
    pub key: String,
    pub label: String,
    pub done: bool,
    pub detail: String,
    /// `settings`, `command:<name>` or empty.
    pub action: String,
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
    /// Clones or updates sources that come from git; returns one line per source.
    async fn sync_sources(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        drop(progress);
        Err("이 화면에서는 소스 동기화를 지원하지 않습니다".into())
    }
    /// Installs the browser used for screenshots.
    async fn install_browser(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        drop(progress);
        Err("이 화면에서는 브라우저 설치를 지원하지 않습니다".into())
    }
    /// Checks one settings value without saving it (read-only probes).
    async fn check(&self, kind: &str, value: &str, extra: &str) -> Check {
        let _ = (kind, value, extra);
        Check {
            ok: false,
            detail: "이 화면에서는 확인을 지원하지 않습니다".into(),
        }
    }
    /// Imported existing manuals (summaries).
    fn references(&self) -> Result<serde_json::Value, String> {
        Err("이 화면에서는 기존 매뉴얼을 지원하지 않습니다".into())
    }
    /// One imported manual with every picture.
    fn reference(&self, id: &str) -> Option<serde_json::Value> {
        let _ = id;
        None
    }
    /// Imports PDFs (paths on the engine's machine); one line per file.
    async fn import_references(&self, paths: Vec<String>) -> Result<Vec<String>, String> {
        let _ = paths;
        Err("이 화면에서는 기존 매뉴얼을 지원하지 않습니다".into())
    }
    /// Sets which feature a manual documents, or marks it as not a feature.
    fn set_reference(
        &self,
        id: &str,
        feature: Option<String>,
        ignored: bool,
    ) -> Result<(), String> {
        let _ = (id, feature, ignored);
        Err("이 화면에서는 기존 매뉴얼을 지원하지 않습니다".into())
    }
    fn reference_file(&self, id: &str, rel: &str) -> Option<std::path::PathBuf> {
        let _ = (id, rel);
        None
    }
    fn reference_page(&self, id: &str, page: u32) -> Result<std::path::PathBuf, String> {
        let _ = (id, page);
        Err("이 화면에서는 기존 매뉴얼을 지원하지 않습니다".into())
    }
    /// Describes every picture of a manual with the worker model.
    async fn describe_reference(&self, id: &str, progress: OnDelta) -> Result<Vec<String>, String> {
        let _ = id;
        drop(progress);
        Err("이 화면에서는 기존 매뉴얼을 지원하지 않습니다".into())
    }
    /// Results of reproducing the old manual's scenes for a feature.
    fn reference_shots(&self, feature: &str) -> Option<serde_json::Value> {
        let _ = feature;
        None
    }
    /// Reproduces the old manual's scenes for a feature, judged per scene. `feature?demo` uses
    /// demo data; otherwise the server's real data.
    async fn capture_reference(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        let _ = feature;
        drop(progress);
        Err("이 화면에서는 기존 매뉴얼 장면 촬영을 지원하지 않습니다".into())
    }
    /// 자동 작업 rules with their next runs, and the latest runs (D-050).
    fn automation(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn save_automation(&self, rules: serde_json::Value) -> Result<serde_json::Value, String> {
        let _ = rules;
        Err("이 화면에서는 자동 작업을 지원하지 않습니다".into())
    }
    /// Runs one rule now.
    async fn run_automation(&self, id: &str, progress: OnDelta) -> Result<Vec<String>, String> {
        let _ = id;
        drop(progress);
        Err("이 화면에서는 자동 작업을 지원하지 않습니다".into())
    }
    /// Runs the rules that are due (once a minute while serving).
    async fn automation_tick(&self) {}
    /// Writes the product adapter from the code and builds the product map (D-049).
    async fn make_adapter(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        drop(progress);
        Err("이 화면에서는 어댑터 만들기를 지원하지 않습니다".into())
    }
    /// Shoots the picture slots of a feature's written manual (D-046). `feature|<block id>`
    /// shoots that slot only (again, when it has a picture).
    async fn capture_document(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        let _ = feature;
        drop(progress);
        Err("이 화면에서는 그림 자리 촬영을 지원하지 않습니다".into())
    }
    /// The editable manual document of a feature (D-032), if any.
    fn document(&self, feature: &str) -> Option<serde_json::Value> {
        let _ = feature;
        None
    }
    /// Saves edited HTML as the next document version.
    /// `changes`: what the editor knows about why blocks changed (see `provenance::Change`).
    fn save_document(
        &self,
        feature: &str,
        html: &str,
        changes: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let _ = (feature, html, changes);
        Err("이 화면에서는 매뉴얼 편집을 지원하지 않습니다".into())
    }
    /// Stores an uploaded picture for a document and returns its file name.
    fn save_document_asset(
        &self,
        feature: &str,
        name: &str,
        data: &[u8],
    ) -> Result<String, String> {
        let _ = (feature, name, data);
        Err("이 화면에서는 그림 올리기를 지원하지 않습니다".into())
    }
    fn document_asset(&self, feature: &str, file: &str) -> Option<std::path::PathBuf> {
        let _ = (feature, file);
        None
    }
    /// Saved versions of a document: `[{version, updated_at}]`.
    fn document_versions(&self, feature: &str) -> serde_json::Value {
        let _ = feature;
        serde_json::json!([])
    }
    fn document_version(&self, feature: &str, n: u32) -> Option<serde_json::Value> {
        let _ = (feature, n);
        None
    }
    /// Why each block says what it says (code facts, verdict), from the last check.
    fn grounding(&self, feature: &str) -> serde_json::Value {
        let _ = feature;
        serde_json::json!({ "version": 0, "at": 0, "blocks": {} })
    }
    /// Checks every block of the document against the code facts.
    async fn ground_document(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        let _ = feature;
        drop(progress);
        Err("이 화면에서는 근거 확인을 지원하지 않습니다".into())
    }
    /// Where each block of a document came from, and the change log per version.
    fn provenance(&self, feature: &str) -> serde_json::Value {
        let _ = feature;
        serde_json::json!({ "blocks": {}, "log": [] })
    }
    /// Pending update proposals for a document.
    fn proposals(&self, feature: &str) -> serde_json::Value {
        let _ = feature;
        serde_json::json!({ "version": 0, "items": [] })
    }
    /// Drops a proposal after it was accepted or rejected.
    fn resolve_proposal(&self, feature: &str, id: &str) -> Result<serde_json::Value, String> {
        let _ = (feature, id);
        Err("이 화면에서는 갱신 제안을 지원하지 않습니다".into())
    }
    /// Compares the document with the code and stores proposals.
    async fn propose_document(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        let _ = feature;
        drop(progress);
        Err("이 화면에서는 갱신 제안을 지원하지 않습니다".into())
    }
    /// Uploads (or with `None` removes) the picture for one old-manual scene.
    fn set_scene_upload(
        &self,
        feature: &str,
        image: &str,
        upload: Option<(&str, &[u8])>,
    ) -> Result<serde_json::Value, String> {
        let _ = (feature, image, upload);
        Err("이 화면에서는 그림 올리기를 지원하지 않습니다".into())
    }
    /// Looks around an SSH server for a code source (D-054): containers, ports, the folder, and
    /// addresses the product may be served at. Read only.
    async fn probe_source(&self, source: serde_json::Value) -> Result<serde_json::Value, String> {
        let _ = source;
        Err("이 화면에서는 SSH 서버 살펴보기를 지원하지 않습니다".into())
    }
    /// A person's instruction for shooting one scene (`None` clears it).
    fn set_scene_instruction(
        &self,
        feature: &str,
        image: &str,
        instruction: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let _ = (feature, image, instruction);
        Err("이 화면에서는 촬영 지시를 지원하지 않습니다".into())
    }
    /// Chooses which picture the new manual uses for one scene (`None` = automatic).
    fn set_scene_choice(
        &self,
        feature: &str,
        image: &str,
        choice: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let _ = (feature, image, choice);
        Err("이 화면에서는 그림 고르기를 지원하지 않습니다".into())
    }
    /// Links an imported old manual to a feature (unlinking any other).
    fn link_reference(&self, feature: &str, id: &str) -> Result<(), String> {
        let _ = (feature, id);
        Err("이 화면에서는 기존 매뉴얼을 지원하지 않습니다".into())
    }
    /// Imports an uploaded PDF and links it to a feature.
    async fn upload_reference(
        &self,
        feature: &str,
        name: &str,
        data: Vec<u8>,
    ) -> Result<serde_json::Value, String> {
        let _ = (feature, name, data);
        Err("이 화면에서는 기존 매뉴얼을 지원하지 않습니다".into())
    }
    /// Makes an earlier document version current again.
    fn restore_document(&self, feature: &str, n: u32) -> Result<serde_json::Value, String> {
        let _ = (feature, n);
        Err("이 화면에서는 매뉴얼 편집을 지원하지 않습니다".into())
    }
    /// Step ③: the document from the old manual (if none yet), then update proposals from the code.
    async fn make_document(&self, feature: &str, progress: OnDelta) -> Result<Vec<String>, String> {
        let _ = feature;
        drop(progress);
        Err("이 화면에서는 매뉴얼 편집을 지원하지 않습니다".into())
    }
    /// The editor's side-panel helper (see `document::assist`); `body` holds request, the whole
    /// manual's html, focus block ids and history.
    async fn assist(
        &self,
        feature: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let _ = (feature, body);
        Err("이 화면에서는 도우미를 지원하지 않습니다".into())
    }
    /// Where every feature's manual stands (old manual, pictures, document, basis, code, publish).
    fn manuals_status(&self) -> serde_json::Value {
        serde_json::json!({ "features": [] })
    }
    /// Confluence target and where this feature was published (if it was).
    fn confluence_status(&self, feature: &str) -> serde_json::Value {
        let _ = feature;
        serde_json::json!({ "configured": false })
    }
    /// Checks and saves the Confluence target: `{page_url, email, token}`.
    async fn confluence_setup(&self, body: serde_json::Value) -> Result<String, String> {
        let _ = body;
        Err("이 화면에서는 컨플루언스 게시를 지원하지 않습니다".into())
    }
    /// Publishes a document: `{title, storage, images: [{name, url}], adopt}`.
    async fn confluence_publish(
        &self,
        feature: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let _ = (feature, body);
        Err("이 화면에서는 컨플루언스 게시를 지원하지 않습니다".into())
    }
    /// Turns the feature's old manual (PDF) into an editable document.
    async fn import_document(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        let _ = feature;
        drop(progress);
        Err("이 화면에서는 매뉴얼 편집을 지원하지 않습니다".into())
    }
    /// Versions of a feature's manual with what changed in each.
    fn manual_versions(&self, feature: &str) -> serde_json::Value {
        let _ = feature;
        serde_json::json!({ "versions": [] })
    }
    fn manual_version(&self, feature: &str, n: u32) -> Option<serde_json::Value> {
        let _ = (feature, n);
        None
    }
    /// One folder level on the engine's machine for the path picker (read only).
    fn browse(&self, path: Option<&str>, ext: Option<&str>) -> Result<serde_json::Value, String> {
        let _ = (path, ext);
        Err("이 화면에서는 폴더 찾아보기를 지원하지 않습니다".into())
    }
    /// The first-run checklist for this project.
    fn setup_steps(&self) -> Vec<SetupStep> {
        Vec::new()
    }
    /// Editable settings of this project (JSON view of `mwa.toml`).
    fn settings(&self) -> Result<serde_json::Value, String> {
        Err("이 화면에서는 설정을 지원하지 않습니다".into())
    }
    /// Validates and saves settings. The server reopens the project afterwards.
    fn save_settings(&self, settings: serde_json::Value) -> Result<(), String> {
        drop(settings);
        Err("이 화면에서는 설정을 지원하지 않습니다".into())
    }
    /// Stores a login profile's password in the user's credential file (never in the project).
    fn set_password(&self, profile: &str, password: &str) -> Result<(), String> {
        let _ = (profile, password);
        Err("이 화면에서는 비밀번호 저장을 지원하지 않습니다".into())
    }
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
        let by_name = chat_context(&m, None, "주문관리 화면 설명해줘").unwrap();
        assert!(by_name.contains("주문 관리") && by_name.contains("/api/sales/accounts"));
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
