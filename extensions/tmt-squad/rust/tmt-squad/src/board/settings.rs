//! A read-only snapshot. Its own instance of the shared scroll owner keeps board scrolls intact.
use super::scroll::{Scrolls, Step, WHEEL_LINES};
use crate::{config::Pane, look::Look, settings::BoardSettings};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind},
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use tmt_cli_style::{Role, grid::Align, table::escape};
use unicode_width::UnicodeWidthStr;

pub(super) struct Overlay {
    pub settings: BoardSettings,
    scrolls: Scrolls,
    display_path: String,
}
impl Overlay {
    pub fn new(settings: BoardSettings) -> Self {
        let display_path = std::env::var("HOME")
            .ok()
            .and_then(|home| {
                settings
                    .path
                    .strip_prefix(&format!("{home}/"))
                    .map(|path| format!("~/{path}"))
            })
            .unwrap_or_else(|| settings.path.clone());
        Self {
            settings,
            display_path,
            scrolls: Scrolls::default(),
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let step = match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(',') => return true,
            KeyCode::Up | KeyCode::Char('k') => Step::Lines(-1),
            KeyCode::Down | KeyCode::Char('j') => Step::Lines(1),
            KeyCode::PageUp => Step::Pages(-1),
            KeyCode::PageDown => Step::Pages(1),
            KeyCode::Home => Step::Top,
            KeyCode::End => Step::Bottom,
            _ => return false,
        };
        self.scrolls.scroll(Pane::Rows, step);
        false
    }
    pub fn mouse(&self, event: MouseEvent) {
        if self.scrolls.pane_at(event.column, event.row).is_some() {
            let direction = match event.kind {
                MouseEventKind::ScrollUp => -1,
                MouseEventKind::ScrollDown => 1,
                _ => return,
            };
            self.scrolls
                .scroll(Pane::Rows, Step::Lines(direction * WHEEL_LINES as isize));
        }
    }
}

fn setting_name(key: &str) -> (&str, &str) {
    key.split_once('.').unwrap_or(match key {
        "layout" => ("layout", "preset"),
        "rows" => ("rows", "grid"),
        "state_patterns" => ("states", "patterns"),
        _ => ("programs", key),
    })
}

