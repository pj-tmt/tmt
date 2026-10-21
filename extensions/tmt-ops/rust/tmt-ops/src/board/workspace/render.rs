//! Dense tree and squad detail over the admitted snapshot. Shared placement,
//! Strip/Outline, Scrolls and the WP1 conversation retain geometry and paint.
use super::{OpsObservation, OpsTarget, SelectionState, manage::Stage};
use crate::{
    board::{
        app::{App, Choice, Hit, HitTarget, TabHit},
        shell::OpsTab,
    },
    config::Pane,
};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::{KeyHint, Modal, Placement, footer as fit_hints, strip};

pub(in crate::board) fn safe(text: &str) -> String {
    tmt_cli_style::table::escape(&crate::board::notes::sanitize(text))
}
/// Fixed name-cell slot: pin state never changes table column geometry.
pub(in crate::board) fn pin_mark(
    target: &OpsTarget,
    preferences: &super::OpsPreferences,
    look: crate::look::Look,
) -> Span<'static> {
    let pinned = match target {
        OpsTarget::Squad(room) => preferences.pinned.contains(room),
        OpsTarget::Member { identity_id, .. } => preferences.agent_pins.contains(identity_id),
        OpsTarget::Pool | OpsTarget::DigestAll => false,
    };
    Span::styled(if pinned { "▪ " } else { "  " }, look.role(Role::Muted))
}

fn member_noun(count: Option<usize>) -> &'static str {
    if count == Some(1) {
        "member"
    } else {
        "members"
    }
}

pub(in crate::board) fn tabs(app: &App, area: Rect) -> Line<'static> {
    let tabs = [OpsTab::Agents, OpsTab::Digest];
    let selected = tabs
        .iter()
        .position(|tab| *tab == app.shell.tab)
        .unwrap_or(0);
    use crate::action::OpsUiAction;
    let keys = [
        (OpsUiAction::Agents, "1"),
        (OpsUiAction::DigestDashboard, "2"),
    ]
    .map(|(action, preferred)| crate::board::keycap::key(app, action, preferred));
    let (line, hits) =
        crate::board::tab_strip::line(&tabs.map(OpsTab::label), &keys, selected, area, app.look());
    app.tab_hits
        .borrow_mut()
        .extend(hits.into_iter().map(|(tab, area)| TabHit {
            x: area.x,
            y: area.y,
            width: area.width,
            tab,
        }));
    line
}

