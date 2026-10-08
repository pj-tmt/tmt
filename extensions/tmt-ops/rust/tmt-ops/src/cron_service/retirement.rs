//! Consumer-scoped owner retirement. Hook references survive renames and reassignments.
use super::*;

const CONSUMER: &str = "squad-cron";
fn reference(job: &Job) -> String {
    format!("{}:{}:{}", job.room_id, job.squad, job.id())
}
fn hook(identity: &str, reference: &str) -> Value {
    json!({"consumer": CONSUMER, "identityId": identity, "reference": reference})
}
pub(super) fn register(core: &Core, job: &Job) -> Result<(), SquadError> {
    if let Some(id) = &job.owner_id {
        core.api("identityHooks.register", hook(id, &reference(job)))?;
    }
    Ok(())
}

/// One bounded page per invocation/tick. State is durable before acknowledgment;
/// announcements are best effort and may be lost on process interruption.
pub fn drain_retired(
    core: &Core,
    _config: &Config,
    now_ms: i64,
) -> Result<Vec<SquadError>, SquadError> {
    let page = core.api(
        "identityHooks.pending",
        json!({"consumer": CONSUMER, "limit": 16}),
    )?;
    let hooks = page["hooks"].as_array().ok_or_else(|| {
        failure(
            "SQUAD_CORE_UNAVAILABLE",
            "identityHooks.pending returned no hook page.",
        )
    })?;
    let mut warnings = Vec::new();
    for pending in hooks {
        let id = text(pending, "identityId")?;
        let reference = text(pending, "reference")?;
        let input = hook(&id, &reference);
        core.api("identityHooks.attempt", input.clone())?;
        let (store, _migration) = store(core)?;
        let changed = store.update(|jobs| {
            let Some(job) = jobs
                .jobs()
                .iter()
                .find(|job| {
                    self::reference(job) == reference && job.owner_id.as_deref() == Some(&id)
                })
                .cloned()
            else {
                return Ok(None);
            };
            let job = jobs.find_mut(&job.squad, &job.id()).expect("matched job");
            job.owner_id = None;
            job.pause = Some(Pause {
                by: id.clone(),
                at_ms: now_ms,
            });
            increment(job).map_err(stored)?;
            Ok(Some(job.clone()))
        })?;
        if let Some(job) = changed {
            let notice = (|| {
                let selected = room(core, &job.squad, &job.room_id)?;
                let roster = selected.roster(core)?;
                let lead = roster
                    .iter()
                    .find(|member| member.is_lead())
                    .ok_or_else(|| {
                        failure(
                            "SQUAD_CRON_NOTICE_FAILED",
                            "The job has no owner, but its squad has no lead to notify.",
                        )
                    })?;
                let refs = core.api("references.resolve", json!({"identityIds": [&id]}))?;
                let name = refs["identities"][0]["name"].as_str().unwrap_or(&id);
                let message = notices::line(
                    &job,
                    &format!(
                        "no owner ({name} retired) · reassign with tmt ops sq cron reassign {} {} <member>",
                        job.squad,
                        job.id()
                    ),
                );
                notices::send(core, &job, "retired", &lead.id, &message, None)
            })();
            if let Err(error) = notice {
                warnings.push(error);
            }
        }
        core.api("identityHooks.ack", input)?;
    }
    if page["pending"]
        .as_u64()
        .is_some_and(|n| n > hooks.len() as u64)
    {
        warnings.push(failure("SQUAD_CRON_RETIREMENT_PENDING", "More owner retirements remain; the next cron command or clock tick processes another page."));
    }
    Ok(warnings)
}
