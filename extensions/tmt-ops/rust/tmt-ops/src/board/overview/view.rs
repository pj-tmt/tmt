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

fn agent_noun(count: usize) -> &'static str {
    if count == 1 { "agent" } else { "agents" }
}

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
                "{} of {} {}",
                observed(if agents {
                    scene.matching_agents
                } else {
                    scene.squads.len()
                }),
                observed(if agents {
                    scene.summary.total
                } else {
                    scene.total_squads
                }),
                if agents {
                    agent_noun(scene.summary.total)
                } else if scene.total_squads == 1 {
                    "squad"
                } else {
                    "squads"
                },
            ),
            look.role(Role::Muted),
        )
    } else if agents {
        let mut spans = vec![Span::styled(
            format!("{} {}", observed(rows.len()), agent_noun(rows.len())),
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
                "{} {} · {} {}",
                scene.squads.len(),
                if scene.squads.len() == 1 {
                    "squad"
                } else {
                    "squads"
                },
                observed(scene.summary.total),
                agent_noun(scene.summary.total),
            ),
            look.role(Role::Muted),
        )
    };
    let band = super::composition::lower::Band::new(app, frame.area().width, area.width);
    let monitor_shown = app
        .overview_panels()
        .shown(crate::config::OverviewPanel::Monitor);
    // Reserve the lower band through placement. Only data rows belong to Rows scroll.
    let mut band_height =
        band.as_ref()
            .map_or(0, |band| band.height as u16)
            .min(if monitor_shown {
                area.height.saturating_sub(LIST_HEADER_ROWS as u16 + 3)
            } else {
                area.height
            });
    let gap = u16::from(band_height > 0 && monitor_shown);
    let mut list_height = if monitor_shown {
        (LIST_HEADER_ROWS + rows.len().max(1))
            .min(usize::from(area.height.saturating_sub(band_height + gap))) as u16
    } else {
        0
    };
    if monitor_shown && band_height > 0 {
        use crate::action::ResizeBoundary as Boundary;
        let total = area.height.saturating_sub(gap);
        list_height = app.pane_resize.size(
            Boundary::OverviewList,
            total,
            list_height,
            LIST_HEADER_ROWS as u16 + 1,
            band_height,
        );
        // Extra space belongs to the lower band only after an explicit resize.
        if app.pane_resize.adjusted(Boundary::OverviewList) {
            band_height = total.saturating_sub(list_height);
        }
        app.pane_resize.record(
            crate::board::pane_resize::Boundary {
                id: Boundary::OverviewList,
                hit: Rect::new(area.x, area.y + list_height + gap, area.width, 1),
                total,
                before: list_height,
                min: LIST_HEADER_ROWS as u16 + 1,
                max: total.saturating_sub(band.as_ref().map_or(0, |band| band.height as u16)),
            },
            frame.area(),
        );
    }
    let lower_available = area.height.saturating_sub(list_height + gap);
    if band.as_ref().is_some_and(|band| band.rows_adjusted(app)) {
        band_height = lower_available;
    }
    if let Some(band) = &band {
        band.register(
            app,
            Rect::new(area.x, area.y + list_height + gap, area.width, band_height),
            lower_available,
            frame.area(),
        );
        for y in 0..band_height {
            band.paint(
                frame,
                app,
                Rect::new(area.x, area.y + list_height + gap + y, area.width, 1),
                usize::from(y),
                lower_available,
            );
        }
    }
    if !monitor_shown {
        return;
    }
    let header_height = (LIST_HEADER_ROWS as u16).min(list_height.saturating_sub(1));
    let data = Rect::new(
        area.x,
        area.y + header_height,
        area.width,
        list_height.saturating_sub(header_height),
    );
    let total = rows.len().max(1);
    *app.row_starts.borrow_mut() = (0..rows.len()).collect();
    if app.follow
        && let Some(i) = rows
            .iter()
            .position(|r| Some(&r.target) == app.workspace.selected.as_ref())
    {
        app.scrolls.reveal_range(Pane::Rows, i..i + 1, data, total);
    }
    // A full two-digit lower-bound label plus the existing overflow mark.
    // Only unread squads need this room; header and every numeric row align.
    let unread = app.unread_identities();
    let team_width = if !unread.is_empty()
        && scene.squads.iter().any(|squad| {
            squad
                .members
                .iter()
                .any(|(id, _)| unread.contains(id.as_str()))
        }) {
        14
    } else {
        10
    };
    // `widest` is the widest agent STATE cell of the list.
    let paint = |frame: &mut Frame, slot: Rect, index: usize, widest: usize| match index {
        0 => {
            for hit in filter_bar::paint(
                frame.buffer_mut(),
                slot,
                &[("agents", "Agents"), ("squads", "Squads")],
                usize::from(!agents),
                &app.search,
                app,
            ) {
                let choice = match hit.id.as_str() {
                    "agents" => Choice::OverviewView(monitor::View::Agents),
                    "squads" => Choice::OverviewView(monitor::View::Squads),
                    _ => Choice::Action(crate::action::Action::parse("search").unwrap()),
                };
                app.workspace_strip_hits
                    .borrow_mut()
                    .push((hit.area, choice));
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
                        " {:<23} {:<11} {:<state$} {:>7}{}",
                        "AGENT",
                        "SQUAD",
                        "STATE",
                        "TOK 1h",
                        if model_width(slot.width, state_width(slot.width, widest)).is_some() {
                            "   MODEL"
                        } else {
                            ""
                        },
                        state = usize::from(state_width(slot.width, widest))
                    )
                } else {
                    format!(
                        " {:<12}{:<team_width$}{:>7}   NOW",
                        "SQUAD", "TEAM", "TOK 1h"
                    )
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
                                "Agents unavailable."
                            } else {
                                "Squads unavailable."
                            }
                        } else if filtered {
                            if agents {
                                "No matching agents."
                            } else {
                                "No matching squads."
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
                return;
            };
            let mut line = if agents {
                agent_line(
                    app,
                    snapshot,
                    row,
                    look,
                    slot.width,
                    state_width(slot.width, widest),
                )
            } else {
                let Some(squad) = scene.squads.get(at - LIST_HEADER_ROWS) else {
                    return;
                };
                // Its TEAM glyphs follow core activity and the stale rule.
                app.admit_activity_clock(None);
                squad_line(
                    app,
                    squad,
                    team_width,
                    look,
                    crate::board::workspace::render::pin_mark(
                        &row.target,
                        &snapshot.preferences,
                        look,
                    ),
                )
            };
            line = crate::board::search_field::highlight_line(line, &app.workspace.query);
            let remaining = usize::from(slot.width).saturating_sub(line.width());
            line.spans.push(Span::raw(" ".repeat(remaining)));
            let selected =
                Some(&row.target) == app.workspace.selected.as_ref() && app.focused() == Pane::Rows;
            if selected {
                line.style = look.selection();
                let mut x = 0;
                for span in &mut line.spans {
                    // An agent's STATE words keep their own colours, as in the tree.
                    let keep = if agents {
                        (AGENT_STATE_X
                            ..AGENT_STATE_X + usize::from(state_width(slot.width, widest)))
                            .contains(&x)
                    } else {
                        (13..13 + team_width).contains(&x)
                            && matches!(span.content.as_ref(), "◆" | "✗")
                    };
                    span.style = look.row_span(true, span.style, keep);
                    x += unicode_width::UnicodeWidthStr::width(span.content.as_ref());
                }
            }
            let displayed = line.to_string();
            strip::paint_left(frame.buffer_mut(), slot, line);
            // The agents list carries no free text (its columns are names, state,
            // tokens and model); only a squad's "now" line can name a member.
            if !agents {
                let source = scene
                    .squads
                    .get(at - LIST_HEADER_ROWS)
                    .map_or_else(String::new, |squad| squad.now.clone());
                crate::board::member_links::paint_plain_hits(
                    frame, app, slot, &source, &displayed, false,
                );
            }
            app.workspace_state_cells.borrow_mut().push(
                Rect::new(
                    slot.x
                        .saturating_add(if agents { AGENT_STATE_X as u16 } else { 13 }),
                    slot.y,
                    if agents {
                        state_width(slot.width, widest)
                    } else {
                        team_width as u16
                    },
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
    };
    // STATE grows only as much as the widest cell of the list needs. Every
    // list row counts, so the header and TOK/NOW stay fixed while rows scroll.
    let widest = if agents {
        rows.iter()
            .map(|row| {
                state_spans(app, row, look, STATE_MAX)
                    .iter()
                    .map(Span::width)
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
    } else {
        0
    };
    for y in 0..header_height {
        paint(
            frame,
            Rect::new(area.x, area.y + y, area.width, 1),
            if header_height < LIST_HEADER_ROWS as u16 && y > 0 {
                usize::from(y) + 1
            } else {
                usize::from(y)
            },
            widest,
        );
    }
    app.scrolls.show_list(
        frame,
        Pane::Rows,
        data,
        total,
        look,
        |frame, viewport, offset| {
            for y in 0..viewport.height {
                paint(
                    frame,
                    Rect::new(viewport.x, viewport.y + y, viewport.width, 1),
                    LIST_HEADER_ROWS + offset + usize::from(y),
                    widest,
                );
            }
        },
    );
}
fn squad_line(
    app: &App,
    squad: &monitor::Squad,
    team_width: usize,
    look: Look,
    pin: Span<'static>,
) -> Line<'static> {
    let name_width = 11usize.saturating_sub(pin.width());
    let members: Vec<_> = squad
        .members
        .iter()
        .map(|(id, state)| team_strip::Member {
            id,
            state: state.as_deref(),
            unread: app.unread_count(id),
        })
        .collect();
    let team = if members.is_empty()
        && squad.observation != crate::board::workspace::OpsObservation::Complete
    {
        Line::styled("—", look.role(Role::Muted))
    } else {
        team_strip::line(&members, squad.lead.as_deref(), team_width, look)
    };
    let padding = team_width.saturating_sub(team.width());
    let mut spans = vec![
        Span::raw(" "),
        pin,
        Span::styled(
            crate::board::view::fit(
                &crate::board::workspace::render::safe(&squad.name),
                name_width,
            ),
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
                crate::source::tokens(n)
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
/// Where an agent row's STATE words start.
const AGENT_STATE_X: usize = 37;

/// The widest agent STATE cell the column ever gives room for.
const STATE_MAX: u16 = 32;

/// The agent STATE column is as wide as the widest cell of the list needs, 12
/// cells at least. It never takes more than the row has after the tokens and
/// at least 16 cells of the model, nor more than `STATE_MAX`.
fn state_width(row: u16, widest: usize) -> u16 {
    let room = row
        .saturating_sub(AGENT_STATE_X as u16 + 1 + 7 + 3 + 16)
        .clamp(12, STATE_MAX);
    u16::try_from(widest).unwrap_or(u16::MAX).clamp(12, room)
}

/// The complete model track yields before the token track at narrow widths.
fn model_width(row: u16, state: u16) -> Option<u16> {
    let width = row.saturating_sub(AGENT_STATE_X as u16 + state + 1 + 7 + 3);
    (width >= 8).then_some(width)
}

/// An agent's STATE cell within `width`: the same activity words and task
/// colours as the agents tree.
fn state_spans(app: &App, row: &OpsTreeRow, look: Look, width: u16) -> Vec<Span<'static>> {
    app.activity_label(row).map_or_else(
        // Without activity, unread replies still replace the words.
        || {
            let unread = app.unread_target(&row.target);
            match (state_cell::unread_mark(unread, look), unread.label()) {
                (Some(mark), Some(label)) => vec![
                    mark,
                    Span::styled(
                        format!(
                            " {}",
                            crate::board::view::fit(&label, usize::from(width).saturating_sub(2))
                        ),
                        look.role(Role::Muted),
                    ),
                ],
                _ => vec![Span::styled("— unreported", look.role(Role::Muted))],
            }
        },
        |label| {
            let (primary, task) = crate::board::workspace::columns::activity_spans(
                app,
                row,
                &label,
                usize::from(width),
                false,
            );
            primary.into_iter().chain(task).collect()
        },
    )
}

fn agent_line(
    app: &App,
    snapshot: &crate::board::workspace::observe::OpsWorkspaceSnapshot,
    row: &OpsTreeRow,
    look: Look,
    row_width: u16,
    width: u16,
) -> Line<'static> {
    let value = monitor::values(app, snapshot, row);
    let mut state = state_spans(app, row, look, width);
    let used = state.iter().map(Span::width).sum::<usize>();
    state.push(Span::raw(
        " ".repeat(usize::from(width).saturating_sub(used)),
    ));
    let tokens = value.tokens.map_or_else(
        || "—".into(),
        |tokens| crate::source::tokens(tokens.round() as u128),
    );
    let pin = crate::board::workspace::render::pin_mark(&row.target, &snapshot.preferences, look);
    let name_width = 23usize.saturating_sub(pin.width() + usize::from(row.lead));
    let model = model_width(row_width, width).map(|width| {
        let text = crate::board::workspace::columns::model_text(
            value.runtime.as_deref(),
            value.model.as_deref(),
        );
        Span::styled(
            format!("   {}", crate::board::view::fit(&text, usize::from(width))),
            look.role(Role::Muted),
        )
    });
    Line::from_iter(
        [
            Span::raw(" "),
            pin,
            Span::styled(if row.lead { "*" } else { "" }, look.role(Role::Muted)),
            Span::styled(
                crate::board::view::fit(
                    &crate::board::workspace::render::safe(&row.title),
                    name_width,
                ),
                look.role(Role::Text),
            ),
            Span::raw(" "),
            Span::styled(
                crate::board::view::fit(value.squad.as_deref().unwrap_or("—"), 11),
                look.role(Role::Muted),
            ),
            Span::raw(" "),
        ]
        .into_iter()
        .chain(state)
        .chain([
            Span::raw(" "),
            Span::styled(
                tmt_cli_style::grid::fit(
                    &tokens,
                    7,
                    tmt_cli_style::grid::Align::Right,
                    tmt_cli_style::grid::Truncate::End,
                ),
                look.role(Role::Muted),
            ),
        ])
        .chain(model),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_model_and_lead_mark_use_acquired_values_without_changing_narrow_token_tracks() {
        use crate::board::{
            app::workspace::tests::{MEMBER, board},
            workspace::OpsTarget,
        };
        for running in [false, true] {
            let mut app = board();
            app.references.agents.insert(
                MEMBER.into(),
                crate::board::inventory::Agent {
                    runtime: running.then(|| "claude".into()),
                    model: Some("claude-sonnet-5-5".into()),
                    ..Default::default()
                },
            );
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .roster
                .squads[0]
                .lead = Some(MEMBER.into());
            app.workspace_reconcile();
            let rows = app.workspace_rows();
            let row = rows
                .iter()
                .find(|row| {
                    matches!(&row.target,
                        OpsTarget::Member { identity_id, .. } if identity_id == MEMBER
                    )
                })
                .unwrap();
            let snapshot = app.workspace_snapshot().unwrap();
            let line = agent_line(&app, snapshot, row, app.look(), 100, state_width(100, 12));
            let shown = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            assert!(shown.contains("*worker"));
            assert_eq!(shown.contains("sonnet-5-5"), running);
            assert!(!shown.contains("claude-"));
            assert!(
                line.spans
                    .last()
                    .unwrap()
                    .content
                    .trim()
                    .ends_with(if running { "sonnet-5-5" } else { "—" })
            );
            let mark = line.spans.iter().find(|span| span.content == "*").unwrap();
            assert_eq!(mark.style, app.look().role(Role::Muted));
            let narrow = agent_line(&app, snapshot, row, app.look(), 60, state_width(60, 12));
            assert_eq!(
                narrow.width(),
                57,
                "the model yields without clipping the token track"
            );
            assert!(model_width(60, state_width(60, 12)).is_none());
        }
    }
}
