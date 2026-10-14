//! Overview column policy. The TUI solver owns cell geometry and truncation.

pub use crate::board::workspace::columns::Column;

pub fn visible(width: u16) -> Vec<Column> {
    crate::board::workspace::columns::visible(width, &Column::ALL)
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
    panels: super::composition::Layout,
    held: Option<(u16, Vec<(Column, ratatui::layout::Rect)>)>,
}
impl Layout {
    pub(super) fn panels(
        &mut self,
        area: ratatui::layout::Rect,
        quota_rows: u16,
    ) -> Vec<(super::composition::Panel, ratatui::layout::Rect)> {
        self.panels.slots(area, quota_rows)
    }
    pub fn slots(&mut self, area: ratatui::layout::Rect) -> Vec<(Column, ratatui::layout::Rect)> {
        if self
            .held
            .as_ref()
            .is_none_or(|(width, _)| *width != area.width)
        {
            let shown = visible(area.width);
            let slots = crate::board::workspace::columns::solve(area.width, &shown);
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
