//! Add/move uses the same membership service as CLI removal. Timers remain in
//! the source squad, durably paused before the move's leave can commit.
use super::*;

fn current_room(core: &Core, squad: &Squad) -> Result<(), SquadError> {
    let shown = core.json(&["room", "show", &squad.room_id])?;
    if shown["room"]["id"] != squad.room_id
        || shown["room"]["name"] != squad::room_name(&squad.name)
        || shown["room"]["retired"] != false
    {
        return Err(SquadError::new(
            "ROOM_REVISION_CONFLICT",
            "The displayed squad changed; refresh before assigning a member.",
        ));
    }
    Ok(())
}

pub fn assign(
    core: &Core,
    source: Option<&Squad>,
    destination: &Squad,
    name: &str,
    move_member: bool,
    layout: Layout,
) -> Result<Outcome, SquadError> {
    current_room(core, destination)?;
    let member = saved(core, name)?;
    if !move_member {
        return add(core, destination, layout, &[member.id]);
    }
    let source = source.ok_or_else(|| {
        SquadError::new(
            "SQUAD_NOT_MEMBER",
            "Common pool members can be added, not moved from a virtual room.",
        )
    })?;
    if source.room_id == destination.room_id {
        return Err(SquadError::new(
            "SQUAD_ASSIGN_INVALID",
            "Choose a different destination squad.",
        ));
    }
    current_room(core, source)?;
    if !source
        .roster(core)?
        .iter()
        .any(|candidate| candidate.id == member.id)
    {
        return Err(SquadError::new(
            "SQUAD_NOT_MEMBER",
            "The selected source membership no longer exists.",
        ));
    }
    let (paused, (added, leave)) = crate::cron_service::with_departure_pause(
        core,
        &source.room_id,
        &member.id,
        jiff::Timestamp::now().as_millisecond(),
        || {
            let added = add(core, destination, layout, std::slice::from_ref(&member.id));
            // A failed/unknown join or subsequent initial-state write does not
            // authorize removal from the original squad, even if join took effect.
            let leave = added
                .as_ref()
                .ok()
                .filter(|outcome| outcome.complete)
                .map(|_| core.json(&["room", "leave", &source.room_id, "--identity", &member.id]));
            (added, leave)
        },
    )?;
    let mut document = json!({"source":source.name,"destination":destination.name,"member":{"id":member.id,"name":member.name},"pausedTimers":paused,"sourceRemoval":null});
    match added {
        Err(error) => return Ok(removal_failed(document, "destination membership", &error)),
        Ok(added) => {
            document["destinationResult"] = added.document;
            if !added.complete {
                return Ok(Outcome {
                    document,
                    complete: false,
                });
            }
        }
    }
    let removed = finish_remove(
        core,
        source,
        &member,
        paused,
        leave.expect("confirmed destination admits leave"),
    )?;
    document["sourceRemoval"] = removed.document;
    Ok(Outcome {
        document,
        complete: removed.complete,
    })
}

#[cfg(test)]
mod tests;
