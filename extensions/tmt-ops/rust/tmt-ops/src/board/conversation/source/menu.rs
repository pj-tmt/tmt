//! The Selection menu: a small bordered box with one action per line (`q› Quote`,
//! `y  Copy`, `n  Add to Notes`) and a dim `Esc clears` line, opened directly below the end of a selection
//! so it is clear which text it acts on.
use crate::board::app::App;
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::Style,
    text::Line,
};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::components::{Modal, Placement, strip};
use unicode_width::UnicodeWidthStr;

const TITLE: &str = "Selection";
const HINT: &str = "Esc clears";
/// Each entry is its key cell, then its word; the key cell marks the Enter default with `›`.
const ENTRIES: [(Entry, &str, &str); 3] = [
    (Entry::Quote, "q›", "Quote"),
    (Entry::Copy, "y ", "Copy"),
    (Entry::Note, "n ", "Add to Notes"),
];

/// What a menu entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Quote,
    Copy,
    Note,
}

/// The painted box and its entry lines of the last frame, for hit testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::board::conversation) struct Menu {
    pub(super) bounds: Rect,
    pub(super) quote: Rect,
    pub(super) copy: Rect,
    pub(super) note: Rect,
}

impl Menu {
    pub(super) fn bounds(&self) -> Rect {
        self.bounds
    }

    pub(super) fn entry_at(&self, point: Position) -> Option<Entry> {
        if self.quote.contains(point) {
            Some(Entry::Quote)
        } else if self.copy.contains(point) {
            Some(Entry::Copy)
        } else if self.note.contains(point) {
            Some(Entry::Note)
        } else {
            None
        }
    }
}

fn label(key: &str, word: &str) -> String {
    format!("{key} {word}")
}

/// The widest line, then one cell of padding and the border on each side.
fn width() -> u16 {
    let lines = ENTRIES
        .iter()
        .map(|(_, key, word)| label(key, word).width())
        .chain([HINT.width()])
        .max()
        .unwrap_or(0);
    // The title sits in the top border with a space on each side.
    (lines + 4).max(TITLE.width() + 4) as u16
}

/// Two borders, one line per entry and the hint.
fn height() -> u16 {
    ENTRIES.len() as u16 + 3
}

/// Where the box goes: its left edge at the selection end (`end` is the cell after the
/// last selected glyph), flipped to end there when the right edge is too near; on the
/// rows below the end, or above the selection's `first` row when the bottom is too
/// near, so it never covers selected text. It stays inside the `pane`; a pane too small
/// for it falls back to the `frame`. `None` when neither has room.
pub(super) fn place(pane: Rect, frame: Rect, end: (u16, u16), first: u16) -> Option<Rect> {
    let (width, height) = (width(), height());
    for bound in [pane, frame] {
        if bound.width < width {
            continue;
        }
        let x = if end.0.saturating_add(width) <= bound.right() {
            end.0.max(bound.x)
        } else {
            end.0.saturating_sub(width).max(bound.x)
        };
        let below = end.1 + 1;
        if below + height <= bound.bottom() {
            return Some(Rect::new(x, below, width, height));
        }
        if first >= bound.y + height {
            return Some(Rect::new(x, first - height, width, height));
        }
    }
    None
}

pub(super) fn paint(
    frame: &mut Frame,
    app: &App,
    pane: Rect,
    end: (u16, u16),
    first: u16,
) -> Option<Menu> {
    let bounds = place(pane, frame.area(), end, first)?;
    let look = app.look();
    let dim = look.role(Role::Dim);
    // The modal clears the cells under the box and draws its titled square border.
    let modal = Modal {
        title: TITLE.into(),
        placement: Placement::Body,
    };
    let areas = modal.areas(bounds, [bounds.width, bounds.height], false, false);
    modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
    let inner = Rect::new(
        bounds.x + 1,
        bounds.y + 1,
        bounds.width - 2,
        bounds.height - 2,
    );
    let can_note = app.conversation_checklist_key().is_some() && !app.loading();
    let mut lines = Vec::new();
    for (index, (entry, key, word)) in ENTRIES.iter().enumerate() {
        let line = Rect::new(inner.x, inner.y + index as u16, inner.width, 1);
        // The chip pads its label by a cell on each side, so the hover fills the line.
        let text = format!(
            "{:<width$}",
            label(key, word),
            width = usize::from(inner.width) - 2
        );
        let painted = crate::board::buttons::paint(
            app,
            frame.buffer_mut(),
            line,
            &[crate::board::buttons::Button::secondary(&text)
                .enabled(*entry != Entry::Note || can_note)
                .idle(Style::default())],
            Align::Left,
            None,
        );
        lines.push(painted[0]?);
    }
    strip::paint_left(
        frame.buffer_mut(),
        Rect::new(
            inner.x + 1,
            inner.y + ENTRIES.len() as u16,
            inner.width - 1,
            1,
        ),
        Line::styled(HINT, dim),
    );
    Some(Menu {
        bounds,
        quote: lines[0],
        copy: lines[1],
        note: if can_note { lines[2] } else { Rect::default() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANE: Rect = Rect::new(10, 5, 60, 20);
    const FRAME: Rect = Rect::new(0, 0, 100, 36);

    fn at(end: (u16, u16), first: u16) -> Rect {
        place(PANE, FRAME, end, first).unwrap()
    }

    #[test]
    fn the_box_is_as_wide_as_its_widest_line_and_holds_every_entry() {
        assert_eq!((width(), height()), (19, 6));
        assert!(TITLE.width() as u16 + 4 <= width());
    }

    #[test]
    fn the_menu_opens_below_the_selection_end() {
        let menu = at((30, 8), 8);
        assert_eq!((menu.x, menu.y), (30, 9));
        assert_eq!((menu.width, menu.height), (19, 6));
    }

    #[test]
    fn near_the_right_edge_it_flips_left_to_end_at_the_selection_end() {
        let menu = at((66, 8), 8);
        assert_eq!((menu.right(), menu.y), (66, 9));
        let menu = at((70, 8), 8);
        assert_eq!(menu.right(), PANE.right(), "the end is the pane's edge");
        assert_eq!(at((12, 8), 8).x, 12, "a roomy left end does not flip");
    }

    #[test]
    fn near_the_bottom_it_opens_above_the_first_selected_row() {
        let menu = at((30, 24), 22);
        assert_eq!((menu.x, menu.bottom()), (30, 22));
        let menu = at((68, 24), 24);
        assert_eq!((menu.right(), menu.bottom()), (68, 24));
    }

    #[test]
    fn a_pane_without_room_falls_back_to_the_frame_and_then_to_no_menu() {
        let short = Rect::new(10, 5, 60, 6);
        let menu = place(short, FRAME, (30, 8), 8).unwrap();
        assert_eq!((menu.y, menu.bottom()), (9, 15), "below, over the frame");
        let tiny = Rect::new(0, 0, 60, 6);
        let menu = place(tiny, tiny, (30, 3), 3);
        assert_eq!(menu, None);
        assert_eq!(
            place(
                Rect::new(10, 5, 10, 20),
                Rect::new(10, 5, 10, 20),
                (12, 8),
                8
            ),
            None
        );
    }

    #[test]
    fn the_box_never_touches_the_selected_rows() {
        for (end, first) in [((30, 8), 6), ((66, 12), 10), ((15, 24), 20), ((15, 12), 12)] {
            let menu = at(end, first);
            assert!(
                menu.y > end.1 || menu.bottom() <= first,
                "{end:?} {first} {menu:?}"
            );
        }
    }
}
