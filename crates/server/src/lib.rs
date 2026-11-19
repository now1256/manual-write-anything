//! `mwa serve`: the engine behind a local HTTP + WebSocket API and the embedded web UI.
//! Listens on loopback only; remote clients come in through an SSH tunnel (D-026).

mod protocol;
mod ws;

use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use axum::routing::get_service;
use mwa_host::{Host, Projects};
use rust_embed::RustEmbed;
use serde_json::json;

pub use protocol::{ClientMsg, ServerMsg, TranscriptItem};

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct WebAssets;

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("{0}은 루프백 주소가 아닙니다. mwa serve는 127.0.0.1에서만 열고 SSH 터널로 접속합니다")]
    NotLoopback(SocketAddr),
    #[error("{0}에서 대기할 수 없습니다: {1}")]
    Bind(SocketAddr, std::io::Error),
    #[error("서버 오류: {0}")]
    Io(#[from] std::io::Error),
}

/// The open project's engine, swappable when the user switches projects.
pub struct Hub {
    host: std::sync::RwLock<Arc<dyn Host>>,
    projects: Option<Arc<dyn Projects>>,
}

impl Hub {
    pub fn new(host: Arc<dyn Host>, projects: Option<Arc<dyn Projects>>) -> Arc<Self> {
        Arc::new(Self {
            host: std::sync::RwLock::new(host),
            projects,
        })
    }

    pub fn host(&self) -> Arc<dyn Host> {
        self.host.read().expect("hub lock").clone()
    }

    /// Reopens the current project so saved settings take effect.
    fn reload(&self) -> Result<(), String> {
        let current = self.host().header().workspace;
        self.open(&current)
    }

    fn open(&self, path: &str) -> Result<(), String> {
        let projects = self
            .projects
            .as_ref()
            .ok_or("프로젝트 전환을 지원하지 않습니다")?;
        if self.host().busy() {
            return Err("촬영이나 매뉴얼 생성이 진행 중입니다. 끝난 뒤에 전환하세요".into());
        }
        let host = projects.open(path)?;
        *self.host.write().expect("hub lock") = host;
        Ok(())
    }
}

pub fn router(host: Arc<dyn Host>) -> Router {
    router_with(Hub::new(host, None), None)
}

/// With `web_dir`, the UI is read from disk on every request instead of the embedded copy,
/// so a rebuilt UI shows up on browser refresh without restarting the server (development).
pub fn router_with(hub: Arc<Hub>, web_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/api/state", get(state))
        .route("/api/map", get(map))
        .route("/api/auth", get(auth))
        .route("/api/shots", get(shots))
        .route("/api/manual/{key}", get(manual))
        .route("/api/shots/file/{*path}", get(shot_file))
        .route("/api/chat-model", axum::routing::post(set_chat_model))
        .route("/api/projects", get(projects).post(create_project))
        .route("/api/settings", get(settings).put(save_settings))
        .route("/api/setup", get(setup))
        .route("/api/fs", get(browse))
        .route("/api/references", get(references))
        .route("/api/references/shots/{feature}", get(reference_shots))
        .route("/api/docs/{feature}", get(document).put(save_document))
        .route(
            "/api/docs/{feature}/assets",
            axum::routing::post(save_document_asset)
                .layer(axum::extract::DefaultBodyLimit::max(20 * 1024 * 1024)),
        )
        .route("/api/docs/{feature}/assets/{file}", get(document_asset))
        .route("/api/docs/{feature}/versions", get(document_versions))
        .route("/api/docs/{feature}/versions/{n}", get(document_version))
        .route("/api/docs/{feature}/proposals", get(proposals))
        .route("/api/docs/{feature}/assist", axum::routing::post(assist))
        .route("/api/docs/{feature}/provenance", get(provenance))
        .route("/api/docs/{feature}/grounding", get(grounding))
        .route(
            "/api/docs/{feature}/confluence",
            get(confluence_status).post(confluence_publish),
        )
        .route(
            "/api/confluence/setup",
            axum::routing::post(confluence_setup),
        )
        .route("/api/manuals", get(manuals_status))
        .route(
            "/api/docs/{feature}/restore/{n}",
            axum::routing::post(restore_document),
        )
        .route(
            "/api/references/shots/{feature}/choice",
            axum::routing::post(scene_choice),
        )
        .route("/api/references/link", axum::routing::post(link_reference))
        .route(
            "/api/references/upload",
            axum::routing::post(upload_reference)
                .layer(axum::extract::DefaultBodyLimit::max(200 * 1024 * 1024)),
        )
        .route(
            "/api/docs/{feature}/proposals/{id}",
            axum::routing::delete(resolve_proposal),
        )
        .route(
            "/api/references/shots/{feature}/upload",
            axum::routing::post(scene_upload)
                .delete(scene_unupload)
                .layer(axum::extract::DefaultBodyLimit::max(20 * 1024 * 1024)),
        )
        .route("/api/manual/{key}/versions", get(manual_versions))
        .route("/api/manual/{key}/versions/{n}", get(manual_version))
        .route(
            "/api/references/import",
            axum::routing::post(import_references),
        )
        .route("/api/references/{id}", get(reference).post(set_reference))
        .route("/api/references/{id}/file/{*path}", get(reference_file))
        .route("/api/references/{id}/page/{n}", get(reference_page))
        .route("/api/check", axum::routing::post(check))
        .route("/api/settings/password", axum::routing::post(set_password))
        .route("/api/projects/open", axum::routing::post(open_project))
        .route("/api/ws", get(ws::upgrade));
    let app = match web_dir {
        Some(dir) => api.fallback_service(get_service(axum::routing::any(move |uri: Uri| {
            let dir = dir.clone();
            async move { disk_file(&dir, &uri) }
        }))),
        None => api.fallback(static_file),
    };
    app.layer(middleware::from_fn(local_only)).with_state(hub)
}

