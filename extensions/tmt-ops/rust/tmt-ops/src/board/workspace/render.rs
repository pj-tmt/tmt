//! Dense tree and squad detail over the admitted snapshot. Shared placement,
//! Strip/Outline, Scrolls and the WP1 conversation retain geometry and paint.
use super::{OpsObservation, OpsTarget, SelectionState, manage::Stage};
use crate::{
    board::{
        app::{App, Choice, Hit, HitTarget, TabHit},
        shell::OpsTab,
    },
    config::{OpsLayout, Pane},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::{Modal, Outline, Placement, strip};

pub(super) fn safe(text: &str) -> String {
    tmt_cli_style::table::escape(&crate::board::notes::sanitize(text))
}
pub(in crate::board) fn tabs(app: &App, area: Rect) -> Line<'static> {
    let look = app.look();
    let mut spans = vec![Span::styled("OPS / UI  ", look.role(Role::Muted))];
    let mut x = area.x.saturating_add(10).min(area.right());
    for (index, tab) in [OpsTab::Overview, OpsTab::Agents, OpsTab::Digest]
        .into_iter()
        .enumerate()
    {
        let mut label = tab.label().to_owned();
        label.push_str("  ");
        let line = Line::from(label.clone());
        let width = (line
            .width()
            .min(usize::from(area.right().saturating_sub(x)))) as u16;
        if width > 0 && usize::from(width) == line.width() && area.height > 0 {
            app.tab_hits.borrow_mut().push(TabHit {
                x,
                y: area.y,
                width,
                tab: index,
            });
        }
        x = x.saturating_add(width);
        let style = if app.shell.tab == tab {
            look.role(Role::Accent)
                .patch(look.selection())
                .add_modifier(Modifier::BOLD)
        } else {
            look.role(Role::Muted)
        };
        spans.push(Span::styled(label, style));
    }
    Line::from(spans)
}
pub(in crate::board) fn summary(app: &App, width: u16) -> Line<'static> {
    let Some(snapshot) = app.workspace_snapshot() else {
        return Line::default();
    };
    if app.loading() {
        return Line::styled("Loading workspace…", app.look().role(Role::Muted));
    }
    if app.shell.tab == OpsTab::Digest {
        return Line::styled(
            crate::board::digest_dashboard::summary(&app.labels),
            app.look().role(Role::Text),
        );
    }
    if !snapshot.notices.is_empty() {
        return Line::styled(
            "Configuration needs migration · ? help shows located settings; file preserved",
            app.look().role(Role::Waiting),
        );
    }
    if app.shell.tab == OpsTab::Overview {
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
            return crate::board::home::summary_of(view, width, app.look());
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
fn observation(observation: OpsObservation) -> &'static str {
    match observation {
        OpsObservation::Complete => "",
        OpsObservation::Stale => "stale",
        OpsObservation::Partial => "partial",
        OpsObservation::Unavailable => "unknown",
    }
}
pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    use crate::board::conversation::layout::{self, Preference};
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let placement = if app.shell.tab == OpsTab::Overview {
        layout::overview(area)
    } else {
        layout::agents(
            area,
            match snapshot.preferences.layout {
                OpsLayout::Auto => Preference::Auto,
                OpsLayout::Side => Preference::Side,
                OpsLayout::Stack => Preference::Stack,
            },
            app.focused() == Pane::Replies,
        )
    };
    if let Some(list) = placement.list {
        if app.shell.tab == OpsTab::Overview {
            crate::board::overview::view::render(frame, app, list);
        } else {
            tree(frame, app, list);
        }
    }
    app.conversation.borrow_mut().pane = placement.conversation;
    if let Some(detail) = placement.conversation {
        if app.workspace_selected_member().is_some() {
            crate::board::conversation::render(frame, app, detail);
        } else {
            squad_detail(frame, app, detail);
        }
    }
}
fn tree(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let title = match app.shell.tab {
        OpsTab::Overview => " Overview ",
        OpsTab::Agents => " Agents ",
        OpsTab::Digest => " Digest ",
    };
    let outline = Outline {
        title: Line::from(title),
        border: look.role(Role::Dim),
        title_style: look.role(Role::Muted),
    };
    // Decoration cannot consume the last available content rows. The shell
    // already names the destination; short terminals keep target navigation.
    let inside = if area.height >= 3 && area.width >= 3 {
        outline.paint_flat(area, frame.buffer_mut());
        outline.inner(area)
    } else {
        area
    };
    let rows = app.workspace_rows();
    let no_matches =
        !app.workspace.query.is_empty() && rows.iter().all(|row| row.target == OpsTarget::Pool);
    let content = rows.len() + usize::from(no_matches);

    *app.row_starts.borrow_mut() = (0..rows.len()).collect();
    if app.follow
        && let Some(index) = rows
            .iter()
            .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
    {
        app.scrolls
            .reveal_range(Pane::Rows, index..index + 1, inside, content);
    }
    app.scrolls.show_paint(
        frame,
        Pane::Rows,
        inside,
        content,
        look,
        |frame, viewport, offset| {
            for (line, tree) in rows
                .iter()
                .skip(offset)
                .take(usize::from(viewport.height))
                .enumerate()
            {
                let text = tree_line(app, tree, inside.width);
                let target = crate::board::app::RowTarget::Workspace(tree.target.clone());
                if let Some(raw) = app.target_row(&target)
                    && crate::board::row_chips::clock(&app.labels, raw, crate::status::now_ms())
                        .is_some()
                {
                    // The dynamic chips are the final spans after the fixed tree text.
                    let fixed = text.spans.first().map_or(0, Span::width);
                    let budget = usize::from(inside.width).saturating_sub(fixed + 1);
                    if text.spans.len() > 1 {
                        app.frame_clocks.borrow_mut().push(
                            crate::board::view::ClockSurface::Labels {
                                target,
                                width: budget,
                            },
                        );
                    }
                }
                let row = Rect::new(viewport.x, viewport.y + line as u16, viewport.width, 1);
                if Some(&tree.target) == app.workspace.selected.as_ref() {
                    frame.buffer_mut().set_style(row, look.selection());
                }
                strip::paint_left(frame.buffer_mut(), row, text.clone());
                // Only a complete visible count publishes a UUID-frozen action.
                let scope = match &tree.target {
                    OpsTarget::Squad(room) => Some((
                        Some(room.clone()),
                        None,
                        crate::board::overview::cron_counts::Scope::Room(room),
                    )),
                    OpsTarget::Member {
                        room_id,
                        identity_id,
                    } => Some((
                        room_id.clone(),
                        Some(identity_id.clone()),
                        crate::board::overview::cron_counts::Scope::Owner {
                            id: identity_id,
                            room: room_id.as_deref(),
                        },
                    )),
                    OpsTarget::Pool => None,
                };
                if let Some((room, owner, scope)) = scope {
                    let count = app.cron_glyphs.label(app.cron.counts.count(scope));
                    if let Some(fixed) = text.spans.first()
                        && fixed.content.ends_with(&count)
                        && fixed.width() <= usize::from(row.width)
                    {
                        app.cron_scope_hits
                            .borrow_mut()
                            .push(crate::board::app::TimerHit {
                                area: Rect::new(
                                    row.x
                                        + (fixed.width()
                                            - unicode_width::UnicodeWidthStr::width(count.as_str()))
                                            as u16,
                                    row.y,
                                    unicode_width::UnicodeWidthStr::width(count.as_str()) as u16,
                                    1,
                                ),
                                target: tree.target.clone(),
                                room,
                                owner,
                            });
                    }
                }
                let mut x = row.x;
                for span in &text.spans {
                    let width = span.width().min(usize::from(u16::MAX)) as u16;
                    if crate::board::row_chips::offers_choice(&span.content)
                        && x.saturating_add(width) <= row.right()
                    {
                        app.chip_hits
                            .borrow_mut()
                            .push((Rect::new(x, row.y, width, 1), offset + line));
                    }
                    x = x.saturating_add(width);
                }
                app.hits.borrow_mut().push(Hit {
                    x: row.x,
                    y: row.y,
                    width: row.width,
                    target: HitTarget::Row(offset + line),
                });
            }
            // Common pool remains a navigable virtual target while the filter
            // has no observed matches. The message has no target or click hit.
            if no_matches
                && rows.len() >= offset
                && rows.len() - offset < usize::from(viewport.height)
            {
                strip::paint_left(
                    frame.buffer_mut(),
                    Rect::new(
                        viewport.x,
                        viewport.y + (rows.len() - offset) as u16,
                        viewport.width,
                        1,
                    ),
                    Line::styled("No matches in this view", look.role(Role::Muted)),
                );
            }
        },
    );
}
/// Format only rows admitted to the scroll viewport; hidden rows need no spans.
fn tree_line(app: &App, row: &super::OpsTreeRow, width: u16) -> Line<'static> {
    let look = app.look();
    let selected = app.workspace.selected.as_ref() == Some(&row.target);
    let cursor = if selected { "› " } else { "  " };
    let mut text = match &row.target {
        OpsTarget::Pool | OpsTarget::Squad(_) => {
            let expanded = match &row.target {
                OpsTarget::Pool => app.workspace.pool_expanded,
                OpsTarget::Squad(id) => app.workspace.expanded.contains(id),
                _ => false,
            };
            let count = row.count.map_or("?".into(), |count| count.to_string());
            format!(
                "{cursor}{} {} · {count}",
                if expanded { "▾" } else { "▸" },
                safe(&row.title)
            )
        }
        OpsTarget::Member { .. } => format!(
            "{cursor}  {}{} · {}",
            safe(&row.title),
            if row.lead { " · lead" } else { "" },
            safe(row.state.as_deref().unwrap_or("unreported"))
        ),
    };
    if let Some(presence) = row.presence {
        text.push_str(if presence {
            " · active"
        } else {
            " · offline"
        });
    }
    if let Some(state) = row.state.as_ref().filter(|_| row.depth == 0) {
        text.push_str(&format!(" · {}", safe(state)));
    }
    let notice = observation(row.observation);
    if !notice.is_empty() {
        text.push_str(&format!(" · {notice}"));
    }
    let role = match row.state.as_deref() {
        Some("blocked") => Role::Blocked,
        Some("waiting") => Role::Waiting,
        Some("working") => Role::Working,
        _ => Role::Text,
    };
    let state_style = app
        .workspace_snapshot()
        .and_then(|snapshot| snapshot.rows.get(&row.target))
        .and_then(|raw| raw["colors"]["state"].as_str())
        .filter(|_| row.state.as_deref() != Some("waiting"))
        .map_or_else(|| look.role(role), |color| look.named(color));
    let style = if selected {
        look.selection()
            .patch(look.row_span(true, state_style, true))
    } else {
        state_style
    };
    let count_scope = match &row.target {
        OpsTarget::Squad(room) => Some(crate::board::overview::cron_counts::Scope::Room(room)),
        OpsTarget::Member {
            room_id,
            identity_id,
        } => Some(crate::board::overview::cron_counts::Scope::Owner {
            id: identity_id,
            room: room_id.as_deref(),
        }),
        OpsTarget::Pool => None,
    };
    if let Some(scope) = count_scope {
        text.push_str(" · ");
        text.push_str(&app.cron_glyphs.label(app.cron.counts.count(scope)));
    }
    let mut line = Line::styled(text, style);
    if let Some(raw) = app
        .workspace_snapshot()
        .and_then(|snapshot| snapshot.rows.get(&row.target))
        .filter(|_| matches!(row.target, OpsTarget::Member { .. }))
    {
        let chips = crate::board::row_chips::of(&app.labels, raw, crate::status::now_ms());
        let count = crate::board::row_chips::prefix(
            &chips,
            usize::from(width).saturating_sub(line.width() + 1),
        );
        if count > 0 {
            line.spans.push(Span::styled(" ", style));
        }
        for chip in chips.into_iter().take(count) {
            line.spans.push(Span::styled(
                chip.text,
                if selected {
                    look.selection()
                        .patch(look.row_span(true, look.role(chip.role), true))
                } else {
                    look.role(chip.role)
                },
            ));
        }
    }
    line
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
                    crate::source::tokens(value.tokens as f64)
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

