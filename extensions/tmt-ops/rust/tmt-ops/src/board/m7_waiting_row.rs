//! M7 WaitingRow: shared name/squad/state/summary tracks, without source effects.
use crate::{
    board::{m6_state_cell, view::fit},
    look::Look,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;

pub(in crate::board) struct Data<'a> {
    pub name: &'a str,
    pub squad: &'a str,
    pub summary: &'a str,
}
pub(in crate::board) fn line(data: &Data<'_>, width: usize, look: Look) -> Line<'static> {
    let name_width = if width >= 60 {
        23
    } else {
        width.saturating_sub(24).min(23)
    };
    let squad_width = if width >= 53 { 11 } else { 0 };
    let mut spans = vec![
        Span::raw(" "),
        m6_state_cell::mark(Some("waiting"), look),
        Span::raw(" "),
        Span::styled(
            fit(&crate::board::notes::sanitize(data.name), name_width),
            look.role(Role::Text),
        ),
        Span::raw(" "),
    ];
    if squad_width > 0 {
        spans.push(Span::styled(
            fit(&crate::board::notes::sanitize(data.squad), squad_width),
            look.role(Role::Muted),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        fit(&m6_state_cell::word(Some("waiting"), look).content, 13),
        look.role(Role::Muted),
    ));
    spans.push(Span::raw(" "));
    let used = Line::from(spans.clone()).width();
    spans.push(Span::styled(
        fit(
            &crate::board::notes::sanitize(data.summary),
            width.saturating_sub(used),
        ),
        look.role(Role::Muted),
    ));
    Line::from(spans)
}
pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    data: &Data<'_>,
    selected: bool,
    look: Look,
) -> [Rect; 2] {
    let mut row = line(data, usize::from(area.width), look);
    if selected {
        row.style = look.selection();
        for (index, span) in row.spans.iter_mut().enumerate() {
            span.style = look.row_span(true, span.style, index == 1);
        }
    }
    tmt_tui::components::strip::paint_left(
        buffer,
        Rect {
            height: area.height.min(1),
            ..area
        },
        row,
    );
    let width = usize::from(area.width);
    let name_width = if width >= 60 {
        23
    } else {
        width.saturating_sub(24).min(23)
    };
    let squad_width = if width >= 53 { 12 } else { 0 };
    [
        Rect::new(area.x.saturating_add(1), area.y, 1, 1).intersection(area),
        Rect::new(
            area.x.saturating_add((4 + name_width + squad_width) as u16),
            area.y,
            13,
            1,
        )
        .intersection(area),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_tracks_match_narrow_and_wide_lists_without_waiting_since_labels() {
        use unicode_width::UnicodeWidthStr;
        let look = Look::new(tmt_cli_style::Theme::default());
        let data = Data {
            name: "tmt-infra-lead",
            squad: "tmt-infra",
            summary: "PR #3176 queued",
        };
        for width in [85, 103, 148] {
            let row = line(&data, width, look).to_string();
            let at = |text: &str| row[..row.find(text).unwrap()].width();
            assert_eq!(at("tmt-infra-lead"), 3);
            let squad_at = row[20..].find("tmt-infra").unwrap() + 20;
            assert_eq!(row[..squad_at].width(), 27);
            assert_eq!(at("waiting"), 39);
            assert_eq!(at("PR #"), 53);
            assert!(!row.contains("ago"));
        }
    }
}
