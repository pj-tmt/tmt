//! The composer: a box as tall as its content (one to six text rows) with a blank padding row above
//! and below, so three rows at least, above one stable info line. The box grows upward, so the info
//! line and footer never move. Short terminals give the padding up first (`padded` is false).
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

mod info;
pub(in crate::board) use info::Info;

/// The text grows up to this many rows, then scrolls inside.
const MAX_TEXT_ROWS: u16 = 6;
/// A blank row above and below the text, in every focus state.
const PADDING_ROWS: u16 = 1;
pub(in crate::board) const MAX_BOX_ROWS: u16 = MAX_TEXT_ROWS + 2 * PADDING_ROWS;

/// Padding rows above plus below the text.
fn padding(padded: bool) -> u16 {
    if padded { 2 * PADDING_ROWS } else { 0 }
}
pub(in crate::board) const GUTTER: u16 = 1;
pub(in crate::board) const TEXT_INSET: u16 = 2;

/// The editable width inside a band of this width.
pub(in crate::board) fn editor_width(band_width: u16) -> u16 {
    let inset = if band_width > TEXT_INSET * 2 {
        TEXT_INSET
    } else {
        0
    };
    band_width.saturating_sub(inset * 2)
}

/// Box rows for this content: one text row when empty, then one more per wrapped or explicit line,
/// capped, plus the padding. Sending or clearing returns to the empty box in the same frame.
pub(in crate::board) fn box_rows(
    text: Option<&tmt_tui::components::Editor>,
    band_width: u16,
    padded: bool,
) -> u16 {
    let width = editor_width(band_width);
    let lines = match text {
        Some(text) if width > 0 => text.layout(width).lines.len(),
        _ => 1,
    };
    lines.clamp(1, usize::from(MAX_TEXT_ROWS)) as u16 + padding(padded)
}

/// Box plus the info line.
pub(in crate::board) fn rows(
    text: Option<&tmt_tui::components::Editor>,
    band_width: u16,
    padded: bool,
) -> u16 {
    box_rows(text, band_width, padded) + 1
}

#[derive(Clone, Copy)]
pub(in crate::board) enum Caption<'a> {
    Message(&'a str),
    Note { to: &'a str },
}
impl Caption<'_> {
    fn recipient(self) -> String {
        super::notes::single_line(match self {
            Self::Message(to) | Self::Note { to, .. } => to,
        })
    }
}

pub(in crate::board) fn style(look: Look, focused: bool) -> Style {
    let fill = look.role(if focused {
        Role::Selection
    } else {
        Role::Track
    });
    if let Some(bg) = fill.bg {
        look.role(Role::Text).bg(bg)
    } else {
        look.role(Role::Text).add_modifier(Modifier::REVERSED)
    }
}

/// Foreground roles still describe text when the terminal fill uses inversion.
pub(in crate::board) fn muted_style(look: Look, focused: bool) -> Style {
    let base = style(look, focused);
    if base.add_modifier.contains(Modifier::REVERSED) {
        // A palette's muted color may match the inverted fill. Keep the
        // contrast-bearing Text ink and dim it without replacing either color.
        base.add_modifier(Modifier::DIM)
    } else {
        base.patch(look.role(Role::Muted))
    }
}

/// How tall the box should be: the drafted box rows (none while empty) and whether it is padded.
#[derive(Clone, Copy)]
pub(in crate::board) struct Shape {
    pub draft_rows: Option<u16>,
    pub padded: bool,
}

pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    caption: Caption<'_>,
    start: &str,
    focused: bool,
    shape: Shape,
    look: Look,
) -> Rect {
    let Shape { draft_rows, padded } = shape;
    let info_rows = u16::from(area.height >= 2);
    let available = area.height.saturating_sub(info_rows);
    let wanted = draft_rows
        .unwrap_or(1 + padding(padded))
        .clamp(1, MAX_BOX_ROWS);
    let rows = available.min(wanted);
    let bottom = area.bottom().saturating_sub(info_rows);
    let box_area = Rect::new(area.x, bottom.saturating_sub(rows), area.width, rows);
    let base = style(look, focused);
    for y in box_area.y..box_area.bottom() {
        strip::paint_left(
            buffer,
            Rect::new(box_area.x, y, box_area.width, 1),
            Line::styled(" ".repeat(usize::from(box_area.width)), base),
        );
    }
    let inset = area.width.saturating_sub(editor_width(area.width)) / 2;
    // A box too short for its padding gives the rows to the text first.
    let text_rows = wanted - padding(padded);
    let pad = (rows.saturating_sub(text_rows) / 2).min(if padded { PADDING_ROWS } else { 0 });
    let editor = Rect::new(
        box_area.x + inset,
        box_area.y + pad,
        box_area.width.saturating_sub(inset * 2),
        box_area.height - 2 * pad,
    );
    if draft_rows.is_none() && !editor.is_empty() {
        let placeholder = match caption {
            Caption::Message(_) => format!("{start} to message {}", caption.recipient()),
            Caption::Note { .. } => format!("{start} note to {}", caption.recipient()),
        };
        strip::paint_left(
            buffer,
            Rect {
                height: 1,
                ..editor
            },
            Line::styled(placeholder, muted_style(look, focused)),
        );
    }
    editor
}

