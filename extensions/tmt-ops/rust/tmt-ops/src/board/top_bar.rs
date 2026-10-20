//! TopBar: fitted caption and globally observed attention; no acquisition.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
mod capture;
#[cfg(test)]
mod host_tests;
mod observations;
pub(in crate::board) use observations::Observations;

pub(in crate::board) struct Attention {
    pub online: String,
    pub unread: super::unread::Count,
    pub waiting: String,
    pub blocked: String,
    pub waiting_positive: bool,
    pub blocked_positive: bool,
    pub complete: bool,
    pub ascii: bool,
}
impl Attention {
    fn line(&self, look: Look) -> Line<'static> {
        self.with_words(look, true)
    }
    fn with_words(&self, look: Look, words: bool) -> Line<'static> {
        let mut spans = vec![
            Span::styled(
                format!(
                    "{} {}{}  ",
                    if self.ascii { "+" } else { "●" },
                    self.online,
                    if words { " online" } else { "" }
                ),
                look.role(Role::Muted),
            ),
            Span::styled(
                if self.ascii { "!" } else { "◆" },
                look.role(if self.waiting_positive {
                    Role::Waiting
                } else {
                    Role::Muted
                }),
            ),
            Span::styled(
                format!(" {}{}  ", self.waiting, if words { " waiting" } else { "" }),
                look.role(Role::Muted),
            ),
            Span::styled(
                if self.ascii { "x" } else { "✗" },
                look.role(if self.blocked_positive {
                    Role::Blocked
                } else {
                    Role::Muted
                }),
            ),
            Span::styled(
                format!(
                    " {}{}{}",
                    self.blocked,
                    if words { " blocked" } else { "" },
                    if self.complete || !words {
                        ""
                    } else {
                        " · totals unknown"
                    }
                ),
                look.role(Role::Muted),
            ),
        ];
        if let Some(mut unread) = super::state_cell::unread(self.unread, look) {
            unread
                .spans
                .push(Span::styled("  ", look.role(Role::Muted)));
            spans.splice(3..3, unread.spans);
        }
        Line::from(spans)
    }
    fn fitted(&self, look: Look, width: usize) -> Line<'static> {
        let mut line = self.line(look);
        // The ≥ counts already carry partial evidence. At narrow widths keep
        // every count before the redundant explanatory suffix or layer name.
        if line.width() > width
            && self.unread.total > 0
            && !self.complete
            && let Some(last) = line.spans.last_mut()
            && let Some(text) = last.content.strip_suffix(" · totals unknown")
        {
            last.content = text.to_owned().into();
        }
        line
    }
}

#[cfg(test)]
fn paint(
    buffer: &mut Buffer,
    area: Rect,
    layer: Option<&str>,
    attention: Option<&Attention>,
    observations: Option<&Observations<'_>>,
    look: Look,
) {
    paint_with_control(buffer, area, layer, attention, observations, look, 0);
}

