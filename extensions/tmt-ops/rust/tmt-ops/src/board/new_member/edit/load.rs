//! Changed-only Save executes on the existing New member action lane.
use super::model::{Plan, Target};
use crate::{
    config::Config,
    core::{Core, SquadError},
    membership::workspace::{Operation, RoomTarget},
};
fn conflict(message: &str) -> SquadError {
    SquadError::new("SQUAD_EDIT_CONFLICT", message)
}
fn room_current(core: &Core, room: &RoomTarget) -> Result<(), SquadError> {
    room.admit(core).map(|_| ())
}
fn identity_current(core: &Core, target: &Target, expected_name: &str) -> Result<(), SquadError> {
    let Target::Member {
        id, observation, ..
    } = target
    else {
        return Ok(());
    };
    let listed = core.json(&["ls"])?;
    let row = listed["identities"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["id"] == *id))
        .ok_or_else(|| conflict("The selected identity disappeared; no successor was selected."))?;
    if row["name"] != expected_name {
        return Err(conflict(
            "The selected identity was renamed; review it again.",
        ));
    }
    let source = crate::board::switch_tool::Source::frozen(
        target.selected(),
        expected_name.into(),
        observation,
    );
    if !source.matches(row) {
        return Err(conflict(
            "The selected agent's runtime or session changed; review it again.",
        ));
    }
    Ok(())
}
fn preset_current(core: &Core, plan: &Plan) -> Result<(), SquadError> {
    if let Some(expected) = &plan.preset {
        let Target::Member { id, .. } = &plan.target else {
            unreachable!()
        };
        let current = super::model::Preset::read(core, id).map_err(|message| conflict(&message))?;
        if &current != expected {
            return Err(conflict(
                "Recorded launch settings changed; reopen before saving.",
            ));
        }
    }
    Ok(())
}
fn preflight(core: &Core, plan: &Plan, config: &Config) -> Result<(), SquadError> {
    config.ensure_current()?;
    if let Some((expected, _)) = &plan.preferences
        && config.workspace_preferences()? != *expected
    {
        return Err(conflict(
            "Workspace preferences changed; reopen before saving.",
        ));
    }
    identity_current(core, &plan.target, plan.target.name())?;
    match &plan.target {
        Target::Squad(room) => room_current(core, room)?,
        Target::Member { source, .. } => {
            if let Some(room) = source {
                room_current(core, room)?;
            }
        }
    }
    if let Some(Operation::Assign { destination, .. }) = &plan.membership {
        room_current(core, destination)?;
    }
    preset_current(core, plan)?;
    if let Some(runtime) = &plan.runtime {
        let outer = runtime.outer.as_ref().ok_or_else(|| {
            conflict("Copy only: this host cannot run the requested launch here. Nothing changed.")
        })?;
        crate::board::host_launch::recheck(
            core,
            runtime.source.host(),
            runtime.source.session(),
            &runtime.command,
            outer,
        )?;
    }
    Ok(())
}
fn membership_details(document: &serde_json::Value) -> String {
    let mut lines = crate::board::app::workspace::operation_result_lines(document);
    for value in [
        document,
        &document["sourceRemoval"],
        &document["destinationResult"],
    ] {
        if let Some(message) = value["failed"]["error"]["message"].as_str() {
            lines.push(message.into());
        }
        if let Some(repair) = value["repair"].as_str() {
            lines.push(format!("Repair explicitly: {repair}"));
        }
    }
    if lines.is_empty() {
        lines.push("The membership change is not fully confirmed; inspect the refreshed squads before trying again.".into());
    }
    lines.join("\n")
}
fn timer_result(lines: &mut Vec<String>, document: &serde_json::Value) {
    if let Some(paused) = document["pausedTimers"].as_array()
        && !paused.is_empty()
    {
        lines.push(format!(
            "✓ Applied · {} source timers paused; they remain in the source squad.",
            paused.len()
        ));
    }
}
fn failed(lines: &mut Vec<String>, step: &str, error: SquadError) {
    let refused = matches!(
        error.code.as_str(),
        "SQUAD_EDIT_CONFLICT"
            | "ROOM_REVISION_CONFLICT"
            | "SQUAD_CONFIG_CHANGED"
            | "INVALID_NAME"
            | "NAME_ALREADY_ACTIVE"
            | "ROOM_NOT_FOUND"
            | "NAME_NOT_FOUND"
            | "API_INPUT_INVALID"
    );
    lines.push(format!(
        "{} {} · {step}: {}",
        if refused { "✗" } else { "?" },
        if refused { "Not applied" } else { "Unknown" },
        error.message
    ));
    lines.push("— Not run · Remaining steps. Close and refresh before reviewing again; nothing is retried automatically.".into());
}
pub(in crate::board::new_member) fn apply(
    core: &Core,
    plan: &Plan,
    mut config: Config,
) -> Result<(String, Option<crate::config::OpsPreferences>), SquadError> {
    if plan.empty() {
        return Ok(("— Not applied · Nothing changed.".into(), None));
    }
    preflight(core, plan, &config)?;
    let mut lines = Vec::new();
    let mut name = plan.target.name().to_owned();
    let mut room = match &plan.target {
        Target::Squad(room) => Some(room.clone()),
        _ => None,
    };
    if let Some(next) = &plan.rename {
        let result = match &plan.target {
            Target::Member { id, .. } => {
                use crate::board::workspace::rename_member::{Outcome, Request};
                Request {
                    identity_id: id.clone(),
                    expected_name: name.clone(),
                    name: next.clone(),
                }
                .apply(core)
                .and_then(|outcome| match outcome {
                    Outcome::Applied { name: actual, .. } if actual == *next => Ok(()),
                    Outcome::Applied { .. } => Err(SquadError::new(
                        "SQUAD_EDIT_UNKNOWN",
                        "Core returned a different name; outcome unknown.",
                    )),
                    Outcome::Conflict { .. } | Outcome::NotFound => Err(conflict(
                        "The selected identity changed; rename was not applied.",
                    )),
                })
            }
            Target::Squad(old) => crate::membership::workspace::apply(
                core,
                &mut config,
                Operation::RenameSquad {
                    room: old.clone(),
                    name: next.clone(),
                },
            )
            .and_then(|outcome| {
                if outcome.complete {
                    Ok(())
                } else {
                    Err(SquadError::new(
                        "SQUAD_EDIT_PARTIAL",
                        membership_details(&outcome.document),
                    ))
                }
            }),
        };
        if let Err(error) = result {
            failed(&mut lines, "Name", error);
            return Ok((lines.join("\n"), None));
        }
        name = next.clone();
        lines.push(format!("✓ Applied · Name: {name}"));
        if let Some(room) = &mut room {
            let shown = match core.json(&["room", "show", &room.id]) {
                Ok(v) => v,
                Err(e) => {
                    failed(&mut lines, "Rename readback", e);
                    return Ok((lines.join("\n"), None));
                }
            };
            if shown["room"]["id"] != room.id
                || shown["room"]["name"] != crate::squad::room_name(&name)
                || shown["room"]["retired"] != false
            {
                failed(
                    &mut lines,
                    "Rename readback",
                    conflict("The renamed room could not be confirmed."),
                );
                return Ok((lines.join("\n"), None));
            }
            let Some(revision) = shown["room"]["revision"].as_u64() else {
                failed(
                    &mut lines,
                    "Rename readback",
                    conflict("The room revision is unavailable."),
                );
                return Ok((lines.join("\n"), None));
            };
            room.name = name.clone();
            room.revision = revision;
        }
    }
    if let Some(operation) = &plan.membership {
        if let Err(error) = identity_current(core, &plan.target, &name) {
            failed(&mut lines, "Squad", error);
            return Ok((lines.join("\n"), None));
        }
        match crate::membership::workspace::apply(core, &mut config, operation.clone()) {
            Ok(outcome) if outcome.complete => {
                lines.push("✓ Applied · Selected squad membership".into());
                timer_result(&mut lines, &outcome.document);
            }
            Ok(outcome) => {
                timer_result(&mut lines, &outcome.document);
                failed(
                    &mut lines,
                    "Squad",
                    SquadError::new("SQUAD_EDIT_PARTIAL", membership_details(&outcome.document)),
                );
                return Ok((lines.join("\n"), None));
            }
            Err(error) => {
                failed(&mut lines, "Squad", error);
                return Ok((lines.join("\n"), None));
            }
        }
    }
    if let Some((enabled, after)) = &plan.alert {
        let Some(room) = &room else { unreachable!() };
        if let Err(error) = room_current(core, room) {
            failed(&mut lines, "Idle alert", error);
            return Ok((lines.join("\n"), None));
        }
        let completed = crate::board::idle_alert_editor::complete(
            core,
            crate::board::idle_alert_editor::Task {
                owner: 0,
                room: room.id.clone(),
                name: name.clone(),
                save: Some((Box::new(config.clone()), *enabled, after.clone())),
            },
        );
        match completed.result {
            Ok(next) => {
                config = *next;
                lines.push("✓ Applied · Idle alert".into());
            }
            Err(error) => {
                failed(&mut lines, "Idle alert", error);
                return Ok((lines.join("\n"), None));
            }
        }
    }
    if let Some(runtime) = &plan.runtime {
        if let Err(error) = identity_current(core, &plan.target, &name) {
            failed(&mut lines, "Runtime", error);
            return Ok((lines.join("\n"), None));
        }
        if let Err(error) = preset_current(core, plan) {
            failed(&mut lines, "Runtime settings", error);
            return Ok((lines.join("\n"), None));
        }
        let mut runtime = runtime.clone();
        runtime.source.name = name.clone();
        match crate::board::switch_tool::apply_edit(core, &runtime) {
            Ok(observation) => {
                let confirmed = observation.confirmed;
                let text = observation.message;
                lines.push(format!(
                    "{} {} · Runtime launch: {text}",
                    if confirmed { "✓" } else { "?" },
                    if confirmed { "Applied" } else { "Unknown" }
                ));
                if !runtime.model.is_empty() {
                    if observation.model.as_deref() == Some(runtime.model.as_str()) {
                        lines.push(format!("✓ Observed · Model: {}", runtime.model));
                    } else {
                        lines.push(format!("? Unknown · Requested model: {}; last observed: {}. The requested setting is not confirmed.",runtime.model,observation.model.as_deref().unwrap_or("unavailable")));
                    }
                }
                if let Some(effort) = &runtime.effort {
                    lines.push(format!(
                        "? Unknown · Requested effort: {effort}. No effort observation is available."
                    ));
                }
                if !confirmed {
                    lines.push("— Not run · No automatic retry.".into());
                    return Ok((lines.join("\n"), None));
                }
            }
            Err(error) => {
                failed(&mut lines, "Runtime", error);
                return Ok((lines.join("\n"), None));
            }
        }
    }
    if let Some((expected, next)) = &plan.preferences {
        let Some(room) = &room else { unreachable!() };
        if let Err(error) = room_current(core, room)
            .and_then(|()| config.save_workspace_preferences(expected, next))
        {
            failed(&mut lines, "Hidden", error);
            return Ok((lines.join("\n"), None));
        }
        lines.push(format!(
            "✓ Applied · {} squad",
            if next.hidden.contains(&room.id) {
                "Hide"
            } else {
                "Show"
            }
        ));
    }
    if lines.is_empty() {
        lines.push("— Not applied · Nothing changed.".into());
    }
    Ok((
        lines.join("\n"),
        plan.preferences.as_ref().map(|(_, next)| next.clone()),
    ))
}
