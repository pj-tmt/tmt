//! Workspace body dispatch. Named factory panes no longer own a UI route.

use crate::board::app::App;
use ratatui::{Frame, layout::Rect};

pub(super) fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    if app.workspace_snapshot().is_some() && app.shell.tab == crate::board::shell::OpsTab::Digest {
        crate::board::digest_dashboard::render(frame, app, area);
    } else {
        crate::board::workspace::render::render(frame, app, area);
    }
}
