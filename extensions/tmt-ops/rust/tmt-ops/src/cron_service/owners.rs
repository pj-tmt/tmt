//! Positive owner observations pause jobs durably. Offline presence and failed
//! reads are not missing ownership; reappearance never clears the stored pause.
use super::*;

pub(super) fn observe(
    core: &Core,
    roster: &[Member],
    id: &str,
) -> Result<Option<String>, SquadError> {
    let refs = core.api("references.resolve", json!({"identityIds": [id]}))?;
    let values = refs["identities"]
        .as_array()
        .filter(|values| values.len() == 1)
        .ok_or_else(|| {
            failure(
                "SQUAD_CORE_UNAVAILABLE",
                "Core did not return the requested owner reference.",
            )
        })?;
    let value = &values[0];
    if value["id"] != id
        || !value["found"].is_boolean()
        || (value["found"] == true && !value["retired"].is_boolean())
    {
        return Err(failure(
            "SQUAD_CORE_UNAVAILABLE",
            "Core returned an incomplete owner reference.",
        ));
    }
    if value["found"] == false
        || value["retired"] == true
        || !roster.iter().any(|member| member.id == id)
    {
        return Ok(None);
    }
    Ok(Some(text(value, "name")?))
}

pub(super) fn pause_missing(job: &mut Job, now_ms: i64) -> Result<(), SquadError> {
    if job.pause.is_none() {
        increment(job)?;
        job.pause = Some(Pause {
            by: job.owner_id.clone().ok_or_else(|| {
                failure("SQUAD_CRON_NO_OWNER", "Missing pause on an unowned job.")
            })?,
            at_ms: now_ms,
        });
    }
    Ok(())
}

/// One transaction persists all positively missing owners before projecting
/// the list. Any unavailable roster/reference leaves the published bytes intact.
pub(super) fn project_jobs(
    core: &Core,
    store: &Store,
    squads: &[Squad],
    count: usize,
    now_ms: i64,
) -> Result<Vec<JobView>, SquadError> {
    if store.read()?.jobs().is_empty() {
        return Ok(Vec::new());
    }
    store.update(|jobs| {
        let mut views = Vec::new();
        for squad in squads {
            let keys: Vec<_> = jobs.jobs().iter().filter(|job| job.squad == squad.name && job.room_id == squad.room_id).map(|job| job.id()).collect();
            if keys.is_empty() { continue; }
            let roster = squad.roster(core).map_err(stored)?;
            let mut observations = std::collections::BTreeMap::new();
            for key in keys {
                let job = jobs.find_mut(&squad.name, &key).expect("locked job");
                let name = if let Some(id) = &job.owner_id {
                    if !observations.contains_key(id) {
                        observations.insert(id.clone(), observe(core, &roster, id).map_err(stored)?);
                    }
                    observations[id].clone()
                } else { None };
                let missing = name.is_none();
                if missing { pause_missing(job, now_ms).map_err(stored)?; }
                let mut view = project_with_owner(job.clone(), name, missing, count, now_ms).map_err(stored)?;
                if missing {
                    view.warnings.push(failure("SQUAD_CRON_OWNER_MISSING", "Paused: owner missing from this squad. Reassign if needed, then resume explicitly."));
                }
                views.push(view);
            }
        }
        Ok(views)
    }).map_err(Into::into)
}
