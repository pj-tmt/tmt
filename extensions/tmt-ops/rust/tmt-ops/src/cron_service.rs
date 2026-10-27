//! One management boundary for CLI, clock and board. Core remains a public port.
use crate::{
    config::Config,
    core::{Core, SquadError},
    squad::{Member, Squad},
};
use serde_json::{Value, json};
use std::path::PathBuf;
use tmt_ops::cron::{self, Job, Jobs, Pause, Schedule, ScheduleInput, Store};

mod notices;
mod owners;
mod retirement;
pub use retirement::drain_retired;

impl From<cron::Error> for SquadError {
    fn from(error: cron::Error) -> Self {
        if error.code == cron::STORE_BUSY {
            return Self::hinted(&error.code, &error.message, " ", cron::BUSY_RETRY);
        }
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

pub use crate::management::ManagementActor as CronActor;

/// Retained selectors never silently follow a reused room name or c-id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobKey {
    pub room_id: String,
    pub id: String,
}
impl JobKey {
    pub fn of(job: &Job) -> Self {
        Self {
            room_id: job.room_id.clone(),
            id: job.id(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobView {
    pub job: Job,
    pub squad: String,
    pub owner_name: Option<String>,
    pub owner_missing: bool,
    pub next_ms: Vec<i64>,
    pub warnings: Vec<SquadError>,
}
impl JobView {
    pub fn state_label(&self) -> &'static str {
        if self.owner_missing {
            "paused · owner missing"
        } else {
            self.job.state()
        }
    }
    pub fn document(&self) -> Value {
        let mut value = self.job.document();
        value["squad"] = json!(self.squad);
        value["owner"] = json!(self.owner_name);
        value["ownerMissing"] = json!(self.owner_missing);
        value["stateLabel"] = json!(self.state_label());
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
    crate::migration::state_root(core)
}
fn store(core: &Core) -> Result<(Store, crate::migration::LegacyGuard), SquadError> {
    let guard = crate::migration::state_guard(core)?;
    let store = Store::new(&root(core)?)?;
    if store.needs_v1_migration()? {
        store.migrate_v1(&migration_rooms(core)?)?;
    }
    Ok((store, guard))
}

fn migration_rooms(core: &Core) -> Result<Vec<(String, String)>, SquadError> {
    let squads = Squad::list(core)?;
    let mut rooms = Vec::with_capacity(squads.len());
    for squad in squads {
        if !crate::config::uuid_like(&squad.room_id) {
            return Err(failure(
                "SQUAD_CRON_STORE_INVALID",
                "Core returned an invalid squad room UUID; resolve the room and retry.",
            ));
        }
        rooms.push((squad.name, squad.room_id));
    }
    Ok(rooms)
}

/// A validated in-progress rename journal can prove the former name of its
/// already-renamed room. Any independently reused name remains ambiguous.
pub(crate) fn prepare_rename_repair(
    core: &Core,
    old: &str,
    new: &str,
    room_id: &str,
) -> Result<(), SquadError> {
    let _guard = crate::migration::state_guard(core)?;
    let store = Store::new(&root(core)?)?;
    if store.needs_v1_migration()? {
        let mut rooms = migration_rooms(core)?;
        if !rooms.iter().any(|(name, id)| name == new && id == room_id) {
            return Err(failure(
                "SQUAD_CRON_STORE_INVALID",
                "The renamed room is unavailable; resolve the squad room and retry.",
            ));
        }
        rooms.push((old.into(), room_id.into()));
        store.migrate_v1(&rooms)?;
    }
    Ok(())
}

/// Read jobs through the service so legacy migration also precedes a clock tick.
pub(crate) fn read_jobs(core: &Core) -> Result<Jobs, SquadError> {
    let (store, _migration) = store(core)?;
    store.read().map_err(Into::into)
}

pub fn actor(core: &Core, explicit: Option<&str>) -> Result<CronActor, SquadError> {
    crate::management::actor(core, explicit, &crate::management::CRON)
}
pub fn member(core: &Core, selector: &str) -> Result<CronActor, SquadError> {
    crate::management::member(core, selector, &crate::management::CRON)
}
fn text(value: &Value, key: &str) -> Result<String, SquadError> {
    crate::management::text(value, key, &crate::management::CRON)
}
fn active_identity(core: &Core, id: &str) -> Result<String, SquadError> {
    crate::management::active_identity(core, id, &crate::management::CRON)
}
fn room(core: &Core, expected: &str) -> Result<Squad, SquadError> {
    let shown = core.json(&["room", "show", expected])?;
    let observed = &shown["room"];
    let current = observed["name"]
        .as_str()
        .and_then(|name| name.strip_prefix("squad-"));
    if observed["id"] != expected
        || observed["retired"] != false
        || !crate::config::uuid_like(expected)
        || current.is_none_or(|name| !crate::squad::valid_name(name))
    {
        return Err(failure(
            "SQUAD_CRON_ROOM_CONFLICT",
            "The squad room is unavailable; reload the job list.",
        ));
    }
    Ok(Squad {
        name: current.expect("checked name").into(),
        room_id: expected.into(),
    })
}
fn admit(core: &Core, selected: &Squad, actor: &CronActor) -> Result<Vec<Member>, SquadError> {
    crate::management::admit(core, selected, actor, &crate::management::CRON)
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
        .find_mut(&key.room_id, &key.id)
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
pub(crate) fn authorize_write(
    core: &Core,
    key: &JobKey,
    actor: &CronActor,
    expected_revision: u64,
) -> Result<(Job, String), SquadError> {
    admitted_job(core, key, expected_revision, Some(actor))
}

/// Scheduled callers do not invent a human actor. Room, revision and active
/// owner membership are checked through the same locked snapshot as manual send.
pub fn admit_scheduled(
    core: &Core,
    key: &JobKey,
    expected_revision: u64,
) -> Result<Job, SquadError> {
    let (job, _) = admitted_job(core, key, expected_revision, None)?;
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
    actor: Option<&CronActor>,
) -> Result<(Job, String), SquadError> {
    let (store, _migration) = store(core)?;
    store
        .update(|jobs| {
            let selected = room(core, &key.room_id).map_err(stored)?;
            let roster = match actor {
                Some(actor) => admit(core, &selected, actor),
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
            if owners::observe(core, &roster, id)
                .map_err(stored)?
                .is_none()
            {
                owners::pause_missing(job, crate::status::now_ms() as i64).map_err(stored)?;
                return Ok(Err(failure(
                    "SQUAD_NOT_A_MEMBER",
                    "The job owner is missing from this squad; the job remains paused.",
                )));
            }
            Ok(Ok((job.clone(), selected.name)))
        })
        .map_err(SquadError::from)?
}

fn project(
    core: &Core,
    job: Job,
    squad: String,
    count: usize,
    now_ms: i64,
) -> Result<JobView, SquadError> {
    let owner_name = job
        .owner_id
        .as_deref()
        .map(|id| active_identity(core, id))
        .transpose()?;
    let missing = job.owner_id.is_none();
    project_with_owner(job, squad, owner_name, missing, count, now_ms)
}
fn project_with_owner(
    job: Job,
    squad: String,
    owner_name: Option<String>,
    owner_missing: bool,
    count: usize,
    now_ms: i64,
) -> Result<JobView, SquadError> {
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
        squad,
        job,
        owner_name,
        owner_missing,
        next_ms,
        warnings: vec![],
    })
}
/// Migrate any legacy timers before the room name can change.
pub(crate) fn prepare_rename(core: &Core) -> Result<(), SquadError> {
    let (_store, _migration) = store(core)?;
    Ok(())
}

/// The membership service calls this before committing a leave/move. This is
/// departure admission, not a manual pause command; it cannot resume any job.
pub(crate) fn with_departure_pause<T>(
    core: &Core,
    room_id: &str,
    identity_id: &str,
    now_ms: i64,
    leave_or_move: impl FnOnce() -> T,
) -> Result<(Vec<String>, T), SquadError> {
    let (store, _migration) = store(core)?;
    store
        .pause_owner_then(room_id, identity_id, now_ms, |_| leave_or_move())
        .map_err(Into::into)
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
    let (store, _migration) = store(core)?;
    let jobs = owners::project_jobs(core, &store, &squads, 1, now_ms)?;
    Ok(List { jobs, warnings })
}
pub fn show_job(
    core: &Core,
    config: &Config,
    key: &JobKey,
    now_ms: i64,
) -> Result<JobView, SquadError> {
    let warnings = drain_retired(core, config, now_ms)?;
    let squad = room(core, &key.room_id)?;
    let (store, _migration) = store(core)?;
    let mut view = owners::project_jobs(core, &store, &[squad], 3, now_ms)?
        .into_iter()
        .find(|view| view.job.id() == key.id && view.job.room_id == key.room_id)
        .ok_or_else(|| {
            failure(
                "SQUAD_CRON_NOT_FOUND",
                "The job does not exist in this squad room.",
            )
        })?;
    view.warnings.extend(warnings);
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
    let (store, _migration) = store(core)?;
    let (before, after, action, changed, squad_name) = store.update(|jobs| {
        let room_id = match &change {
            Change::Add { room_id, .. } => room_id,
            Change::Existing { key, .. } => &key.room_id,
        };
        let selected = room(core, room_id).map_err(stored)?;
        let roster = admit(core, &selected, actor).map_err(stored)?;
        match change {
            Change::Add {
                squad: _,
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
                let mut job = Job::new(room_id, owner_id, message, schedule);
                if paused {
                    job.pause = Some(Pause {
                        by: actor.id.clone(),
                        at_ms: now_ms,
                    });
                }
                let id = jobs.insert(job)?;
                let job = jobs
                    .find_mut(&selected.room_id, &id)
                    .expect("inserted job")
                    .clone();
                retirement::register(core, &job).map_err(stored)?;
                Ok(Ok((None, job, "added", true, selected.name)))
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
                        if owners::observe(core, &roster, id)
                            .map_err(stored)?
                            .is_none()
                        {
                            owners::pause_missing(job, now_ms).map_err(stored)?;
                            return Ok(Err(failure(
                                "SQUAD_NOT_A_MEMBER",
                                "The owner is missing from this squad; the job stays paused.",
                            )));
                        }
                        job.pause = None;
                        "resumed"
                    }
                    Mutation::Reassign { owner_id } => {
                        owner(core, &roster, &owner_id).map_err(stored)?;
                        if job.pause.is_none()
                            && let Some(old) = &job.owner_id
                            && owners::observe(core, &roster, old)
                                .map_err(stored)?
                                .is_none()
                        {
                            job.pause = Some(Pause {
                                by: old.clone(),
                                at_ms: now_ms,
                            });
                        }
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
                    jobs.remove(&key.room_id, &key.id);
                }
                Ok(Ok((Some(before), after, action, changed, selected.name)))
            }
        }
    })??;
    if changed {
        warnings.extend(notices::changed(
            core,
            actor,
            before.as_ref(),
            &after,
            &squad_name,
            action,
        ));
    }
    // A committed mutation must not turn into an apparent rollback if a name lookup fails.
    let job = match project(core, after.clone(), squad_name.clone(), 3, now_ms) {
        Ok(view) => view,
        Err(error) => {
            warnings.push(error);
            JobView {
                squad: squad_name,
                job: after,
                owner_name: None,
                owner_missing: false, // Failed observation is unknown, not absence.
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

/// The inverse of [`schedule_text`], for the board's one-line editor:
/// `every 3h [from 09:00]`, `daily 23:00`, `weekdays 09:00`, `mon,thu 10:00`,
/// or a five-field cron expression (optionally after `cron `). Parsing is
/// delegated to `Schedule::parse`, which owns grammar, zones and next-slot math.
pub fn parse_schedule_text(text: &str, zone: &str, now_ms: i64) -> Result<Schedule, SquadError> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let input = match words.as_slice() {
        ["every", duration] => ScheduleInput::Every {
            duration,
            from: None,
        },
        ["every", duration, "from", from] => ScheduleInput::Every {
            duration,
            from: Some(from),
        },
        ["daily", time] => ScheduleInput::At { time, on: None },
        ["cron", ..] if words.len() == 6 => ScheduleInput::Cron(text.trim_start()[4..].trim()),
        [on, time] if time.contains(':') => ScheduleInput::At { time, on: Some(on) },
        _ if words.len() == 5 => ScheduleInput::Cron(text.trim()),
        _ => {
            return Err(failure(
                "SQUAD_CRON_SCHEDULE_INVALID",
                "Use every 30m [from 09:00], daily 09:00, weekdays 09:00, mon,thu 10:00 or a five-field cron expression.",
            ));
        }
    };
    Ok(Schedule::parse(input, zone, now_ms)?)
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
