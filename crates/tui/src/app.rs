//! TUI state and input handling. Pure (no terminal I/O) so it can be unit-tested.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use mwa_productmap::map::{Feature, ProductMap};

use crate::host::{Header, Status, Who, chat_context};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Features,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Chat,
    /// Index into `map.features`.
    Feature(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub who: Who,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help,
    Map,
    Doctor,
    Auth,
    Clear,
    Quit,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Chat,
    Run(Command),
}

/// Results coming back from background work.
#[derive(Debug)]
pub enum AppEvent {
    Delta(String),
    ChatDone,
    ChatFailed(String),
    Lines { title: String, lines: Vec<String> },
    MapBuilt(Box<ProductMap>, Vec<String>),
    Failed(String),
    Status(Status),
}

/// A row of the feature list: a group heading or a selectable feature.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Group(String),
    Feature(usize),
}

pub const HELP: &[&str] = &[
    "/map     코드에서 제품 지도를 다시 만듭니다",
    "/doctor  설치·로그인·소스 상태를 점검합니다",
    "/auth    로그인 프로파일 상태를 봅니다",
    "/clear   대화를 지웁니다",
    "/quit    종료합니다 (Ctrl+C)",
    "Tab 기능 목록 ↔ 입력창 · ↑↓ 기능 선택 · Enter 기능 보기 · Esc 대화로 · PgUp/PgDn 스크롤",
];

pub struct App {
    pub header: Header,
    pub map: Option<ProductMap>,
    pub rows: Vec<Row>,
    /// Index into `rows`, always a `Row::Feature` when any exist.
    pub sel: usize,
    pub focus: Focus,
    pub pane: Pane,
    pub msgs: Vec<Msg>,
    pub input: String,
    /// Cursor position in chars.
    pub cursor: usize,
    /// What is running in the background, if anything.
    pub busy: Option<String>,
    pub spinner: usize,
    pub status: Status,
    /// Lines scrolled up from the bottom of the chat.
    pub scroll: u16,
    pub quit: bool,
}

impl App {
    pub fn new(header: Header, map: Option<ProductMap>, status: Status) -> Self {
        let mut app = App {
            header,
            map: None,
            rows: Vec::new(),
            sel: 0,
            focus: Focus::Input,
            pane: Pane::Chat,
            msgs: Vec::new(),
            input: String::new(),
            cursor: 0,
            busy: None,
            spinner: 0,
            status,
            scroll: 0,
            quit: false,
        };
        let intro = match &map {
            Some(m) => format!(
                "제품 지도를 불러왔습니다: 기능 {}개, 화면 {}개. 왼쪽에서 기능을 고르거나 질문을 입력하세요. /help 로 명령을 봅니다.",
                m.features.len(),
                m.coverage.screens
            ),
            None => "제품 지도가 아직 없습니다. /map 으로 만드세요. /help 로 명령을 봅니다.".into(),
        };
        app.set_map(map);
        app.system(intro);
        app
    }

    pub fn set_map(&mut self, map: Option<ProductMap>) {
        self.rows.clear();
        if let Some(m) = &map {
            let mut last_group: Option<String> = None;
            for (i, f) in m.features.iter().enumerate() {
                let group = f.group.clone().unwrap_or_else(|| f.license_menu.clone());
                if last_group.as_ref() != Some(&group) {
                    self.rows.push(Row::Group(group.clone()));
                    last_group = Some(group);
                }
                self.rows.push(Row::Feature(i));
            }
        }
        self.map = map;
        self.sel = self
            .rows
            .iter()
            .position(|r| matches!(r, Row::Feature(_)))
            .unwrap_or(0);
        if matches!(self.pane, Pane::Feature(_)) {
            self.pane = Pane::Chat;
        }
    }

    pub fn selected_feature(&self) -> Option<&Feature> {
        match self.rows.get(self.sel)? {
            Row::Feature(i) => self.map.as_ref()?.features.get(*i),
            Row::Group(_) => None,
        }
    }

