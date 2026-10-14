//! Independently placed Overview panels, ready for host-owned order/visibility.
use crate::config::OverviewPanels;
use ratatui::layout::Rect;
mod policy;
pub(super) use policy::Budget;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Panel {
    Summary,
    Digest,
    Quota,
    Chart,
    Table,
    Exchanges,
    Legend,
}

#[derive(Default)]
pub(super) struct Layout {
    held: Option<(PanelKey, Vec<(Panel, Rect)>)>,
}
#[derive(PartialEq, Eq)]
struct PanelKey {
    width: u16,
    height: u16,
    quota_rows: u16,
    policy: OverviewPanels,
    chart_demand: u16,
}
impl Layout {
    pub fn slots(
        &mut self,
        area: Rect,
        quota_rows: u16,
        policy: &OverviewPanels,
        chart_demand: u16,
    ) -> Vec<(Panel, Rect)> {
        let key = PanelKey {
            width: area.width,
            height: area.height,
            quota_rows,
            policy: policy.clone(),
            chart_demand,
        };
        if self.held.as_ref().is_none_or(|(old, _)| *old != key) {
            self.held = Some((
                key,
                solve(
                    Rect::new(0, 0, area.width, area.height),
                    &Budget::new(area.height, quota_rows, policy).tracks(policy, chart_demand),
                ),
            ));
        }
        self.held
            .as_ref()
            .unwrap()
            .1
            .iter()
            .map(|(panel, rect)| {
                (
                    *panel,
                    Rect::new(area.x + rect.x, area.y + rect.y, rect.width, rect.height)
                        .intersection(area),
                )
            })
            .collect()
    }
}

/// Paint budgets do not change sources, chart coverage or target admission.
/// Short screens preserve the target table before optional context panels.
#[cfg(test)]
pub(super) fn panels(area: Rect, quota_rows: u16) -> Vec<(Panel, Rect)> {
    if area.is_empty() {
        return vec![];
    }
    let summary = u16::from(area.height >= 9);
    let digest = u16::from(area.height >= 10);
    let legend = if area.height >= 18 { 2 } else { 0 };
    let quota = quota_rows.min(area.height.saturating_sub(7 + summary + digest + legend));
    let context = area
        .height
        .saturating_sub(6 + summary + digest + legend + quota);
    let chart = if context >= 16 {
        11
    } else {
        u16::from(context >= 1)
    };
    let exchanges = if context.saturating_sub(chart) >= 5 {
        5
    } else {
        0
    };
    let fixed = summary + digest + quota + chart + exchanges + legend;
    let table = area.height.saturating_sub(fixed);
    let tracks = [
        (Panel::Summary, summary),
        (Panel::Digest, digest),
        (Panel::Quota, quota),
        (Panel::Chart, chart),
        (Panel::Table, table),
        (Panel::Exchanges, exchanges),
        (Panel::Legend, legend),
    ];
    solve(area, &tracks)
}

fn solve(area: Rect, tracks: &[(Panel, u16)]) -> Vec<(Panel, Rect)> {
    if area.is_empty() {
        return Vec::new();
    }
    let mut markup = String::from("<tmt-view version=\"1\"><tmt-row class=\"flex-col\">");
    for (index, (_, height)) in tracks
        .iter()
        .enumerate()
        .filter(|(_, (_, height))| *height > 0)
    {
        markup.push_str(&format!(
            "<tmt-text id=\"p{index}\" class=\"h-{height} shrink-0\"> </tmt-text>"
        ));
    }
    markup.push_str("</tmt-row></tmt-view>");
    let file = "ops.overview.panels.xml";
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
        .expect("admitted Overview panels");
    let solved =
        tmt_tui::geometry::layout(&root, [area.width, area.height], tmt_tui::text::measure)
            .expect("Overview panel geometry");
    tracks
        .iter()
        .enumerate()
        .filter_map(|(index, (panel, height))| {
            if *height == 0 {
                return None;
            }
            let id = format!("p{index}");
            let cell = solved.iter().find(|cell| {
                cell.node
                    .id
                    .as_ref()
                    .is_some_and(|path| path.last() == Some(&id))
            })?;
            Some((
                *panel,
                Rect::new(
                    area.x,
                    area.y.saturating_add(cell.rect.y.max(0) as u16),
                    area.width,
                    *height,
                )
                .intersection(area),
            ))
        })
        .collect()
}

mod render;
pub(in crate::board) use render::render;

#[cfg(test)]
mod tests;
