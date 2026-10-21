//! Sortable column headers of the Agents list.
//!
//! A header word is a control: a click sorts by its column and a second click
//! reverses it. The click only changes the saved order; it never selects,
//! sends or acts on a row. Hit areas join the painted strip hits, so the
//! pointer and the published hit list share one source.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

use super::{overview::cron_counts::GlyphSet, view_state::Order};
use crate::config::{OpsSort, OverviewColumn, OverviewDirection, OverviewSort};

/// A header that sorts: a column of the flat Agents list, or the squad order a
/// column of the Squads tree stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    Overview(OverviewColumn),
    Squads(OpsSort),
}

impl Header {
    /// The order a click asks for: this column's own first direction, or the
    /// opposite one when the column already is the order.
    pub(in crate::board) fn clicked(self, order: Order) -> Order {
        match self {
            Self::Overview(column) => Order {
                overview: Some(match order.overview {
                    Some(current) if current.column == column => current.reverse(),
                    _ => OverviewSort::initial(column),
                }),
                ..order
            },
            Self::Squads(sort) if order.agents == sort => Order {
                reversed: !order.reversed,
                ..order
            },
            Self::Squads(sort) => Order {
                agents: sort,
                reversed: false,
                ..order
            },
        }
    }

    /// The direction this header is sorted in, when it is the shown order.
    fn direction(self, order: &Order) -> Option<OverviewDirection> {
        use OverviewDirection::{Ascending, Descending};
        match self {
            Self::Overview(column) => order
                .overview
                .filter(|sort| sort.column == column)
                .map(|sort| sort.direction),
            Self::Squads(sort) => (order.agents == sort).then(|| {
                // Names run A to Z; attention runs most first.
                match (sort == OpsSort::Name, order.reversed) {
                    (true, false) | (false, true) => Ascending,
                    _ => Descending,
                }
            }),
        }
    }
}

/// What the headers show: the shown order and the header under the pointer.
#[derive(Clone, Copy)]
pub(in crate::board) struct Marks {
    order: Order,
    hover: Option<Header>,
    glyphs: GlyphSet,
}

impl Marks {
    pub(in crate::board) fn new(order: Order, hover: Option<Header>, glyphs: GlyphSet) -> Self {
        Self {
            order,
            hover,
            glyphs,
        }
    }

    /// The direction of the sorted header, `↕` on a hovered one; never colour alone.
    pub(in crate::board) fn mark(&self, header: Header) -> Option<&'static str> {
        let ascii = matches!(self.glyphs, GlyphSet::Ascii);
        match header.direction(&self.order) {
            Some(OverviewDirection::Ascending) => Some(if ascii { "^" } else { "↑" }),
            Some(OverviewDirection::Descending) => Some(if ascii { "v" } else { "↓" }),
            None if self.hover == Some(header) => Some(if ascii { "~" } else { "↕" }),
            None => None,
        }
    }

    pub(in crate::board) fn style(
        &self,
        header: Header,
        base: Style,
        look: crate::look::Look,
    ) -> Style {
        if self.hover == Some(header) {
            base.patch(look.role(Role::Accent))
        } else {
            base
        }
    }

    /// A header word with its mark one cell after it.
    pub(in crate::board) fn word(
        &self,
        header: Header,
        word: &str,
        base: Style,
        look: crate::look::Look,
    ) -> Span<'static> {
        let text = match self.mark(header) {
            Some(mark) => format!("{word} {mark}"),
            None => word.to_owned(),
        };
        Span::styled(text, self.style(header, base, look))
    }
}

/// The painted width of a header word with its mark cell.
pub(in crate::board) fn hit_width(word: &str) -> u16 {
    word.width() as u16 + 2
}

/// The flat list's header line, painted cell by cell so each sortable word can
/// carry its own style, with the hit areas of those words.
pub(in crate::board) struct Row<'a> {
    marks: &'a Marks,
    look: crate::look::Look,
    base: Style,
    slot: Rect,
    used: u16,
    spans: Vec<Span<'static>>,
    hits: Vec<(Rect, Header)>,
}

impl<'a> Row<'a> {
    pub(in crate::board) fn new(
        slot: Rect,
        marks: &'a Marks,
        look: crate::look::Look,
        base: Style,
    ) -> Self {
        Self {
            marks,
            look,
            base,
            slot,
            used: 0,
            spans: Vec::new(),
            hits: Vec::new(),
        }
    }

    /// Text that does not sort.
    pub(in crate::board) fn plain(&mut self, text: &str) {
        self.used += text.width() as u16;
        self.spans.push(Span::styled(text.to_owned(), self.base));
    }