/// Binds `addr` (loopback only) and serves until the process ends.
pub async fn serve(host: Arc<dyn Host>, addr: SocketAddr) -> Result<(), ServeError> {
    let listener = bind(addr).await?;
    axum::serve(listener, router(host)).await?;
    Ok(())
}

/// Serves on an already bound listener (see [`bind`]).
pub async fn serve_listener(
    listener: tokio::net::TcpListener,
    hub: Arc<Hub>,
    web_dir: Option<PathBuf>,
) -> Result<(), ServeError> {
    axum::serve(listener, router_with(hub, web_dir)).await?;
    Ok(())
}

pub async fn bind(addr: SocketAddr) -> Result<tokio::net::TcpListener, ServeError> {
    if !addr.ip().is_loopback() {
        return Err(ServeError::NotLoopback(addr));
    }
    tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| ServeError::Bind(addr, e))
}

fn is_local_host(host: &str) -> bool {
    let name = host.rsplit_once(':').map_or(host, |(h, p)| {
        if p.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    matches!(
        name,
        "localhost" | "127.0.0.1" | "[::1]" | "tauri.localhost"
    )
}

/// Allowed browser origins: the page itself on a loopback host, or the Tauri webview.
fn is_allowed_origin(origin: &str) -> bool {
    if origin == "tauri://localhost" {
        return true;
    }
    match origin.split_once("://") {
        Some(("http" | "https", rest)) => is_local_host(rest),
        _ => false,
    }
}

/// Blocks DNS-rebinding (foreign Host header) and cross-site requests (foreign Origin).
async fn local_only(headers: HeaderMap, req: Request, next: Next) -> Response {
    let host_ok = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_none_or(is_local_host);
    let origin_ok = headers
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .is_none_or(is_allowed_origin);
    if host_ok && origin_ok {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "local access only").into_response()
    }
}

async fn state(State(hub): State<Arc<Hub>>) -> Json<serde_json::Value> {
    let host = hub.host();
    Json(json!({
        "header": host.header(),
        "status": host.status(),
        "has_map": host.load_map().is_some(),
        "chat_models": host.chat_models(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn map(State(hub): State<Arc<Hub>>) -> Response {
    let host = hub.host();
    match host.load_map() {
        Some(m) => Json(m).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "제품 지도가 없습니다. /map 으로 만드세요"})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct ModelChoice {
    model: String,
}

async fn set_chat_model(State(hub): State<Arc<Hub>>, Json(body): Json<ModelChoice>) -> Response {
    let host = hub.host();
    match host.set_chat_model(&body.model) {
        Ok(()) => Json(json!({ "status": host.status() })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn shots(State(hub): State<Arc<Hub>>) -> Response {
    let host = hub.host();
    match host.shots_manifest() {
        Some(m) => Json(m).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "아직 촬영하지 않았습니다. /shot 으로 촬영하세요"})),
        )
            .into_response(),
    }
}

async fn manual(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> Response {
    let host = hub.host();
    match host.manual_run(&key) {
        Some(m) => Json(m).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "아직 이 기능의 매뉴얼이 없습니다. /gen 으로 만드세요"})),
        )
            .into_response(),
    }
}

async fn shot_file(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Response {
    let host = hub.host();
    match host.shot_file(&path).and_then(|p| std::fs::read(p).ok()) {
        Some(data) => file_response(&path, data),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn auth(State(hub): State<Arc<Hub>>) -> Json<serde_json::Value> {
    let host = hub.host();
    Json(json!({ "lines": host.auth_lines() }))
}

async fn projects(State(hub): State<Arc<Hub>>) -> Json<serde_json::Value> {
    Json(json!({
        "current": hub.host().header().workspace,
        "items": hub.projects.as_ref().map(|p| p.list()).unwrap_or_default(),
        "switchable": hub.projects.is_some(),
    }))
}

#[derive(serde::Deserialize)]
struct NewProject {
    name: String,
    #[serde(default)]
    path: Option<String>,
}

async fn create_project(State(hub): State<Arc<Hub>>, Json(body): Json<NewProject>) -> Response {
    let Some(projects) = hub.projects.clone() else {
        return bad_request("프로젝트를 만들 수 없습니다");
    };
    let path = body
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty());
    match projects.create(body.name.trim(), path) {
        Ok(entry) => Json(entry).into_response(),
        Err(e) => bad_request(&e),
    }
}

#[derive(serde::Deserialize)]
struct OpenProject {
    path: String,
}

async fn open_project(State(hub): State<Arc<Hub>>, Json(body): Json<OpenProject>) -> Response {
    match hub.open(body.path.trim()) {
        Ok(()) => Json(json!({ "current": hub.host().header().workspace })).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn settings(State(hub): State<Arc<Hub>>) -> Response {
    match hub.host().settings() {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn save_settings(
    State(hub): State<Arc<Hub>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if let Err(e) = hub.host().save_settings(body) {
        return bad_request(&e);
    }
    let note = match hub.reload() {
        Ok(()) => "저장하고 다시 불러왔습니다".to_string(),
        Err(e) => format!("저장했습니다. 다시 불러오지 못해 서버를 다시 켜야 반영됩니다: {e}"),
    };
    Json(json!({ "note": note })).into_response()
}

#[derive(serde::Deserialize)]
struct PasswordBody {
    profile: String,
    password: String,
}

async fn set_password(State(hub): State<Arc<Hub>>, Json(body): Json<PasswordBody>) -> Response {
    match hub.host().set_password(&body.profile, &body.password) {
        Ok(()) => {
            Json(json!({ "note": "저장했습니다 (사용자 자격 파일, 권한 600)" })).into_response()
        }
        Err(e) => bad_request(&e),
    }
}

#[derive(serde::Deserialize)]
struct BrowseQuery {
    path: Option<String>,
    ext: Option<String>,
}

async fn browse(
    State(hub): State<Arc<Hub>>,
    axum::extract::Query(q): axum::extract::Query<BrowseQuery>,
) -> Response {
    match hub.host().browse(q.path.as_deref(), q.ext.as_deref()) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn reference_shots(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
) -> Response {
    match hub.host().reference_shots(&feature) {
        Some(v) => Json(v).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn document(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
) -> Response {
    match hub.host().document(&feature) {
        Some(v) => Json(v).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(serde::Deserialize)]
struct DocBody {
    html: String,
    #[serde(default)]
    changes: serde_json::Value,
    /// Version the edits started from (refused when the document has moved on).
    #[serde(default)]
    base: Option<u32>,
}

async fn save_document(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
    Json(b): Json<DocBody>,
) -> Response {
    let changes = if b.changes.is_null() {
        json!([])
    } else {
        b.changes
    };
    if let (Some(base), Some(cur)) = (
        b.base,
        hub.host()
            .document(&feature)
            .and_then(|d| d["version"].as_u64()),
    ) && u64::from(base) != cur
    {
        return bad_request(&format!(
            "문서가 그사이 바뀌어 저장하지 않았습니다 (지금 버전 {cur}, 편집은 버전 {base} 기준). 새 버전을 불러와 다시 고치세요"
        ));
    }
    match hub.host().save_document(&feature, &b.html, changes) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

#[derive(serde::Deserialize)]
struct AssetQuery {
    name: String,
}

/// Raw picture bytes in the body, file name in `?name=`.
async fn save_document_asset(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<AssetQuery>,
    body: axum::body::Bytes,
) -> Response {
    match hub.host().save_document_asset(&feature, &q.name, &body) {
        Ok(file) => Json(json!({ "file": file })).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn document_asset(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path((feature, file)): axum::extract::Path<(String, String)>,
) -> Response {
    match hub
        .host()
        .document_asset(&feature, &file)
        .and_then(|p| std::fs::read(p).ok())
    {
        Some(data) => file_response(&file, data),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn document_versions(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    Json(hub.host().document_versions(&feature))
}

async fn document_version(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path((feature, n)): axum::extract::Path<(String, u32)>,
) -> Response {
    match hub.host().document_version(&feature, n) {
        Some(v) => Json(v).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn manuals_status(State(hub): State<Arc<Hub>>) -> Json<serde_json::Value> {
    Json(hub.host().manuals_status())
}

async fn confluence_status(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    Json(hub.host().confluence_status(&feature))
}

async fn confluence_setup(
    State(hub): State<Arc<Hub>>,
    Json(b): Json<serde_json::Value>,
) -> Response {
    match hub.host().confluence_setup(b).await {
        Ok(found) => Json(json!({ "found": found })).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn confluence_publish(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
    Json(b): Json<serde_json::Value>,
) -> Response {
    match hub.host().confluence_publish(&feature, b).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn grounding(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    Json(hub.host().grounding(&feature))
}

async fn provenance(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    Json(hub.host().provenance(&feature))
}

async fn proposals(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    Json(hub.host().proposals(&feature))
}

async fn resolve_proposal(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path((feature, id)): axum::extract::Path<(String, String)>,
) -> Response {
    match hub.host().resolve_proposal(&feature, &id) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

#[derive(serde::Deserialize)]
struct SceneUploadQuery {
    image: String,
    #[serde(default)]
    name: String,
}

/// Raw picture bytes in the body; `?image=` is the old manual's picture it stands in for.
async fn scene_upload(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<SceneUploadQuery>,
    body: axum::body::Bytes,
) -> Response {
    match hub
        .host()
        .set_scene_upload(&feature, &q.image, Some((&q.name, &body)))
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn scene_unupload(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<SceneUploadQuery>,
) -> Response {
    match hub.host().set_scene_upload(&feature, &q.image, None) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn restore_document(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path((feature, n)): axum::extract::Path<(String, u32)>,
) -> Response {
    match hub.host().restore_document(&feature, n) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

#[derive(serde::Deserialize)]
struct SceneChoice {
    image: String,
    #[serde(default)]
    choice: Option<String>,
}

async fn scene_choice(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
    Json(b): Json<SceneChoice>,
) -> Response {
    match hub.host().set_scene_choice(&feature, &b.image, b.choice) {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

#[derive(serde::Deserialize)]
struct LinkBody {
    feature: String,
    id: String,
}

async fn link_reference(State(hub): State<Arc<Hub>>, Json(b): Json<LinkBody>) -> Response {
    match hub.host().link_reference(&b.feature, &b.id) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => bad_request(&e),
    }
}

#[derive(serde::Deserialize)]
struct UploadQuery {
    feature: String,
    name: String,
}

/// Raw PDF bytes in the body.
async fn upload_reference(
    State(hub): State<Arc<Hub>>,
    axum::extract::Query(q): axum::extract::Query<UploadQuery>,
    body: axum::body::Bytes,
) -> Response {
    match hub
        .host()
        .upload_reference(&q.feature, &q.name, body.to_vec())
        .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn assist(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(feature): axum::extract::Path<String>,
    Json(b): Json<serde_json::Value>,
) -> Response {
    match hub.host().assist(&feature, b).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn manual_versions(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    Json(hub.host().manual_versions(&key))
}

async fn manual_version(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path((key, n)): axum::extract::Path<(String, u32)>,
) -> Response {
    match hub.host().manual_version(&key, n) {
        Some(v) => Json(v).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn references(State(hub): State<Arc<Hub>>) -> Response {
    match hub.host().references() {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn reference(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    match hub.host().reference(&id) {
        Some(v) => Json(v).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(serde::Deserialize)]
struct ImportBody {
    paths: Vec<String>,
}

async fn import_references(State(hub): State<Arc<Hub>>, Json(b): Json<ImportBody>) -> Response {
    match hub.host().import_references(b.paths).await {
        Ok(lines) => Json(json!({ "lines": lines })).into_response(),
        Err(e) => bad_request(&e),
    }
}

#[derive(serde::Deserialize)]
struct RefChoice {
    #[serde(default)]
    feature: Option<String>,
    #[serde(default)]
    ignored: bool,
}

async fn set_reference(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(b): Json<RefChoice>,
) -> Response {
    match hub.host().set_reference(&id, b.feature, b.ignored) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => bad_request(&e),
    }
}

async fn reference_file(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path((id, path)): axum::extract::Path<(String, String)>,
) -> Response {
    match hub
        .host()
        .reference_file(&id, &path)
        .and_then(|p| std::fs::read(p).ok())
    {
        Some(data) => file_response(&path, data),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn reference_page(
    State(hub): State<Arc<Hub>>,
    axum::extract::Path((id, n)): axum::extract::Path<(String, u32)>,
) -> Response {
    match hub
        .host()
        .reference_page(&id, n)
        .and_then(|p| std::fs::read(p).map_err(|e| e.to_string()))
    {
        Ok(data) => file_response("page.png", data),
        Err(e) => bad_request(&e),
    }
}

async fn setup(State(hub): State<Arc<Hub>>) -> Json<serde_json::Value> {
    Json(json!({ "steps": hub.host().setup_steps() }))
}

#[derive(serde::Deserialize)]
struct CheckBody {
    kind: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    extra: String,
}

async fn check(State(hub): State<Arc<Hub>>, Json(b): Json<CheckBody>) -> Json<mwa_host::Check> {
    Json(hub.host().check(&b.kind, &b.value, &b.extra).await)
}

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
}

fn file_response(path: &str, data: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    ([(header::CONTENT_TYPE, mime.as_ref().to_string())], data).into_response()
}

/// Static file from `dir`; only plain relative paths (no `..`, no absolute) are served.
fn disk_file(dir: &Path, uri: &Uri) -> Response {
    let rel = uri.path().trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let rel_path = Path::new(rel);
    if !rel_path
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match std::fs::read(dir.join(rel_path)) {
        Ok(data) => file_response(rel, data),
        Err(_) if !rel.contains('.') => match std::fs::read(dir.join("index.html")) {
            Ok(data) => file_response("index.html", data),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        },
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let file = WebAssets::get(path).map(|f| (path, f)).or_else(|| {
        // Client-side routes fall back to the app shell; missing assets stay 404.
        (!path.contains('.')).then(|| WebAssets::get("index.html").map(|f| ("index.html", f)))?
    });
    match file {
        Some((p, f)) => file_response(p, f.data.into_owned()),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_file_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("mwa-web-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "shell").unwrap();
        std::fs::write(dir.join("assets/a.js"), "js").unwrap();
        let get = |p: &str| disk_file(&dir, &p.parse::<Uri>().unwrap()).status();
        assert_eq!(get("/"), StatusCode::OK);
        assert_eq!(get("/assets/a.js"), StatusCode::OK);
        assert_eq!(
            get("/some/route"),
            StatusCode::OK,
            "client routes get the shell"
        );
        assert_eq!(get("/missing.css"), StatusCode::NOT_FOUND);
        assert_eq!(get("/../../etc/passwd"), StatusCode::NOT_FOUND);
        assert_eq!(get("/assets/../../x"), StatusCode::NOT_FOUND);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn origin_and_host_rules() {
        for ok in [
            "http://localhost:7700",
            "http://127.0.0.1:5173",
            "tauri://localhost",
            "http://tauri.localhost",
        ] {
            assert!(is_allowed_origin(ok), "{ok}");
        }
        for bad in [
            "http://evil.com",
            "http://localhost.evil.com",
            "null",
            "file://",
            "http://192.168.0.10:7700",
        ] {
            assert!(!is_allowed_origin(bad), "{bad}");
        }
        assert!(
            is_local_host("localhost:7700")
                && is_local_host("[::1]:7700")
                && is_local_host("127.0.0.1")
        );
        assert!(!is_local_host("192.168.0.10:7700") && !is_local_host("evil.com"));
    }
}