pub(in crate::board) fn summary(app: &App, width: u16) -> Line<'static> {
    let Some(snapshot) = app.workspace_snapshot() else {
        return Line::default();
    };
    if app.loading() {
        return Line::styled("Loading workspace…", app.look().role(Role::Muted));
    }
    if !snapshot.notices.is_empty() {
        return Line::styled(
            "Configuration needs migration · ? help shows located settings; file preserved",
            app.look().role(Role::Waiting),
        );
    }
    if app.shell.tab == OpsTab::Agents {
        let complete = snapshot.roster.observation == OpsObservation::Complete
            && snapshot.roster.membership_complete
            && snapshot
                .roster
                .squads
                .iter()
                .all(|squad| squad.observation == OpsObservation::Complete);
        if complete
            && app.error.is_none()
            && let Some(view) = &app.view
        {
            return crate::board::home::summary_of(app, view, width, app.look());
        }
        let status = if app.error.is_some() || snapshot.roster.observation == OpsObservation::Stale
        {
            "stale"
        } else {
            "partial or unavailable"
        };
        return Line::styled(
            format!(
                "Observed {} squads · {} agents · {status}; totals unknown",
                snapshot.roster.squads.len(),
                snapshot.roster.members.len()
            ),
            app.look().role(Role::Muted),
        );
    }
    let name = app
        .workspace
        .selected
        .as_ref()
        .and_then(|target| snapshot.rows.get(target))
        .and_then(|row| row["name"].as_str())
        .unwrap_or("Select a squad or agent");
    let status = match app.workspace_selection_state() {
        SelectionState::Missing => " · target removed",
        SelectionState::Hidden => " · hidden target",
        SelectionState::Unknown => " · target unavailable",
        _ => "",
    };
    Line::styled(
        format!(
            "{} · {}{}{}",
            app.shell.tab.label(),
            safe(name),
            status,
            if app.error.is_some() { " · stale" } else { "" }
        ),
        app.look().role(Role::Text),
    )
}
pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    if matches!(app.shell.tab, OpsTab::Agents) {
        crate::board::overview::chart::host::render_panel(
            frame,
            app,
            app.overview_usage_band(area),
            crate::status::now_ms(),
        );
    }
    let Some(placement) = app.workspace_placement(area) else {
        return;
    };
    use crate::action::ResizeBoundary as Boundary;
    let (columns, rows) = (Boundary::AgentsColumns, Boundary::AgentsRows);
    app.pane_resize.record_panes(
        placement,
        columns,
        rows,
        app.agents_list_min_columns(),
        frame.area(),
    );
    if matches!(app.shell.tab, OpsTab::Agents) {
        let usage = app.overview_usage_band(area);
        if usage.height > 1
            && app
                .overview_panels()
                .shown(crate::config::OverviewPanel::Monitor)
        {
            let min = crate::board::overview::chart::host::overview_demand(
                app,
                area.width.saturating_sub(1),
                area.height,
                crate::status::now_ms(),
            );
            app.pane_resize.record(
                crate::board::pane_resize::Boundary {
                    id: Boundary::AgentsUsage,
                    hit: Rect::new(
                        area.x,
                        usage.bottom(),
                        placement.list.map_or(0, |list| list.width),
                        1,
                    ),
                    total: area.height,
                    before: usage.height,
                    min,
                    max: area
                        .height
                        .saturating_sub(crate::board::overview::composition::minimum_height()),
                },
                frame.area(),
            );
        }
    }
    if app.shell.tab == OpsTab::Agents
        && let (Some(list), Some(detail)) = (placement.list, placement.conversation)
        && list.y == detail.y
    {
        crate::board::divider::paint(
            frame.buffer_mut(),
            Rect::new(list.right(), list.y, 1, list.height),
            true,
            app.look(),
        );
    }
    if let Some(list) = placement.list {
        if app.flat_agents() {
            crate::board::overview::composition::render(frame, app, list);
        } else {
            tree(frame, app, list);
        }
    }
    app.conversation.borrow_mut().pane = placement.conversation;
    if let Some(detail) = placement.conversation {
        if app.shell.tab == OpsTab::Agents
            && app.right_tab == crate::board::shell::RightTab::Measure
        {
            crate::board::overview::composition::measure::render(frame, app, detail);
        } else if app.shell.tab == OpsTab::Agents {
            crate::board::agents_focus::render(frame, app, detail);
        } else if let Some(target) = app.workspace_retained_search_target() {
            if matches!(target, OpsTarget::Member { .. }) {
                crate::board::conversation::render_retained(frame, app, detail, target);
            } else {
                squad_detail(frame, app, detail);
            }
        } else if app.workspace_selected_member().is_some() {
            crate::board::conversation::render(frame, app, detail);
        } else {
            squad_detail(frame, app, detail);
        }
    }
    if app.shell.tab == OpsTab::Agents {
        crate::board::pane_chrome::mark_focused_rule(
            frame.buffer_mut(),
            app,
            placement.list,
            placement.conversation,
        );
    }
}
fn tree(frame: &mut Frame, app: &App, area: Rect) {
    use super::columns::{self, AgentsField as Field};
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let look = app.look();
    let inside = area;
    // The section rule yields before the table header, range and last target.
    let rule_height = u16::from(inside.height >= 4);
    let header_height = u16::from(inside.height >= 3);
    let slots = app.agents_layout.borrow_mut().slots(Rect {
        y: inside.y + rule_height,
        ..inside
    });
    let totals = if app.workspace.query.is_empty() {
        let squads = snapshot
            .roster
            .squads
            .iter()
            .filter(|squad| {
                app.workspace
                    .visibility
                    .shows_squad(&squad.room_id, &snapshot.preferences)
                    && (app.workspace.filter != super::AgentsFilter::Pinned
                        || snapshot.preferences.pinned.contains(&squad.room_id)
                        || squad
                            .members
                            .iter()
                            .any(|id| snapshot.preferences.agent_pins.contains(id)))
            })
            .count();
        let (_, agents) = app
            .workspace
            .search_counts(&snapshot.roster, &snapshot.preferences);
        Line::styled(
            format!(
                "{squads} {} · {agents} {}",
                if squads == 1 { "squad" } else { "squads" },
                if agents == 1 { "agent" } else { "agents" }
            ),
            look.role(Role::Muted),
        )
    } else if !app.searching {
        Line::default()
    } else {
        let (matched, total) = app
            .workspace
            .search_counts(&snapshot.roster, &snapshot.preferences);
        Line::styled(
            format!("{matched} of {total} agents"),
            look.role(Role::Muted),
        )
    };
    controls_rule(
        frame,
        app,
        Rect {
            height: rule_height,
            ..inside
        },
    );
    if let Some(chip) = columns::paint_agents_header(
        frame.buffer_mut(),
        Rect {
            y: inside.y + rule_height,
            height: header_height,
            ..inside
        },
        &slots,
        look,
        app.cron_glyphs,
        app.tok_range(),
    ) {
        app.workspace_strip_hits
            .borrow_mut()
            .push((chip, Choice::TokRange(app.tok_range().next())));
    }
    let body = Rect::new(
        inside.x,
        inside.y + rule_height + header_height,
        inside.width,
        inside.height.saturating_sub(rule_height + header_height),
    );
    let rows = app.workspace_rows();
    *app.row_starts.borrow_mut() = (0..rows.len()).collect();
    if app.follow
        && let Some(index) = rows
            .iter()
            .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
    {
        app.scrolls
            .reveal_table_range(Pane::Rows, index..index + 1, body);
    }
    let paint_rows = |frame: &mut Frame, viewport: Rect, offset: usize| {
        for (index, row) in rows
            .iter()
            .enumerate()
            .skip(offset)
            .take(usize::from(viewport.height))
        {
            let y = viewport.y + (index - offset) as u16;
            let selected = Some(&row.target) == app.workspace.selected.as_ref();
            let stripe = Rect::new(viewport.x, y, viewport.width, 1);
            let scope = match &row.target {
                OpsTarget::Squad(room) => Some(
                    crate::board::overview::cron_counts::CronCountScope::Room(room),
                ),
                OpsTarget::Member {
                    room_id,
                    identity_id,
                } => Some(crate::board::overview::cron_counts::CronCountScope::Owner {
                    id: identity_id,
                    room: room_id.as_deref(),
                }),
                OpsTarget::Pool | OpsTarget::DigestAll => None,
            };
            let timers = scope.and_then(|scope| app.cron.counts.count(scope));
            let tokens = match &row.target {
                OpsTarget::Squad(room) => snapshot
                    .roster
                    .squads
                    .iter()
                    .find(|squad| &squad.room_id == room)
                    .and_then(|squad| app.overview_squad_usage(squad)),
                OpsTarget::Member { identity_id, .. } => app
                    .overview_member_usage(identity_id)
                    .map(|reading| (reading.tokens, reading.partial)),
                OpsTarget::Pool | OpsTarget::DigestAll => None,
            }
            .map_or_else(
                || "—".into(),
                |(tokens, partial)| {
                    format!(
                        "{}{}",
                        if partial { "~" } else { "" },
                        crate::source::tokens(tokens)
                    )
                },
            );
            let model_width = slots
                .iter()
                .find(|(field, _)| *field == Field::Model)
                .map_or(0, |(_, rect)| usize::from(rect.width));
            let state_width = slots
                .iter()
                .find(|(field, _)| *field == Field::State)
                .map_or(0, |(_, rect)| usize::from(rect.width));
            let (state, state_primary_width) = if let Some(label) = app.activity_label(row) {
                let (primary, task) = columns::activity_spans(app, row, &label, state_width, false);
                let width = primary.iter().map(Span::width).sum();
                (
                    Line::from_iter(primary.into_iter().chain(task)),
                    Some(width),
                )
            } else {
                let squad = if let OpsTarget::Squad(room) = &row.target {
                    snapshot
                        .roster
                        .squads
                        .iter()
                        .find(|squad| &squad.room_id == room)
                } else {
                    None
                };
                let ids: Vec<_> = squad.map_or_else(
                    || {
                        snapshot
                            .roster
                            .pool()
                            .map(|member| member.id.as_str())
                            .collect()
                    },
                    |squad| squad.members.iter().map(String::as_str).collect(),
                );
                let stale = app.activity_stale(std::time::Instant::now());
                app.admit_activity_clock(None);
                let now = crate::status::now_ms();
                let members: Vec<_> = ids
                    .iter()
                    .map(|id| {
                        // TEAM needs only the activity glyph, not a cloned display row/task.
                        let label = snapshot.roster.members.get(*id).map(|member| {
                            let state = squad.and_then(|squad| squad.member_states.get(*id));
                            crate::board::activity::Member {
                                activity: member.activity,
                                waiting: state.map_or(member.waiting, |state| state.waiting),
                                idle_over: state.is_some_and(|state| state.idle_over),
                                task: None,
                            }
                            .label(stale, now)
                        });
                        crate::board::team_strip::Member {
                            id,
                            over: label.as_ref().is_some_and(|label| label.idle_over),
                            state: label.map(|label| label.state),
                            unread: app.unread_count(id),
                        }
                    })
                    .collect();
                let state = if members.is_empty()
                    && snapshot.roster.observation != OpsObservation::Complete
                {
                    Line::styled("—", look.role(Role::Muted))
                } else {
                    crate::board::team_strip::line(
                        &members,
                        squad.and_then(|squad| squad.lead.as_deref()),
                        state_width,
                        look,
                        app.cron_glyphs,
                    )
                };
                (state, None)
            };
            let expanded = rows
                .get(index + 1)
                .is_some_and(|next| next.depth > row.depth)
                || match &row.target {
                    OpsTarget::Squad(room) => app.workspace.expanded.contains(room),
                    OpsTarget::Pool => app.workspace.pool_expanded,
                    _ => false,
                };
            let pinned = match &row.target {
                OpsTarget::Squad(room) => snapshot.preferences.pinned.contains(room),
                OpsTarget::Member { identity_id, .. } => {
                    snapshot.preferences.agent_pins.contains(identity_id)
                }
                OpsTarget::Pool | OpsTarget::DigestAll => false,
            };
            let model: String = match &row.target {
                OpsTarget::Member { identity_id, .. } => {
                    let agent = app.references.agents.get(identity_id);
                    columns::model_text(
                        agent.and_then(|agent| agent.runtime.as_deref()),
                        agent.and_then(|agent| agent.model.as_deref()),
                    )
                }
                // A squad's idle alert, else what it is doing now.
                OpsTarget::Squad(room) => snapshot
                    .roster
                    .squads
                    .iter()
                    .find(|squad| &squad.room_id == room)
                    .map(|squad| {
                        squad
                            .idle_alert
                            .and_then(|alert| alert.label(model_width))
                            .unwrap_or_else(|| {
                                // The tree never discloses a waiting preview.
                                let (now, pending) =
                                    crate::board::overview::monitor::squad_now(snapshot, squad);
                                if pending {
                                    "waiting on you".to_owned()
                                } else {
                                    crate::board::workspace::render::safe(&now)
                                }
                            })
                    })
                    .unwrap_or_default(),
                OpsTarget::Pool | OpsTarget::DigestAll => String::new(),
            };
            // Decided once per row: focus in this agent's conversation.
            let in_conversation = selected
                && matches!(row.target, OpsTarget::Member { .. })
                && app.focused() == Pane::Replies;
            app.conversation_tinted
                .set(app.conversation_tinted.get() || in_conversation);
            let painted = crate::board::tree_row::paint(
                frame.buffer_mut(),
                stripe,
                &slots,
                &crate::board::tree_row::Row {
                    source: row,
                    expanded,
                    last: !rows.get(index + 1).is_some_and(|next| next.depth == 1),
                    pinned,
                    state,
                    state_primary_width,
                    tokens,
                    model: model.clone(),
                    timers: timers.map(|count| count.total),
                    selected,
                    in_conversation,
                    query: &app.workspace.query,
                },
                look,
            );
            if let Some(area) = painted.task {
                crate::board::conversation::source::publish(
                    app,
                    crate::board::app::RowTarget::Workspace(row.target.clone()),
                    area,
                );
            }
            // A squad's "now" line can name a member.
            if matches!(row.target, OpsTarget::Squad(_))
                && let Some(area) = painted.model
            {
                let displayed = crate::board::view::fit(&model, usize::from(area.width));
                crate::board::member_links::paint_plain_hits(
                    frame, app, area, &model, &displayed, false,
                );
            }
            app.workspace_state_cells
                .borrow_mut()
                .extend(painted.state.into_iter().chain(painted.task));
            if let Some(area) = painted.timers {
                let (room, owner) = match &row.target {
                    OpsTarget::Squad(room) => (Some(room.clone()), None),
                    OpsTarget::Member {
                        room_id,
                        identity_id,
                    } => (room_id.clone(), Some(identity_id.clone())),
                    OpsTarget::Pool | OpsTarget::DigestAll => (None, None),
                };
                app.cron_scope_hits
                    .borrow_mut()
                    .push(crate::board::app::TimerHit {
                        area,
                        target: row.target.clone(),
                        room,
                        owner,
                    });
            }
            app.hits.borrow_mut().push(Hit {
                x: stripe.x,
                y,
                width: stripe.width,
                target: HitTarget::Row(index),
            });
        }
        if rows.is_empty() && offset == 0 {
            let (message, action) = if app.workspace.query.is_empty() {
                ("No agents in this view", None)
            } else {
                ("No matching agents.", Some(("Esc", "clear filter")))
            };
            crate::board::empty_state::paint(frame.buffer_mut(), viewport, message, action, look);
        }
    };
    app.scrolls
        .show_table(frame, Pane::Rows, body, rows.len(), look, paint_rows);
    if body.height >= 2 {
        paint_totals(
            frame,
            Rect::new(body.x, body.bottom() - 1, body.width, 1),
            totals,
        );
    }
}

