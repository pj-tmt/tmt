//! One management boundary for CLI, clock and board. Core remains a public port.
use crate::{
    config::Config,
    core::{Core, SquadError},
    me,
    squad::{Member, Squad},
};
use serde_json::{Value, json};
use std::path::PathBuf;
use tmt_squad::cron::{self, Job, Jobs, Pause, Schedule, Store};

mod notices;
mod retirement;
pub use retirement::drain_retired;

impl From<cron::Error> for SquadError {
    fn from(error: cron::Error) -> Self {
        Self::new(&error.code, error.message)
    }
}
fn stored(error: SquadError) -> cron::Error {
    cron::Error {
        code: error.code,
        message: error.message,
    }
}
fn failure(code: &str, message: &str) -> SquadError {
    SquadError::new(code, message)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronActor {
    pub id: String,
    pub name: String,
}
impl From<me::Me> for CronActor {
    fn from(value: me::Me) -> Self {
        Self {
            id: value.id,
            name: value.name,
        }
    }
}

/// Retained selectors never silently follow a reused room name or c-id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobKey {
    pub squad: String,
    pub room_id: String,
    pub id: String,
}
impl JobKey {
    pub fn of(job: &Job) -> Self {
        Self {
            squad: job.squad.clone(),
            room_id: job.room_id.clone(),
            id: job.id(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobView {
    pub job: Job,
    pub owner_name: Option<String>,
    pub next_ms: Vec<i64>,
    pub warnings: Vec<SquadError>,
}
impl JobView {
    pub fn document(&self) -> Value {
        let mut value = self.job.document();
        value["owner"] = json!(self.owner_name);
        value["scheduleText"] = json!(schedule_text(&self.job.schedule));
        value["nextMs"] = json!(self.next_ms);
        value["warnings"] = json!(
            self.warnings
                .iter()
                .map(SquadError::to_json)
                .collect::<Vec<_>>()
        );
        value
    }
}
pub struct List {
    pub jobs: Vec<JobView>,
    pub warnings: Vec<SquadError>,
}
pub struct Applied {
    pub job: JobView,
    pub changed: bool,
    pub warnings: Vec<SquadError>,
}

pub enum Mutation {
    Edit {
        message: Option<String>,
        schedule: Option<Schedule>,
    },
    Remove,
    Pause,
    Resume,
    Reassign {
        owner_id: String,
    },
}
pub enum Change {
    Add {
        squad: String,
        room_id: String,
        owner_id: String,
        message: String,
        schedule: Schedule,
        paused: bool,
    },
    Existing {
        key: JobKey,
        expected_revision: u64,
        mutation: Mutation,
    },
}

pub fn root(core: &Core) -> Result<PathBuf, SquadError> {
    let value = core.api("storage.root", json!({}))?;
    let path = value["dataRoot"]
        .as_str()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            failure(
                "SQUAD_CORE_UNAVAILABLE",
                "storage.root returned no absolute dataRoot.",
            )
        })?;
    Ok(path)
}
fn store(core: &Core) -> Result<Store, SquadError> {
    Ok(Store::new(&root(core)?)?)
}

/// Resolve once at invocation/opening. Apply revalidates this explicit UUID;
/// an identified member or ambiguous runtime never falls back to the user.
pub fn actor(
    core: &Core,
    config: &Config,
    explicit: Option<&str>,
) -> Result<CronActor, SquadError> {
    if let Some(selector) = explicit {
        let shown = core.json(&["identity", "show", selector])?;
        let value = &shown["identity"];
        return Ok(CronActor {
            id: text(value, "id")?,
            name: text(value, "name")?,
        });
    }
    if let Some(caller) = me::caller(core)? {
        return Ok(caller.me.into());
    }
    me::current(core, config)?
        .map(CronActor::from)
        .ok_or_else(|| {
            failure(
                "SQUAD_SENDER_UNKNOWN",
                "Record yourself with tmt squad me <name>, or supply --identity.",
            )
        })
}
fn text(value: &Value, key: &str) -> Result<String, SquadError> {
    value[key].as_str().map(str::to_owned).ok_or_else(|| {
        failure(
            "SQUAD_CORE_UNAVAILABLE",
            "Core returned an incomplete cron reference.",
        )
    })
}
fn active_identity(core: &Core, id: &str) -> Result<String, SquadError> {
    let refs = core.api("references.resolve", json!({"identityIds": [id]}))?;
    let value = &refs["identities"][0];
    if value["id"] != id || value["found"] != true || value["retired"] != false {
        return Err(failure(
            "SQUAD_CRON_IDENTITY_UNAVAILABLE",
            "The selected identity no longer exists or has retired.",
        ));
    }
    text(value, "name")
}
fn room(core: &Core, name: &str, expected: &str) -> Result<Squad, SquadError> {
    let selected = Squad::resolve(core, Some(name))?;
    if selected.room_id != expected {
        return Err(failure(
            "SQUAD_CRON_ROOM_CONFLICT",
            "The squad room changed; reload the job list.",
        ));
    }
    Ok(selected)
}
fn admit(
    core: &Core,
    config: &Config,
    selected: &Squad,
    actor: &CronActor,
) -> Result<Vec<Member>, SquadError> {
    active_identity(core, &actor.id)?;
    let roster = selected.roster(core)?;
    let user = me::current(core, config)?.is_some_and(|user| user.id == actor.id);
    if !user
        && !roster
            .iter()
            .any(|member| member.id == actor.id && member.is_lead())
    {
        return Err(failure(
            "SQUAD_CRON_PERMISSION_DENIED",
            "Only the recorded user or this squad's lead can change jobs.",
        ));
    }
    Ok(roster)
}
fn owner(core: &Core, roster: &[Member], id: &str) -> Result<(), SquadError> {
    active_identity(core, id)?;
    if !roster.iter().any(|member| member.id == id) {
        return Err(failure(
            "SQUAD_NOT_A_MEMBER",
            "The job owner must be a current member of this squad.",
        ));
    }
    Ok(())
}
fn expected<'a>(
    jobs: &'a mut Jobs,
    key: &JobKey,
    revision: u64,
) -> Result<&'a mut Job, SquadError> {
    let job = jobs
        .find_mut(&key.squad, &key.id)
        .ok_or_else(|| failure("SQUAD_CRON_NOT_FOUND", "The job no longer exists."))?;
    if job.room_id != key.room_id {
        return Err(failure(
            "SQUAD_CRON_ROOM_CONFLICT",
            "The job belongs to a different squad room.",
        ));
    }
    if job.revision != revision {
        return Err(failure(
            "SQUAD_CRON_REVISION_CONFLICT",
            "The job changed; reload before trying again.",
        ));
    }
    Ok(job)
}
fn increment(job: &mut Job) -> Result<(), SquadError> {
    job.revision = job.revision.checked_add(1).ok_or_else(|| {
        failure(
            "SQUAD_CRON_REVISION_CONFLICT",
            "Job revision space exhausted.",
        )
    })?;
    Ok(())
}

