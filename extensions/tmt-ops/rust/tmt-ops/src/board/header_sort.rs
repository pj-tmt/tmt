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

use super::{overview::cron_counts::GlyphSet, view_state::Order, workspace::columns::TokHeading};
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

/// One clickable part of an Agents header, each with its own hit area and hover:
/// a sort mark or word, and the two parts of the TOK caption (`TOK 1h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Sort(Header),
    /// The TOK/`≈$` word: switches the whole column between tokens and money.
    Unit,
    /// The range word (`1h`): the next shared TOK range.
    Window,
}

/// What the headers show: the shown order and the control under the pointer.
#[derive(Clone, Copy)]
pub(in crate::board) struct Marks {
    order: Order,
    hover: Option<Control>,
    glyphs: GlyphSet,
}

impl Marks {
    pub(in crate::board) fn new(order: Order, hover: Option<Control>, glyphs: GlyphSet) -> Self {
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
            None if self.hover == Some(Control::Sort(header)) => {
                Some(if ascii { "~" } else { "↕" })
            }
            None => None,
        }
    }

    pub(in crate::board) fn style(
        &self,
        header: Header,
        base: Style,
        look: crate::look::Look,
    ) -> Style {
        self.lit(Control::Sort(header), base, look)
    }

    fn lit(&self, control: Control, base: Style, look: crate::look::Look) -> Style {
        if self.hover == Some(control) {
            base.patch(look.role(Role::Accent))
        } else {
            base
        }
    }

    /// The right-aligned TOK caption of a `width`-cell column as its three controls:
    /// the sort mark takes the cell before the unit word, so no column moves, and a
    /// hovered unit word shows the unit a click switches to. `None` when the caption
    /// leaves no cell for the mark, which then has no controls. The hit areas are
    /// offsets into the cell and follow the unit on screen, never the hover.
    pub(in crate::board) fn tokens(
        &self,
        sort: Option<Header>,
        heading: TokHeading,
        width: u16,
        base: Style,
        look: crate::look::Look,
    ) -> Option<TokCell> {
        let (unit, window) = (heading.unit(), heading.range.argument());
        let caption = (unit.width() + 1 + window.width()) as u16;
        let lead = width.checked_sub(caption).filter(|lead| *lead > 0)?;
        let mark = sort.and_then(|header| self.mark(header)).unwrap_or(" ");
        let mark_style = sort.map_or(base, |header| self.style(header, base, look));
        let unit_style = self.lit(Control::Unit, base, look);
        let mut spans = vec![Span::styled(" ".repeat(usize::from(lead) - 1), base)];
        if self.hover == Some(Control::Unit) {
            // The other unit may be a cell wider: it then takes the mark's cell.
            let other = heading.other_unit();
            let pad = unit.width().saturating_sub(other.width());
            if other.width() <= unit.width() {
                spans.push(Span::styled(mark, mark_style));
            }
            spans.push(Span::styled(
                format!("{}{other}", " ".repeat(pad)),
                unit_style,
            ));
        } else {
            spans.push(Span::styled(mark.to_owned(), mark_style));
            spans.push(Span::styled(unit.to_owned(), unit_style));
        }
        spans.push(Span::styled(" ", base));
        spans.push(Span::styled(
            window.to_owned(),
            self.lit(Control::Window, base, look),
        ));
        let (unit_width, window_width) = (unit.width() as u16, window.width() as u16);
        let mut parts = vec![
            (lead, unit_width, Control::Unit),
            (lead + unit_width + 1, window_width, Control::Window),
        ];
        if let Some(header) = sort {
            parts.push((lead - 1, 1, Control::Sort(header)));
        }
        Some(TokCell { spans, parts })
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

/// The TOK caption painted as spans, with the offset, width and control of each part.
pub(in crate::board) struct TokCell {
    pub spans: Vec<Span<'static>>,
    pub parts: Vec<(u16, u16, Control)>,
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
    hits: Vec<(Rect, Control)>,
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
        self.hit(Control::Sort(header), self.used, hit_width(word));
        self.used += span.content.width() as u16;
        self.spans.push(span);
        self.plain(&" ".repeat(pad));
    }

    /// The TOK caption of a right-aligned `width`-cell column; see `Marks::tokens`.
    /// A caption with no room for its mark is plain text.
    pub(in crate::board) fn tokens(&mut self, header: Header, heading: TokHeading, width: u16) {
        let Some(cell) = self
            .marks
            .tokens(Some(header), heading, width, self.base, self.look)
        else {
            self.plain(heading.caption());
            return;
        };
        for (from, size, control) in cell.parts {
            self.hit(control, self.used + from, size);
        }
        self.used += width;
        self.spans.extend(cell.spans);
    }

    fn hit(&mut self, control: Control, from: u16, width: u16) {
        let area = Rect::new(self.slot.x.saturating_add(from), self.slot.y, width, 1)
            .intersection(self.slot);
        if area.width > 0 {
            self.hits.push((area, control));
        }
    }

    pub(in crate::board) fn finish(self) -> (Line<'static>, Vec<(Rect, Control)>) {
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