/// The counts sit at the right of the rows line, below the list they describe.
pub(in crate::board) fn paint_totals(frame: &mut Frame, line: Rect, totals: Line<'static>) {
    // The range text on the left is at most 36 cells wide; yield before they meet.
    if totals.width() == 0 || usize::from(line.width) < totals.width() + 40 {
        return;
    }
    tmt_tui::components::strip::paint_right(
        frame.buffer_mut(),
        Rect::new(line.x, line.y, line.width.saturating_sub(1), 1),
        totals,
    );
}

/// The left pane's section rule carries the list controls: the filter chips
/// and the search field.
pub(in crate::board) fn controls_rule(frame: &mut Frame, app: &App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let look = app.look();
    let focused = app.focused() == Pane::Rows;
    let rule = Line::styled(
        "─".repeat(usize::from(area.width)),
        look.role(if focused { Role::Muted } else { Role::Dim }),
    );
    tmt_tui::components::strip::paint_left(frame.buffer_mut(), Rect { height: 1, ..area }, rule);
    let inside = Rect::new(
        area.x + 2.min(area.width),
        area.y,
        area.width.saturating_sub(2),
        1,
    );
    use super::AgentsFilter;
    let search_count = app
        .workspace_snapshot()
        .filter(|_| !app.search.is_empty() && !app.searching)
        .map(|snapshot| {
            let (matched, total) = app
                .workspace
                .search_counts(&snapshot.roster, &snapshot.preferences);
            // A partial observation counts approximately, as the shell counts do.
            let mark = if snapshot.roster.observation == super::OpsObservation::Complete {
                ""
            } else {
                "~"
            };
            format!("{mark}{matched} of {mark}{total}")
        });
    for hit in crate::board::filter_bar::paint(
        frame.buffer_mut(),
        inside,
        &[
            ("all", "Squads"),
            ("agents", "Agents"),
            ("pinned", "Pinned"),
        ],
        match app.workspace.filter {
            AgentsFilter::All => 0,
            AgentsFilter::Agents => 1,
            AgentsFilter::Pinned => 2,
        },
        app,
        crate::action::OpsUiAction::AgentsFilter,
        search_count.as_deref(),
    ) {
        let choice = match hit.id.as_str() {
            "all" => Choice::AgentsFilter(AgentsFilter::All),
            "agents" => Choice::AgentsFilter(AgentsFilter::Agents),
            "pinned" => Choice::AgentsFilter(AgentsFilter::Pinned),
            "clear-search" => Choice::ClearSearch,
            _ => Choice::Action(crate::action::Action::parse("search").unwrap()),
        };
        app.workspace_strip_hits
            .borrow_mut()
            .push((hit.area, choice));
    }
}

