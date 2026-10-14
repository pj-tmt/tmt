//! Overview column policy. The TUI solver owns cell geometry and truncation.

pub use crate::config::OverviewColumn as Column;

impl Column {
    pub fn title(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::Squad => "Squad",
            Self::State => "State",
            Self::TokensPerHour => "TOK/h",
            Self::Digest => "Digest",
            Self::Cron => "Cron",
            Self::Runtime => "Runtime",
            Self::Model => "Model",
        }
    }

    /// Minimum useful cell allocation, not a second layout solver. Agent grows
    /// into the space left by these tracks; the template clips every row to one
    /// line and uses the shared grapheme measurer for ellipses.
    pub fn minimum(self) -> u16 {
        match self {
            Self::Agent => 12,
            Self::Squad | Self::State => 10,
            Self::TokensPerHour => 7,
            Self::Cron => 9,
            Self::Digest => 16,
            Self::Runtime => 8,
            Self::Model => 14,
        }
    }
}

/// Choose whole columns before materialization. Hidden values remain in the
/// acquired row and its detail; no value is shortened or reformatted here.
pub fn visible(width: u16) -> Vec<Column> {
    let mut columns = Column::ALL.to_vec();
    for removable in [
        Column::Model,
        Column::Runtime,
        Column::Squad,
        Column::Digest,
        Column::Cron,
    ] {
        let needed: u16 = columns.iter().map(|column| column.minimum()).sum::<u16>()
            + columns.len().saturating_sub(1) as u16;
        if needed <= width {
            break;
        }
        columns.retain(|column| *column != removable);
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_columns_drop_whole_and_keep_the_monitor_identity() {
        for width in [60, 80, 100, 140, 160] {
            let shown = visible(width);
            assert!(shown.contains(&Column::Agent));
            assert!(shown.contains(&Column::State));
            assert!(shown.contains(&Column::TokensPerHour));
            assert!(
                shown.iter().map(|column| column.minimum()).sum::<u16>() + shown.len() as u16 - 1
                    <= width
            );
            assert!(
                Column::ALL
                    .iter()
                    .filter(|column| shown.contains(column))
                    .copied()
                    .eq(shown.iter().copied())
            );
        }
        assert_eq!(visible(160), Column::ALL);
        assert!(!visible(60).contains(&Column::Model));
        assert_eq!(
            visible(0),
            vec![Column::Agent, Column::State, Column::TokensPerHour]
        );
    }
}

/// One cached markup shape; shared geometry owns every painted column and hit.
#[derive(Default)]
pub(in crate::board) struct Layout {
    held: Option<(u16, Vec<(Column, ratatui::layout::Rect)>)>,
}
impl Layout {
    pub fn slots(&mut self, area: ratatui::layout::Rect) -> Vec<(Column, ratatui::layout::Rect)> {
        if self
            .held
            .as_ref()
            .is_none_or(|(width, _)| *width != area.width)
        {
            let shown = visible(area.width);
            let fixed: u16 = shown
                .iter()
                .filter(|c| **c != Column::Agent)
                .map(|c| c.minimum())
                .sum();
            let agent = area
                .width
                .saturating_sub(fixed + shown.len().saturating_sub(1) as u16);
            let mut markup =
                String::from("<tmt-view version=\"1\"><tmt-row class=\"flex-row gap-1 h-1\">");
            for (index, column) in shown.iter().enumerate() {
                let width = if *column == Column::Agent {
                    agent
                } else {
                    column.minimum()
                };
                markup.push_str(&format!(
                    "<tmt-text id=\"c{index}\" class=\"w-{width} shrink-0 truncate\">{}</tmt-text>",
                    column.title()
                ));
            }
            markup.push_str("</tmt-row></tmt-view>");
            let file = "ops.overview.columns.xml";
            let template = crate::board::view::scene::compile(
                file,
                &markup,
                &tmt_tui::binding::Schema::Object(Default::default()),
            );
            let root = template
                .materialize(
                    file,
                    &serde_json::json!({}),
                    &crate::board::picker_surface::Data,
                )
                .expect("column template");
            let cells = tmt_tui::geometry::layout(&root, [area.width, 1], tmt_tui::text::measure)
                .expect("column geometry");
            let slots = shown
                .into_iter()
                .enumerate()
                .filter_map(|(i, column)| {
                    let id = format!("c{i}");
                    let cell = cells.iter().find(|cell| {
                        cell.node
                            .id
                            .as_ref()
                            .is_some_and(|path| path.last() == Some(&id))
                    })?;
                    Some((
                        column,
                        ratatui::layout::Rect::new(
                            cell.rect.x.max(0) as u16,
                            0,
                            cell.rect.width.min(u32::from(u16::MAX)) as u16,
                            1,
                        ),
                    ))
                })
                .collect();
            self.held = Some((area.width, slots));
        }
        self.held
            .as_ref()
            .unwrap()
            .1
            .iter()
            .map(|(c, r)| {
                (
                    *c,
                    ratatui::layout::Rect::new(
                        area.x.saturating_add(r.x),
                        area.y,
                        r.width,
                        area.height.min(1),
                    )
                    .intersection(area),
                )
            })
            .collect()
    }
}
