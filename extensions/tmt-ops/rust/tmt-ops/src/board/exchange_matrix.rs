//! Shared content-free exchange matrix. Hosts supply ranked UUID identities and
//! retain focus, navigation and effects; this component owns only projection/paint.
use super::usage_report::model::Traffic;
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use std::collections::BTreeSet;
use tmt_cli_style::{Role, theme::screen};
use tmt_tui::components::strip;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct ExchangeParticipant {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct CellHit {
    pub area: Rect,
    pub sender: String,
    pub recipient: String,
}

/// Zero and missing observations have no fill; six positive levels share the
/// same bounded style helper as hourly activity. Arithmetic never overflows.
pub(in crate::board) fn level(count: u64, maximum: u64) -> Option<u8> {
    (count > 0).then(|| {
        ((u128::from(count) * 6)
            .div_ceil(u128::from(maximum.max(count)))
            .clamp(1, 6)
            - 1) as u8
    })
}

pub(in crate::board) fn participants(
    ranked: &[ExchangeParticipant],
    selected: Option<&str>,
    maximum: usize,
) -> Vec<ExchangeParticipant> {
    let mut seen = BTreeSet::new();
    let mut result: Vec<_> = ranked
        .iter()
        .filter(|row| seen.insert(&row.id))
        .take(maximum)
        .cloned()
        .collect();
    if maximum > 0
        && let Some(selected) = selected.and_then(|id| ranked.iter().find(|row| row.id == id))
        && !result.iter().any(|row| row.id == selected.id)
    {
        result.pop();
        result.push(selected.clone());
    }
    result
}

/// Paint only complete cells and return their clipped, exact UUID geometry.
/// Missing traffic is `—`, observed zero is `·`, and self is always `─`.
pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    ranked: &[ExchangeParticipant],
    selected: Option<&str>,
    traffic: Option<&Traffic>,
    look: Look,
) -> Vec<CellHit> {
    let maximum = traffic.map_or(0, |traffic| traffic.maximum);
    let minimum_cell = maximum.to_string().len().max(5) as u16 + 1;
    let name_cells = 12u16.min(area.width / 3);
    let slots = usize::from(area.width.saturating_sub(name_cells) / minimum_cell)
        .min(5)
        .min(usize::from(area.height.saturating_sub(1)));
    let value_cells = if slots == 0 {
        minimum_cell
    } else {
        (area.width.saturating_sub(name_cells) / slots as u16).max(minimum_cell)
    };
    let rows = participants(ranked, selected, slots);
    if rows.is_empty() || area.height == 0 {
        return Vec::new();
    }
    let fit = |text: &str, width: u16| {
        super::view::fit(&super::notes::single_line(text), usize::from(width))
    };
    let abbreviated: Vec<_> = rows.iter().map(|row| fit(&row.name, value_cells)).collect();
    let numbered = abbreviated.iter().collect::<BTreeSet<_>>().len() != abbreviated.len();
    let label = |index: usize, name: &str| {
        if numbered {
            format!("{} {name}", index + 1)
        } else {
            name.to_owned()
        }
    };
    let mut header = vec![Span::styled(
        fit("from\\to", name_cells),
        look.role(Role::Muted),
    )];
    for (index, row) in rows.iter().enumerate() {
        header.push(Span::styled(
            fit(&label(index, &row.name), value_cells),
            look.role(Role::Muted),
        ));
    }
    strip::paint_left(buffer, Rect { height: 1, ..area }, Line::from(header));
    let mut hits = Vec::new();
    for (row_index, sender) in rows.iter().enumerate() {
        let y = area.y + row_index as u16 + 1;
        strip::paint_left(
            buffer,
            Rect::new(area.x, y, name_cells, 1),
            Line::styled(
                fit(&label(row_index, &sender.name), name_cells),
                look.role(Role::Text),
            ),
        );
        for (column, recipient) in rows.iter().enumerate() {
            let cell = Rect::new(
                area.x + name_cells + column as u16 * value_cells,
                y,
                value_cells,
                1,
            )
            .intersection(area);
            let count = traffic.map(|traffic| {
                traffic
                    .messages
                    .get(&Some(sender.id.clone()))
                    .and_then(|recipients| recipients.get(&Some(recipient.id.clone())))
                    .copied()
                    .unwrap_or(0)
            });
            let same = sender.id == recipient.id;
            let text = if same {
                "─".into()
            } else {
                match count {
                    None => "—".into(),
                    Some(0) => "·".into(),
                    Some(n) => n.to_string(),
                }
            };
            let style = if same {
                look.role(Role::Dim)
            } else {
                count.and_then(|n| level(n, maximum)).map_or_else(
                    || look.role(Role::Text),
                    |level| screen::neutral_ramp(&look.theme, look.depth, level),
                )
            };
            strip::paint_left(
                buffer,
                cell,
                Line::styled(
                    format!(
                        "{:>width$} ",
                        text,
                        width = usize::from(cell.width.saturating_sub(1))
                    ),
                    style,
                ),
            );
            if !same && cell.width == value_cells {
                hits.push(CellHit {
                    area: cell,
                    sender: sender.id.clone(),
                    recipient: recipient.id.clone(),
                });
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn top_five_keeps_selection_and_deduplicates_uuid_occurrences() {
        let mut rows: Vec<_> = (0..8)
            .map(|i| ExchangeParticipant {
                id: i.to_string(),
                name: format!("seat{i}"),
            })
            .collect();
        rows.insert(1, rows[0].clone());
        let result = participants(&rows, Some("7"), 5);
        assert_eq!(
            result.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["0", "1", "2", "3", "7"]
        );
        assert!(participants(&rows, Some("7"), 0).is_empty());
        for step in 1..=6 {
            assert_eq!(level(step, 6), Some(step as u8 - 1));
        }
        assert_eq!(level(0, 0), None);
        assert_eq!(level(u64::MAX, u64::MAX), Some(5));
    }
}
