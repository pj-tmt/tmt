//! A read-only snapshot. Its own instance of the shared scroll owner keeps board scrolls intact.
use super::scroll::{Scrolls, Step, WHEEL_LINES};
use crate::{config::Pane, look::Look, settings::BoardSettings};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind},
    layout::Rect,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use tmt_cli_style::{Role, table::escape};

pub(super) struct Overlay {
    pub settings: BoardSettings,
    scrolls: Scrolls,
}
impl Overlay {
    pub fn new(settings: BoardSettings) -> Self {
        Self {
            settings,
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

pub(super) fn render(frame: &mut Frame, overlay: &Overlay, look: Look, body: Rect) {
    let area = body;
    frame.render_widget(Clear, area);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(look.role(Role::Dim))
        .title(" settings · read-only ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let footer = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: u16::from(inner.height > 0),
        ..inner
    };
    let content = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let width = usize::from(content.width);
    let mut lines = Vec::new();
    let mut add = |text: String, role| {
        lines.extend(
            super::notes::wrap(&escape(&text), width)
                .into_iter()
                .map(|line| Line::styled(line, look.role(role))),
        );
    };
    add(
        format!(
            "{} · {}",
            overlay
                .settings
                .context
                .as_deref()
                .unwrap_or("board defaults"),
            overlay.settings.host
        ),
        Role::Accent,
    );
    add(overlay.settings.path.clone(), Role::Muted);
    for notice in &overlay.settings.notices {
        add(notice.clone(), Role::Waiting);
    }
    for entry in &overlay.settings.entries {
        add(entry.key.clone(), Role::Accent);
        add(
            format!("  {}", crate::settings::display(&entry.value)),
            Role::Text,
        );
        add(format!("  from {}", entry.source), Role::Muted);
    }
    overlay
        .scrolls
        .show(frame, Pane::Rows, content, lines, look.role(Role::Dim));
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
    fn overlay_scrolls_full_escaped_values_and_closes_without_row_actions() {
        let mut shown = BoardSettings {
            path: "/isolated/squad.toml".into(),
            context: Some("x".into()),
            host: "plain",
            entries: Vec::new(),
            notices: Vec::new(),
        };
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
