//! Rendering. Layout: header · (features | chat or feature detail) · input · status bar.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Paragraph, Wrap};

use crate::app::{App, Focus, Pane, Row};
use crate::host::Who;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, input, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let left_w = if body.width >= 90 { 34 } else { body.width / 3 };
    let [left, right] =
        Layout::horizontal([Constraint::Length(left_w), Constraint::Min(20)]).areas(body);

    draw_header(f, app, header);
    draw_features(f, app, left);
    match app.pane {
        Pane::Chat => draw_chat(f, app, right),
        Pane::Feature(i) => draw_feature(f, app, i, right),
    }
    draw_input(f, app, input);
    draw_status(f, app, status);
}

fn block(title: &str, focused: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focused { ACCENT } else { DIM }))
        .title(Span::styled(format!(" {title} "), Style::new().bold()))
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(" mwa ", Style::new().fg(Color::Black).bg(ACCENT).bold()),
        Span::raw(" "),
        Span::styled(app.header.workspace.clone(), Style::new().fg(DIM)),
    ];
    for s in &app.header.sources {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(s.name.clone(), Style::new().bold()));
        spans.push(Span::raw(format!(" {}", &s.sha[..s.sha.len().min(7)])));
        if s.preview {
            spans.push(Span::styled(" 미리보기", Style::new().fg(Color::Yellow)));
        }
    }
    f.render_widget(Line::from(spans), area);
}

fn draw_features(f: &mut Frame, app: &App, area: Rect) {
    let count = app.map.as_ref().map_or(0, |m| m.features.len());
    let b = block(&format!("기능 {count}"), app.focus == Focus::Features);
    let inner = b.inner(area);
    f.render_widget(b, area);
    let Some(map) = &app.map else {
        f.render_widget(
            Paragraph::new("지도 없음\n/map 으로 만드세요").fg(DIM),
            inner,
        );
        return;
    };
    let lines: Vec<Line> = app
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| match row {
            Row::Group(g) => Line::styled(format!("▾ {g}"), Style::new().fg(DIM).bold()),
            Row::Feature(fi) => {
                let feat = &map.features[*fi];
                let apis: usize = feat.screens.iter().map(|s| s.apis.len()).sum();
                let selected = i == app.sel;
                let marker = if selected { "▸ " } else { "  " };
                let mut style = Style::new();
                if selected {
                    style = style.add_modifier(Modifier::BOLD);
                    if app.focus == Focus::Features {
                        style = style.fg(Color::Black).bg(ACCENT);
                    } else {
                        style = style.fg(ACCENT);
                    }
                }
                Line::from(vec![
                    Span::styled(format!("{marker}{}", feat.label), style),
                    Span::styled(format!("  {apis}"), Style::new().fg(DIM)),
                ])
            }
        })
        .collect();
    // Keep the selection visible.
    let h = inner.height as usize;
    let offset = app.sel.saturating_sub(h.saturating_sub(2));
    f.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);
}

fn chat_text(app: &App) -> Text<'static> {
    let mut lines: Vec<Line> = Vec::new();
    for m in &app.msgs {
        let (prefix, style) = match m.who {
            Who::User => ("› ", Style::new().fg(ACCENT).bold()),
            Who::Assistant => ("● ", Style::new()),
            Who::System => ("· ", Style::new().fg(DIM)),
            Who::Error => ("! ", Style::new().fg(Color::Red)),
        };
        let body = if m.who == Who::Assistant && m.text.is_empty() {
            "…".to_string()
        } else {
            m.text.clone()
        };
        for (i, l) in body.lines().enumerate() {
            let p = if i == 0 { prefix } else { "  " };
            lines.push(Line::styled(format!("{p}{l}"), style));
        }
        lines.push(Line::raw(""));
    }
    Text::from(lines)
}

fn draw_chat(f: &mut Frame, app: &App, area: Rect) {
    let b = block("대화", false);
    let inner = b.inner(area);
    f.render_widget(b, area);
    let p = Paragraph::new(chat_text(app)).wrap(Wrap { trim: false });
    let total = p.line_count(inner.width) as u16;
    let bottom = total.saturating_sub(inner.height);
    let top = bottom.saturating_sub(app.scroll);
    f.render_widget(p.scroll((top, 0)), inner);
}

