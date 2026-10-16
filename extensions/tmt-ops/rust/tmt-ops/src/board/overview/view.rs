//! One switchable list over acquired squads and canonical agent memberships.
use super::monitor;
use crate::{
    board::{
        app::{App, Choice, Hit, HitTarget},
        empty_state, filter_bar, section_rule, state_cell, team_strip,
        workspace::OpsTreeRow,
    },
    config::Pane,
    look::Look,
};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

const LIST_HEADER_ROWS: usize = 4;

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let look = app.look();
    let scene = monitor::scene(app, snapshot);
    let rows = app.workspace_rows();
    let agents = app.overview_view == monitor::View::Agents;
    let filtered = !app.workspace.query.is_empty()
        || !app.workspace.visibility.scope.is_empty()
        || (!app.workspace.visibility.reveal_hidden && !snapshot.preferences.hidden.is_empty());
    let observed = |n: usize| {
        if scene.summary.complete {
            n.to_string()
        } else if n == 0 {
            "—".into()
        } else {
            format!("~{n}")
        }
    };
    let count = if !app.workspace.query.is_empty() {
        Line::styled(
            format!(
                "{} of {} agents",
                observed(scene.matching_agents),
                observed(scene.summary.total)
            ),
            look.role(Role::Muted),
        )
    } else if agents {
        let mut spans = vec![Span::styled(
            format!("{} agents", observed(rows.len())),
            look.role(Role::Muted),
        )];
        if scene.waiting > 0 {
            spans.extend([
                Span::styled(" · ", look.role(Role::Muted)),
                state_cell::mark(Some("waiting"), look),
                Span::styled(
                    format!(" {} waiting first", observed(scene.waiting)),
                    look.role(Role::Muted),
                ),
            ]);
        }
        Line::from(spans)
    } else {
        Line::styled(
            format!(
                "{} squads · {} agents",
                scene.squads.len(),
                observed(scene.summary.total)
            ),
            look.role(Role::Muted),
        )
    };
    let total = LIST_HEADER_ROWS + rows.len().max(1);
    *app.row_starts.borrow_mut() = (0..rows.len()).map(|i| LIST_HEADER_ROWS + i).collect();
    if app.follow
        && let Some(i) = rows
            .iter()
            .position(|r| Some(&r.target) == app.workspace.selected.as_ref())
    {
        app.scrolls.reveal_range(
            Pane::Rows,
            LIST_HEADER_ROWS + i..LIST_HEADER_ROWS + i + 1,
            area,
            total,
        );
    }
    app.scrolls.show_list(
        frame,
        Pane::Rows,
        area,
        total,
        look,
        |frame, viewport, offset| {
            for y in 0..viewport.height {
                let index = offset + usize::from(y);
                let slot = Rect::new(viewport.x, viewport.y + y, viewport.width, 1);
                match index {
                    0 => {
                        for hit in filter_bar::paint(
                            frame.buffer_mut(),
                            slot,
                            &[("squads", "Squads"), ("agents", "Agents")],
                            usize::from(agents),
                            &app.workspace.query,
                            look,
                        ) {
                            let view = if hit.id == "agents" {
                                monitor::View::Agents
                            } else {
                                monitor::View::Squads
                            };
                            app.workspace_strip_hits
                                .borrow_mut()
                                .push((hit.area, Choice::OverviewView(view)));
                        }
                    }
                    1 => (),
                    2 => section_rule::paint(
                        frame.buffer_mut(),
                        slot,
                        if agents { "Agents" } else { "Squads" },
                        count.clone(),
                        app.focused() == Pane::Rows,
                        look,
                    ),
                    3 => strip::paint_left(
                        frame.buffer_mut(),
                        slot,
                        Line::styled(
                            if agents {
                                format!(
                                    " {:<23} {:<11} {:<12} {:>7}   NOW",
                                    "AGENT", "SQUAD", "STATE", "TOK 1h"
                                )
                            } else {
                                format!(" {:<12}{:<10}{:>7}   NOW", "SQUAD", "TEAM", "TOK 1h")
                            },
                            look.role(Role::Muted),
                        ),
                    ),
                    at if at >= LIST_HEADER_ROWS => {
                        let Some(row) = rows.get(at - LIST_HEADER_ROWS) else {
                            if at == LIST_HEADER_ROWS {
                                empty_state::paint(
                                    frame.buffer_mut(),
                                    Rect::new(
                                        slot.x.saturating_add(1),
                                        slot.y,
                                        slot.width.saturating_sub(1),
                                        1,
                                    ),
                                    if !scene.summary.complete {
                                        if agents {
                                            "Agents unavailable"
                                        } else {
                                            "Squads unavailable"
                                        }
                                    } else if filtered {
                                        if agents {
                                            "No matching agents"
                                        } else {
                                            "No matching squads"
                                        }
                                    } else if agents {
                                        "No agents yet."
                                    } else {
                                        "No squads yet."
                                    },
                                    None,
                                    look,
                                );
                            }
                            continue;
                        };
                        let mut line = if agents {
                            agent_line(app, snapshot, row, look)
                        } else {
                            let Some(squad) = scene.squads.get(at - LIST_HEADER_ROWS) else {
                                continue;
                            };
                            squad_line(squad, look)
                        };
                        let remaining = usize::from(slot.width).saturating_sub(line.width());
                        line.spans.push(Span::raw(" ".repeat(remaining)));
                        let selected = Some(&row.target) == app.workspace.selected.as_ref()
                            && app.focused() == Pane::Rows;
                        if selected {
                            line.style = look.selection();
                            let mut x = 0;
                            for span in &mut line.spans {
                                let state = if agents {
                                    (37..49).contains(&x)
                                } else {
                                    (13..23).contains(&x)
                                };
                                span.style = look.row_span(
                                    true,
                                    span.style,
                                    state && matches!(span.content.as_ref(), "◆" | "✗"),
                                );
                                x += unicode_width::UnicodeWidthStr::width(span.content.as_ref());
                            }
                        }
                        strip::paint_left(frame.buffer_mut(), slot, line);
                        app.workspace_state_cells.borrow_mut().push(
                            Rect::new(
                                slot.x.saturating_add(if agents { 37 } else { 13 }),
                                slot.y,
                                if agents { 12 } else { 10 },
                                1,
                            )
                            .intersection(slot),
                        );
                        app.hits.borrow_mut().push(Hit {
                            x: slot.x,
                            y: slot.y,
                            width: slot.width,
                            target: HitTarget::Row(at - LIST_HEADER_ROWS),
                        });
                    }
                    _ => (),
                }
            }
        },
    );
}
fn squad_line(squad: &monitor::Squad, look: Look) -> Line<'static> {
    let members: Vec<_> = squad
        .members
        .iter()
        .map(|(id, state)| team_strip::Member {
            id,
            state: state.as_deref(),
        })
        .collect();
    let team = if members.is_empty()
        && squad.observation != crate::board::workspace::OpsObservation::Complete
    {
        Line::styled("—", look.role(Role::Muted))
    } else {
        team_strip::line(&members, squad.lead.as_deref(), 10, look)
    };
    let padding = 10usize.saturating_sub(team.width());
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            crate::board::view::fit(&crate::board::workspace::render::safe(&squad.name), 11),
            look.role(Role::Text),
        ),
        Span::raw(" "),
    ];
    spans.extend(team.spans);
    spans.push(Span::raw(" ".repeat(padding)));
    let tokens = squad.tokens.map_or_else(
        || "—".into(),
        |(n, partial)| {
            format!(
                "{}{}",
                if partial { "~" } else { "" },
                crate::source::tokens(n as f64)
            )
        },
    );
    spans.push(Span::styled(
        format!(
            "{}   ",
            tmt_cli_style::grid::fit(
                &tokens,
                7,
                tmt_cli_style::grid::Align::Right,
                tmt_cli_style::grid::Truncate::End
            )
        ),
        look.role(Role::Muted),
    ));
    if squad.pending {
        spans.extend([state_cell::mark(Some("waiting"), look), Span::raw(" ")]);
    }
    spans.push(Span::styled(
        crate::board::workspace::render::safe(&squad.now),
        look.role(Role::Muted),
    ));
    Line::from(spans)
}
fn agent_line(
    app: &App,
    snapshot: &crate::board::workspace::observe::OpsWorkspaceSnapshot,
    row: &OpsTreeRow,
    look: Look,
) -> Line<'static> {
    let value = monitor::values(app, snapshot, row);
    let state = if monitor::waits(snapshot, row) {
        Some("waiting")
    } else {
        row.state.as_deref()
    };
    let tokens = value
        .tokens
        .map_or_else(|| "—".into(), crate::source::tokens);
    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            crate::board::view::fit(&crate::board::workspace::render::safe(&row.title), 23),
            look.role(Role::Text),
        ),
        Span::raw(" "),
        Span::styled(
            crate::board::view::fit(value.squad.as_deref().unwrap_or("—"), 11),
            look.role(Role::Muted),
        ),
        Span::raw(" "),
        state_cell::mark(state, look),
        Span::raw(" "),
        Span::styled(
            crate::board::view::fit(&state_cell::word(state, look).content, 10),
            look.role(Role::Muted),
        ),
        Span::raw(" "),
        Span::styled(
            format!(
                "{}   ",
                tmt_cli_style::grid::fit(
                    &tokens,
                    7,
                    tmt_cli_style::grid::Align::Right,
                    tmt_cli_style::grid::Truncate::End
                )
            ),
            look.role(Role::Muted),
        ),
        Span::styled(
            crate::board::workspace::render::safe(&monitor::summary_text(
                snapshot.rows.get(&row.target),
            )),
            look.role(Role::Muted),
        ),
    ])
}
