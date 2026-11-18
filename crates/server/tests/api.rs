//! Server end to end with a fake host: REST, WebSocket chat/commands, and access rules.

use std::sync::Arc;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use mwa_host::{Header, Host, OnDelta, Status, Who};
use mwa_productmap::map::ProductMap;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

struct FakeHost;

fn mini() -> ProductMap {
    serde_json::from_str(include_str!("../../productmap/tests/snapshots/mini.json")).unwrap()
}

#[async_trait]
impl Host for FakeHost {
    fn header(&self) -> Header {
        Header {
            workspace: "/ws".into(),
            sources: vec![],
        }
    }
    fn load_map(&self) -> Option<ProductMap> {
        Some(mini())
    }
    fn status(&self) -> Status {
        Status {
            model: "claude·sonnet".into(),
            spent_usd: 0.1,
            budget_usd: 5.0,
            quota: None,
        }
    }
    async fn chat(
        &self,
        transcript: Vec<(Who, String)>,
        context: Option<String>,
        on_delta: OnDelta,
    ) -> Result<(), String> {
        let last = &transcript.last().unwrap().1;
        on_delta("echo:");
        on_delta(last);
        // Context reaches the host when a feature is selected or named.
        on_delta(
            if context.is_some_and(|c| c.contains("/api/source/systems")) {
                "|ctx"
            } else {
                "|noctx"
            },
        );
        Ok(())
    }
    async fn build_map(&self) -> Result<(ProductMap, Vec<String>), String> {
        Ok((mini(), vec!["지도 완료".into()]))
    }
    async fn doctor(&self) -> Vec<String> {
        vec!["소스 OK".into()]
    }
    fn auth_lines(&self) -> Vec<String> {
        vec!["admin 저장됨".into()]
    }
    async fn capture_menus(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        progress("[1/2] a");
        progress("[2/2] b");
        Ok(vec!["촬영 성공 2/2".into()])
    }
    fn shots_manifest(&self) -> Option<Value> {
        Some(json!({"shots": [{"key": "a", "ok": true, "file": "shots/menus/a.png"}]}))
    }
    fn shot_file(&self, rel: &str) -> Option<std::path::PathBuf> {
        (rel == "menus/a.png").then(|| {
            let p = std::env::temp_dir().join(format!("mwa-shot-test-{}.png", std::process::id()));
            std::fs::write(&p, b"\x89PNG\r\n\x1a\nfake").unwrap();
            p
        })
    }
    fn chat_models(&self) -> Vec<String> {
        vec!["haiku".into(), "sonnet".into()]
    }
    fn set_chat_model(&self, model: &str) -> Result<(), String> {
        if model == "haiku" {
            Ok(())
        } else {
            Err(format!("쓸 수 없는 모델입니다: {model}"))
        }
    }
}

async fn start() -> String {
    let listener = mwa_server::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mwa_server::router(Arc::new(FakeHost)))
            .await
            .unwrap()
    });
    format!("127.0.0.1:{}", addr.port())
}