/// Manual send shares management admission and an immutable locked snapshot.
/// Dispatch happens after return, never while the jobs lock is held.
pub fn authorize_write(
    core: &Core,
    config: &Config,
    key: &JobKey,
    actor: &CronActor,
    expected_revision: u64,
) -> Result<Job, SquadError> {
    admitted_job(core, key, expected_revision, Some((config, actor)))
}

/// Scheduled callers do not invent a human actor. Room, revision and active
/// owner membership are checked through the same locked snapshot as manual send.
pub fn admit_scheduled(
    core: &Core,
    key: &JobKey,
    expected_revision: u64,
) -> Result<Job, SquadError> {
    let job = admitted_job(core, key, expected_revision, None)?;
    if job.state() != "on" {
        return Err(failure(
            "SQUAD_CRON_NOT_ON",
            "This job is paused or has no owner.",
        ));
    }
    Ok(job)
}
fn admitted_job(
    core: &Core,
    key: &JobKey,
    expected_revision: u64,
    actor: Option<(&Config, &CronActor)>,
) -> Result<Job, SquadError> {
    store(core)?
        .update(|jobs| {
            let selected = room(core, &key.squad, &key.room_id).map_err(stored)?;
            let roster = match actor {
                Some((config, actor)) => admit(core, config, &selected, actor),
                None => selected.roster(core),
            }
            .map_err(stored)?;
            let job = expected(jobs, key, expected_revision).map_err(stored)?;
            let id = job.owner_id.as_deref().ok_or_else(|| {
                stored(failure(
                    "SQUAD_CRON_NO_OWNER",
                    "Reassign this job before sending or resuming it.",
                ))
            })?;
            owner(core, &roster, id).map_err(stored)?;
            Ok(job.clone())
        })
        .map_err(Into::into)
}