/// Format only supplied observations, independently of scroll and width.
/// Model observation survives disabled sampling; missing readings/share never
/// become zero and each reading retains its configured window.
fn squad_usage(
    usage: Option<&crate::board::app::HomeUsage<'_>>,
    model: Option<&str>,
) -> Vec<String> {
    let reading = |value: Option<crate::board::rate::Reading>| {
        value.map_or_else(
            || "not reported".into(),
            |value| {
                format!(
                    "{}{} tok",
                    if value.partial { "~" } else { "" },
                    crate::source::tokens(value.tokens)
                )
            },
        )
    };
    let mut text = Vec::new();
    if let Some(model) = usage.and_then(|usage| usage.lead_model).or(model) {
        text.push(format!(
            "Lead model: {}",
            safe(crate::source::model_name(model))
        ));
    }
    if let Some(usage) = usage {
        for (label, values) in [("Usage", usage.squad), ("Lead usage", usage.lead)] {
            let pieces: Vec<_> = values
                .iter()
                .zip(usage.windows)
                .map(|(value, window)| format!("{} {}", window.label(), reading(*value)))
                .collect();
            text.push(format!("{label}: {}", pieces.join(" · ")));
        }
        text.push(format!(
            "Lead share: {}",
            usage.share.as_ref().map_or_else(
                || "not reported".into(),
                |share| format!(
                    "{}{:.0}%",
                    if share.partial { "~" } else { "" },
                    share.fraction * 100.0
                ),
            )
        ));
    } else {
        text.push("Usage: not reported".into());
    }
    text
}

