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

fn safe(text: &str) -> String {
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
        if tab == OpsTab::Overview
            && let Some(home) = app.view.as_ref().and_then(|view| view.home.as_ref())
        {
            if app.error.is_some() {
                label.push_str(" stale");
            } else if home.incomplete || !home.workspace.roster.membership_complete {
                label.push_str(" ?");
            } else if home.summary.waiting + home.summary.blocked > 0 {
                label.push_str(&format!(
                    " !{}",
                    home.summary.waiting + home.summary.blocked
                ));
            }
        }
        label.push_str("  ");
        let line = Line::from(label.clone());
        let width = (line
            .width()
            .min(usize::from(area.right().saturating_sub(x)))) as u16;
        if width > 0 && area.height > 0 {
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
        tree(frame, app, list);
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
    outline.paint_flat(area, frame.buffer_mut());
    let inside = outline.inner(area);
    let rows = app.workspace_rows();

    *app.row_starts.borrow_mut() = (0..rows.len()).collect();
    if app.follow
        && let Some(index) = rows
            .iter()
            .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
    {
        app.scrolls
            .reveal_range(Pane::Rows, index..index + 1, inside, rows.len());
    }
    app.scrolls.show_paint(
        frame,
        Pane::Rows,
        inside,
        rows.len(),
        look,
        |frame, viewport, offset| {
            for (line, tree) in rows
                .iter()
                .skip(offset)
                .take(usize::from(viewport.height))
                .enumerate()
            {
                let text = tree_line(app, tree, inside.width);
                let row = Rect::new(viewport.x, viewport.y + line as u16, viewport.width, 1);
                if Some(&tree.target) == app.workspace.selected.as_ref() {
                    frame.buffer_mut().set_style(row, look.selection());
                }
                strip::paint_left(frame.buffer_mut(), row, text.clone());
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
    if let Some(target @ OpsTarget::Squad(_)) = app
        .workspace
        .selected
        .as_ref()
        .filter(|_| app.workspace_selection_visible())
    {
        use super::manage::Kind;
        use crate::action::{Action, OpsUiAction};
        let actions = [
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
        ];
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
                if let Some(usage) = app.home_usage(&squad.name, std::time::Instant::now()) {
                    let pieces: Vec<_> = usage.squad.iter().zip(usage.windows).map(|(reading,window)| format!("{} {}", window.label(), reading.map_or_else(|| "not reported".into(), |reading| format!("{}{}", if reading.partial { "~" } else { "" }, crate::source::tokens(reading.tokens as f64))))).collect();
                    text.push(format!("Usage: {}", pieces.join(" · ")));
                } else { text.push("Usage: not reported".into()); }
                let jobs=app.cron.cron.as_ref().filter(|_| app.cron.failure.is_none()).map(|cron|cron.jobs.iter().filter(|job|job.job.room_id==*id).count());
                text.push(format!("Timers: {}",jobs.map_or("not reported".into(),|count|count.to_string())));

            },
            Some(OpsTarget::Pool)=>{text.push("Agents without a squad".into());text.push("Expand the pool to select an agent; add it to an existing squad or create one.".into());},
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
    let Some(draft) = app.workspace_form.as_ref() else {
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
    let lines: Vec<_> = text
        .lines()
        .flat_map(|line| crate::board::notes::wrap(&safe(line), usize::from(areas.content.width)))
        .collect();
    if draft.stage == Stage::Fields {
        let prompt_height = (lines
            .len()
            .min(usize::from(areas.content.height.saturating_sub(1))))
            as u16;
        for (index, line) in lines.iter().take(usize::from(prompt_height)).enumerate() {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(
                    areas.content.x,
                    areas.content.y + index as u16,
                    areas.content.width,
                    1,
                ),
                Line::styled(line.clone(), look.role(Role::Text)),
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
        for (index, line) in lines
            .iter()
            .skip(scroll.offset())
            .take(usize::from(viewport.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(viewport.x, viewport.y + index as u16, viewport.width, 1),
                Line::styled(line.clone(), look.role(Role::Text)),
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