/// Reserve the overflow label outside the editable/pointer rectangle. Narrowing
/// can increase the line count, so converge while the editor width only shrinks.
pub(in crate::board) fn viewport(
    area: Rect,
    text: &tmt_tui::components::Editor,
) -> (Rect, Option<(Rect, String)>) {
    use unicode_width::UnicodeWidthStr;
    if area.is_empty() {
        return (area, None);
    }
    let mut width = area.width;
    loop {
        let count = text.layout(width).lines.len();
        if count <= usize::from(area.height) {
            return (area, None);
        }
        let label = format!("↑ {} more", count - usize::from(area.height));
        let reserved = (label.width() + 1).min(usize::from(area.width.saturating_sub(1))) as u16;
        let next = area.width - reserved;
        if next >= width {
            let editor = Rect { width, ..area };
            let hint = Rect::new(
                editor.right().saturating_add(1).min(area.right()),
                area.bottom() - 1,
                area.right()
                    .saturating_sub(editor.right().saturating_add(1)),
                1,
            );
            return (editor, (!hint.is_empty()).then_some((hint, label)));
        }
        width = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placeholder_keeps_the_same_background_as_its_padding_in_every_look() {
        use tmt_cli_style::{Base, Depth, Theme};
        let background = |cell: &ratatui::buffer::Cell| {
            if cell.modifier.contains(Modifier::REVERSED) {
                cell.fg
            } else {
                cell.bg
            }
        };
        for (base, depth) in [
            (Base::Tmt, Depth::TrueColor),
            (Base::TmtLight, Depth::TrueColor),
            (Base::Terminal, Depth::Ansi16),
            (Base::Tmt, Depth::None),
        ] {
            for focused in [false, true] {
                let look = Look {
                    theme: Theme::new(base),
                    depth,
                };
                let area = Rect::new(0, 0, 60, MAX_BOX_ROWS + 1);
                let mut buffer = Buffer::empty(area);
                let editor = paint(
                    &mut buffer,
                    area,
                    Caption::Message("worker"),
                    "⏎",
                    focused,
                    Shape {
                        draft_rows: None,
                        padded: true,
                    },
                    look,
                );
                assert_eq!(
                    background(&buffer[(editor.x, editor.y)]),
                    background(&buffer[(area.right() - 1, editor.y)]),
                    "{base:?}/{depth:?}, focused={focused}: a muted placeholder must not change the box fill"
                );
                let placeholder = &buffer[(editor.x, editor.y)];
                let padding = &buffer[(area.right() - 1, editor.y)];
                if padding.modifier.contains(Modifier::REVERSED) {
                    assert_eq!(placeholder.bg, padding.bg, "keep the inverted text ink");
                    assert!(placeholder.modifier.contains(Modifier::DIM));
                }
            }
        }
    }
    #[test]
    fn the_box_pads_its_text_grows_upward_and_never_moves_info_or_footer() {
        let look = Look::default();
        let area = Rect::new(0, 0, 80, MAX_BOX_ROWS + 1);
        let mut anchor = None;
        // (focused, drafted box rows, expected text rows, expected box rows)
        for (focused, draft, text, boxed) in [
            (false, None, 1, 3),
            (true, None, 1, 3),
            (false, Some(3), 1, 3),
            (true, Some(3), 1, 3),
            (true, Some(4), 2, 4),
            (true, Some(5), 3, 5),
            (true, Some(8), 6, 8),
            (true, Some(12), 6, 8),
        ] {
            let mut buffer = Buffer::empty(area);
            let editor = paint(
                &mut buffer,
                area,
                Caption::Message("worker"),
                "⏎",
                focused,
                Shape {
                    draft_rows: draft,
                    padded: true,
                },
                look,
            );
            assert_eq!(editor.height, text, "{focused} {draft:?}");
            // The padding rows stay above and below the text at every height.
            let box_top = editor.y - 1;
            assert_eq!(area.bottom() - 1 - box_top, boxed, "{focused} {draft:?}");
            assert_eq!(editor.bottom() + 1, area.bottom() - 1);
            let row = |y: u16| -> String { (0..80).map(|x| buffer[(x, y)].symbol()).collect() };
            assert!(row(box_top).trim().is_empty() && row(editor.bottom()).trim().is_empty());
            assert_eq!(
                buffer[(0, box_top)].bg,
                buffer[(0, editor.y)].bg,
                "the padding shares the box fill"
            );
            // The info line below the box never moves.
            assert_eq!(
                editor.bottom() + 1,
                *anchor.get_or_insert(editor.bottom() + 1)
            );
            assert_eq!(
                buffer[(0, area.bottom() - 1)].bg,
                ratatui::style::Color::Reset,
                "info is outside the box fill"
            );
            let text_on_screen = buffer
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert_eq!(
                text_on_screen.contains("⏎ to message worker"),
                draft.is_none(),
                "the placeholder shows on the single empty row only"
            );
        }
    }
    #[test]
    fn short_terminals_give_the_padding_up_first() {
        let look = Look::default();
        // Unpadded: the old one-row box.
        let area = Rect::new(0, 0, 80, 2);
        let mut buffer = Buffer::empty(area);
        let editor = paint(
            &mut buffer,
            area,
            Caption::Message("worker"),
            "⏎",
            true,
            Shape {
                draft_rows: None,
                padded: false,
            },
            look,
        );
        assert_eq!((editor.y, editor.height), (0, 1));
        // Padded but squeezed to four rows: three text rows keep the text, not the padding.
        let area = Rect::new(0, 0, 80, 4);
        let mut buffer = Buffer::empty(area);
        let editor = paint(
            &mut buffer,
            area,
            Caption::Message("worker"),
            "⏎",
            true,
            Shape {
                draft_rows: Some(5),
                padded: true,
            },
            look,
        );
        assert_eq!((editor.y, editor.height), (0, 3));
    }
    #[test]
    fn box_rows_count_wrapped_and_explicit_lines_up_to_the_cap() {
        let editor = |text: &str| tmt_tui::components::Editor::from(text);
        for (padded, extra) in [(false, 0), (true, 2)] {
            assert_eq!(box_rows(None, 80, padded), 1 + extra);
            assert_eq!(box_rows(Some(&editor("")), 80, padded), 1 + extra);
            assert_eq!(box_rows(Some(&editor("one line")), 80, padded), 1 + extra);
            assert_eq!(box_rows(Some(&editor("a\nb")), 80, padded), 2 + extra);
            assert_eq!(
                box_rows(Some(&editor("a\nb\nc\nd\ne\nf")), 80, padded),
                6 + extra
            );
            assert_eq!(
                box_rows(Some(&editor(&"x\n".repeat(20))), 80, padded),
                6 + extra
            );
            // Wrapping counts: 30 columns wrap at an editor width of 20.
            assert_eq!(
                box_rows(Some(&editor(&"x".repeat(30))), 24, padded),
                2 + extra
            );
            assert_eq!(rows(Some(&editor("a\nb")), 80, padded), 3 + extra);
            assert_eq!(box_rows(Some(&editor("a\nb")), 0, padded), 1 + extra);
        }
    }
    #[test]
    fn the_empty_note_placeholder_uses_the_same_single_text_row() {
        let area = Rect::new(0, 0, 59, MAX_BOX_ROWS + 1);
        let mut buffer = Buffer::empty(area);
        let editor = paint(
            &mut buffer,
            area,
            Caption::Note { to: "lead" },
            "⏎",
            true,
            Shape {
                draft_rows: None,
                padded: true,
            },
            Look::default(),
        );
        assert_eq!(editor.height, 1);
        let text = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("⏎ note to lead"));
        assert!(!text.contains("about worker"));
    }
}

