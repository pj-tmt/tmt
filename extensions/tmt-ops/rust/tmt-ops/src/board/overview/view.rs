//! Monitor paint uses the existing Scrolls, hit reset and shared TUI geometry.
use super::{columns::Column, monitor};
use crate::{
    board::{
        app::{App, Hit, HitTarget},
        view::fit,
    },
    config::Pane,
};
use ratatui::{Frame, layout::Rect, text::Line};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let look = app.look();
    let rows = app.workspace_rows();
    let header = Rect {
        height: area.height.min(1),
        ..area
    };
    let slots = app.overview_layout.borrow_mut().slots(header);
    for (column, slot) in &slots {
        let mark = snapshot
            .preferences
            .overview_sort
            .filter(|order| order.column == *column)
            .map_or("", |order| {
                if order.direction == crate::config::OverviewDirection::Ascending {
                    " ↑"
                } else {
                    " ↓"
                }
            });
        strip::paint_left(
            frame.buffer_mut(),
            *slot,
            Line::styled(
                fit(
                    &format!("{}{mark}", column.title()),
                    usize::from(slot.width),
                ),
                look.role(Role::Muted),
            ),
        );
        if !slot.is_empty() {
            app.overview_sort_hits.borrow_mut().push((*slot, *column));
        }
    }
    let body = Rect::new(
        area.x,
        area.y.saturating_add(header.height),
        area.width,
        area.height.saturating_sub(header.height),
    );
    *app.row_starts.borrow_mut() = (0..rows.len()).collect();
    if app.follow
        && let Some(index) = rows
            .iter()
            .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
    {
        app.scrolls
            .reveal_range(Pane::Rows, index..index + 1, body, rows.len());
    }
    app.scrolls.show_paint(
        frame,
        Pane::Rows,
        body,
        rows.len().max(1),
        look,
        |frame, viewport, offset| {
            for (line, row) in rows
                .iter()
                .skip(offset)
                .take(usize::from(viewport.height))
                .enumerate()
            {
                let values = monitor::values(app, snapshot, row);
                let y = viewport.y + line as u16;
                let selected = Some(&row.target) == app.workspace.selected.as_ref();
                if selected {
                    frame.buffer_mut().set_style(
                        Rect::new(viewport.x, y, viewport.width, 1),
                        look.selection(),
                    );
                }
                for (column, header) in &slots {
                    let slot = Rect::new(header.x, y, header.width, 1).intersection(viewport);
                    if *column == Column::Cron
                        && !slot.is_empty()
                        && let crate::board::workspace::OpsTarget::Member { identity_id, .. } =
                            &row.target
                    {
                        app.cron_scope_hits
                            .borrow_mut()
                            .push(crate::board::app::TimerHit {
                                area: slot,
                                target: row.target.clone(),
                                room: None,
                                owner: Some(identity_id.clone()),
                            });
                    }
                    if *column == Column::Digest
                        && let crate::board::workspace::OpsTarget::Member { identity_id, .. } =
                            &row.target
                    {
                        let labels =
                            crate::board::digest_dashboard::labels(&app.labels, identity_id);
                        if !labels.is_empty() {
                            let mut x = slot.x;
                            for (label, role, action) in labels {
                                let width = unicode_width::UnicodeWidthStr::width(label.as_str())
                                    .min(usize::from(u16::MAX))
                                    as u16;
                                if width > slot.right().saturating_sub(x) {
                                    break;
                                }
                                let cell = Rect::new(x, y, width, 1);
                                let style = if selected {
                                    look.selection().patch(look.row_span(
                                        true,
                                        look.role(role),
                                        true,
                                    ))
                                } else {
                                    look.role(role)
                                };
                                strip::paint_left(
                                    frame.buffer_mut(),
                                    cell,
                                    Line::styled(label, style),
                                );
                                if action {
                                    app.chip_hits.borrow_mut().push((cell, offset + line));
                                }
                                x = x.saturating_add(width + 1);
                            }
                            continue;
                        }
                    }
                    let text = crate::board::notes::sanitize(&if *column == Column::Cron {
                        app.cron_glyphs.label(values.cron)
                    } else {
                        values.text(*column, row)
                    });
                    let role = match column {
                        Column::State | Column::Agent => match row.state.as_deref() {
                            Some("blocked") => Role::Blocked,
                            Some("waiting") => Role::Waiting,
                            Some("working") => Role::Working,
                            _ => Role::Text,
                        },
                        _ => Role::Muted,
                    };
                    let state_style = if matches!(column, Column::State | Column::Agent)
                        && row.state.as_deref() != Some("waiting")
                    {
                        let target = crate::board::app::RowTarget::Workspace(row.target.clone());
                        app.target_row(&target)
                            .and_then(|raw| raw["colors"]["state"].as_str())
                            .map_or_else(|| look.role(role), |color| look.named(color))
                    } else {
                        look.role(role)
                    };
                    let style = if selected {
                        look.selection()
                            .patch(look.row_span(true, state_style, true))
                    } else {
                        state_style
                    };
                    strip::paint_left(
                        frame.buffer_mut(),
                        slot,
                        Line::styled(fit(&text, usize::from(slot.width)), style),
                    );
                }
                app.hits.borrow_mut().push(Hit {
                    x: viewport.x,
                    y,
                    width: viewport.width,
                    target: HitTarget::Row(offset + line),
                });
            }
            if rows.is_empty() {
                strip::paint_left(
                    frame.buffer_mut(),
                    viewport,
                    Line::styled(
                        if app.workspace.query.is_empty() {
                            "No observed agents"
                        } else {
                            "No matching agents"
                        },
                        look.role(Role::Muted),
                    ),
                );
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{app::Effect, shell::OpsTab, workspace::OpsTarget};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    };

    fn draw(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
        terminal
            .draw(|frame| {
                crate::board::view::prepare(app, frame.area());
                crate::board::view::render(frame, app);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }
    #[test]
    fn injected_ascii_counts_render_and_all_count_surfaces_freeze_uuid_scopes() {
        for tab in [OpsTab::Overview, OpsTab::Agents] {
            let mut app = crate::board::app::workspace::tests::board();
            app.shell.tab = tab;
            app.cron_glyphs = crate::board::overview::cron_counts::GlyphSet::from_ascii(true);
            app.cron.replace(Ok(crate::board::cronboard::test_cron(
                vec![],
                tmt_ops::cron::ClockStatus::NoClock,
            )));
            app.overview_rows.borrow_mut().clear();
            let text = draw(&mut app);
            assert!(text.contains("CRON 0/0"), "{text}");
            assert!(!text.contains('◷'), "color depth does not select glyphs");
            let hits = app.cron_scope_hits.borrow().clone();
            assert!(!hits.is_empty());
            if tab == OpsTab::Agents {
                assert!(
                    hits.iter()
                        .any(|hit| matches!(hit.target, OpsTarget::Squad(_)))
                );
                assert!(
                    hits.iter().filter(|hit| hit.owner.is_some()).count() >= 2,
                    "member and shared conversation publish counts"
                );
            }
            let original = app.workspace.selected.clone();
            for hit in hits {
                app.cron_list = None;
                app.workspace.selected = original.clone();
                app.workspace_reconcile();
                draw(&mut app);
                let effect = app.mouse(
                    MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: hit.area.x,
                        row: hit.area.y,
                        modifiers: KeyModifiers::NONE,
                    },
                    std::time::Instant::now(),
                );
                assert_eq!(effect, Effect::None);
                let list = app
                    .cron_list
                    .as_ref()
                    .expect("a complete painted count opens timers");
                assert_eq!(list.scope, hit.room);
                assert_eq!(list.owner, hit.owner);
                assert_eq!(app.workspace.selected.as_ref(), Some(&hit.target));
            }
        }
    }
}