    /// A sortable word in a `width`-cell column, mark after it.
    pub(in crate::board) fn word(&mut self, header: Header, word: &str, width: u16) {
        let span = self.marks.word(header, word, self.base, self.look);
        let pad = usize::from(width).saturating_sub(span.content.width());
        self.hit(header, self.used, hit_width(word));
        self.used += span.content.width() as u16;
        self.spans.push(span);
        self.plain(&" ".repeat(pad));
    }

    /// The `TOK` word of a right-aligned `caption` in a `width`-cell column:
    /// its mark takes the cell before the word, so no column moves, and the
    /// rest of the caption is the window word, which is its own control.
    pub(in crate::board) fn tokens(&mut self, header: Header, caption: &str, width: u16) {
        let (word, window) = caption.split_at(caption.find(' ').unwrap_or(caption.len()));
        let lead = usize::from(width).saturating_sub(caption.width());
        if lead == 0 {
            self.plain(caption);
            return;
        }
        self.plain(&" ".repeat(lead - 1));
        let mark = self.marks.mark(header).unwrap_or(" ");
        self.hit(header, self.used, word.width() as u16 + 1);
        self.spans.push(Span::styled(
            format!("{mark}{word}"),
            self.marks.style(header, self.base, self.look),
        ));
        self.used += word.width() as u16 + 1;
        self.plain(window);
    }

    fn hit(&mut self, header: Header, from: u16, width: u16) {
        let area = Rect::new(self.slot.x.saturating_add(from), self.slot.y, width, 1)
            .intersection(self.slot);
        if area.width > 0 {
            self.hits.push((area, header));
        }
    }

    pub(in crate::board) fn finish(self) -> (Line<'static>, Vec<(Rect, Header)>) {
        (Line::from(self.spans), self.hits)
    }
}

/// The hit area of a tree header word starting `lead` cells into its slot.
pub(in crate::board) fn word_hit(slot: Rect, lead: u16, word: &str) -> Option<Rect> {
    let area =
        Rect::new(slot.x.saturating_add(lead), slot.y, hit_width(word), 1).intersection(slot);
    (area.width > 0).then_some(area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use OverviewDirection::{Ascending, Descending};

    fn flat(column: OverviewColumn, direction: OverviewDirection) -> Order {
        Order {
            overview: Some(OverviewSort { column, direction }),
            ..Order::default()
        }
    }

    #[test]
    fn a_flat_header_starts_in_its_own_direction_and_reverses_on_a_second_click() {
        let tok = Header::Overview(OverviewColumn::TokensPerHour);
        let order = tok.clicked(Order::default());
        assert_eq!(order, flat(OverviewColumn::TokensPerHour, Descending));
        assert_eq!(
            tok.clicked(order),
            flat(OverviewColumn::TokensPerHour, Ascending)
        );
        let name = Header::Overview(OverviewColumn::Agent);
        assert_eq!(
            name.clicked(order),
            flat(OverviewColumn::Agent, Ascending),
            "another column starts over"
        );
    }

    #[test]
    fn a_tree_header_picks_its_squad_order_and_reverses_on_a_second_click() {
        let state = Header::Squads(OpsSort::Attention);
        let name = Header::Squads(OpsSort::Name);
        let by_name = name.clicked(Order {
            agents: OpsSort::Attention,
            reversed: true,
            ..Order::default()
        });
        assert_eq!((by_name.agents, by_name.reversed), (OpsSort::Name, false));
        let reversed = name.clicked(by_name);
        assert_eq!((reversed.agents, reversed.reversed), (OpsSort::Name, true));
        assert!(!state.clicked(reversed).reversed);
    }

    #[test]
    fn a_tree_names_run_up_and_attention_runs_down() {
        let order = |agents, reversed| Order {
            agents,
            reversed,
            ..Order::default()
        };
        let name = Header::Squads(OpsSort::Name);
        let state = Header::Squads(OpsSort::Attention);
        assert_eq!(
            name.direction(&order(OpsSort::Name, false)),
            Some(Ascending)
        );
        assert_eq!(
            name.direction(&order(OpsSort::Name, true)),
            Some(Descending)
        );
        assert_eq!(
            state.direction(&order(OpsSort::Attention, false)),
            Some(Descending)
        );
        assert_eq!(
            state.direction(&order(OpsSort::Attention, true)),
            Some(Ascending)
        );
        assert_eq!(state.direction(&order(OpsSort::Members, false)), None);
    }
}