#[cfg(test)]
mod viewport_tests {
    use super::*;
    #[test]
    fn overflow_never_overwrites_typed_text_or_fabricates_a_pointer_cell() {
        let text: tmt_tui::components::Editor = "first\nsecond\nthird\nfourth\nfifth".into();
        let area = Rect::new(2, 5, 40, 3);
        let (editor, hint) = viewport(area, &text);
        let (hint, label) = hint.unwrap();
        assert_eq!(editor.height, 3);
        assert!(editor.right() < hint.x);
        assert!(editor.intersection(hint).is_empty());
        assert!(label.starts_with("↑ ") && label.ends_with(" more"));
        assert_eq!(text.as_str(), "first\nsecond\nthird\nfourth\nfifth");
    }
    #[test]
    fn every_small_field_keeps_its_editor_within_the_available_envelope() {
        for width in 0..=160 {
            for height in 0..=7 {
                let area = Rect::new(3, 4, width, height);
                for (focused, draft, padded) in [
                    (false, false, false),
                    (true, false, true),
                    (false, true, false),
                    (true, true, true),
                ] {
                    let mut buffer = Buffer::empty(area);
                    let editor = paint(
                        &mut buffer,
                        area,
                        Caption::Message("worker"),
                        "⏎",
                        focused,
                        Shape {
                            draft_rows: draft.then_some(5),
                            padded,
                        },
                        Look::default(),
                    );
                    assert!(editor.x >= area.x && editor.y >= area.y);
                    assert!(editor.right() <= area.right() && editor.bottom() <= area.bottom());
                    if width > 0 && height > 0 {
                        assert!(editor.height > 0);
                    }
                }
            }
        }
    }
}
