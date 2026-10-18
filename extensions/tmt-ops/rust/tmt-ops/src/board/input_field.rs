//! A fixed composer envelope: a one/three-row box above one stable info line.
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

/// Reserve the maximum box height even while its empty idle paint is one row.
pub(in crate::board) const ROWS: u16 = 4;
pub(in crate::board) const GUTTER: u16 = 1;
pub(in crate::board) const TEXT_INSET: u16 = 2;

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

pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    caption: Caption<'_>,
    start: &str,
    focused: bool,
    has_draft: bool,
    look: Look,
) -> Rect {
    let info_rows = u16::from(area.height >= 2);
    let available = area.height.saturating_sub(info_rows);
    let rows = available.min(if focused || has_draft { 3 } else { 1 });
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
    let inset = if area.width > TEXT_INSET * 2 {
        TEXT_INSET
    } else {
        0
    };
    let editor = Rect::new(
        box_area.x + inset,
        box_area.y,
        box_area.width.saturating_sub(inset * 2),
        box_area.height,
    );
    if !has_draft && !editor.is_empty() {
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
                let area = Rect::new(0, 0, 60, ROWS);
                let mut buffer = Buffer::empty(area);
                let editor = paint(
                    &mut buffer,
                    area,
                    Caption::Message("worker"),
                    "⏎",
                    focused,
                    false,
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
    fn empty_idle_box_grows_upward_without_moving_info_or_footer() {
        let look = Look::default();
        let area = Rect::new(0, 0, 80, ROWS);
        let mut idle = Buffer::empty(area);
        let empty = paint(
            &mut idle,
            area,
            Caption::Message("worker"),
            "⏎",
            false,
            false,
            look,
        );
        assert_eq!(empty.height, 1);
        assert_eq!(empty.bottom(), area.bottom() - 1);
        assert!(
            idle.content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .contains("⏎ to message worker")
        );
        for focused in [false, true] {
            let mut buffer = Buffer::empty(area);
            let editor = paint(
                &mut buffer,
                area,
                Caption::Message("worker"),
                "⏎",
                focused,
                true,
                look,
            );
            assert_eq!(editor.height, 3);
            assert_eq!(editor.bottom(), empty.bottom());
            assert_eq!(
                buffer[(0, 3)].bg,
                ratatui::style::Color::Reset,
                "info is outside the box fill"
            );
            let text = buffer
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(!text.contains("as you") && !text.contains("Esc cancel"));
        }
    }
    #[test]
    fn focused_empty_and_note_placeholder_use_the_same_three_row_editor() {
        let area = Rect::new(0, 0, 59, ROWS);
        let mut buffer = Buffer::empty(area);
        let editor = paint(
            &mut buffer,
            area,
            Caption::Note { to: "lead" },
            "⏎",
            true,
            false,
            Look::default(),
        );
        assert_eq!(editor.height, 3);
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
                for (focused, draft) in [(false, false), (true, false), (false, true), (true, true)]
                {
                    let mut buffer = Buffer::empty(area);
                    let editor = paint(
                        &mut buffer,
                        area,
                        Caption::Message("worker"),
                        "⏎",
                        focused,
                        draft,
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
