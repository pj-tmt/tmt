//! Overlay dispatch and existing action-menu/switcher paint.

use crate::board::app::{App, Switcher};
use crate::config::TabColors;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
};
use tmt_cli_style::mark::Mark;

pub(super) fn render(frame: &mut Frame, app: &App, body: Rect, look: crate::look::Look) {
    if let Some(checklist) = &app.checklist
        && checklist.active
    {
        checklist.render(frame, look, body);
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
    if let Some(switcher) = &app.switcher {
        render_switcher(frame, app, switcher, body);
    }
    if let Some(list) = &app.cron_list {
        list.render(
            &app.cron,
            &app.row_details.expanded,
            app.cron.now_ms(),
            app.clock_place(),
            frame,
            look,
            body,
        );
    }
    if let Some(picker) = &app.view_picker {
        crate::board::view_picker::render(frame, picker, look, body);
    }
    if let Some(picker) = &app.theme_picker {
        crate::board::theme_picker::render(frame, picker, look, body);
    }
    if let Some(overlay) = &app.settings {
        crate::board::settings::render(frame, overlay, look, body);
    }
    crate::board::row_detail::render_reader(frame, app, body);
}

/// The quick switcher: the query, then the matching tabs with their counts
/// and state colors; hidden ones are marked.
pub(super) fn render_switcher(frame: &mut Frame, app: &App, switcher: &Switcher, body: Rect) {
    use std::sync::OnceLock;
    use tmt_tui::components::surface;
    const FILE: &str = "squad.switcher.xml";
    const MARKUP: &str = r#"<tmt-view version="1"><tmt-picker id="switcher" title="switch" placement="center" class="w-48"><tmt-text id="query" slot="query" bind="$.query" token="text"/><tmt-list id="choices" bind="$.rows" empty="(no matching tab)"><tmt-row class="flex-row gap-1"><tmt-row class="flex-row w-5 shrink-0 gap-0"><tmt-cell bind="row.pick" class="w-3 shrink-0" token="muted"/><tmt-cell bind="row.cursor" class="w-1 shrink-0" token="text"/><tmt-cell id="mark" bind="row.mark" class="w-1 shrink-0"/></tmt-row><tmt-cell bind="row.label" class="truncate-middle"/><tmt-cell bind="row.count" class="shrink-0"/><tmt-cell id="blocked" bind="row.blocked" class="shrink-0"/></tmt-row></tmt-list><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-picker></tmt-view>"#;
    static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
    let template = TEMPLATE.get_or_init(|| {
        crate::board::picker_surface::compile(
            FILE,
            MARKUP,
            crate::board::picker_surface::schema(&[
                "pick", "cursor", "mark", "label", "count", "blocked",
            ]),
        )
    });
    let look = app.look();
    let keys = app.switcher_keys();
    let query = switcher.query();
    let found = crate::board::tabs::matching(&keys, &query);
    let mut surface = switcher.surface.borrow_mut();
    surface.reconcile(
        found
            .iter()
            .map(|key| tmt_tui::components::ListRow {
                id: (*key).clone(),
                disabled: false,
            })
            .collect(),
    );
    let selected = surface.picker.list.selected().map(str::to_owned);
    let rows: Vec<_> = found.into_iter().map(|key| {
        let attention = app.attention.get(key).copied().unwrap_or_default();
        let (mark, count) = if attention.waiting > 0 { (Mark::Decision.symbol(), attention.waiting) }
            else if attention.blocked > 0 { (Mark::Failed.symbol(), attention.blocked) } else { (" ",0) };
        let label = if app.hidden.contains(key) { format!("{} (hidden)", crate::board::tabs::label(key)) } else { crate::board::tabs::label(key).into() };
        serde_json::json!({"id":key,"disabled":false,"pick":if app.picks.contains(key) { "[x]" } else { "[ ]" },"cursor":if selected.as_deref() == Some(key.as_str()) {"›"} else {""},"mark":mark,"label":label,"count":if count > 0 { count.to_string() } else { String::new() },
            "blocked":if attention.waiting > 0 && attention.blocked > 0 { format!("{}{}", Mark::Failed.symbol(), attention.blocked) } else { String::new() }})
    }).collect();
    let query_width = tmt_tui::components::Modal {
        title: "switch".into(),
        placement: tmt_tui::components::Placement::Center,
    }
    .areas(body, [48, body.height], true, false)
    .content
    .width;
    let query = format!(
        "› {}",
        surface
            .picker
            .query_visible(query_width.saturating_sub(2))
            .unwrap_or_default()
    );
    let pick_keys = app
        .pick_keys()
        .iter()
        .map(|key| {
            if key == "space" {
                "Space".to_owned()
            } else {
                key.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    let footer = if pick_keys.is_empty() {
        "↑↓ choose · Enter opens · Esc closes".into()
    } else {
        format!("{pick_keys} pick/unpick · Enter opens · Esc closes")
    };
    let value =
        serde_json::json!({"rows":rows,"query":query,"footer":footer,"status":"","notes":[]});
    surface.render(FILE, template, value, frame, look, body);
    let default = TabColors::default();
    let colors = app.view.as_ref().map_or(&default, |view| &view.tab_colors);
    if let Some(map) = &surface.frame {
        for hit in &map.hits {
            let Some(key) = hit.row_id.as_deref() else {
                continue;
            };
            let attention = app.attention.get(key).copied().unwrap_or_default();
            let color = match hit.id.last().map(String::as_str) {
                Some("mark") if attention.waiting > 0 => &colors.waiting,
                Some("mark") if attention.blocked > 0 => &colors.blocked,
                Some("blocked") if attention.waiting > 0 && attention.blocked > 0 => {
                    &colors.blocked
                }
                _ => continue,
            };
            // The component owns clipped span geometry and selection. Squad's
            // existing tab-color policy decorates only those semantic mark spans.
            let selected = surface.picker.list.selected() == Some(key);
            let mark = look.named(color).add_modifier(Modifier::BOLD);
            let style = if selected {
                look.selection().patch(look.row_span(true, mark, true))
            } else {
                Style {
                    fg: Some(mark.fg.unwrap_or_default()),
                    ..mark
                }
            };
            frame.buffer_mut().set_style(hit.rect, style);
        }
    }
}