fn project(core: &Core, job: Job, count: usize, now_ms: i64) -> Result<JobView, SquadError> {
    let owner_name = job
        .owner_id
        .as_deref()
        .map(|id| active_identity(core, id))
        .transpose()?;
    let mut next_ms = Vec::new();
    if job.state() == "on" {
        let mut after = now_ms;
        for _ in 0..count.min(3) {
            let Some(next) = job.schedule.next_after(after)? else {
                break;
            };
            next_ms.push(next);
            after = next;
        }
    }
    Ok(JobView {
        job,
        owner_name,
        next_ms,
        warnings: vec![],
    })
}
pub fn list_jobs(
    core: &Core,
    config: &Config,
    scope: Option<&str>,
    now_ms: i64,
) -> Result<List, SquadError> {
    let warnings = drain_retired(core, config, now_ms)?;
    let squads = if let Some(name) = scope {
        vec![Squad::resolve(core, Some(name))?]
    } else {
        Squad::list(core)?
    };
    let jobs = store(core)?
        .read()?
        .jobs()
        .iter()
        .filter(|job| {
            squads
                .iter()
                .any(|squad| squad.name == job.squad && squad.room_id == job.room_id)
        })
        .cloned()
        .map(|job| project(core, job, 1, now_ms))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(List { jobs, warnings })
}
pub fn show_job(
    core: &Core,
    config: &Config,
    key: &JobKey,
    now_ms: i64,
) -> Result<JobView, SquadError> {
    let warnings = drain_retired(core, config, now_ms)?;
    room(core, &key.squad, &key.room_id)?;
    let jobs = store(core)?.read()?;
    let job = jobs
        .jobs()
        .iter()
        .find(|job| job.squad == key.squad && job.id() == key.id && job.room_id == key.room_id)
        .ok_or_else(|| {
            failure(
                "SQUAD_CRON_NOT_FOUND",
                "The job does not exist in this squad room.",
            )
        })?;
    let mut view = project(core, job.clone(), 3, now_ms)?;
    view.warnings = warnings;
    Ok(view)
}