// Serialized JSON punctuation is a break opportunity only outside quoted strings.
fn value_lines(text: &str, width: usize, structured: bool) -> Vec<String> {
    let (mut quoted, mut escaped) = (false, false);
    let mut lines = Vec::new();
    let mut line = String::new();
    for token in text.split_inclusive(|ch| {
        if ch == '"' && !escaped {
            quoted = !quoted;
        }
        let boundary = structured && !quoted && matches!(ch, ',' | ':');
        escaped = quoted && ch == '\\' && !escaped;
        boundary
    }) {
        if !line.is_empty() && line.width() + token.width() > width {
            lines.push(std::mem::take(&mut line));
        }
        if token.width() <= width {
            line.push_str(token);
        } else {
            let mut parts = super::notes::wrap(token, width);
            line = parts.pop().unwrap_or_default();
            lines.extend(parts);
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    if lines.len() > 4 {
        let more = lines.len() - 3;
        lines.truncate(3);
        lines.push(format!("… ({more} more)"));
    }
    lines
}

pub(super) fn render(frame: &mut Frame, overlay: &Overlay, look: Look, body: Rect) {
    frame.render_widget(Clear, body);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(look.role(Role::Dim))
        .title(" settings · read-only ");
    let mut inner = block.inner(body);
    frame.render_widget(block, body);
    inner.x += u16::from(inner.width > 0);
    inner.width = inner.width.saturating_sub(2);
    let footer = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: u16::from(inner.height > 0),
        ..inner
    };
    let content = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let settings = &overlay.settings;
    let width = usize::from(content.width);
    let row_width = width.saturating_sub(2);
    let key_width = settings
        .entries
        .iter()
        .map(|entry| setting_name(&entry.key).1.width())
        .max()
        .unwrap_or(0)
        .min(row_width / 4);
    let source_width = (row_width / 3).min(42);
    let value_width = row_width
        .saturating_sub(key_width + source_width + 4)
        .max(1);
    let fit = |text: &str, width: usize| {
        tmt_tui::text::fit_line(
            text,
            width.min(usize::from(u16::MAX)) as u16,
            tmt_tui::style::TextFlow::Middle,
            Align::Left,
        )
    };
    let context = settings.context.as_deref().unwrap_or("board defaults");
    let heading = format!("{context} {} · {}", settings.host, overlay.display_path);
    let mut lines = vec![Line::styled(fit(&heading, width), look.role(Role::Dim))];
    let cell = |text: &str, width, role| Span::styled(fit(text, width), look.role(role));
    lines.push(Line::styled(
        fit(
            "Full values: tmt sq config show --json (--squad/--tab)",
            width,
        ),
        look.role(Role::Dim),
    ));
    for notice in &settings.notices {
        lines.push(Line::styled(fit(notice, width), look.role(Role::Waiting)));
    }
    let mut groups = Vec::new();
    for entry in &settings.entries {
        let group = setting_name(&entry.key).0;
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    for group in groups {
        lines.push(Line::styled(group.to_owned(), look.role(Role::Dim)));
        for entry in settings
            .entries
            .iter()
            .filter(|entry| setting_name(&entry.key).0 == group)
        {
            let name = setting_name(&entry.key).1;
            let (value, role) = match &entry.value {
                serde_json::Value::Null => ("unset".into(), Role::Dim),
                serde_json::Value::Array(items) if items.is_empty() => ("none".into(), Role::Dim),
                value => (crate::settings::display(value), Role::Text),
            };
            for (index, value) in value_lines(
                &escape(&value),
                value_width,
                entry.value.is_array() || entry.value.is_object(),
            )
            .into_iter()
            .enumerate()
            {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    cell(if index == 0 { name } else { "" }, key_width, Role::Accent),
                    Span::raw("  "),
                    cell(&value, value_width, role),
                    Span::raw("  "),
                    cell(
                        if index == 0 { &entry.source } else { "" },
                        source_width,
                        Role::Dim,
                    ),
                ]));
            }
            if let Some(description) = &entry.description {
                for line in super::notes::wrap(
                    &escape(description),
                    row_width.saturating_sub(key_width + 2).max(1),
                ) {
                    lines.push(Line::from(vec![
                        Span::raw(" ".repeat(key_width + 4)),
                        Span::styled(line, look.role(Role::Muted)),
                    ]));
                }
            }
        }
    }
    let count = lines.len();
    let (offset, shown) =
        overlay
            .scrolls
            .show(frame, Pane::Rows, content, lines, look.role(Role::Dim));
    if shown < usize::from(content.height) {
        let indicator = Rect {
            y: content.y + shown as u16,
            height: 1,
            ..content
        };
        frame.render_widget(Clear, indicator);
        frame.render_widget(
            Paragraph::new(format!(
                "{}–{} of {count}",
                offset + 1,
                (offset + shown).min(count)
            ))
            .style(look.role(Role::Dim)),
            indicator,
        );
    }
    frame.render_widget(
        Paragraph::new(super::view::fit(
            "↑↓ scroll · PgUp/PgDn page · Esc close",
            usize::from(footer.width),
        ))
        .style(look.role(Role::Muted)),
        footer,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyModifiers};
    #[test]
    fn structured_values_break_at_punctuation_preserve_quotes_and_cap_lines() {
        for text in ["x,y:z", r#"x\"y,z:q"#] {
            let value = serde_json::json!({"a":text, "b":1}).to_string();
            let lines = value_lines(&value, 20, true);
            assert_eq!(lines.concat(), value);
            assert!(
                lines[..lines.len() - 1]
                    .iter()
                    .all(|line| line.ends_with([',', ':']))
            );
        }
        let lines = value_lines(
            &serde_json::json!((0..40).collect::<Vec<_>>()).to_string(),
            12,
            true,
        );
        assert_eq!(lines.len(), 4);
        assert!(lines[3].starts_with("… (") && lines[3].ends_with(" more)"));
    }
    #[test]
    fn overlay_scrolls_escaped_values_and_closes_without_row_actions() {
        let mut shown = BoardSettings {
            path: "/isolated/squad.toml".into(),
            context: Some("x".into()),
            host: "plain",
            entries: Vec::new(),
            notices: Vec::new(),
        };
        shown.push("board.mode", serde_json::Value::Null, "preset:team");
        shown.push("board.collapsed", serde_json::json!([]), "preset:team");
        // CJK fixture data exercises wide terminal cells.
        for index in 0..20 {
            shown.push(
                format!("key.{index}"),
                serde_json::json!("long value with wide characters 日本語 and\nnewlines"),
                "preset:team",
            );
        }
        let mut overlay = Overlay::new(shown);
        let look = Look::new(tmt_cli_style::Theme::default());
        for width in [80, 24, 80] {
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
            terminal
                .draw(|frame| render(frame, &overlay, look, frame.area()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            if width >= 80 {
                assert!(text.contains("unset") && text.contains("none"));
            }
            assert_eq!(buffer[(1, 1)].symbol(), " ", "content has inset padding");
            assert!(!overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
            overlay.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
            terminal
                .draw(|frame| render(frame, &overlay, look, frame.area()))
                .unwrap();
            assert!(overlay.scrolls.offset(Pane::Rows) > 0);
            overlay.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
            assert_eq!(overlay.scrolls.offset(Pane::Rows), 0);
        }
        let mut app = super::super::app::App::new(Some("x".into()));
        app.apply(super::super::app::tests::snapshot(
            "x",
            serde_json::json!([]),
        ));
        app.settings = Some(overlay);
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            super::super::app::Effect::None
        );
        app.apply(super::super::app::tests::snapshot(
            "x",
            serde_json::json!([]),
        ));
        assert!(
            app.settings.is_some(),
            "refresh retains the opening settings snapshot"
        );
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            super::super::app::Effect::None
        );
        assert!(app.settings.is_none());
    }
}