pub(in crate::board) fn selection_message(state: SelectionState) -> &'static str {
    match state {
        SelectionState::Hidden => {
            "Selected target is hidden by the filter or collapse. Select a visible target explicitly."
        }
        SelectionState::Missing => {
            "Selected target no longer exists. Select a new target explicitly."
        }
        SelectionState::Unknown => "Selected target could not be observed. Refresh before acting.",
        _ => "Select a squad or agent.",
    }
}

fn squad_detail(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let title = app
        .selected_row()
        .or_else(|| {
            app.workspace_retained_search_target()
                .and_then(|target| snapshot.rows.get(target))
        })
        .and_then(|row| row["name"].as_str())
        .unwrap_or("Selection");
    let mut inside = crate::board::pane_chrome::paint(frame, app, area, Pane::Replies, title);
    if let Some(target) = app.workspace.selected.as_ref().filter(|target| {
        app.workspace_selection_visible()
            && (matches!(target, OpsTarget::Squad(_))
                || matches!(target, OpsTarget::Pool) && snapshot.roster.members.is_empty())
    }) {
        use super::manage::Kind;
        use crate::action::{Action, OpsUiAction};
        let mut actions = if matches!(target, OpsTarget::Squad(_)) {
            vec![
                ("Add member", Choice::WorkspaceBegin(Kind::Add)),
                ("Set/clear lead", Choice::WorkspaceBegin(Kind::Lead)),
                ("Rename", Choice::WorkspaceBegin(Kind::RenameSquad)),
                (
                    "Pin",
                    Choice::Action(Action::parse(OpsUiAction::Pin.spec().name).unwrap()),
                ),
                (
                    "Timers",
                    Choice::Action(Action::parse(OpsUiAction::Timers.spec().name).unwrap()),
                ),
            ]
        } else {
            vec![]
        }
        .into_iter()
        .map(|(label, choice)| (label.to_owned(), choice))
        .collect::<Vec<_>>();
        if let OpsTarget::Squad(id) = target {
            let verb = if snapshot.preferences.hidden.contains(id) {
                OpsUiAction::ShowSquad
            } else {
                OpsUiAction::HideSquad
            };
            let action = Action::parse(verb.spec().name).unwrap();
            actions.push((
                crate::board::view::footer::hint_word(app, &action),
                Choice::Action(action),
            ));
        }
        if matches!(target, OpsTarget::Pool)
            && let Some(action) = app
                .workspace_bindings()
                .values()
                .find(|a| matches!(a.verb, crate::action::Verb::Ui(OpsUiAction::DiscoverAgents)))
                .cloned()
        {
            actions.push((
                "Add an agent already running".into(),
                Choice::Action(action),
            ));
        }
        let mut x = inside.x;
        let mut y = inside.y;
        for (label, choice) in actions {
            let width = unicode_width::UnicodeWidthStr::width(label.as_str()) as u16 + 2;
            if x.saturating_add(width) > inside.right() {
                x = inside.x;
                y = y.saturating_add(1);
            }
            if y < inside.bottom() && width <= inside.width {
                let Some(control) = crate::board::buttons::paint(
                    app,
                    frame.buffer_mut(),
                    Rect::new(x, y, width, 1),
                    &[crate::board::buttons::Button::secondary(label.as_str())],
                    tmt_cli_style::grid::Align::Left,
                    None,
                )[0] else {
                    continue;
                };
                app.workspace_action_hits
                    .borrow_mut()
                    .push((control, target.clone(), choice));
                x += width + 1;
            }
        }
        let used = y
            .saturating_sub(inside.y)
            .saturating_add(1)
            .min(inside.height);
        inside.y += used;
        inside.height -= used;
    }
    if (app.workspace_selection_visible() || app.workspace_retained_search_target().is_some())
        && let Some(target) = app.workspace.selected.as_ref()
    {
        roster_preview(
            frame,
            app,
            inside,
            target,
            app.workspace_selection_visible(),
        );
        return;
    }
    let text = selection_message(app.workspace_selection_state());
    let lines: Vec<_> = crate::board::notes::wrap(text, usize::from(inside.width))
        .into_iter()
        .map(|text| Line::styled(text, look.role(Role::Text)))
        .collect();
    app.scrolls.show(frame, Pane::Replies, inside, lines, look);
}

