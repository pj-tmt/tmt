//! The Selection menu: `q Quote` and `y Copy` on one row, opened directly below the
//! end of a selection so it is clear which text they act on.
use crate::board::app::App;
use ratatui::{
    Frame,
    layout::{Position, Rect},
};
use tmt_cli_style::grid::Align;
use unicode_width::UnicodeWidthStr;

const QUOTE: &str = "q Quote";
const COPY: &str = "y Copy";

/// What a menu entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Quote,
    Copy,
}

/// The painted entries of the last frame, for hit testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::board::conversation) struct Menu {
    pub(super) quote: Rect,
    pub(super) copy: Rect,
}

impl Menu {
    pub(super) fn bounds(&self) -> Rect {
        self.quote.union(self.copy)
    }

    pub(super) fn entry_at(&self, point: Position) -> Option<Entry> {
        if self.quote.contains(point) {
            Some(Entry::Quote)
        } else if self.copy.contains(point) {
            Some(Entry::Copy)
        } else {
            None
        }
    }
}

/// The two padded chips and the cell between them.
fn width() -> u16 {
    (QUOTE.width() + 2 + 1 + COPY.width() + 2) as u16
}

/// Where the menu goes: its left edge at the selection end (`end` is the cell after the
/// last selected glyph), flipped to end there when the right edge is too near; on the
/// row below the end, or above the selection's `first` row when the bottom is too
/// near, so it never covers selected text. `None` when `area` has no room for it.
pub(super) fn place(area: Rect, end: (u16, u16), first: u16) -> Option<Rect> {
    let width = width();
    if area.width < width {
        return None;
    }
    let x = if end.0.saturating_add(width) <= area.right() {
        end.0
    } else {
        end.0.saturating_sub(width).max(area.x)
    };
    let y = if end.1 + 1 < area.bottom() {
        end.1 + 1
    } else if first > area.y {
        first - 1
    } else {
        return None;
    };
    Some(Rect::new(x, y, width, 1))
}

pub(super) fn paint(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    end: (u16, u16),
    first: u16,
) -> Option<Menu> {
    let at = place(area, end, first)?;
    let painted = crate::board::buttons::paint(
        app,
        frame.buffer_mut(),
        at,
        &[
            crate::board::buttons::Button::secondary(QUOTE),
            crate::board::buttons::Button::secondary(COPY),
        ],
        Align::Left,
        None,
    );
    Some(Menu {
        quote: painted[0]?,
        copy: painted[1]?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(10, 5, 60, 20);

    #[test]
    fn the_menu_opens_below_the_selection_end() {
        let at = place(AREA, (30, 8), 8).unwrap();
        assert_eq!((at.x, at.y, at.height), (30, 9, 1));
        assert_eq!(at.width, 18);
    }

    #[test]
    fn near_the_right_edge_it_flips_left_to_end_at_the_selection_end() {
        let at = place(AREA, (66, 8), 8).unwrap();
        assert_eq!((at.right(), at.y), (66, 9));
        assert!(at.x >= AREA.x);
        let at = place(AREA, (70, 8), 8).unwrap();
        assert_eq!(at.right(), AREA.right(), "the end is the area's edge");
        let at = place(AREA, (12, 8), 8).unwrap();
        assert_eq!(at.x, 12, "a roomy left end does not flip");
    }

    #[test]
    fn on_the_last_row_it_opens_above_the_first_selected_row() {
        let at = place(AREA, (30, 24), 22).unwrap();
        assert_eq!((at.x, at.y), (30, 21));
        let at = place(AREA, (68, 24), 24).unwrap();
        assert_eq!((at.right(), at.y), (68, 23));
    }

    #[test]
    fn with_no_room_below_or_above_there_is_no_menu() {
        assert_eq!(place(AREA, (30, 5), 5).map(|at| at.y), Some(6));
        let one = Rect::new(10, 5, 60, 1);
        assert_eq!(place(one, (30, 5), 5), None);
        assert_eq!(place(Rect::new(10, 5, 17, 20), (12, 8), 8), None);
    }

    #[test]
    fn a_menu_below_never_touches_the_selected_rows() {
        for (end, first) in [((30, 8), 6), ((66, 12), 10), ((15, 24), 20)] {
            let at = place(AREA, end, first).unwrap();
            assert!(at.y > end.1 || at.y < first, "{end:?} {first} {at:?}");
        }
    }
}
