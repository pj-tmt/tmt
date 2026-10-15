//! Pure panel budgets. Hidden context gives rows back to the target table;
//! the chart's demand is resolved before rectangles are assigned or painted.
use super::Panel;
use crate::config::{OverviewPanel as Id, OverviewPanels};

pub(in crate::board::overview) struct Budget {
    heights: [u16; 7],
    pub chart_and_target: u16,
}
impl Budget {
    pub fn new(height: u16, quota_rows: u16, policy: &OverviewPanels) -> Self {
        let target = policy.shown(Id::Monitor);
        let minimum = if target { 6 } else { 0 };
        let summary = u16::from(policy.shown(Id::Summary) && height >= 9);
        let digest = u16::from(policy.shown(Id::Digest) && height >= 10);
        let legend = if target && height >= 18 { 2 } else { 0 };
        let quota = if policy.shown(Id::Subscriptions) {
            quota_rows.min(height.saturating_sub(minimum + 1 + summary + digest + legend))
        } else {
            0
        };
        let context = height.saturating_sub(minimum + summary + digest + quota + legend);
        let chart = if !policy.shown(Id::Usage) {
            0
        } else if context >= 16 {
            11
        } else {
            u16::from(context >= 1)
        };
        let exchanges = if policy.shown(Id::Exchanges) && context.saturating_sub(chart) >= 5 {
            5
        } else {
            0
        };
        let table = if target {
            height.saturating_sub(summary + digest + quota + chart + exchanges + legend)
        } else {
            0
        };
        Self {
            heights: [summary, digest, quota, chart, table, exchanges, legend],
            chart_and_target: chart + table,
        }
    }
    /// The unused nominal chart allocation belongs to the shown monitor, even
    /// when the two panels are separated or the chart moves below the monitor.
    pub fn tracks(&self, policy: &OverviewPanels, chart_demand: u16) -> Vec<(Panel, u16)> {
        let mut heights = self.heights;
        let used = if heights[3] == 0 {
            0
        } else {
            chart_demand.min(
                self.chart_and_target
                    .saturating_sub(u16::from(policy.shown(Id::Monitor) && heights[4] > 0)),
            )
        };
        if policy.shown(Id::Monitor) {
            heights[4] = self.chart_and_target - used;
        }
        heights[3] = used;
        let mut tracks: Vec<_> = policy
            .order()
            .iter()
            .map(|id| {
                let (panel, index) = match id {
                    Id::Summary => (Panel::Summary, 0),
                    Id::Digest => (Panel::Digest, 1),
                    Id::Subscriptions => (Panel::Quota, 2),
                    Id::Usage => (Panel::Chart, 3),
                    Id::Monitor => (Panel::Table, 4),
                    Id::Exchanges => (Panel::Exchanges, 5),
                };
                (panel, heights[index])
            })
            .filter(|(_, rows)| *rows > 0)
            .collect();
        if heights[6] > 0 {
            tracks.push((Panel::Legend, heights[6]));
        }
        tracks
    }
}

#[cfg(test)]
mod tests;