fn roster_preview(
    frame: &mut Frame,
    app: &App,
    mut area: Rect,
    target: &OpsTarget,
    interactive: bool,
) {
    use super::columns::{self, RosterField};
    let snapshot = app.workspace_snapshot().unwrap();
    let look = app.look();
    let squad = match target {
        OpsTarget::Squad(room) => snapshot.roster.squads.iter().find(|s| &s.room_id == room),
        _ => None,
    };
    let mut members: Vec<_> = match squad {
        Some(squad) => snapshot.roster.members(squad).collect(),
        None => snapshot.roster.pool().collect(),
    };
    members.sort_by(|a, b| {
        let lead = squad.and_then(|s| s.lead.as_deref());
        (lead != Some(a.id.as_str()))
            .cmp(&(lead != Some(b.id.as_str())))
            .then(a.name.cmp(&b.name))
            .then(a.id.cmp(&b.id))
    });
    members.dedup_by(|a, b| a.id == b.id);
    let observation = squad.map_or(snapshot.roster.observation, |s| s.observation);
    let mut savings_lines = 0..0;
    let facts: Vec<Line<'static>> = squad.map_or_else(Vec::new, |squad| {
        let usage = app.home_usage(&squad.name, std::time::Instant::now());
        let mut facts: Vec<_> = squad_usage(usage.as_ref(), app.home_lead_model(&squad.name))
            .iter()
            .flat_map(|fact| crate::board::notes::wrap(fact, usize::from(area.width)))
            .map(|text| Line::styled(text, look.role(Role::Muted)))
            .collect();
        let savings = app.digest_savings(squad.members.iter().map(String::as_str));
        let caption = app.savings_caption(&savings, usize::from(area.width));
        let start = facts.len();
        facts.extend(
            crate::board::notes::wrap(&caption, usize::from(area.width))
                .into_iter()
                .map(|text| Line::styled(text, look.role(Role::Dim))),
        );
        savings_lines = start..facts.len();
        if let Some(start) = app.digest_savings_start(&savings, usize::from(area.width)) {
            facts.extend(
                crate::board::notes::wrap(&start, usize::from(area.width))
                    .into_iter()
                    .map(|text| Line::styled(text, look.role(Role::Dim))),
            );
        }
        facts
    });
    if area.height >= 5 {
        let lead = squad
            .and_then(|s| s.lead.as_ref())
            .and_then(|id| snapshot.roster.members.get(id))
            .map_or("—", |m| m.name.as_str());
        let count = (squad.is_some_and(|s| s.observation == OpsObservation::Complete)
            || squad.is_none() && snapshot.roster.complete())
        .then_some(members.len());
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, area.y, area.width, 1),
            Line::styled(
                format!(
                    "Roster preview · Lead {} · {} {}{}",
                    safe(lead),
                    count.map_or_else(|| "unknown".into(), |n| n.to_string()),
                    member_noun(count),
                    if observation == OpsObservation::Complete {
                        ""
                    } else {
                        " · partial observation"
                    }
                ),
                look.role(Role::Muted),
            ),
        );
        area.y += 1;
        area.height -= 1;
    }
    let header_height = u16::from(area.height >= 3);
    let slots = app.agents_layout.borrow_mut().roster_slots(area);
    columns::paint_roster_header(
        frame.buffer_mut(),
        Rect {
            height: header_height,
            ..area
        },
        &slots,
        look,
    );
    let body = Rect::new(
        area.x,
        area.y + header_height,
        area.width,
        area.height.saturating_sub(header_height),
    );
    let member_rows = members.len().max(1);
    app.scrolls.show_table(
        frame,
        Pane::Replies,
        body,
        member_rows + facts.len(),
        look,
        |frame, viewport, offset| {
            for (index, member) in members
                .iter()
                .enumerate()
                .skip(offset)
                .take(usize::from(viewport.height))
            {
                let row = super::member_row(
                    member,
                    squad.map(|s| s.room_id.as_str()),
                    squad.is_some_and(|s| s.lead.as_deref() == Some(member.id.as_str())),
                    observation,
                    squad.and_then(|s| s.member_states.get(&member.id)),
                );
                let y = viewport.y + (index - offset) as u16;
                let raw = snapshot.rows.get(&row.target);
                for (field, slot) in &slots {
                    let cell = Rect::new(slot.x, y, slot.width, 1).intersection(viewport);
                    if *field == RosterField::State
                        && let Some(label) = app.activity_label(&row)
                    {
                        columns::paint_activity(frame.buffer_mut(), app, &row, &label, cell, false);
                        continue;
                    }
                    let text = match field {
                        RosterField::Member => safe(&member.name),
                        RosterField::State => format!(
                            "{} {}",
                            columns::state_symbol(row.state.as_deref()),
                            safe(row.state.as_deref().unwrap_or("unreported"))
                        ),
                        RosterField::Task => {
                            let task = raw.and_then(|r| r["fields"]["task"].as_str()).unwrap_or("");
                            let next = raw.and_then(|r| r["fields"]["next"].as_str()).unwrap_or("");
                            safe(&match (task.is_empty(), next.is_empty()) {
                                (true, true) => "—".into(),
                                (false, true) => task.into(),
                                (true, false) => next.into(),
                                _ => format!("{task} → {next}"),
                            })
                        }
                    };
                    let style = if *field == RosterField::State {
                        app.workspace_state_cells.borrow_mut().push(cell);
                        columns::state_style(app, &row)
                    } else {
                        look.role(Role::Text)
                    };
                    let line = if *field == RosterField::State {
                        crate::board::state_cell::unread(app.unread_target(&row.target), look)
                    } else {
                        None
                    }
                    .unwrap_or_else(|| {
                        Line::styled(
                            crate::board::view::fit(&text, usize::from(cell.width)),
                            style,
                        )
                    });
                    let displayed = line.to_string();
                    strip::paint_left(frame.buffer_mut(), cell, line);
                    if *field == RosterField::Task {
                        crate::board::conversation::source::publish(
                            app,
                            crate::board::app::RowTarget::Workspace(row.target.clone()),
                            cell,
                        );
                        crate::board::member_links::paint_plain_hits(
                            frame, app, cell, &text, &displayed, false,
                        );
                    }
                }
                if interactive {
                    app.workspace_action_hits.borrow_mut().push((
                        Rect::new(viewport.x, y, viewport.width, 1),
                        target.clone(),
                        Choice::WorkspaceTarget(row.target),
                    ));
                }
            }
            if members.is_empty() && offset == 0 {
                strip::paint_left(
                    frame.buffer_mut(),
                    Rect {
                        height: u16::from(viewport.height > 0),
                        ..viewport
                    },
                    Line::styled(
                        if snapshot.roster.complete() {
                            "No members in this scope"
                        } else {
                            "Members unavailable · refresh before acting"
                        },
                        look.role(Role::Muted),
                    ),
                );
            }
            for (index, fact) in facts.iter().enumerate() {
                let line = member_rows + index;
                if line >= offset && line < offset + usize::from(viewport.height) {
                    if interactive && savings_lines.contains(&index) {
                        app.savings_hit(
                            viewport.x,
                            viewport.y + (line - offset) as u16,
                            fact.width(),
                            viewport.width,
                        );
                    }
                    strip::paint_left(
                        frame.buffer_mut(),
                        Rect::new(
                            viewport.x,
                            viewport.y + (line - offset) as u16,
                            viewport.width,
                            1,
                        ),
                        fact.clone(),
                    );
                }
            }
        },
    );
}
fn whole_hints(hints: &[(&str, &str)], width: u16) -> String {
    let hints: Vec<_> = hints
        .iter()
        .map(|(key, description)| KeyHint {
            key: (*key).into(),
            description: (*description).into(),
        })
        .collect();
    fit_hints(&hints, width, " · ")
}

