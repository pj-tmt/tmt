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
                format!(
                    " {}{}  ",
                    self.waiting,
                    if words { " need you" } else { "" }
                ),
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
    let caption = Caption {
        layer,
        ..Caption::default()
    };
    paint_with_control(buffer, area, caption, attention, observations, look, 0);
}

/// What the caption names after the mark: the named view layer and the running version,
/// with the Ops build's own when it differs from the CLI's.
#[derive(Clone, Copy, Default)]
pub(in crate::board) struct Caption<'a> {
    pub layer: Option<&'a str>,
    pub version: Option<&'a str>,
    pub ops: Option<&'a str>,
}

/// Paints the caption and attention strip. A control of `control_width` columns takes the right
/// end of the caption side when the stage keeps the whole caption beside it; its rectangle is
/// returned for the caller to paint.
pub(in crate::board) fn paint_with_control(
    buffer: &mut Buffer,
    area: Rect,
    Caption {
        layer,
        version,
        ops,
    }: Caption<'_>,
    attention: Option<&Attention>,
    observations: Option<&Observations<'_>>,
    look: Look,
    control_width: u16,
) -> Option<Rect> {
    // Whole semantic segments yield in the approved order. Never fit a partial
    // percentage or token count into the space between the two owners. The Ops build's
    // version yields first, then the CLI's, before any of them; the mark never does.
    let stages = [
        (2, true, true, true, true),
        (1, true, true, true, true),
        (0, true, true, true, true),
        (0, false, true, true, true),
        (0, false, false, true, true),
        (0, false, false, false, true),
        (0, false, false, false, false),
    ];
    for (index, (named, weekly, words, window, brand)) in stages.into_iter().enumerate() {
        // A stage that names a version the caption has none of is the stage below it.
        if (named == 2 && ops.is_none()) || (named > 0 && version.is_none()) {
            continue;
        }
        let version = version.filter(|_| named > 0);
        let ops = ops.filter(|_| named == 2);
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
        let mut caption = Line::from(vec![Span::styled(" ▚ TMT", look.role(Role::Text))]);
        if let Some(version) = version {
            let ops = ops.map(|ops| format!(" · ops {ops}")).unwrap_or_default();
            caption.spans.push(Span::styled(
                format!(" {version}{ops}"),
                look.role(Role::Muted),
            ));
        }
        caption.spans.extend([
            Span::styled(if brand { "   " } else { "" }, look.role(Role::Text)),
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
        if version.is_some() && control_width > 0 && control.is_none() {
            continue;
        }
        if control.is_some() {
            left_width = left_width.saturating_sub(control_width + 1);
        }
        let extra = observations.map(|value| {
            let width = (index + 1 == stages.len())
                .then(|| usize::from(left_width).saturating_sub(caption.width()));
            value.line(weekly, window, width, look)
        });
        if version.is_some()
            && caption.width() + extra.as_ref().map_or(0, Line::width) > usize::from(left_width)
        {
            continue;
        }
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
                Caption::default(),
                Some(&attention),
                None,
                Look::default(),
                15,
            );
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains("OPS / UI"), "{text}");
            assert!(
                text.contains("online") && text.contains("need you") && text.contains("blocked"),
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
    fn attention() -> Attention {
        Attention {
            online: "28".into(),
            unread: Default::default(),
            waiting: "9".into(),
            blocked: "1".into(),
            waiting_positive: true,
            blocked_positive: true,
            complete: true,
            ascii: false,
        }
    }
    fn top(
        width: u16,
        version: Option<(&str, Option<&str>)>,
        control: u16,
    ) -> (String, Option<Rect>) {
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        let control = paint_with_control(
            &mut buffer,
            area,
            Caption {
                layer: None,
                version: version.map(|(version, _)| version),
                ops: version.and_then(|(_, ops)| ops),
            },
            Some(&attention()),
            None,
            Look::default(),
            control,
        );
        let text = buffer.content.iter().map(|cell| cell.symbol()).collect();
        (text, control)
    }
    const VERSION: (&str, Option<&str>) = ("5.0.0-alpha.378", Some("0.1.0-alpha.4"));
    #[test]
    fn the_version_follows_the_mark_and_a_missing_one_leaves_the_mark_alone() {
        let (text, _) = top(150, Some(VERSION), 0);
        assert!(
            text.starts_with(" ▚ TMT 5.0.0-alpha.378 · ops 0.1.0-alpha.4   OPS / UI"),
            "{text:?}"
        );
        let (loading, _) = top(150, None, 0);
        assert!(loading.starts_with(" ▚ TMT   OPS / UI"), "{loading:?}");
        let muted = Look::default().role(Role::Muted);
        let area = Rect::new(0, 0, 150, 1);
        let mut buffer = Buffer::empty(area);
        paint_with_control(
            &mut buffer,
            area,
            Caption {
                layer: None,
                version: Some("5.0.0"),
                ops: None,
            },
            None,
            None,
            Look::default(),
            0,
        );
        assert_eq!(buffer[(7, 0)].symbol(), "5");
        assert_eq!(Some(buffer[(7, 0)].fg), muted.fg, "the version is muted");
        assert_eq!(
            Some(buffer[(2, 0)].fg),
            Look::default().role(Role::Text).fg,
            "the mark keeps its role"
        );
    }
    #[test]
    fn the_version_yields_before_any_count_the_caption_or_the_control_and_the_mark_stays() {
        let (mut shown, mut dropped, mut only_cli) = (0, 0, 0);
        for width in (14..=150).rev() {
            for control in [0, 15] {
                let (with, with_control) = top(width, Some(VERSION), control);
                let (without, without_control) = top(width, None, control);
                if with.contains("5.0.0-alpha.378") {
                    shown += 1;
                    only_cli += usize::from(!with.contains("ops 0.1.0"));
                    assert_eq!(
                        with_control.is_some(),
                        without_control.is_some(),
                        "{width}: the version never takes the control's room"
                    );
                    for word in ["online", "need you", "blocked", "OPS / UI"] {
                        assert_eq!(
                            with.contains(word),
                            without.contains(word),
                            "{width} {word}"
                        );
                    }
                } else {
                    dropped += 1;
                    assert_eq!(with, without, "{width}: a dropped version leaves no trace");
                    assert_eq!(with_control, without_control, "{width}");
                }
                assert_eq!(
                    with.starts_with(" ▚ TMT"),
                    without.starts_with(" ▚ TMT"),
                    "{width}: the version never costs the mark: {with:?}"
                );
                if width >= 60 {
                    assert!(with.starts_with(" ▚ TMT"), "{width}: {with:?}");
                }
            }
        }
        assert!(shown > 0 && dropped > 0, "{shown} shown, {dropped} dropped");
        assert!(only_cli > 0, "the Ops build yields before the CLI version");
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
        assert!(text.contains("online") && text.contains("need you") && text.contains("blocked"));
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
                && text.contains("need you")
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
        for word in ["≥28 online", "≥9 need you", "50+ unread", "≥1 blocked"] {
            assert!(text.contains(word), "{text}");
        }
    }
}