pub fn apply(
    core: &Core,
    config: &Config,
    actor: &CronActor,
    change: Change,
    now_ms: i64,
) -> Result<Applied, SquadError> {
    let mut warnings = drain_retired(core, config, now_ms)?;
    let store = store(core)?;
    let (before, after, action, changed) = store.update(|jobs| {
        let (name, room_id) = match &change {
            Change::Add { squad, room_id, .. } => (squad, room_id),
            Change::Existing { key, .. } => (&key.squad, &key.room_id),
        };
        let selected = room(core, name, room_id).map_err(stored)?;
        let roster = admit(core, config, &selected, actor).map_err(stored)?;
        match change {
            Change::Add {
                squad,
                room_id,
                owner_id,
                message,
                schedule,
                paused,
            } => {
                owner(core, &roster, &owner_id).map_err(stored)?;
                if message.trim().is_empty() || message.len() > 1048576 {
                    return Err(stored(failure(
                        "SQUAD_CRON_MESSAGE_INVALID",
                        "Use a nonempty message of at most 1 MiB.",
                    )));
                }
                let mut job = Job::new(squad.clone(), room_id, owner_id, message, schedule);
                if paused {
                    job.pause = Some(Pause {
                        by: actor.id.clone(),
                        at_ms: now_ms,
                    });
                }
                let id = jobs.insert(job)?;
                let job = jobs.find_mut(&squad, &id).expect("inserted job").clone();
                retirement::register(core, &job).map_err(stored)?;
                Ok((None, job, "added", true))
            }
            Change::Existing {
                key,
                expected_revision,
                mutation,
            } => {
                let job = expected(jobs, &key, expected_revision).map_err(stored)?;
                let before = job.clone();
                let action = match mutation {
                    Mutation::Edit { message, schedule } => {
                        if let Some(message) = message {
                            if message.trim().is_empty() || message.len() > 1048576 {
                                return Err(stored(failure(
                                    "SQUAD_CRON_MESSAGE_INVALID",
                                    "Use a nonempty message of at most 1 MiB.",
                                )));
                            }
                            job.message = message;
                        }
                        if let Some(schedule) = schedule {
                            job.schedule = schedule;
                        }
                        "edited"
                    }
                    Mutation::Remove => "removed",
                    Mutation::Pause => {
                        if job.pause.is_none() {
                            job.pause = Some(Pause {
                                by: actor.id.clone(),
                                at_ms: now_ms,
                            });
                        }
                        "paused"
                    }
                    Mutation::Resume => {
                        let id = job.owner_id.as_deref().ok_or_else(|| {
                            stored(failure(
                                "SQUAD_CRON_NO_OWNER",
                                "Reassign this job before resuming it.",
                            ))
                        })?;
                        owner(core, &roster, id).map_err(stored)?;
                        job.pause = None;
                        "resumed"
                    }
                    Mutation::Reassign { owner_id } => {
                        owner(core, &roster, &owner_id).map_err(stored)?;
                        job.owner_id = Some(owner_id);
                        retirement::register(core, job).map_err(stored)?;
                        // Reassignment keeps an explicit pause; no-owner jobs remain paused.
                        "reassigned"
                    }
                };
                let changed = before != *job || action == "removed";
                if changed {
                    increment(job).map_err(stored)?;
                }
                let after = job.clone();
                if action == "removed" {
                    jobs.remove(&key.squad, &key.id);
                }
                Ok((Some(before), after, action, changed))
            }
        }
    })?;
    if changed {
        warnings.extend(notices::changed(
            core,
            actor,
            before.as_ref(),
            &after,
            action,
        ));
    }
    // A committed mutation must not turn into an apparent rollback if a name lookup fails.
    let job = match project(core, after.clone(), 3, now_ms) {
        Ok(view) => view,
        Err(error) => {
            warnings.push(error);
            JobView {
                job: after,
                owner_name: None,
                next_ms: vec![],
                warnings: vec![],
            }
        }
    };
    Ok(Applied {
        job,
        changed,
        warnings,
    })
}

pub fn schedule_text(schedule: &Schedule) -> String {
    let value = schedule.document();
    match value["kind"].as_str() {
        Some("every") => {
            let seconds = value["seconds"].as_i64().unwrap_or_default();
            let (n, unit) = [(86400, "d"), (3600, "h"), (60, "m"), (1, "s")]
                .into_iter()
                .find(|(scale, _)| seconds % scale == 0)
                .map(|(scale, unit)| (seconds / scale, unit))
                .expect("seconds fit");
            let from = value["from"]
                .as_str()
                .map(|v| format!(" from {v}"))
                .unwrap_or_default();
            format!("every {n}{unit}{from}")
        }
        Some("at") => format!(
            "{} {}",
            value["on"].as_str().unwrap_or("daily"),
            value["at"].as_str().unwrap_or_default()
        ),
        _ => format!("cron {}", value["expression"].as_str().unwrap_or_default()),
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
