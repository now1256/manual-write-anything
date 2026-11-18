//! Full-screen terminal UI (Codex/Aider style): feature list, chat, commands.

pub mod app;
pub mod ui;

pub use mwa_host as host;

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc;

use app::{Action, App, AppEvent, Command, HELP};
use host::Host;

/// Runs the UI until the user quits. Restores the terminal on exit and on panic.
pub async fn run(host: Arc<dyn Host>) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, host).await;
    ratatui::restore();
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    host: Arc<dyn Host>,
) -> std::io::Result<()> {
    let mut app = App::new(host.header(), host.load_map(), host.status());
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));

    while !app.quit {
        terminal.draw(|f| ui::draw(f, &app))?;
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(Event::Key(key))) => {
                    if let Some(action) = app.handle_key(key) {
                        dispatch(&mut app, action, &host, &tx);
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e),
                None => break,
            },
            Some(ev) = rx.recv() => app.apply(ev),
            _ = tick.tick() => {
                if app.busy.is_some() {
                    app.spinner = app.spinner.wrapping_add(1);
                }
            }
        }
    }
    Ok(())
}

fn dispatch(
    app: &mut App,
    action: Action,
    host: &Arc<dyn Host>,
    tx: &mpsc::UnboundedSender<AppEvent>,
) {
    match action {
        Action::Chat => {
            let (host, tx) = (host.clone(), tx.clone());
            let transcript = app.transcript();
            let context = app.context();
            tokio::spawn(async move {
                let delta_tx = tx.clone();
                let on_delta: host::OnDelta = Box::new(move |t| {
                    let _ = delta_tx.send(AppEvent::Delta(t.to_string()));
                });
                let ev = match host.chat(transcript, context, on_delta).await {
                    Ok(()) => AppEvent::ChatDone,
                    Err(e) => AppEvent::ChatFailed(e),
                };
                let _ = tx.send(ev);
                let _ = tx.send(AppEvent::Status(host.status()));
            });
        }
        Action::Run(cmd) => run_command(app, cmd, host, tx),
    }
}

fn run_command(
    app: &mut App,
    cmd: Command,
    host: &Arc<dyn Host>,
    tx: &mpsc::UnboundedSender<AppEvent>,
) {
    let busy_refusal = |app: &mut App| {
        if let Some(what) = &app.busy {
            let msg = format!("{what}이 끝난 뒤에 실행해 주세요.");
            app.error(msg);
            true
        } else {
            false
        }
    };
    match cmd {
        Command::Help => app.system(HELP.join("\n")),
        Command::Clear => app.msgs.clear(),
        Command::Quit => app.quit = true,
        Command::Auth => {
            let lines = host.auth_lines();
            app.apply(AppEvent::Lines {
                title: "로그인 프로파일".into(),
                lines,
            });
        }
        Command::Unknown(c) => app.error(format!("알 수 없는 명령 /{c}. /help 를 보세요.")),
        Command::Map => {
            if busy_refusal(app) {
                return;
            }
            app.busy = Some("제품 지도 생성".into());
            let (host, tx) = (host.clone(), tx.clone());
            tokio::spawn(async move {
                let ev = match host.build_map().await {
                    Ok((map, lines)) => AppEvent::MapBuilt(Box::new(map), lines),
                    Err(e) => AppEvent::Failed(format!("지도 생성 실패: {e}")),
                };
                let _ = tx.send(ev);
            });
        }
        Command::Doctor => {
            if busy_refusal(app) {
                return;
            }
            app.busy = Some("점검".into());
            let (host, tx) = (host.clone(), tx.clone());
            tokio::spawn(async move {
                let lines = host.doctor().await;
                let _ = tx.send(AppEvent::Lines {
                    title: "점검 결과".into(),
                    lines,
                });
                let _ = tx.send(AppEvent::Status(host.status()));
            });
        }
    }
}

/// Renders the initial screen into text (for `mwa ui --snapshot` and tests).
pub fn snapshot(host: &dyn Host, width: u16, height: u16, open_feature: bool) -> String {
    let mut app = App::new(host.header(), host.load_map(), host.status());
    if open_feature && let Some(app::Row::Feature(i)) = app.rows.get(app.sel).cloned() {
        app.pane = app::Pane::Feature(i);
        app.focus = app::Focus::Features;
    }
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal.draw(|f| ui::draw(f, &app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..height {
        let mut line = String::new();
        let mut skip = 0;
        for x in 0..width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            line.push_str(sym);
            // Wide glyphs occupy two cells; the next cell is a placeholder.
            skip = ratatui::text::Span::raw(sym).width().saturating_sub(1);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use host::{Header, OnDelta, SourceBadge, Status, Who};
    use mwa_productmap::map::ProductMap;

    struct FakeHost;

    fn mini_map() -> ProductMap {
        let text = include_str!("../../productmap/tests/snapshots/mini.json");
        serde_json::from_str(text).unwrap()
    }

    #[async_trait]
    impl Host for FakeHost {
        fn header(&self) -> Header {
            Header {
                workspace: "/work/acme-manual".into(),
                sources: vec![SourceBadge {
                    name: "frontend".into(),
                    sha: "1e01d15376f1".into(),
                    preview: true,
                }],
            }
        }
        fn load_map(&self) -> Option<ProductMap> {
            Some(mini_map())
        }
        fn status(&self) -> Status {
            Status {
                model: "claude·sonnet".into(),
                spent_usd: 0.42,
                budget_usd: 5.0,
                quota: None,
            }
        }
        async fn chat(
            &self,
            _: Vec<(Who, String)>,
            _: Option<String>,
            _: OnDelta,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn build_map(&self) -> Result<(ProductMap, Vec<String>), String> {
            Ok((mini_map(), vec![]))
        }
        async fn doctor(&self) -> Vec<String> {
            vec![]
        }
        fn auth_lines(&self) -> Vec<String> {
            vec![]
        }
    }

    #[test]
    fn renders_layout() {
        let s = snapshot(&FakeHost, 100, 24, false);
        assert!(s.contains("mwa"), "{s}");
        assert!(s.contains("기능 5"), "{s}");
        assert!(s.contains("연결 관리"), "{s}");
        assert!(s.contains("제품 지도를 불러왔습니다"), "{s}");
        assert!(s.contains("claude·sonnet"), "{s}");
        assert!(s.contains("$0.42 / $5.00"), "{s}");
        assert!(s.contains("미리보기"), "{s}");
    }

    #[test]
    fn renders_feature_detail() {
        let s = snapshot(&FakeHost, 110, 30, true);
        assert!(s.contains("/api/source/systems"), "{s}");
        assert!(s.contains("SRC_LINK_VIEW"), "{s}");
    }
}
