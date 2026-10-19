//! Squad board, member usage and room-scoped timer observations.
use super::*;

pub(super) fn render(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    y: &mut u16,
    squad: Option<&OpsSquad>,
    lead: Option<&serde_json::Value>,
) {
    if area.is_empty() {
        return;
    }
    let look = app.look();
    let Some(squad) = squad else {
        line(
            frame,
            area,
            y,
            neutral(" Squad observation unavailable.", look),
        );
        return;
    };
    line(
        frame,
        area,
        y,
        identity_line::squad(
            lead.and_then(|row| row["name"].as_str()),
            squad.lead.as_ref().map_or_else(
                || neutral("—", look),
                |id| {
                    member_state(
                        app,
                        &OpsTarget::Member {
                            room_id: Some(squad.room_id.clone()),
                            identity_id: id.clone(),
                        },
                        usize::from(area.width.saturating_sub(20)),
                    )
                },
            ),
            &squad.name,
            look,
        ),
    );
    blank(area, y);
    rule(frame, app, area, y, "Board", Line::default());
    for key in ["task", "pending", "next"] {
        let value = lead
            .and_then(|row| row["fields"][key].as_str())
            .filter(|value| !value.is_empty());
        if *y < area.bottom() {
            let value_area = key_value::paint_value(
                frame.buffer_mut(),
                Rect::new(area.x, *y, area.width, 1),
                &key_value::Row {
                    key,
                    value: value.map_or(
                        key_value::Value::Unreported("—"),
                        key_value::Value::Observed,
                    ),
                },
                look,
            );
            if let Some(value_area) = value_area {
                if let Some(value) = value {
                    let value = crate::board::notes::sanitize(value);
                    crate::board::member_links::paint_plain_hits(
                        frame, app, value_area, &value, &value, false,
                    );
                }
                if let Some(target) = app.conversation_target() {
                    crate::board::conversation::source::publish(app, target, value_area);
                }
            }
            *y += 1;
        }
    }
    blank(area, y);
    if area.bottom().saturating_sub(*y) < 2 {
        return;
    }
    let total = app.overview_squad_usage(squad);
    let mut note = format!(
        "last 60m · {} tok",
        total.map_or_else(
            || "—".into(),
            |(tokens, partial)| format!(
                "{}{}",
                if partial { "~" } else { "" },
                crate::source::tokens(tokens)
            )
        )
    );
    let savings = app.digest_savings(squad.members.iter().map(String::as_str));
    let budget = usize::from(area.width).saturating_sub(
        unicode_width::UnicodeWidthStr::width("── Usage ")
            + 5
            + unicode_width::UnicodeWidthStr::width(note.as_str())
            + 3,
    );
    let estimate = app.compact_savings_caption(&savings, budget);
    if let Some(estimate) = &estimate {
        note.push_str(" · ");
        note.push_str(estimate);
    }
    let usage_y = *y;
    rule(frame, app, area, y, "Usage", neutral(note, look));
    if let Some(estimate) = estimate
        && *y == usage_y + 1
        && app.workspace_selection_visible()
    {
        let width = unicode_width::UnicodeWidthStr::width(estimate.as_str()) as u16;
        app.savings_hit(area.right() - 3 - width, usage_y, usize::from(width), width);
    }
    let mut members: Vec<_> = squad
        .members
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    // Positive observations first; unavailable evidence stays distinct from
    // the observed zero rows at the bottom of the approved usage list.
    members.sort_by_cached_key(|id| {
        let tokens = app.overview_member_usage(id).map(|reading| reading.tokens);
        match tokens {
            Some(n) if n > 0 => (0, std::cmp::Reverse(n)),
            None => (1, std::cmp::Reverse(0)),
            Some(_) => (2, std::cmp::Reverse(0)),
        }
    });
    let data = spark_row::Data {
        rows: members
            .iter()
            .take(usize::from(area.bottom().saturating_sub(*y + 1)))
            .map(|id| spark_row::Row {
                name: format!(
                    " {}",
                    app.workspace_snapshot()
                        .and_then(|snapshot| snapshot.roster.members.get(*id))
                        .map_or("—", |member| member.name.as_str())
                ),
                values: app
                    .overview_member_trend::<12>(id)
                    .map(|value| value.map(|n| n as u64))
                    .to_vec(),
                total: token_label(app.overview_member_usage(id)),
                share: share(app.overview_member_usage(id), total),
                muted: false,
                quota: false,
            })
            .collect(),
        name_width: 12,
        share_caption: "% of squad".into(),
        show_axis: false,
        axis: Default::default(),
        buckets: 12,
        no_data: false,
    };
    let usage_width = area.width.saturating_sub(1);
    if let Ok(scene) = spark_row::Scene::new(&data, usage_width, 0) {
        let rows = scene.rows().min(area.bottom().saturating_sub(*y));
        if rows > 0 {
            let rect = Rect::new(area.x, *y, usage_width, rows);
            let mut buffer = Buffer::empty(rect);
            scene
                .paint(&mut buffer, &look.theme, look.depth, &[], None)
                .expect("admitted member spark rows");
            frame.buffer_mut().merge(&buffer);
            for (index, id) in members
                .iter()
                .enumerate()
                .take(usize::from(rows.saturating_sub(1)))
            {
                let target = OpsTarget::Member {
                    room_id: Some(squad.room_id.clone()),
                    identity_id: (*id).clone(),
                };
                if app.workspace_selection_visible()
                    && app
                        .workspace_snapshot()
                        .is_some_and(|snapshot| snapshot.roster.contains(&target))
                {
                    app.workspace_action_hits.borrow_mut().push((
                        Rect::new(area.x, *y + 1 + index as u16, area.width, 1),
                        OpsTarget::Squad(squad.room_id.clone()),
                        super::super::app::Choice::WorkspaceTarget(target),
                    ));
                }
            }
            *y += rows;
        }
    }
    blank(area, y);
    let model = timers::project_room(app, &squad.room_id);
    let note = if model.total == 0 && app.cron.cron.is_some() && app.cron.failure.is_none() {
        String::new()
    } else {
        model.note
    };
    rule(frame, app, area, y, "Timers", neutral(note, look));
    if model.rows.is_empty() {
        line(
            frame,
            area,
            y,
            neutral(
                if app.cron.cron.is_none() {
                    " Timers unavailable.".into()
                } else {
                    let mut empty = format!(" No timers in {} yet.", squad.name);
                    if let Some((key, _)) =
                        app.workspace_bindings().into_iter().find(|(_, action)| {
                            action.verb
                                == crate::action::Verb::Ui(crate::action::OpsUiAction::Timers)
                        })
                    {
                        let instruction = format!(
                            " {} timers adds one.",
                            super::super::view::footer::key_label(&key)
                        );
                        if unicode_width::UnicodeWidthStr::width(empty.as_str())
                            + unicode_width::UnicodeWidthStr::width(instruction.as_str())
                            <= usize::from(area.width)
                        {
                            empty.push_str(&instruction);
                        }
                    }
                    empty
                },
                look,
            ),
        );
    } else {
        for row in &model.rows {
            line(
                frame,
                area,
                y,
                timer_rows::line(row, usize::from(area.width), look),
            );
        }
        if model.total > model.rows.len() {
            line(
                frame,
                area,
                y,
                neutral(format!(" +{} more", model.total - model.rows.len()), look),
            );
        }
    }
    blank(area, y);
}
