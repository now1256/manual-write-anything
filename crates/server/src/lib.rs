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
