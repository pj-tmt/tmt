//! Workspace body dispatch. Named factory panes no longer own a UI route.

use crate::board::app::App;
use ratatui::{Frame, layout::Rect};

pub(super) fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    if app.workspace_snapshot().is_some() && app.shell.tab == crate::board::shell::OpsTab::Digest {
        crate::board::digest_dashboard::render(frame, app, area);
    } else {
        let area = if app.shell.tab == crate::board::shell::OpsTab::Agents
            && app.composer_projection().is_some()
            && app
                .workspace_placement(area)
                .is_some_and(|placement| placement.conversation.is_none())
        {
            let height =
                super::waiting::input_lines(app, area).min(usize::from(area.height)) as u16;
            app.input_band.set(Some(Rect::new(
                area.x + area.width.min(crate::board::input_field::GUTTER),
                area.bottom() - height,
                area.width
                    .saturating_sub(crate::board::input_field::GUTTER * 2),
                height,
            )));
            let list_height = area.height - height;
            Rect {
                height: list_height,
                ..area
            }
        } else {
            area
        };
        app.conversation.borrow_mut().area = area;
        crate::board::workspace::render::render(frame, app, area);
    }
}
