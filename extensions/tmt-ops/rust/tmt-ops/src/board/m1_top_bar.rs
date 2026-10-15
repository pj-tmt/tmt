//! M1 TopBar: fitted caption and globally observed attention; no acquisition.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) struct Attention {
    pub online: String,
    pub waiting: String,
    pub blocked: String,
    pub waiting_positive: bool,
    pub blocked_positive: bool,
    pub complete: bool,
    pub ascii: bool,
}
impl Attention {
    fn line(&self, look: Look) -> Line<'static> {
        Line::from(vec![
            Span::styled(
                format!(
                    "{} {} online  ",
                    if self.ascii { "+" } else { "●" },
                    self.online
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
                format!(" {} waiting  ", self.waiting),
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
                    " {} blocked{}",
                    self.blocked,
                    if self.complete {
                        ""
                    } else {
                        " · totals unknown"
                    }
                ),
                look.role(Role::Muted),
            ),
        ])
    }
}

pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    layer: Option<&str>,
    attention: Option<&Attention>,
    look: Look,
) {
    let mut caption = Line::from(vec![
        Span::styled(" ▚ TMT   ", look.role(Role::Text)),
        Span::styled("OPS / UI", look.role(Role::Muted)),
    ]);
    let caption_width = caption.width() as u16;
    let attention = attention.map(|value| value.line(look));
    let left_width = attention.as_ref().map_or(area.width, |line| {
        area.width
            .saturating_sub((line.width() as u16).saturating_add(2))
    });
    let name_width = left_width.saturating_sub(caption_width + 3);
    if let Some(name) = layer.filter(|_| name_width > 0) {
        let name = super::notes::single_line(name);
        let fitted = super::view::fit(&name, usize::from(name_width).min(name.width()));
        caption
            .spans
            .push(Span::styled(format!(" · {fitted}"), look.role(Role::Muted)));
    }
    if left_width < caption_width {
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
    if let Some(attention) = attention {
        strip::paint_right(buffer, area, attention);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_nonzero_attention_glyphs_use_state_colour() {
        let look = Look::default();
        let mut attention = Attention {
            online: "28".into(),
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
