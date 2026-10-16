//! StateCell: the state glyph owns attention colour; its words stay neutral.
use crate::look::Look;
use ratatui::text::Span;
use tmt_cli_style::Role;

pub(in crate::board) fn glyph(state: Option<&str>) -> &'static str {
    match state {
        Some("working") => "●",
        Some("waiting") => "◆",
        Some("blocked") => "✗",
        Some("review") => "◐",
        Some("idle") => "◌",
        Some("offline") => "○",
        _ => "?",
    }
}
pub(in crate::board) fn mark(state: Option<&str>, look: Look) -> Span<'static> {
    Span::styled(
        glyph(state),
        look.role(match state {
            Some("waiting") => Role::Waiting,
            Some("blocked") => Role::Blocked,
            _ => Role::Muted,
        }),
    )
}
pub(in crate::board) fn word(state: Option<&str>, look: Look) -> Span<'static> {
    Span::styled(
        crate::board::notes::sanitize(state.unwrap_or("unreported")),
        look.role(Role::Muted),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;
    fn line(state: Option<&str>, _: Option<&str>, look: Look) -> Line<'static> {
        Line::from(vec![mark(state, look), Span::raw(" "), word(state, look)])
    }
    #[test]
    fn only_attention_marks_use_attention_roles() {
        let look = Look::new(tmt_cli_style::Theme::default());
        for state in [
            Some("working"),
            Some("idle"),
            Some("waiting"),
            Some("blocked"),
            None,
        ] {
            let row = line(state, None, look);
            assert_eq!(row.spans[2].style, look.role(Role::Muted));
            if !matches!(state, Some("waiting" | "blocked")) {
                assert_eq!(row.spans[0].style, look.role(Role::Muted));
            }
        }
        assert_eq!(line(None, None, look).to_string(), "? unreported");
    }
}
