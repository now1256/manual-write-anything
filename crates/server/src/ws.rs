use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use mwa_host::{Host, Who, chat_context};
use tokio::sync::mpsc;

use crate::protocol::{ClientMsg, ServerMsg};

pub async fn upgrade(ws: WebSocketUpgrade, State(hub): State<Arc<crate::Hub>>) -> Response {
    ws.on_upgrade(move |socket| session(socket, hub))
}

/// Each message goes to the engine open at that moment: saving the settings reopens it, and
/// a command sent right after must see the new settings.
async fn session(socket: WebSocket, hub: Arc<crate::Hub>) {
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
        status: hub.host().status(),
    });

    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else { continue };
        match serde_json::from_str::<ClientMsg>(&text) {
            Ok(m) => handle(m, &hub.host(), &tx),
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
                    "shotref" => match arg {
                        None => ServerMsg::Error {
                            id,
                            message: "어떤 기능의 장면을 찍을지 정해지지 않았습니다".into(),
                        },
                        Some(feature) => {
                            let progress_tx = tx.clone();
                            let progress = Box::new(move |t: &str| {
                                let _ = progress_tx.send(ServerMsg::Progress {
                                    id,
                                    text: t.to_string(),
                                });
                            });
                            match host.capture_reference(&feature, progress).await {
                                Ok(lines) => ServerMsg::Lines {
                                    id,
                                    title: "기존 매뉴얼 장면 촬영".into(),
                                    lines,
                                },
                                Err(message) => ServerMsg::Error { id, message },
                            }
                        }
                    },
                    "auto" => match arg {
                        None => ServerMsg::Error {
                            id,
                            message: "어떤 자동 작업을 돌릴지 정해지지 않았습니다".into(),
                        },
                        Some(rule) => {
                            let progress_tx = tx.clone();
                            let progress = Box::new(move |t: &str| {
                                let _ = progress_tx.send(ServerMsg::Progress {
                                    id,
                                    text: t.to_string(),
                                });
                            });
                            match host.run_automation(&rule, progress).await {
                                Ok(lines) => ServerMsg::Lines {
                                    id,
                                    title: "자동 작업".into(),
                                    lines,
                                },
                                Err(message) => ServerMsg::Error { id, message },
                            }
                        }
                    },
                    "adapter" => {
                        let progress_tx = tx.clone();
                        let progress = Box::new(move |t: &str| {
                            let _ = progress_tx.send(ServerMsg::Progress {
                                id,
                                text: t.to_string(),
                            });
                        });
                        match host.make_adapter(progress).await {
                            Ok(lines) => ServerMsg::Lines {
                                id,
                                title: "코드 읽고 제품 파악".into(),
                                lines,
                            },
                            Err(message) => ServerMsg::Error { id, message },
                        }
                    }
                    "shotdoc" => match arg {
                        None => ServerMsg::Error {
                            id,
                            message: "어떤 기능의 그림 자리를 찍을지 정해지지 않았습니다".into(),
                        },
                        Some(feature) => {
                            let progress_tx = tx.clone();
                            let progress = Box::new(move |t: &str| {
                                let _ = progress_tx.send(ServerMsg::Progress {
                                    id,
                                    text: t.to_string(),
                                });
                            });
                            match host.capture_document(&feature, progress).await {
                                Ok(lines) => ServerMsg::Lines {
                                    id,
                                    title: "그림 자리 촬영".into(),
                                    lines,
                                },
                                Err(message) => ServerMsg::Error { id, message },
                            }
                        }
                    },
                    "docmake" => match arg {
                        None => ServerMsg::Error {
                            id,
                            message: "어떤 기능의 매뉴얼을 만들지 정해지지 않았습니다".into(),
                        },
                        Some(feature) => {
                            let progress_tx = tx.clone();
                            let progress = Box::new(move |t: &str| {
                                let _ = progress_tx.send(ServerMsg::Progress {
                                    id,
                                    text: t.to_string(),
                                });
                            });
                            match host.make_document(&feature, progress).await {
                                Ok(lines) => ServerMsg::Lines {
                                    id,
                                    title: "매뉴얼 만들기".into(),
                                    lines,
                                },
                                Err(message) => ServerMsg::Error { id, message },
                            }
                        }
                    },
                    "docground" => match arg {
                        None => ServerMsg::Error {
                            id,
                            message: "어떤 기능의 근거를 확인할지 정해지지 않았습니다".into(),
                        },
                        Some(feature) => {
                            let progress_tx = tx.clone();
                            let progress = Box::new(move |t: &str| {
                                let _ = progress_tx.send(ServerMsg::Progress {
                                    id,
                                    text: t.to_string(),
                                });
                            });
                            match host.ground_document(&feature, progress).await {
                                Ok(lines) => ServerMsg::Lines {
                                    id,
                                    title: "근거 확인".into(),
                                    lines,
                                },
                                Err(message) => ServerMsg::Error { id, message },
                            }
                        }
                    },
                    "docupdate" => match arg {
                        None => ServerMsg::Error {
                            id,
                            message: "어떤 기능의 매뉴얼을 비교할지 정해지지 않았습니다".into(),
                        },
                        Some(feature) => {
                            let progress_tx = tx.clone();
                            let progress = Box::new(move |t: &str| {
                                let _ = progress_tx.send(ServerMsg::Progress {
                                    id,
                                    text: t.to_string(),
                                });
                            });
                            match host.propose_document(&feature, progress).await {
                                Ok(lines) => ServerMsg::Lines {
                                    id,
                                    title: "갱신 제안".into(),
                                    lines,
                                },
                                Err(message) => ServerMsg::Error { id, message },
                            }
                        }
                    },
                    "docref" => match arg {
                        None => ServerMsg::Error {
                            id,
                            message: "어떤 기능의 기존 매뉴얼을 옮길지 정해지지 않았습니다".into(),
                        },
                        Some(feature) => {
                            let progress_tx = tx.clone();
                            let progress = Box::new(move |t: &str| {
                                let _ = progress_tx.send(ServerMsg::Progress {
                                    id,
                                    text: t.to_string(),
                                });
                            });
                            match host.import_document(&feature, progress).await {
                                Ok(lines) => ServerMsg::Lines {
                                    id,
                                    title: "매뉴얼 문서".into(),
                                    lines,
                                },
                                Err(message) => ServerMsg::Error { id, message },
                            }
                        }
                    },
                    "describe" => match arg {
                        None => ServerMsg::Error {
                            id,
                            message: "어떤 기존 매뉴얼을 설명할지 정해지지 않았습니다".into(),
                        },
                        Some(doc) => {
                            let progress_tx = tx.clone();
                            let progress = Box::new(move |t: &str| {
                                let _ = progress_tx.send(ServerMsg::Progress {
                                    id,
                                    text: t.to_string(),
                                });
                            });
                            match host.describe_reference(&doc, progress).await {
                                Ok(lines) => ServerMsg::Lines {
                                    id,
                                    title: "장면 설명".into(),
                                    lines,
                                },
                                Err(message) => ServerMsg::Error { id, message },
                            }
                        }
                    },
                    "browser" => {
                        let progress_tx = tx.clone();
                        let progress = Box::new(move |t: &str| {
                            let _ = progress_tx.send(ServerMsg::Progress {
                                id,
                                text: t.to_string(),
                            });
                        });
                        match host.install_browser(progress).await {
                            Ok(lines) => ServerMsg::Lines {
                                id,
                                title: "브라우저 설치".into(),
                                lines,
                            },
                            Err(message) => ServerMsg::Error { id, message },
                        }
                    }
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
