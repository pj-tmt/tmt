//! Transient footer presentation. App owns the message; the UI loop supplies time.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use std::time::{Duration, Instant};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

const LIFETIME: Duration = Duration::from_secs(4);

#[derive(Default)]
pub(super) struct Clock {
    deadline: Option<Instant>,
    /// A progress notice stays hidden until this core time: a job that is done by then
    /// paints nothing.
    visible_ms: Option<u64>,
    pub failed: bool,
}
impl Clock {
    pub fn start(&mut self, now: Instant, failed: bool) {
        self.deadline = Some(now + LIFETIME);
        self.visible_ms = None;
        self.failed = failed;
    }
    pub fn hold_until(&mut self, visible_ms: u64) {
        self.visible_ms = Some(visible_ms);
    }
    pub fn hidden_until(&self, now_ms: u64) -> Option<u64> {
        self.visible_ms.filter(|visible| now_ms < *visible)
    }
    pub fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Identity-shaped text is cosmetic here: resolve from the held inventory, never
/// acquire a name or let a UUID dominate a transient user-facing notice.
pub(super) fn names(text: &str, mut name: impl FnMut(&str) -> Option<String>) -> String {
    let mut output = String::new();
    let mut offset = 0;
    for (start, _) in text.char_indices() {
        if start < offset {
            continue;
        }
        if let Some(candidate) = text.get(start..start + 36)
            && crate::config::uuid_like(candidate)
        {
            output.push_str(&text[offset..start]);
            output.push_str(&name(candidate).unwrap_or_else(|| "unavailable target".into()));
            offset = start + 36;
        }
    }
    output.push_str(&text[offset..]);
    crate::board::notes::sanitize(&output)
}

/// Returns the remaining hint area; never adds a row or paints over its caller.
pub(super) fn paint(buffer: &mut Buffer, area: Rect, text: &str, failed: bool, look: Look) -> Rect {
    if area.width == 0 || area.height == 0 || text.is_empty() {
        return area;
    }
    let text = text.trim_start_matches("✗ ");
    let marker = if failed { "✗ " } else { "" };
    let width = (text.width() + marker.width() + 2).min(usize::from(area.width.div_ceil(2))) as u16;
    let chip = Rect::new(area.right() - width, area.y, width, 1);
    let fill = look.role(Role::Text).patch(look.role(Role::Track));
    strip::paint_left(
        buffer,
        chip,
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                marker,
                if failed {
                    look.role(Role::Blocked)
                } else {
                    look.role(Role::Text)
                },
            ),
            Span::styled(
                crate::board::view::fit(
                    text,
                    usize::from(width.saturating_sub(marker.width() as u16 + 2)),
                ),
                look.role(Role::Text),
            ),
        ])
        .style(fill),
    );
    Rect {
        width: area.width.saturating_sub(width + 1),
        ..area
    }
}

pub(super) fn paint_links(
    frame: &mut ratatui::Frame,
    app: &super::app::App,
    area: Rect,
    text: &str,
    failed: bool,
) -> Rect {
    let hints = paint(frame.buffer_mut(), area, text, failed, app.look());
    if hints == area {
        return hints;
    }
    let text = text.trim_start_matches("✗ ");
    let marker = if failed { 2 } else { 0 };
    let width = (text.width() + marker + 2).min(usize::from(area.width.div_ceil(2))) as u16;
    let content = Rect::new(
        area.right() - width + 1 + marker as u16,
        area.y,
        width.saturating_sub(marker as u16 + 2),
        1,
    );
    let shown = super::view::fit(text, usize::from(content.width));
    super::member_links::paint_plain_hits(frame, app, content, text, &shown, false);
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expiry_is_bounded_replacement_restarts_it_and_dismissal_clears_failure() {
        let now = Instant::now();
        let mut clock = Clock::default();
        clock.start(now, true);
        assert!(!clock.expired(now + LIFETIME - Duration::from_millis(1)));
        assert!(clock.expired(now + LIFETIME));
        clock.start(now + Duration::from_secs(2), false);
        assert!(!clock.expired(now + LIFETIME));
        assert!(clock.expired(now + Duration::from_secs(6)));
        clock.clear();
        assert!(!clock.failed && !clock.expired(now + Duration::from_secs(60)));
    }
    #[test]
    fn a_progress_notice_is_hidden_until_the_spinner_delay_and_a_result_shows_at_once() {
        let mut app = crate::board::app::App::new(None);
        let delay = crate::board::view::SPINNER_DELAY.as_millis() as u64;
        let now = 1_791_600_000_000;
        crate::status::with_now_ms(now, || {
            app.set_progress("Copying…".into());
            assert_eq!(app.shown_notice(), None, "a fast job paints nothing");
            assert!(app.notice.is_some(), "the notice is held, not dropped");
        });
        crate::status::with_now_ms(now + delay - 1, || assert_eq!(app.shown_notice(), None));
        crate::status::with_now_ms(now + delay, || {
            assert_eq!(app.shown_notice(), Some("Copying…"));
        });
        crate::status::with_now_ms(now, || {
            app.set_notice(Some("Copied.".into()), false);
            assert_eq!(app.shown_notice(), Some("Copied."), "a result is immediate");
            app.set_progress("Saving…".into());
            app.set_notice(None, false);
            assert_eq!(app.shown_notice(), None);
            app.set_progress("Saving…".into());
            app.set_notice(Some("✗ failed".into()), true);
            assert_eq!(
                app.shown_notice(),
                Some("✗ failed"),
                "a failure is immediate"
            );
        });
    }
    #[test]
    fn notices_use_observed_names_and_never_unresolved_identity_ids() {
        const ID: &str = "22222222-2222-4222-8222-222222222222";
        let text = format!("Showing {ID} · tmux 0:7.2");
        assert_eq!(
            names(&text, |_| Some("night-2".into())),
            "Showing night-2 · tmux 0:7.2"
        );
        assert_eq!(
            names(&text, |_| None),
            "Showing unavailable target · tmux 0:7.2"
        );
    }
    #[test]
    fn chip_reserves_right_edge_and_only_the_failure_glyph_uses_blocked_color() {
        let look = Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::Tmt),
            depth: tmt_cli_style::Depth::TrueColor,
        };
        let area = Rect::new(2, 1, 103, 1);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 110, 3));
        let hints = paint(&mut buffer, area, "Could not show night-2", true, look);
        assert!(hints.width > 0 && hints.right() < area.right());
        let cells = (area.x..area.right())
            .map(|x| &buffer[(x, area.y)])
            .collect::<Vec<_>>();
        assert_eq!(cells.iter().filter(|cell| cell.symbol() == "✗").count(), 1);
        for cell in cells {
            if cell.symbol() == "✗" {
                assert_eq!(cell.fg, look.role(Role::Blocked).fg.unwrap());
            } else if !cell.symbol().trim().is_empty() {
                assert_eq!(cell.fg, look.role(Role::Text).fg.unwrap());
            }
        }
        assert_eq!(buffer[(0, 0)].symbol(), " ");
    }
}
