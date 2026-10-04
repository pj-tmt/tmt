//! Lead notebook lines, selection, links and clipped hits.

use crate::board::{
    app::{App, Notes},
    markdown,
    notes::wrap,
};
use crate::config::{NotesRender, Pane};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::Role;

/// Both notebook panes share inert text, Markdown and placeholder styles.
pub(in crate::board) fn notebook_lines(
    notes: &Notes,
    width: usize,
    look: crate::look::Look,
    render: NotesRender,
) -> Vec<Line<'static>> {
    let text = match notes {
        Notes::Text(text) => text.as_str(),
        Notes::Missing => "(no notes yet)",
        Notes::NoLead => "(the squad has no lead)",
        Notes::NotShown => "",
        Notes::Failed(error) => error.as_str(),
    };
    match (notes, render) {
        (Notes::Text(text), NotesRender::Markdown) => markdown::render(text, width, look),
        (Notes::Text(text), NotesRender::Plain) => {
            wrap(text, width).into_iter().map(Line::from).collect()
        }
        _ => wrap(text, width)
            .into_iter()
            .map(|line| Line::styled(line, look.role(Role::Dim)))
            .collect(),
    }
}

pub(super) fn render_notes(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(view) = &app.view else { return };
    let width = usize::from(area.width).saturating_sub(2).max(1);
    let mut derived = view.derived.borrow_mut();
    let rebuilt = derived
        .notes
        .as_ref()
        .is_none_or(|notes| notes.width != width || notes.look != look);
    if rebuilt {
        let (lines, sources, links, hits) = match &view.notes {
            Notes::Text(text) if view.render == NotesRender::Markdown => {
                let mapped = markdown::render_links(text, width, look, &view.links);
                (mapped.lines, mapped.sources, mapped.links, mapped.hits)
            }
            Notes::Text(text) => {
                let mut lines = Vec::new();
                let mut sources = Vec::new();
                for (source, line) in text.split('\n').enumerate() {
                    for wrapped in wrap(line, width) {
                        lines.push(Line::from(wrapped));
                        sources.push(source);
                    }
                }
                (lines, sources, Vec::new(), Vec::new())
            }
            _ => (
                notebook_lines(&view.notes, width, look, view.render),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        };
        derived.notes = Some(crate::board::derived::NotebookLines {
            width,
            look,
            lines,
            sources,
            links,
            hits,
        });
    }
    let notes = derived.notes.as_ref().expect("prepared notes");
    let (lines, sources) = (&notes.lines, &notes.sources);
    let selected_link = notes
        .links
        .iter()
        .position(|link| app.note_link.as_ref() == Some(&(link.target.clone(), link.offset)));
    let mut selected = None;
    let mut marked = std::collections::BTreeSet::new();
    if let (Some(key), Notes::Text(text)) = (app.shown_tab(), &view.notes) {
        let mut cursors = app.note_cursors.borrow_mut();
        let cursor = cursors.entry(key.to_owned()).or_default();
        if rebuilt {
            cursor.reconcile(text);
        }
        if app.focused_pane() == Some(Pane::Notes) {
            selected = selected_link
                .map(|id| notes.links[id].source)
                .or(Some(cursor.source));
            let visual = selected_link
                .and_then(|id| {
                    notes
                        .hits
                        .iter()
                        .find(|hit| hit.link == id)
                        .map(|hit| hit.line)
                })
                .or_else(|| cursor.visual(sources));
            if cursor.follow
                && let Some(line) = visual
            {
                app.scrolls
                    .reveal_range(Pane::Notes, line..line + 1, area, lines.len());
            }
        }
        for item in view.document["squad"]["noteAnnotations"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(quote) = item["quote"].as_str() else {
                continue;
            };
            let old = item["line"].as_u64().unwrap_or_default() as usize;
            let source = text
                .split('\n')
                .enumerate()
                .filter(|(_, line)| line.starts_with(quote))
                .min_by_key(|(at, _)| (at.abs_diff(old), std::cmp::Reverse(*at)))
                .map(|(at, _)| at);
            if let Some(at) = sources.iter().position(|line| Some(*line) == source) {
                marked.insert(at);
            }
        }
    }
    let (offset, viewport) = app.scrolls.show_with(
        frame,
        Pane::Notes,
        area,
        lines,
        look.role(Role::Dim),
        |at, line| {
            let mut line = line.clone();
            let mut cell = 0;
            for span in &mut line.spans {
                let end = cell + span.width();
                if notes.hits.iter().any(|hit| {
                    hit.line == at
                        && Some(hit.link) == selected_link
                        && hit.start < end
                        && hit.end > cell
                }) {
                    span.style = look
                        .row_span(true, span.style, false)
                        .add_modifier(Modifier::BOLD);
                }
                cell = end;
            }
            let is_selected = sources.get(at).copied() == selected && selected.is_some();
            if is_selected {
                line.style = look.selection();
                for span in &mut line.spans {
                    span.style = look.row_span(true, span.style, false);
                }
            }
            line.spans.insert(
                0,
                Span::styled(
                    if marked.contains(&at) { "✎ " } else { "  " },
                    look.row_span(is_selected, look.role(Role::Muted), false),
                ),
            );
            if is_selected {
                let rest = usize::from(area.width).saturating_sub(line.width());
                line.spans.push(Span::styled(
                    " ".repeat(rest),
                    look.row_span(true, Style::default(), false),
                ));
            }
            line
        },
    );
    app.link_hits.borrow_mut().extend(
        notes
            .hits
            .iter()
            .filter(|hit| hit.line >= offset && hit.line < offset + viewport && hit.start < width)
            .map(|hit| {
                (
                    Rect {
                        x: area.x.saturating_add(2 + hit.start as u16),
                        y: area.y + (hit.line - offset) as u16,
                        width: hit.end.min(width).saturating_sub(hit.start) as u16,
                        height: 1,
                    },
                    hit.link,
                )
            }),
    );
    app.note_hits
        .borrow_mut()
        .extend(
            sources
                .iter()
                .enumerate()
                .skip(offset)
                .take(viewport)
                .map(|(line, _)| {
                    (
                        Rect {
                            y: area.y + (line - offset) as u16,
                            height: 1,
                            ..area
                        },
                        line,
                    )
                }),
        );
}