/// Paints the caption and attention strip. A control of `control_width` columns takes the right
/// end of the caption side when the stage keeps the whole caption beside it; its rectangle is
/// returned for the caller to paint.
pub(in crate::board) fn paint_with_control(
    buffer: &mut Buffer,
    area: Rect,
    layer: Option<&str>,
    attention: Option<&Attention>,
    observations: Option<&Observations<'_>>,
    look: Look,
    control_width: u16,
) -> Option<Rect> {
    // Whole semantic segments yield in the approved order. Never fit a partial
    // percentage or token count into the space between the two owners.
    let stages = [
        (true, true, true, true),
        (false, true, true, true),
        (false, false, true, true),
        (false, false, false, true),
        (false, false, false, false),
    ];
    for (index, (weekly, words, window, brand)) in stages.into_iter().enumerate() {
        let right = attention.map(|value| {
            if words {
                value.fitted(look, usize::from(area.width))
            } else {
                value.with_words(look, false)
            }
        });
        let mut left_width = right.as_ref().map_or(area.width, |line| {
            area.width
                .saturating_sub((line.width() as u16).saturating_add(2))
        });
        let mut caption = Line::from(vec![
            Span::styled(
                if brand { " ▚ TMT   " } else { " ▚ TMT" },
                look.role(Role::Text),
            ),
            Span::styled(if brand { "OPS / UI" } else { "" }, look.role(Role::Muted)),
        ]);
        let control = (control_width > 0
            && control_width.saturating_add(caption.width() as u16 + 1) <= left_width
            && area.height > 0)
            .then(|| {
                Rect::new(
                    area.x + left_width - control_width,
                    area.y,
                    control_width,
                    1,
                )
            });
        if control.is_some() {
            left_width = left_width.saturating_sub(control_width + 1);
        }
        let extra = observations.map(|value| {
            let width = (index + 1 == stages.len())
                .then(|| usize::from(left_width).saturating_sub(caption.width()));
            value.line(weekly, window, width, look)
        });
        if let Some(extra) = extra.filter(|line| !line.spans.is_empty()) {
            if caption.width() + extra.width() > usize::from(left_width) {
                if index + 1 < stages.len() {
                    continue;
                }
                // At extremely small sizes preserve attention evidence rather
                // than clipping an observation into an apparent smaller value.
            } else {
                caption.spans.extend(extra.spans);
            }
        }
        let name_width = left_width.saturating_sub(caption.width() as u16 + 3);
        if let Some(name) = layer.filter(|_| name_width > 0) {
            let name = super::notes::single_line(name);
            let fitted = super::view::fit(&name, usize::from(name_width).min(name.width()));
            caption
                .spans
                .push(Span::styled(format!(" · {fitted}"), look.role(Role::Muted)));
        }
        if caption.width() > usize::from(left_width) {
            caption = Line::styled(
                super::view::fit(&caption.to_string(), usize::from(left_width)),
                look.role(Role::Muted),
            );
        }
        strip::paint_left(
            buffer,
            Rect {
                width: left_width,
                ..area
            },
            caption,
        );
        if let Some(right) = right {
            strip::paint_right(buffer, area, right);
        }
        return control;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_control_never_clips_the_shell_caption_or_attention() {
        let attention = Attention {
            online: "28".into(),
            unread: Default::default(),
            waiting: "9".into(),
            blocked: "1".into(),
            waiting_positive: true,
            blocked_positive: true,
            complete: true,
            ascii: false,
        };
        for width in [60, 100] {
            let area = Rect::new(0, 0, width, 1);
            let mut buffer = Buffer::empty(area);
            let control = paint_with_control(
                &mut buffer,
                area,
                None,
                Some(&attention),
                None,
                Look::default(),
                15,
            );
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains("OPS / UI"), "{text}");
            assert!(
                text.contains("online") && text.contains("waiting") && text.contains("blocked"),
                "{text}"
            );
            assert_eq!(control.is_some(), width == 100);
            if let Some(control) = control {
                assert_eq!(control.width, 15);
                assert!(control.right() <= area.right());
                assert!(
                    buffer.content[usize::from(control.x)..usize::from(control.right())]
                        .iter()
                        .all(|cell| cell.symbol() == " ")
                );
            }
        }
    }
    #[test]
    fn only_nonzero_attention_glyphs_use_state_colour() {
        let look = Look::default();
        let mut attention = Attention {
            online: "28".into(),
            unread: Default::default(),
            waiting: "9".into(),
            blocked: "1".into(),
            waiting_positive: true,
            blocked_positive: true,
            complete: true,
            ascii: false,
        };
        let line = attention.line(look);
        assert_eq!(line.spans[1].style, look.role(Role::Waiting));
        assert_eq!(line.spans[3].style, look.role(Role::Blocked));
        for index in [0, 2, 4] {
            assert_eq!(line.spans[index].style, look.role(Role::Muted));
        }
        attention.waiting_positive = false;
        attention.blocked_positive = false;
        assert!(
            attention
                .line(look)
                .spans
                .iter()
                .all(|span| span.style == look.role(Role::Muted))
        );
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 1));
        paint(
            &mut buffer,
            Rect::new(0, 0, 60, 1),
            Some("a very long named layer that yields"),
            Some(&attention),
            None,
            look,
        );
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("online") && text.contains("waiting") && text.contains("blocked"));
    }
}

#[cfg(test)]
mod narrow_tests {
    use super::*;
    #[test]
    fn partial_attention_wins_over_the_caption_without_a_named_layer_too() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 1));
        let attention = Attention {
            online: "≥28".into(),
            unread: Default::default(),
            waiting: "≥9".into(),
            blocked: "≥1".into(),
            waiting_positive: true,
            blocked_positive: true,
            complete: false,
            ascii: false,
        };
        paint(
            &mut buffer,
            Rect::new(0, 0, 60, 1),
            None,
            Some(&attention),
            None,
            Look::default(),
        );
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            text.contains("online")
                && text.contains("waiting")
                && text.contains("blocked")
                && text.contains("totals unknown"),
            "{text}"
        );
    }
}

#[cfg(test)]
mod unread_tests {
    use super::*;
    #[test]
    fn unread_is_neutral_except_its_glyph_and_attention_fits_before_a_named_layer() {
        let attention = Attention {
            online: "≥28".into(),
            unread: super::super::unread::Count {
                total: 50,
                more: true,
            },
            waiting: "≥9".into(),
            blocked: "≥1".into(),
            waiting_positive: true,
            blocked_positive: true,
            complete: false,
            ascii: false,
        };
        let look = Look::default();
        let line = attention.line(look);
        assert_eq!(line.spans[3].style, look.role(Role::Unread));
        assert_eq!(line.spans[4].style, look.role(Role::Muted));
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 1));
        paint(
            &mut buffer,
            Rect::new(0, 0, 60, 1),
            Some("long layer name yields first"),
            Some(&attention),
            None,
            look,
        );
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        for word in ["≥28 online", "≥9 waiting", "50+ unread", "≥1 blocked"] {
            assert!(text.contains(word), "{text}");
        }
    }
}