fn draw_feature(f: &mut Frame, app: &App, i: usize, area: Rect) {
    let Some(feat) = app.map.as_ref().and_then(|m| m.features.get(i)) else {
        return draw_chat(f, app, area);
    };
    let b = block(&format!("{} · Esc 대화로", feat.label), false);
    let inner = b.inner(area);
    f.render_widget(b, area);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("경로     ", Style::new().fg(DIM)),
            Span::raw(feat.href.clone()),
        ]),
        Line::from(vec![
            Span::styled("메뉴     ", Style::new().fg(DIM)),
            Span::raw(format!(
                "{} > {}",
                feat.group.clone().unwrap_or_default(),
                feat.label
            )),
        ]),
        Line::from(vec![
            Span::styled("라이선스 ", Style::new().fg(DIM)),
            Span::raw(feat.license_menu.clone()),
        ]),
        Line::from(vec![
            Span::styled("노출 권한 ", Style::new().fg(DIM)),
            Span::raw(if feat.permissions.is_empty() {
                "없음 (항상 보임)".to_string()
            } else {
                feat.permissions.join(", ")
            }),
        ]),
        Line::raw(""),
    ];
    for s in &feat.screens {
        lines.push(Line::styled(
            format!("화면 {}", s.route),
            Style::new().bold().fg(ACCENT),
        ));
        lines.push(Line::styled(format!("  {}", s.file), Style::new().fg(DIM)));
        if !s.ui_permissions.is_empty() {
            lines.push(Line::raw(format!(
                "  화면 안 권한 검사: {}",
                s.ui_permissions.join(", ")
            )));
        }
        for a in &s.apis {
            let backend = a.backend.clone().unwrap_or_else(|| "백엔드 미확인".into());
            let ok = a.operation.is_some();
            lines.push(Line::from(vec![
                Span::raw(format!("  {:<6} ", a.method)),
                Span::raw(a.path.clone()),
                Span::styled(" → ", Style::new().fg(DIM)),
                Span::styled(
                    backend,
                    Style::new().fg(if ok { Color::Green } else { Color::Yellow }),
                ),
            ]));
        }
        lines.push(Line::raw(""));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Input;
    let b = block("입력", focused).title_bottom(
        Line::styled(" /help /map /doctor /auth ", Style::new().fg(DIM)).right_aligned(),
    );
    let inner = b.inner(area);
    f.render_widget(b, area);
    let placeholder = app.input.is_empty() && !focused;
    let text = if placeholder {
        Line::styled("> 질문을 입력하세요", Style::new().fg(DIM))
    } else {
        Line::raw(format!("> {}", app.input))
    };
    f.render_widget(Paragraph::new(text), inner);
    if focused {
        let before: String = app.input.chars().take(app.cursor).collect();
        let x = inner.x + 2 + ratatui::text::Line::raw(before).width() as u16;
        f.set_cursor_position(Position::new(
            x.min(inner.right().saturating_sub(1)),
            inner.y,
        ));
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.status;
    let mut spans = vec![
        Span::styled(
            format!(" {} ", s.model),
            Style::new().fg(Color::Black).bg(DIM),
        ),
        Span::raw(format!("  예산 ${:.2} / ${:.2}", s.spent_usd, s.budget_usd)),
    ];
    if let Some(q) = &s.quota {
        let window = match q.window.as_str() {
            "seven_day" => "7일",
            "five_hour" => "5시간",
            other => other,
        };
        if let Some(u) = q.utilization {
            spans.push(Span::raw(format!("  {window} 한도 {:.0}%", u * 100.0)));
        }
    }
    if let Some(what) = &app.busy {
        spans.push(Span::styled(
            format!("  {} {what} 중…", SPINNER[app.spinner % SPINNER.len()]),
            Style::new().fg(ACCENT),
        ));
    }
    let hint = Span::styled(
        "Tab 이동 · Enter 보내기 · Ctrl+C 종료 ",
        Style::new().fg(DIM),
    );
    let [l, r] = Layout::horizontal([Constraint::Min(10), Constraint::Length(hint.width() as u16)])
        .areas(area);
    f.render_widget(Line::from(spans), l);
    f.render_widget(Line::from(hint), r);
}