pub(in crate::board) fn dialog(frame: &mut Frame, app: &App, body: Rect) {
    let Some(draft) = app.workspace_form.as_ref().filter(|draft| draft.shown) else {
        return;
    };
    draft.controls.set([None, None]);
    let look = app.look();
    let short = body.height < 8;
    let modal = Modal {
        title: draft.title(),
        placement: if short {
            Placement::Body
        } else {
            Placement::Center
        },
    };
    let bounds = if short { frame.area() } else { body };
    let probe = modal.areas(bounds, [80, 26], true, false);
    let text = match &draft.stage {
        Stage::Fields => app
            .input
            .as_ref()
            .map(|input| input.prompt.clone())
            .unwrap_or_default(),
        Stage::Review => draft.review(),
        Stage::Pending => format!(
            "Pending: checking and applying the frozen operation.\n{}\nNo automatic retry.",
            draft.review()
        ),
        Stage::Result(text) => text.clone(),
    };
    let rename_id = draft.operation.as_ref().and_then(|change| match change {
        super::manage::Change::RenameMember(request) => Some(format!("ID {}", request.identity_id)),
        _ => None,
    });
    let lines: Vec<_> = text
        .lines()
        .flat_map(|line| {
            let role = if rename_id.as_deref() == Some(line) {
                Role::Muted
            } else {
                Role::Text
            };
            crate::board::notes::wrap(&safe(line), usize::from(probe.content.width))
                .into_iter()
                .map(move |line| (line, role))
        })
        .collect();
    let editor_rows = if draft.stage == Stage::Fields {
        app.reference_editor().map_or(1, |editor| {
            editor.layout(probe.content.width).lines.len().clamp(1, 6)
        })
    } else {
        0
    };
    // Existing chrome reserves two border rows, a position and a footer row.
    let demand_height = lines
        .len()
        .max(1)
        .saturating_add(editor_rows)
        .saturating_add(4)
        .min(26) as u16;
    let areas = modal.areas(bounds, [80, demand_height], true, false);
    modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
    app.input_band.set(Some(areas.outer));
    if draft.stage == Stage::Fields {
        let prompt_height = (lines
            .len()
            .min(usize::from(areas.content.height.saturating_sub(1))))
            as u16;
        for (index, (line, role)) in lines.iter().take(usize::from(prompt_height)).enumerate() {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(
                    areas.content.x,
                    areas.content.y + index as u16,
                    areas.content.width,
                    1,
                ),
                Line::styled(line.clone(), look.role(*role)),
            );
        }
        let input = Rect::new(
            areas.content.x,
            areas.content.y + prompt_height,
            areas.content.width,
            areas.content.height.saturating_sub(prompt_height),
        );
        crate::board::view::waiting::editor(frame, app, input);
    } else {
        let viewport = areas.content;
        let mut scroll = draft.scroll.borrow_mut();
        scroll.update(viewport, lines.len(), None);
        for (index, (line, role)) in lines
            .iter()
            .skip(scroll.offset())
            .take(usize::from(viewport.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(viewport.x, viewport.y + index as u16, viewport.width, 1),
                Line::styled(line.clone(), look.role(*role)),
            );
        }
        if areas.position.height > 0 {
            strip::paint_left(
                frame.buffer_mut(),
                areas.position,
                Line::styled(
                    format!("Read · {}", scroll.position()),
                    if app
                        .overlay_focus
                        .focused()
                        .and_then(|field| field.last())
                        .is_some_and(|field| field == "read")
                    {
                        look.selection()
                    } else {
                        look.role(Role::Muted)
                    },
                ),
            );
        }
    }
    let focused = app
        .overlay_focus
        .focused()
        .and_then(|field| field.last())
        .map(String::as_str)
        .or(Some("cancel"));
    if draft.stage == Stage::Review {
        let apply_label = draft
            .operation
            .as_ref()
            .map(|operation| operation.confirm_label())
            .unwrap_or_default();
        let apply_enabled = !apply_label.is_empty()
            && areas.content.height > 1
            && (draft.kind != super::manage::Kind::RenameMember
                || (draft.scroll.borrow().offset() == 0
                    && lines.len() <= usize::from(areas.content.height)));
        let buttons = [
            crate::board::buttons::Button::secondary("Cancel")
                .focused(focused == Some("cancel"))
                .enabled(areas.content.height > 1),
            crate::board::buttons::Button::primary(apply_label)
                .focused(focused == Some("apply"))
                .enabled(apply_enabled),
        ];
        let painted = crate::board::buttons::paint(
            app,
            frame.buffer_mut(),
            areas.footer,
            &buttons,
            tmt_cli_style::grid::Align::Left,
            Some(crate::board::app::Overlay::Workspace),
        );
        draft.controls.set([
            painted[0].filter(|_| areas.content.height > 1),
            painted[1].filter(|_| apply_enabled),
        ]);
        let start = painted
            .iter()
            .flatten()
            .map(|area| area.right())
            .max()
            .unwrap_or(areas.footer.x)
            .saturating_add(1)
            .min(areas.footer.right());
        if start < areas.footer.right() {
            let hint = Rect::new(
                start,
                areas.footer.y,
                areas.footer.right() - start,
                areas.footer.height,
            );
            strip::paint_left(
                frame.buffer_mut(),
                hint,
                Line::styled(
                    whole_hints(
                        &[
                            ("Esc", "cancel"),
                            ("Enter", "select"),
                            ("Tab", "read/actions"),
                        ],
                        hint.width,
                    ),
                    look.role(Role::Muted),
                ),
            );
        }
    } else {
        let label = if draft.stage == Stage::Fields {
            "Cancel"
        } else {
            "Close"
        };
        let area = crate::board::buttons::paint(
            app,
            frame.buffer_mut(),
            areas.footer,
            &[crate::board::buttons::Button::secondary(label).focused(focused == Some("cancel"))],
            tmt_cli_style::grid::Align::Left,
            Some(crate::board::app::Overlay::Workspace),
        )[0];
        let width = area.map_or(0, |area| area.width + 1);
        draft.controls.set([area, None]);
        let hints: &[(&str, &str)] = if draft.stage == Stage::Fields {
            &[("Esc", "cancel"), ("Enter", "next")]
        } else {
            &[
                ("Esc", "close"),
                ("Read:", "arrows/PgUp/PgDn"),
                ("no", "automatic retry"),
            ]
        };
        let remaining = Rect {
            x: areas
                .footer
                .x
                .saturating_add(width)
                .min(areas.footer.right()),
            width: areas.footer.width.saturating_sub(width),
            ..areas.footer
        };
        strip::paint_left(
            frame.buffer_mut(),
            remaining,
            Line::styled(whole_hints(hints, remaining.width), look.role(Role::Muted)),
        );
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use crate::{
        board::{
            app::{HomeUsage, UsageShare},
            rate::Reading,
        },
        config::TokenWindow,
    };
    #[test]
    fn missing_zero_partial_and_supplied_share_keep_distinct_meanings() {
        let usage = HomeUsage {
            lead_model: Some("unfamiliar-model-123"),
            windows: TokenWindow::DEFAULTS,
            lead: [
                Some(Reading {
                    tokens: 0,
                    partial: false,
                    span: 60_000,
                }),
                None,
                Some(Reading {
                    tokens: 7_500,
                    partial: true,
                    span: 60_000,
                }),
            ],
            squad: [None; 3],
            share: Some(UsageShare {
                fraction: 0.375,
                partial: true,
            }),
        };
        let text = squad_usage(Some(&usage), None).join("\n");
        assert!(text.contains("Lead model: unfamiliar-model-123"));
        assert!(text.contains("1m 0 tok"));
        assert!(text.contains("5m not reported"));
        assert!(text.contains("1h ~7.5k tok"));
        assert!(
            text.contains("Lead share: ~38%"),
            "share is supplied independently of absent totals: {text}"
        );
        assert!(text.contains("Usage: 1m not reported"));
    }
    #[test]
    fn disabled_sampling_keeps_independent_model_without_inventing_usage() {
        assert_eq!(
            squad_usage(None, Some("off-model")),
            ["Lead model: off-model", "Usage: not reported"]
        );
        assert_eq!(squad_usage(None, None), ["Usage: not reported"]);
    }
    #[test]
    fn large_values_and_mixed_windows_cannot_hide_an_independent_observation() {
        let usage = HomeUsage {
            lead_model: None,
            windows: [
                TokenWindow::FIVE_MINUTES,
                TokenWindow::HOUR,
                TokenWindow::parse("24h").unwrap(),
            ],
            lead: [
                Some(Reading {
                    tokens: 3_700_000,
                    partial: true,
                    span: 60_000,
                }),
                Some(Reading {
                    tokens: u128::MAX,
                    partial: false,
                    span: 60_000,
                }),
                Some(Reading {
                    tokens: 12_400_000,
                    partial: true,
                    span: 60_000,
                }),
            ],
            squad: [None; 3],
            share: None,
        };
        let text = squad_usage(Some(&usage), None).join("\n");
        assert!(text.contains("5m ~3.7M tok"));
        let lead = text
            .lines()
            .find(|line| line.starts_with("Lead usage:"))
            .unwrap();
        assert!(
            lead.contains("1h ") && !lead.contains("1h not reported"),
            "{lead}"
        );
        assert!(text.contains("24h ~12.4M tok"));
        assert!(text.contains("Lead share: not reported"));
    }
}
