//! Overlay dispatch and existing action-menu/switcher paint.

use super::fit;
use crate::board::app::{App, Switcher};
use crate::config::TabColors;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
};
use tmt_cli_style::mark::Mark;

pub(super) fn render(frame: &mut Frame, app: &App, body: Rect, look: crate::look::Look) {
    if app.help {
        crate::board::help::render(frame, app, body);
    }
    if let Some(menu) = &app.menu {
        let height = (menu.entries.len() as u16 + 2).min(body.height);
        let width = body.width.min(48);
        let area = Rect {
            x: body.x + (body.width - width) / 2,
            y: body.y + (body.height - height) / 2,
            width,
            height,
        };
        let lines: Vec<Line> = menu
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let line = Line::from(fit(
                    &format!(" {:<9} {}", entry.key, entry.label),
                    usize::from(width.saturating_sub(2)),
                ));
                if index == menu.selected {
                    line.style(Style::new().add_modifier(Modifier::REVERSED))
                } else {
                    line
                }
            })
            .collect();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(format!(" {} · Enter runs, Esc closes ", menu.title)),
            ),
            area,
        );
    }
    if let Some(switcher) = &app.switcher {
        render_switcher(frame, app, switcher, body);
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
}

/// The quick switcher: the query, then the matching tabs with their counts
/// and state colors; hidden ones are marked.
pub(super) fn render_switcher(frame: &mut Frame, app: &App, switcher: &Switcher, body: Rect) {
    use std::sync::OnceLock;
    use tmt_tui::components::surface;
    const FILE: &str = "squad.switcher.xml";
    const MARKUP: &str = r#"<tmt-view version="1"><tmt-picker id="switcher" title="switch" placement="center" class="w-48"><tmt-text id="query" slot="query" bind="$.query" token="text"/><tmt-list id="choices" bind="$.rows" empty="(no matching tab)"><tmt-row class="flex-row gap-1"><tmt-cell id="mark" bind="row.mark" class="w-1 shrink-0"/><tmt-cell bind="row.label" class="truncate-middle"/><tmt-cell bind="row.count" class="shrink-0"/><tmt-cell id="blocked" bind="row.blocked" class="shrink-0"/></tmt-row></tmt-list><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-picker></tmt-view>"#;
    static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
    let template = TEMPLATE.get_or_init(|| {
        crate::board::picker_surface::compile(
            FILE,
            MARKUP,
            crate::board::picker_surface::schema(&["mark", "label", "count", "blocked"]),
        )
    });
    let look = app.look();
    let keys = app.switchable();
    let query = switcher.query();
    let found = crate::board::tabs::matching(&keys, &query);
    let rows: Vec<_> = found.into_iter().map(|key| {
        let attention = app.attention.get(key).copied().unwrap_or_default();
        let (mark, count) = if attention.waiting > 0 { (Mark::Decision.symbol(), attention.waiting) }
            else if attention.blocked > 0 { (Mark::Failed.symbol(), attention.blocked) } else { (" ",0) };
        let label = if app.hidden.contains(key) { format!("{} (hidden)", crate::board::tabs::label(key)) } else { crate::board::tabs::label(key).into() };
        serde_json::json!({"id":key,"disabled":false,"mark":mark,"label":label,"count":if count > 0 { count.to_string() } else { String::new() },
            "blocked":if attention.waiting > 0 && attention.blocked > 0 { format!("{}{}", Mark::Failed.symbol(), attention.blocked) } else { String::new() }})
    }).collect();
    let mut surface = switcher.surface.borrow_mut();
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
    let value = serde_json::json!({"rows":rows,"query":query,"footer":"↑↓ choose · Enter opens · Esc closes","status":"","notes":[]});
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
