//! Workspace body dispatch. Named factory panes no longer own a UI route.

use crate::board::app::App;
use ratatui::{Frame, layout::Rect};

pub(super) fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    if app.workspace_snapshot().is_some() && app.shell.tab == crate::board::shell::OpsTab::Digest {
        crate::board::digest_dashboard::render(frame, app, area);
    } else {
        let area = if app.shell.tab == crate::board::shell::OpsTab::Agents
            && app.composer_projection().is_some()
            && app.workspace_snapshot().is_some_and(|snapshot| {
                use crate::{
                    board::conversation::layout::{self, Preference},
                    config::{OpsLayout, Pane},
                };
                layout::agents(
                    area,
                    match snapshot.preferences.layout {
                        OpsLayout::Auto => Preference::Auto,
                        OpsLayout::Side => Preference::Side,
                        OpsLayout::Stack => Preference::Stack,
                    },
                    app.focused() == Pane::Replies,
                )
                .conversation
                .is_none()
            }) {
            let height =
                super::waiting::input_lines(app, area).min(usize::from(area.height)) as u16;
            app.input_band.set(Some(Rect::new(
                area.x,
                area.bottom() - height,
                area.width,
                height,
            )));
            let list_height = area.height - height;
            // The composer paints the shared boundary. Let a decorated tree's
            // bottom border occupy that same row, rather than stacking rules.
            let shared_boundary = u16::from(list_height >= 3 && area.width >= 3);
            Rect {
                height: list_height + shared_boundary,
                ..area
            }
        } else {
            area
        };
        app.conversation.borrow_mut().area = area;
        crate::board::workspace::render::render(frame, app, area);
    }
}
