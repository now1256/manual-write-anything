use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use mwa_host::{Host, Who, chat_context};
use tokio::sync::mpsc;

use crate::protocol::{ClientMsg, ServerMsg};

pub async fn upgrade(ws: WebSocketUpgrade, State(hub): State<Arc<crate::Hub>>) -> Response {
    // A session stays with the project that was open when it connected; the UI reloads
    // after switching projects.
    let host = hub.host();
    ws.on_upgrade(move |socket| session(socket, host))
}

async fn session(socket: WebSocket, host: Arc<dyn Host>) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let text = serde_json::to_string(&msg).expect("server messages serialize");
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    let _ = tx.send(ServerMsg::Status {
        status: host.status(),
    });

    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else { continue };
        match serde_json::from_str::<ClientMsg>(&text) {
            Ok(m) => handle(m, &host, &tx),
            Err(e) => {
                let _ = tx.send(ServerMsg::Error {
                    id: 0,
                    message: format!("잘못된 메시지: {e}"),
                });
            }
        }
    }
    drop(tx);
    let _ = writer.await;
}

fn handle(msg: ClientMsg, host: &Arc<dyn Host>, tx: &mpsc::UnboundedSender<ServerMsg>) {
    let (host, tx) = (host.clone(), tx.clone());
    match msg {
        ClientMsg::Chat {
            id,
            transcript,
            selected,
        } => {
            tokio::spawn(async move {
                let last_user = transcript
                    .iter()
                    .rev()
                    .find(|t| t.who == Who::User)
                    .map(|t| t.text.clone())
                    .unwrap_or_default();
                let context = host
                    .load_map()
                    .and_then(|m| chat_context(&m, selected.as_deref(), &last_user));
                let delta_tx = tx.clone();
                let on_delta = Box::new(move |t: &str| {
                    let _ = delta_tx.send(ServerMsg::Delta {
                        id,
                        text: t.to_string(),
                    });
                });
                let items = transcript.into_iter().map(|t| (t.who, t.text)).collect();
                let done = match host.chat(items, context, on_delta).await {
                    Ok(()) => ServerMsg::ChatDone { id },
                    Err(message) => ServerMsg::Error { id, message },
                };
                let _ = tx.send(done);
                let _ = tx.send(ServerMsg::Status {
                    status: host.status(),
                });
            });
        }
        ClientMsg::Command { id, name, arg } => {
            tokio::spawn(async move {
                let msg = match name.as_str() {
                    "map" => match host.build_map().await {
                        Ok((_, lines)) => ServerMsg::MapBuilt { id, lines },
                        Err(message) => ServerMsg::Error { id, message },
                    },
                    "sync" => {
                        let progress_tx = tx.clone();
                        let progress = Box::new(move |t: &str| {
                            let _ = progress_tx.send(ServerMsg::Progress {
                                id,
                                text: t.to_string(),
                            });
                        });
                        match host.sync_sources(progress).await {
                            Ok(lines) => ServerMsg::Lines {
                                id,
                                title: "소스 동기화".into(),
                                lines,
                            },
                            Err(message) => ServerMsg::Error { id, message },
                        }
                    }
                    "doctor" => ServerMsg::Lines {
                        id,
                        title: "점검 결과".into(),
                        lines: host.doctor().await,
                    },
                    "gen" => match arg {
                        None => ServerMsg::Error {
                            id,
                            message: "어떤 기능의 매뉴얼을 만들지 기능을 먼저 고르세요".into(),
                        },
                        Some(key) => {
                            let progress_tx = tx.clone();
                            let progress = Box::new(move |t: &str| {
                                let _ = progress_tx.send(ServerMsg::Progress {
                                    id,
                                    text: t.to_string(),
                                });
                            });
                            match host.generate_manual(&key, progress).await {
                                Ok(lines) => ServerMsg::ManualDone { id, key, lines },
                                Err(message) => ServerMsg::Error { id, message },
                            }
                        }
                    },
                    "shot" => {
                        let progress_tx = tx.clone();
                        let progress = Box::new(move |t: &str| {
                            let _ = progress_tx.send(ServerMsg::Progress {
                                id,
                                text: t.to_string(),
                            });
                        });
                        match host.capture_menus(progress).await {
                            Ok(lines) => ServerMsg::ShotsDone { id, lines },
                            Err(message) => ServerMsg::Error { id, message },
                        }
                    }
                    "auth" => ServerMsg::Lines {
                        id,
                        title: "로그인 프로파일".into(),
                        lines: host.auth_lines(),
                    },
                    other => ServerMsg::Error {
                        id,
                        message: format!("알 수 없는 명령 /{other}"),
                    },
                };
                let _ = tx.send(msg);
                let _ = tx.send(ServerMsg::Status {
                    status: host.status(),
                });
            });
        }
    }
}
