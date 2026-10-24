//! One presentation of acquired recipient counts and optional context usage.
use super::*;
use crate::board::{inventory::ContextUsage, unread::Count};
use ratatui::text::Span;
use tmt_cli_style::theme::screen::{self, ChipKind, ChipState};
use unicode_width::UnicodeWidthStr;

pub(in crate::board) struct Info<'a> {
    pub name: &'a str,
    /// The new-line key the board names, the same source as the footer caption.
    pub newline: crate::board::app::NewlineKey,
    pub unread: Count,
    pub open: Option<usize>,
    pub context: Option<ContextUsage>,
}
/// Where the info line answers a click: the send hint, and the unread count when something is unread.
pub(in crate::board) struct Hits {
    pub send: Option<Rect>,
    pub unread: Option<Rect>,
}
impl Info<'_> {
    fn left(&self, window: bool, words: bool, look: Look) -> Line<'static> {
        let mut spans = vec![Span::styled(
            crate::board::notes::single_line(self.name),
            look.role(Role::Muted),
        )];
        spans.push(Span::raw("  "));
        spans.push(
            crate::board::state_cell::unread_mark(self.unread, look)
                .unwrap_or_else(|| Span::styled("●", look.role(Role::Muted))),
        );
        spans.push(Span::styled(
            format!(
                " {}{}{}",
                self.unread.total,
                if self.unread.more { "+" } else { "" },
                if words { " unread" } else { "" }
            ),
            look.role(Role::Muted),
        ));
        if let Some(count) = self.open {
            spans.push(Span::styled(" · ", look.role(Role::Muted)));
            spans.push(if count > 0 {
                crate::board::state_cell::mark(Some("waiting"), look)
            } else {
                Span::styled("◆", look.role(Role::Muted))
            });
            spans.push(Span::styled(
                format!(" {count}{}", if words { " open request" } else { "" }),
                look.role(Role::Muted),
            ));
        }
        if let Some(context) = self.context {
            let mut text = format!(
                " · ctx {}",
                crate::source::tokens(u128::from(context.used_tokens))
            );
            if let Some(limit) = context.window_tokens.filter(|_| window) {
                text.push_str(&format!(" / {}", crate::source::tokens(u128::from(limit))));
            }
            spans.push(Span::styled(text, look.role(Role::Muted)));
        }
        Line::from(spans)
    }
    pub(in crate::board) fn paint(
        &self,
        buffer: &mut Buffer,
        area: Rect,
        look: Look,
        hover: bool,
    ) -> Hits {
        let none = Hits {
            send: None,
            unread: None,
        };
        if area.is_empty() || area.height < 2 {
            return none;
        }
        let line = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        let newline = self.newline.glyph();
        let full = format!("⏎ send  {newline} new line  @ mention");
        let keys = format!("⏎  {newline}  @");
        let (full, keys) = (full.as_str(), keys.as_str());
        let full_left = self.left(true, true, look);
        let (left, right) = if full_left.width() + full.width() + 2 <= usize::from(line.width) {
            (full_left, full)
        } else if full_left.width() + keys.width() + 2 <= usize::from(line.width) {
            (full_left, keys)
        } else {
            let used = self.left(false, true, look);
            let left = if used.width() + keys.width() + 2 <= usize::from(line.width) {
                used
            } else {
                self.left(false, false, look)
            };
            (
                left,
                if keys.width() + 2 <= usize::from(line.width) {
                    keys
                } else {
                    "⏎"
                },
            )
        };
        let width = right.width().min(usize::from(line.width)) as u16;
        let right_area = Rect::new(line.right() - width, line.y, width, 1);
        let left_area = Rect::new(line.x, line.y, line.width.saturating_sub(width + 2), 1);
        let unread = (self.unread.total > 0)
            .then(|| {
                // Name and two spaces, then the mark and its count: the spans `left` builds first.
                let before: usize = left.spans[..2].iter().map(Span::width).sum();
                let chip: usize = left.spans[2..4].iter().map(Span::width).sum();
                Rect::new(left_area.x + before as u16, line.y, chip as u16, 1)
                    .intersection(left_area)
            })
            .filter(|rect| !rect.is_empty());
        strip::paint_left(buffer, left_area, left);
        if let Some(rect) = unread.filter(|_| hover) {
            let style = screen::chip(&look.theme, ChipKind::Secondary, ChipState::Hot, look.depth);
            for x in rect.x..rect.right() {
                buffer[(x, rect.y)].set_style(style);
            }
        }
        strip::paint_left(
            buffer,
            right_area,
            Line::styled(right, look.role(Role::Muted)),
        );
        let send = right.split("  ").next().unwrap_or_default();
        Hits {
            send: (send.width() <= usize::from(right_area.width)).then_some(Rect {
                width: send.width() as u16,
                ..right_area
            }),
            unread,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_keeps_known_zero_and_omits_an_unknown_window_and_observation() {
        let look = Look::default();
        let mut info = Info {
            name: "worker",
            newline: crate::board::app::NewlineKey::Alt,
            unread: Count::default(),
            open: Some(0),
            context: None,
        };
        assert!(!info.left(true, true, look).to_string().contains("ctx"));
        info.context = Some(ContextUsage {
            used_tokens: 0,
            window_tokens: None,
            observed_at_ms: 123,
        });
        assert!(info.left(true, true, look).to_string().ends_with("ctx 0"));
        info.context.as_mut().unwrap().window_tokens = Some(258000);
        assert!(
            info.left(true, true, look)
                .to_string()
                .ends_with("ctx 0 / 258k")
        );
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 4));
        let area = buffer.area;
        let hit = info.paint(&mut buffer, area, look, false).send.unwrap();
        assert_eq!(hit.y, 3);
        let row: String = buffer
            .content
            .chunks(40)
            .nth(3)
            .unwrap()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(row.contains("Alt+⏎") && row.contains('@'));
    }

    #[test]
    fn the_new_line_key_follows_the_one_capability_source_at_every_width() {
        use crate::board::app::NewlineKey;
        let look = Look::default();
        for (key, named) in [(NewlineKey::Alt, "Alt+⏎"), (NewlineKey::Shift, "Shift+⏎")] {
            let info = Info {
                name: "worker",
                newline: key,
                unread: Count::default(),
                open: Some(0),
                context: None,
            };
            assert_eq!(key.glyph(), named);
            assert!(key.hint().starts_with(&named.replace('⏎', "Enter")));
            for width in [160, 100, 60, 40, 20] {
                let mut buffer = Buffer::empty(Rect::new(0, 0, width, 2));
                let area = buffer.area;
                let hit = info.paint(&mut buffer, area, look, false).send.unwrap();
                let row: String = buffer
                    .content
                    .chunks(usize::from(width))
                    .nth(1)
                    .unwrap()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(hit.width >= 1, "{width}: the send hit survives");
                if width >= 100 {
                    assert!(row.contains(&format!("{named} new line")), "{width}: {row}");
                }
                let other = if key == NewlineKey::Alt {
                    "Shift+⏎"
                } else {
                    "Alt+⏎"
                };
                assert!(!row.contains(other), "{width}: {row}");
            }
        }
    }
}