    pub fn system(&mut self, text: impl Into<String>) {
        self.msgs.push(Msg {
            who: Who::System,
            text: text.into(),
        });
        self.scroll = 0;
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.msgs.push(Msg {
            who: Who::Error,
            text: text.into(),
        });
        self.scroll = 0;
    }

    /// Conversation so far, without UI notices.
    pub fn transcript(&self) -> Vec<(Who, String)> {
        self.msgs
            .iter()
            .filter(|m| matches!(m.who, Who::User | Who::Assistant) && !m.text.is_empty())
            .map(|m| (m.who, m.text.clone()))
            .collect()
    }

    /// Details for the model about the open feature and features named in the latest message.
    pub fn context(&self) -> Option<String> {
        let map = self.map.as_ref()?;
        let selected = match self.pane {
            Pane::Feature(i) => map.features.get(i).map(|f| f.key.as_str()),
            Pane::Chat => None,
        };
        let last = self
            .msgs
            .iter()
            .rev()
            .find(|m| m.who == Who::User)
            .map_or("", |m| m.text.as_str());
        chat_context(map, selected, last)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => {
                self.quit = true;
                return None;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Features if !self.rows.is_empty() => Focus::Input,
                    Focus::Features => Focus::Input,
                    Focus::Input if !self.rows.is_empty() => Focus::Features,
                    Focus::Input => Focus::Input,
                };
                return None;
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(10);
                return None;
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(10);
                return None;
            }
            KeyCode::Esc => {
                self.pane = Pane::Chat;
                return None;
            }
            _ => {}
        }
        match self.focus {
            Focus::Features => self.features_key(key),
            Focus::Input => self.input_key(key),
        }
    }

    fn features_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::Enter | KeyCode::Right => {
                if let Some(Row::Feature(i)) = self.rows.get(self.sel) {
                    self.pane = Pane::Feature(*i);
                }
            }
            KeyCode::Left => self.pane = Pane::Chat,
            KeyCode::Char(c) => {
                // Typing jumps to the input box.
                self.focus = Focus::Input;
                self.insert(c);
            }
            _ => {}
        }
        None
    }

    fn move_sel(&mut self, delta: i32) {
        let mut i = self.sel as i32;
        loop {
            i += delta;
            if i < 0 || i as usize >= self.rows.len() {
                return;
            }
            if matches!(self.rows[i as usize], Row::Feature(_)) {
                self.sel = i as usize;
                if let (Pane::Feature(_), Row::Feature(f)) = (self.pane, &self.rows[self.sel]) {
                    self.pane = Pane::Feature(*f);
                }
                return;
            }
        }
    }

    fn insert(&mut self, c: char) {
        let byte = self.byte_at(self.cursor);
        self.input.insert(byte, c);
        self.cursor += 1;
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map_or(self.input.len(), |(b, _)| b)
    }

    fn input_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char(c) => self.insert(c),
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let b = self.byte_at(self.cursor);
                self.input.remove(b);
            }
            KeyCode::Delete if self.cursor < self.input.chars().count() => {
                let b = self.byte_at(self.cursor);
                self.input.remove(b);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.input.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.chars().count(),
            KeyCode::Enter => return self.submit(),
            _ => {}
        }
        None
    }

    fn submit(&mut self) -> Option<Action> {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return None;
        }
        if let Some(cmd) = text.strip_prefix('/') {
            self.input.clear();
            self.cursor = 0;
            return Some(Action::Run(parse_command(cmd)));
        }
        if let Some(what) = &self.busy {
            self.error(format!("{what}이 끝난 뒤에 보내 주세요."));
            return None;
        }
        self.input.clear();
        self.cursor = 0;
        self.msgs.push(Msg {
            who: Who::User,
            text,
        });
        self.msgs.push(Msg {
            who: Who::Assistant,
            text: String::new(),
        });
        self.busy = Some("답변".into());
        self.scroll = 0;
        Some(Action::Chat)
    }

    pub fn apply(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Delta(t) => {
                if let Some(m) = self.msgs.iter_mut().rev().find(|m| m.who == Who::Assistant) {
                    m.text.push_str(&t);
                }
            }
            AppEvent::ChatDone => self.busy = None,
            AppEvent::ChatFailed(e) => {
                if self
                    .msgs
                    .last()
                    .is_some_and(|m| m.who == Who::Assistant && m.text.is_empty())
                {
                    self.msgs.pop();
                }
                self.busy = None;
                self.error(format!("답변 실패: {e}"));
            }
            AppEvent::Lines { title, lines } => {
                self.busy = None;
                self.system(format!("{title}\n{}", lines.join("\n")));
            }
            AppEvent::MapBuilt(map, lines) => {
                self.busy = None;
                self.set_map(Some(*map));
                self.system(format!(
                    "제품 지도를 다시 만들었습니다.\n{}",
                    lines.join("\n")
                ));
            }
            AppEvent::Failed(e) => {
                self.busy = None;
                self.error(e);
            }
            AppEvent::Status(s) => self.status = s,
        }
    }
}

