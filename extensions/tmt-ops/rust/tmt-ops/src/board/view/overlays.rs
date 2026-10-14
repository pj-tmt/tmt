//! Shared overlay dispatch; each current controller owns its surface.

use crate::board::app::App;
use ratatui::{Frame, layout::Rect};

pub(super) fn render(frame: &mut Frame, app: &App, body: Rect, look: crate::look::Look) {
    if let Some(recovery) = &app.recovery
        && recovery.active
    {
        recovery.render(frame, look, body);
    }
    if let Some(discovery) = &app.discovery
        && discovery.active
    {
        discovery.render(frame, look, body);
    }
    if let Some(checklist) = &app.checklist
        && checklist.active
    {
        checklist.render(frame, look, body);
    }
    if app.usage_controls {
        crate::board::overview::chart::host::render_controls(frame, app, body);
    }
    if let Some(details) = &app.usage_details {
        details.borrow_mut().render(frame, app, body);
    }
    if let Some(details) = &app.quota_details {
        details.borrow_mut().render(frame, app, body);
    }
    if app.help {
        crate::board::help::render(frame, app, body);
    }
    if let Some(menu) = &app.menu {
        menu.surface
            .borrow_mut()
            .get_or_insert_with(|| {
                crate::board::menu_surface::MenuSurface::new(&menu.title, &menu.entries)
            })
            .render(&menu.entries, menu.selected, frame, look, body);
    }
    if let Some(list) = &app.cron_list {
        let scope_name = list.scope.as_deref().and_then(|room| {
            app.workspace_snapshot()?
                .roster
                .squads
                .iter()
                .find(|squad| squad.room_id == room)
                .map(|squad| squad.name.as_str())
        });
        list.render(
            &app.cron,
            scope_name,
            &app.row_details.expanded,
            app.cron.now_ms(),
            app.clock_place(),
            frame,
            look,
            body,
        );
    }
    if let Some(picker) = &app.theme_picker {
        crate::board::theme_picker::render(frame, picker, look, body);
    }
    if let Some(overlay) = &app.settings {
        crate::board::settings::render(frame, overlay, look, body);
    }
    crate::board::row_detail::render_reader(frame, app, body);
}