#[tokio::test]
async fn rest_endpoints() {
    let base = start().await;
    let c = reqwest::Client::new();
    let state: Value = c
        .get(format!("http://{base}/api/state"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["status"]["model"], "claude·sonnet");
    assert_eq!(state["has_map"], true);
    let map: Value = c
        .get(format!("http://{base}/api/map"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(map["features"].as_array().unwrap().len(), 5);
    let auth: Value = c
        .get(format!("http://{base}/api/auth"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(auth["lines"][0], "admin 저장됨");
    let index = c.get(format!("http://{base}/")).send().await.unwrap();
    assert!(
        index.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let spa = c
        .get(format!("http://{base}/features/x"))
        .send()
        .await
        .unwrap();
    assert_eq!(spa.status(), 200, "client routes fall back to index.html");
    assert_eq!(
        c.get(format!("http://{base}/missing.js"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}

#[tokio::test]
async fn shots_manifest_files_and_progress() {
    let base = start().await;
    let c = reqwest::Client::new();
    let m: Value = c
        .get(format!("http://{base}/api/shots"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(m["shots"][0]["key"], "a");
    let img = c
        .get(format!("http://{base}/api/shots/file/menus/a.png"))
        .send()
        .await
        .unwrap();
    assert_eq!(img.status(), 200);
    assert_eq!(img.headers()["content-type"], "image/png");
    let missing = c
        .get(format!("http://{base}/api/shots/file/menus/b.png"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    let req = format!("ws://{base}/api/ws").into_client_request().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    recv_until(&mut ws, "status").await;
    ws.send(Message::Text(
        json!({"type": "command", "id": 9, "name": "shot"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let msgs = recv_until(&mut ws, "shots_done").await;
    let progress: Vec<&str> = msgs
        .iter()
        .filter(|m| m["type"] == "progress")
        .map(|m| m["text"].as_str().unwrap())
        .collect();
    assert_eq!(progress, vec!["[1/2] a", "[2/2] b"]);
    assert_eq!(msgs.last().unwrap()["lines"][0], "촬영 성공 2/2");
}

#[tokio::test]
async fn chat_model_switch() {
    let base = start().await;
    let c = reqwest::Client::new();
    let state: Value = c
        .get(format!("http://{base}/api/state"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["chat_models"], json!(["haiku", "sonnet"]));
    let ok = c
        .post(format!("http://{base}/api/chat-model"))
        .json(&json!({"model": "haiku"}))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let bad = c
        .post(format!("http://{base}/api/chat-model"))
        .json(&json!({"model": "gpt"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let cross = c
        .post(format!("http://{base}/api/chat-model"))
        .header("origin", "http://evil.com")
        .json(&json!({"model": "haiku"}))
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), 403, "cross-site POST blocked");
}

#[tokio::test]
async fn foreign_origin_and_host_are_rejected() {
    let base = start().await;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("http://{base}/api/state"))
        .header("origin", "http://evil.com")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = c
        .get(format!("http://{base}/api/state"))
        .header("host", "evil.com")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = c
        .get(format!("http://{base}/api/state"))
        .header("origin", "tauri://localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn non_loopback_bind_is_refused() {
    let err = mwa_server::bind("0.0.0.0:0".parse().unwrap())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("루프백"));
}

async fn recv_until(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    ty: &str,
) -> Vec<Value> {
    let mut seen = Vec::new();
    while let Some(Ok(m)) = ws.next().await {
        if let Message::Text(t) = m {
            let v: Value = serde_json::from_str(&t).unwrap();
            let done = v["type"] == ty;
            seen.push(v);
            if done {
                break;
            }
        }
    }
    seen
}

#[tokio::test]
async fn websocket_chat_streams_and_commands_run() {
    let base = start().await;
    let req = format!("ws://{base}/api/ws").into_client_request().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();

    let first = recv_until(&mut ws, "status").await;
    assert_eq!(first[0]["status"]["budget_usd"], 5.0);

    let chat = json!({"type": "chat", "id": 1, "selected": null,
        "transcript": [{"who": "user", "text": "연결관리 알려줘"}]});
    ws.send(Message::Text(chat.to_string().into()))
        .await
        .unwrap();
    let msgs = recv_until(&mut ws, "chat_done").await;
    let text: String = msgs
        .iter()
        .filter(|m| m["type"] == "delta")
        .map(|m| m["text"].as_str().unwrap())
        .collect();
    assert_eq!(text, "echo:연결관리 알려줘|ctx");

    let cmd = json!({"type": "command", "id": 2, "name": "map"});
    ws.send(Message::Text(cmd.to_string().into()))
        .await
        .unwrap();
    let msgs = recv_until(&mut ws, "map_built").await;
    assert_eq!(msgs.last().unwrap()["lines"][0], "지도 완료");

    ws.send(Message::Text(
        json!({"type": "command", "id": 3, "name": "nope"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let msgs = recv_until(&mut ws, "error").await;
    assert_eq!(msgs.last().unwrap()["id"], 3);

    ws.send(Message::Text("not json".into())).await.unwrap();
    let msgs = recv_until(&mut ws, "error").await;
    assert!(
        msgs.last().unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("잘못된 메시지")
    );
}