pub fn parse_command(text: &str) -> Command {
    match text.split_whitespace().next().unwrap_or("") {
        "help" | "h" | "?" => Command::Help,
        "map" => Command::Map,
        "doctor" => Command::Doctor,
        "auth" => Command::Auth,
        "clear" => Command::Clear,
        "quit" | "q" | "exit" => Command::Quit,
        other => Command::Unknown(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn typed(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_editing_and_korean_cursor() {
        let mut app = App::new(Header::default(), None, Status::default());
        typed(&mut app, "안녕x");
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Left));
        typed(&mut app, "가");
        assert_eq!(app.input, "안가녕");
        app.handle_key(key(KeyCode::Home));
        app.handle_key(key(KeyCode::Delete));
        assert_eq!(app.input, "가녕");
    }

    #[test]
    fn submit_chat_and_commands() {
        let mut app = App::new(Header::default(), None, Status::default());
        typed(&mut app, "/map");
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Some(Action::Run(Command::Map))
        );
        typed(&mut app, "질문");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), Some(Action::Chat));
        assert_eq!(app.transcript(), vec![(Who::User, "질문".to_string())]);
        assert!(app.busy.is_some());
        // While busy, a second message is refused but commands still work.
        typed(&mut app, "또");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), None);
        assert_eq!(app.msgs.last().unwrap().who, Who::Error);
        app.apply(AppEvent::Delta("답".into()));
        app.apply(AppEvent::Delta("변".into()));
        app.apply(AppEvent::ChatDone);
        assert!(app.busy.is_none());
        assert_eq!(app.transcript().last().unwrap().1, "답변");
    }

    #[test]
    fn failed_chat_removes_empty_placeholder() {
        let mut app = App::new(Header::default(), None, Status::default());
        typed(&mut app, "q");
        app.handle_key(key(KeyCode::Enter));
        app.apply(AppEvent::ChatFailed("x".into()));
        assert!(app.transcript().iter().all(|(w, _)| *w == Who::User));
        assert_eq!(app.msgs.last().unwrap().who, Who::Error);
    }

    #[test]
    fn context_includes_features_named_in_message() {
        let map: ProductMap =
            serde_json::from_str(include_str!("../../productmap/tests/snapshots/mini.json"))
                .unwrap();
        let mut app = App::new(Header::default(), Some(map), Status::default());
        assert_eq!(app.context(), None);
        typed(&mut app, "연결관리 화면 설명해줘");
        app.handle_key(key(KeyCode::Enter));
        let ctx = app.context().unwrap();
        assert!(
            ctx.contains("연결 관리") && ctx.contains("/api/source/systems"),
            "{ctx}"
        );
    }

    #[test]
    fn parse_commands() {
        assert_eq!(parse_command("quit"), Command::Quit);
        assert_eq!(parse_command("doctor now"), Command::Doctor);
        assert_eq!(parse_command("nope"), Command::Unknown("nope".into()));
    }
}