fn squad_detail(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let title = app
        .selected_row()
        .and_then(|row| row["name"].as_str())
        .unwrap_or("Selection");
    let outline = Outline {
        title: Line::from(safe(title)),
        border: look.role(Role::Dim),
        title_style: look.role(Role::Muted),
    };
    outline.paint_flat(area, frame.buffer_mut());
    let mut inside = outline.inner(area);
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
        };
        if matches!(target, OpsTarget::Pool)
            && let Some(action) = app
                .workspace_bindings()
                .values()
                .find(|a| matches!(a.verb, crate::action::Verb::Ui(OpsUiAction::DiscoverAgents)))
                .cloned()
        {
            actions.push(("Add an agent already running", Choice::Action(action)));
        }
        let mut x = inside.x;
        let mut y = inside.y;
        for (label, choice) in actions {
            let label = format!(" {label} ");
            let width = label.len() as u16;
            if x.saturating_add(width) > inside.right() {
                x = inside.x;
                y = y.saturating_add(1);
            }
            if y < inside.bottom() && width <= inside.width {
                let control = Rect::new(x, y, width, 1);
                strip::paint_left(
                    frame.buffer_mut(),
                    control,
                    Line::styled(label, look.role(Role::Text)),
                );
                app.workspace_action_hits
                    .borrow_mut()
                    .push((control, target.clone(), choice));
                x += width;
            }
        }
        let used = y
            .saturating_sub(inside.y)
            .saturating_add(1)
            .min(inside.height);
        inside.y += used;
        inside.height -= used;
    }
    let mut text = Vec::new();
    match app.workspace_selection_state() {
        SelectionState::Hidden=>text.push("Selected target is hidden by the filter or collapse. Select a visible target explicitly.".into()),
        SelectionState::Missing=>text.push("Selected target no longer exists. Select a new target explicitly.".into()),
        SelectionState::Unknown=>text.push("Selected target could not be observed. Refresh before acting.".into()),
        _=>match app.workspace.selected.as_ref() {
            Some(OpsTarget::Squad(id))=>if let Some(squad)=snapshot.roster.squads.iter().find(|squad|squad.room_id==*id) {
                let lead=squad.lead.as_ref().and_then(|id|snapshot.roster.members.get(id)).map_or("not set",|member|member.name.as_str());
                text.push(format!("Lead: {}",safe(lead)));
                text.push(format!("Members: {}",if squad.observation==OpsObservation::Complete {squad.members.len().to_string()}else{"unknown (partial roster)".into()}));
                let states=&squad.member_states;
                for state in ["working","blocked","review","idle"] {
                    let count = states.values().filter(|member|member.state.as_deref()==Some(state)).count();
                    text.push(if squad.observation == OpsObservation::Complete && states.len() == squad.members.len() { format!("{state}: {count}") } else { format!("{state}: {count} observed; total unknown") });
                }
                let usage = app.home_usage(&squad.name, std::time::Instant::now());
                text.extend(squad_usage(usage.as_ref(), app.home_lead_model(&squad.name)));
                let jobs=app.cron.cron.as_ref().filter(|_| app.cron.failure.is_none()).map(|cron|cron.jobs.iter().filter(|job|job.job.room_id==*id).count());
                text.push(format!("Timers: {}",jobs.map_or("not reported".into(),|count|count.to_string())));

            },
            Some(OpsTarget::Pool)=>{
                if snapshot.roster.members.is_empty() {
                    text.push("No agents are listed yet.".into());
                    text.push("Start an agent in a tmux pane, then choose Add an agent already running from the Agents list. Scan, review the pane and confirm its name or existing identity.".into());
                } else {
                    text.push("Agents without a squad".into());
                    text.push("Expand the pool to select an agent; add it to an existing squad or create one.".into());
                }
            },
            _=>{},
        },
    }
    let lines: Vec<_> = text
        .iter()
        .flat_map(|text| crate::board::notes::wrap(text, usize::from(inside.width)))
        .map(|text| Line::styled(text, look.role(Role::Text)))
        .collect();
    app.scrolls.show(frame, Pane::Replies, inside, lines, look);
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
    let areas = modal.areas(
        if short { frame.area() } else { body },
        [80, 26],
        true,
        false,
    );
    modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
    app.input_band.set(Some(areas.outer));
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
            crate::board::notes::wrap(&safe(line), usize::from(areas.content.width))
                .into_iter()
                .map(move |line| (line, role))
        })
        .collect();
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
        let cancel = " Cancel ";
        let apply = " Apply this exact operation ";
        let mut controls = [None, None];
        for (index, (label, field, x)) in [(cancel, "cancel", 0), (apply, "apply", 9)]
            .into_iter()
            .enumerate()
        {
            let width = label.len() as u16;
            if areas.footer.height > 0
                && areas.footer.width >= x + width
                && areas.content.height > 1
                && (index == 0
                    || draft.kind != super::manage::Kind::RenameMember
                    || (draft.scroll.borrow().offset() == 0
                        && lines.len() <= usize::from(areas.content.height)))
            {
                let area = Rect::new(areas.footer.x + x, areas.footer.y, width, 1);
                let style = if focused == Some(field) {
                    look.selection()
                } else {
                    look.role(Role::Text)
                };
                strip::paint_left(frame.buffer_mut(), area, Line::styled(label, style));
                controls[index] = Some(area);
            }
        }
        draft.controls.set(controls);
        if areas.footer.width > 37 {
            let hint = Rect::new(
                areas.footer.x + 37,
                areas.footer.y,
                areas.footer.width - 37,
                areas.footer.height,
            );
            strip::paint_left(
                frame.buffer_mut(),
                hint,
                Line::styled(
                    "Tab read/actions · Enter select · Esc cancel",
                    look.role(Role::Muted),
                ),
            );
        }
    } else {
        let label = if draft.stage == Stage::Fields {
            " Cancel "
        } else {
            " Close "
        };
        let width = label.len() as u16;
        if areas.footer.height > 0 && areas.footer.width >= width {
            let area = Rect::new(areas.footer.x, areas.footer.y, width, 1);
            strip::paint_left(
                frame.buffer_mut(),
                area,
                Line::styled(label, look.role(Role::Text)),
            );
            draft.controls.set([Some(area), None]);
        }
        let hint = if draft.stage == Stage::Fields {
            "Enter next · Esc cancel"
        } else {
            "Read: arrows/PgUp/PgDn · Esc close · no automatic retry"
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
            Line::styled(hint, look.role(Role::Muted)),
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
        assert!(text.contains("1h ~8k tok"));
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
