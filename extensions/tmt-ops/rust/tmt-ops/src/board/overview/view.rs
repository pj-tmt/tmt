//! The flat attention list of canonical agent memberships, one row per identity.
use super::monitor;
use crate::{
    board::{
        app::{App, Hit, HitTarget},
        empty_state, section_rule, state_cell,
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

/// The section rule and the column header precede the data rows.
const LIST_HEADER_ROWS: usize = 2;

fn agent_noun(count: usize) -> &'static str {
    if count == 1 { "agent" } else { "agents" }
}

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    if !app
        .overview_panels()
        .shown(crate::config::OverviewPanel::Monitor)
    {
        return;
    }
    let look = app.look();
    let scene = monitor::scene(app, snapshot);
    let rows = app.workspace_rows();
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
    let count = if !app.workspace.query.is_empty() && !app.searching {
        Line::default()
    } else if !app.workspace.query.is_empty() {
        Line::styled(
            format!(
                "{} of {} {}",
                observed(scene.matching_agents),
                observed(scene.summary.total),
                agent_noun(scene.summary.total),
            ),
            look.role(Role::Muted),
        )
    } else {
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
    };
    let header_height = (LIST_HEADER_ROWS as u16).min(area.height.saturating_sub(1));
    let data = Rect::new(
        area.x,
        area.y + header_height,
        area.width,
        area.height.saturating_sub(header_height),
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
    // `widest` is the widest agent STATE cell of the list.
    let paint = |frame: &mut Frame, slot: Rect, index: usize, widest: usize| match index {
        0 => section_rule::paint(
            frame.buffer_mut(),
            slot,
            "Agents",
            count.clone(),
            app.focused() == Pane::Rows,
            look,
        ),
        1 => strip::paint_left(
            frame.buffer_mut(),
            slot,
            Line::styled(
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
                ),
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
                            "Agents unavailable."
                        } else if filtered {
                            "No matching agents."
                        } else {
                            "No agents yet."
                        },
                        None,
                        look,
                    );
                }
                return;
            };
            let mut line = agent_line(
                app,
                snapshot,
                row,
                look,
                slot.width,
                state_width(slot.width, widest),
            );
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
                    let keep = (AGENT_STATE_X
                        ..AGENT_STATE_X + usize::from(state_width(slot.width, widest)))
                        .contains(&x);
                    span.style = look.row_span(true, span.style, keep);
                    x += unicode_width::UnicodeWidthStr::width(span.content.as_ref());
                }
            }
            strip::paint_left(frame.buffer_mut(), slot, line);
            app.workspace_state_cells.borrow_mut().push(
                Rect::new(
                    slot.x.saturating_add(AGENT_STATE_X as u16),
                    slot.y,
                    state_width(slot.width, widest),
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
    // list row counts, so the header and TOK stay fixed while rows scroll.
    let widest = rows
        .iter()
        .map(|row| {
            state_spans(app, row, look, STATE_MAX)
                .iter()
                .map(Span::width)
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    for y in 0..header_height {
        paint(
            frame,
            Rect::new(area.x, area.y + y, area.width, 1),
            usize::from(y),
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
