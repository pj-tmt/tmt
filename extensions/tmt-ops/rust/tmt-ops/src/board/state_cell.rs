//! StateCell: the state glyph owns attention colour; `activity::Label` owns the words.
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
        Some("offline" | "ended") => "○",
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
/// The same unread mark on tables, strips and global attention.
pub(in crate::board) fn unread_mark(
    count: super::unread::Count,
    look: Look,
) -> Option<Span<'static>> {
    (count.total > 0).then(|| Span::styled("●", look.role(Role::Unread)))
}

/// Shared unread override: colour belongs to the glyph alone.
pub(in crate::board) fn unread(
    count: super::unread::Count,
    look: Look,
) -> Option<ratatui::text::Line<'static>> {
    Some(ratatui::text::Line::from(vec![
        unread_mark(count, look)?,
        Span::styled(format!(" {}", count.label()?), look.role(Role::Muted)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_attention_marks_use_attention_roles() {
        let look = Look::new(tmt_cli_style::Theme::default());
        for state in [
            Some("working"),
            Some("idle"),
            Some("ended"),
            Some("waiting"),
            Some("blocked"),
            None,
        ] {
            let role = match state {
                Some("waiting") => Role::Waiting,
                Some("blocked") => Role::Blocked,
                _ => Role::Muted,
            };
            assert_eq!(mark(state, look).style, look.role(role));
        }
        assert_eq!(glyph(Some("ended")), "○");
        assert_eq!(glyph(None), "?");
    }
}

#[cfg(test)]
mod unread_tests {
    use super::*;
    #[test]
    fn unread_words_are_neutral_and_zero_is_not_a_state_even_with_a_partial_page() {
        let mut look = Look::default();
        let count = crate::board::unread::Count {
            total: 2,
            more: true,
        };
        let line = unread(count, look).unwrap();
        assert_eq!(line.to_string(), "● 2+ unread");
        assert_eq!(line.spans[0].style, look.role(Role::Unread));
        assert_eq!(line.spans[1].style, look.role(Role::Muted));
        assert!(
            unread(
                crate::board::unread::Count {
                    total: 0,
                    more: true
                },
                look
            )
            .is_none()
        );
        look.depth = tmt_cli_style::Depth::None;
        let plain = unread(count, look).unwrap();
        assert_eq!(plain.to_string(), "● 2+ unread");
        assert!(
            plain
                .spans
                .iter()
                .all(|span| span.style.fg.is_none() && span.style.bg.is_none())
        );
    }
}
