//! Shared overlay dispatch; each current controller owns its surface.

use crate::board::app::App;
use ratatui::{Frame, layout::Rect};

pub(super) fn render(frame: &mut Frame, app: &App, body: Rect, look: crate::look::Look) {
    app.peek_areas.clear();
    if let Some(switch) = &app.switch_tool {
        switch.render(frame, app, look, body);
    }
    if let Some(member) = &app.new_member {
        member.render(frame, app, look, body);
    }
    if let Some(session) = &app.member_session {
        session.render(frame, app, look, body);
    }
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
    if let Some(details) = &app.quota_details {
        details.borrow_mut().render(frame, app, body);
    }
    if app.help {
        crate::board::help::render(frame, app, body);
    }
    if let Some(menu) = &app.menu {
        let anchor = menu.anchor;
        let body = if let Some(anchor) = anchor {
            let Some(area) =
                crate::board::member_links::menu_area(body, anchor, menu.entries.len() as u16 + 4)
            else {
                return;
            };
            area
        } else {
            body
        };
        let mut surface = menu.surface.borrow_mut();
        let surface = surface.get_or_insert_with(|| {
            crate::board::menu_surface::MenuSurface::with_anchor(
                &menu.title,
                &menu.entries,
                anchor.is_some(),
            )
        });
        surface.render(&menu.entries, menu.selected, frame, look, body);
        // An anchored dropdown fills its area; a docked menu reports where its sheet was drawn.
        let outer = if anchor.is_some() {
            Some(body)
        } else {
            surface.outer()
        };
        if let Some(outer) = outer {
            app.peek_areas.set(crate::board::peek::Peek::Menu, outer);
        }
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
        if let Some(outer) = list.modal_outer() {
            app.peek_areas.set(crate::board::peek::Peek::Timers, outer);
        }
    }
    if let Some(picker) = &app.theme_picker {
        crate::board::theme_picker::render(frame, picker, look, body);
    }
    if let Some(picker) = &app.panels_picker {
        crate::board::panels_picker::render(frame, picker, look, body);
    }
    if let Some(overlay) = &app.settings {
        crate::board::settings::render(frame, overlay, look, body);
    }
    crate::board::row_detail::render_reader(frame, app, body);
    if let Some(details) = &app.monitor_details {
        details.render(frame, app, body);
    }
    app.render_paste_preview(frame, body);
    if let Some(review) = &app.cron_review {
        review.render(frame, app, body);
    }
}
