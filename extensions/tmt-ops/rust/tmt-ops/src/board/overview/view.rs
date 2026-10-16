//! Squads and waiting-only rows over one admitted source and one scroll owner.
use super::monitor;
use crate::{
    board::{
        app::{App, Hit, HitTarget},
        empty_state, section_rule, state_cell, team_strip, waiting_row,
    },
    config::Pane,
};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let look = app.look();
    let scene = monitor::scene(app, snapshot);
    let rows = app.workspace_rows();
    let squads = scene.squads.len().max(1);
    let needs_rule = 3 + squads;
    let waiting_start = needs_rule + 1;
    let waiting = rows.len().max(1);
    let last = waiting_start + waiting;
    let total = last + 1;
    *app.row_starts.borrow_mut() = (0..rows.len()).map(|index| waiting_start + index).collect();
    if app.follow
        && let Some(index) = rows
            .iter()
            .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
    {
        app.scrolls.reveal_range(
            Pane::Rows,
            waiting_start + index..waiting_start + index + 1,
            area,
            total,
        );
    }
    app.scrolls.show_paint(
        frame,
        Pane::Rows,
        area,
        total,
        look,
        |frame, viewport, offset| {
            for line in 0..usize::from(viewport.height) {
                let index = offset + line;
                let slot = Rect::new(viewport.x, viewport.y + line as u16, viewport.width, 1);
                match index {
                    0 => section_rule::paint(
                        frame.buffer_mut(),
                        slot,
                        &if app.workspace.query.is_empty() {
                            "Squads".into()
                        } else {
                            format!(
                                "Squads · / {}",
                                crate::board::notes::sanitize(&app.workspace.query)
                            )
                        },
                        Line::styled(
                            format!(
                                "{} squads · {} agents",
                                scene.squads.len(),
                                if scene.summary.complete {
                                    scene.summary.total.to_string()
                                } else if scene.summary.total == 0 {
                                    "—".into()
                                } else {
                                    format!("~{}", scene.summary.total)
                                }
                            ),
                            look.role(Role::Muted),
                        ),
                        app.focused() == Pane::Rows,
                        look,
                    ),
                    1 => strip::paint_left(
                        frame.buffer_mut(),
                        slot,
                        Line::styled(
                            format!(" {:<12}{:<10}{:>7}   NOW", "SQUAD", "TEAM", "TOK 1h"),
                            look.role(Role::Muted),
                        ),
                    ),
                    at if (2..2 + squads).contains(&at) => {
                        if let Some(squad) = scene.squads.get(at - 2) {
                            let mut spans = vec![
                                Span::raw(" "),
                                Span::styled(
                                    crate::board::view::fit(
                                        &crate::board::workspace::render::safe(&squad.name),
                                        11,
                                    ),
                                    look.role(Role::Text),
                                ),
                                Span::raw(" "),
                            ];
                            let members: Vec<_> = squad
                                .members
                                .iter()
                                .map(|(id, state)| team_strip::Member {
                                    id,
                                    state: state.as_deref(),
                                })
                                .collect();
                            let team = if members.is_empty()
                                && squad.observation
                                    != crate::board::workspace::OpsObservation::Complete
                            {
                                Line::styled("—", look.role(Role::Muted))
                            } else {
                                team_strip::line(&members, squad.lead.as_deref(), 10, look)
                            };
                            let padding = 10usize.saturating_sub(team.width());
                            spans.extend(team.spans);
                            spans.push(Span::raw(" ".repeat(padding)));
                            let tokens = squad.tokens.map_or_else(
                                || "—".into(),
                                |(tokens, partial)| {
                                    format!(
                                        "{}{}",
                                        if partial { "~" } else { "" },
                                        crate::source::tokens(tokens as f64)
                                    )
                                },
                            );
                            spans.push(Span::styled(
                                format!(
                                    "{}   ",
                                    tmt_tui::text::fit_line(
                                        &tokens,
                                        7,
                                        tmt_tui::style::TextFlow::Truncate,
                                        tmt_cli_style::grid::Align::Right
                                    )
                                ),
                                look.role(Role::Muted),
                            ));
                            if squad.pending {
                                spans.push(state_cell::mark(Some("waiting"), look));
                                spans.push(Span::raw(" "));
                            }
                            spans.push(Span::styled(
                                crate::board::workspace::render::safe(&squad.now),
                                look.role(Role::Muted),
                            ));
                            strip::paint_left(frame.buffer_mut(), slot, Line::from(spans));
                        } else {
                            empty_state::paint(
                                frame.buffer_mut(),
                                slot,
                                if scene.summary.complete {
                                    "No matching squads"
                                } else {
                                    "Squads unavailable"
                                },
                                None,
                                look,
                            );
                        }
                    }
                    at if at == needs_rule => {
                        use crate::action::{OpsUiAction, Verb};
                        let mut note = vec![
                            state_cell::mark(Some("waiting"), look),
                            Span::styled(
                                format!(
                                    " {} waiting",
                                    if scene.summary.complete {
                                        rows.len().to_string()
                                    } else if rows.is_empty() {
                                        "—".into()
                                    } else {
                                        format!("~{}", rows.len())
                                    }
                                ),
                                look.role(Role::Muted),
                            ),
                        ];
                        if let Some((key, _)) = app.bindings().iter().find(|(_, action)| {
                            action.verb == Verb::Ui(OpsUiAction::OverviewFilter)
                        }) {
                            note.extend([
                                Span::styled(" · ", look.role(Role::Muted)),
                                Span::styled(key.clone(), look.role(Role::Text)),
                                Span::styled(" filter", look.role(Role::Muted)),
                            ]);
                        }
                        section_rule::paint(
                            frame.buffer_mut(),
                            slot,
                            "Needs you",
                            Line::from(note),
                            app.focused() == Pane::Rows,
                            look,
                        );
                    }
                    at if (waiting_start..last).contains(&at) => {
                        if let Some(row) = rows.get(at - waiting_start) {
                            let selected = Some(&row.target) == app.workspace.selected.as_ref()
                                && app.focused() == Pane::Rows;
                            let squad = match &row.target {
                                crate::board::workspace::OpsTarget::Member {
                                    room_id: Some(id),
                                    ..
                                } => snapshot
                                    .roster
                                    .squads
                                    .iter()
                                    .find(|squad| &squad.room_id == id)
                                    .map_or("—", |squad| squad.name.as_str()),
                                _ => "Common pool",
                            };
                            let state_cells = waiting_row::paint(
                                frame.buffer_mut(),
                                slot,
                                &waiting_row::Data {
                                    name: &row.title,
                                    squad,
                                    summary: &monitor::summary_text(snapshot.rows.get(&row.target)),
                                },
                                selected,
                                look,
                            );
                            app.workspace_state_cells
                                .borrow_mut()
                                .extend(state_cells.into_iter().filter(|area| !area.is_empty()));
                            app.hits.borrow_mut().push(Hit {
                                x: slot.x,
                                y: slot.y,
                                width: slot.width,
                                target: HitTarget::Row(at - waiting_start),
                            });
                        } else {
                            empty_state::paint(
                                frame.buffer_mut(),
                                slot,
                                if !app.workspace.query.is_empty() {
                                    "No matching waiting agents"
                                } else if scene.summary.complete {
                                    "No agents waiting on you"
                                } else {
                                    "Waiting agents unavailable"
                                },
                                None,
                                look,
                            );
                        }
                    }
                    at if at == last => {
                        use crate::action::{OpsUiAction, Verb};
                        let bindings = app.bindings();
                        let agents = bindings
                            .iter()
                            .find(|(_, action)| action.verb == Verb::Ui(OpsUiAction::Agents))
                            .map(|(key, _)| key.as_str());
                        let chat = bindings.get("enter").filter(|action| {
                            matches!(action.verb, Verb::Ui(OpsUiAction::Inspect) | Verb::Talk)
                                && crate::board::view::footer::row_allows(app, action)
                        });
                        let everyone = app.workspace.query.is_empty()
                            && app.workspace.visibility.scope.is_empty()
                            && (snapshot.preferences.hidden.is_empty()
                                || app.workspace.visibility.reveal_hidden)
                            && app.workspace.filter == crate::board::workspace::AgentsFilter::All;
                        let destination = if everyone {
                            "Agents for everyone"
                        } else {
                            "Agents"
                        };
                        let hint = match (chat, agents) {
                            (Some(_), Some(key)) if !rows.is_empty() => {
                                format!("⏎ open chat  {key} {destination}")
                            }
                            (_, Some(key)) => format!("{key} {destination}"),
                            _ => String::new(),
                        };
                        strip::paint_left(
                            frame.buffer_mut(),
                            Rect::new(
                                slot.x + slot.width.min(3),
                                slot.y,
                                slot.width.saturating_sub(3),
                                1,
                            ),
                            crate::board::key_hints::line(
                                &hint,
                                look,
                                look.role(Role::Text),
                                " · ",
                            ),
                        );
                    }
                    _ => {}
                }
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
            let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
            let id = crate::board::app::workspace::tests::MEMBER;
            home.workspace.roster.members.get_mut(id).unwrap().waiting = true;
            home.workspace.roster.squads[0]
                .member_states
                .get_mut(id)
                .unwrap()
                .waiting = true;
            for (target, row) in &mut home.workspace.rows {
                if matches!(target,OpsTarget::Member{identity_id,..}if identity_id==id) {
                    row["pending"] = serde_json::json!("Approve this change?");
                }
            }
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
