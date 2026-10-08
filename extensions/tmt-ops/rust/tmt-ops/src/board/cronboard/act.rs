//! Carries out one cron request on the board's execute path. Every effect goes
//! through `cron_service` or `cron_clock::send_now`; nothing here decides
//! permission, membership or revision, and a failure is never retried.

use crate::{
    config::Config,
    core::Core,
    cron_service::{self as service, Change, CronActor, JobKey, Mutation, schedule_text},
};

/// What a control asks for. The actor, job key and viewed revision were taken
/// when the interaction opened; the service revalidates all of them on apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronRequest {
    Add {
        actor: CronActor,
        squad: String,
        room_id: String,
        owner: String,
        message: String,
        schedule: String,
        zone: String,
    },
    Existing {
        actor: CronActor,
        key: JobKey,
        revision: u64,
        op: Op,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// A field left `None` stays exactly as stored.
    Edit {
        message: Option<String>,
        schedule: Option<String>,
        zone: String,
    },
    Reassign {
        owner: String,
    },
    Pause,
    Resume,
    Remove,
    Send,
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// A committed change can still report best-effort announcement warnings.
fn with_warnings(done: String, warnings: &[crate::core::SquadError]) -> String {
    match warnings {
        [] => done,
        [first, rest @ ..] => {
            let more = if rest.is_empty() {
                String::new()
            } else {
                format!(" (+{} more)", rest.len())
            };
            format!("{done} ! {}{more}", first.message)
        }
    }
}

pub fn act(core: &Core, request: CronRequest) -> Result<String, String> {
    let config = Config::load(core).map_err(|error| error.message)?;
    let now = now_ms();
    run(core, &config, request, now).map_err(|error| error.message)
}

fn run(
    core: &Core,
    config: &Config,
    request: CronRequest,
    now: i64,
) -> Result<String, crate::core::SquadError> {
    let (actor, change, verb) = match request {
        CronRequest::Add {
            actor,
            squad,
            room_id,
            owner,
            message,
            schedule,
            zone,
        } => {
            let owner = service::member(core, &owner)?;
            let schedule = service::parse_schedule_text(&schedule, &zone, now)?;
            (
                actor,
                Change::Add {
                    squad,
                    room_id,
                    owner_id: owner.id,
                    message,
                    schedule,
                    paused: false,
                },
                "Added",
            )
        }
        CronRequest::Existing {
            actor,
            key,
            revision,
            op,
        } => {
            let (mutation, verb) = match op {
                Op::Send => {
                    let operation = crate::cron_clock::manual_operation()?;
                    let sent = crate::cron_clock::send_now(
                        core, config, &key, &actor, revision, &operation,
                    )?;
                    return Ok(format!(
                        "Accepted {} {} for its owner (delivery is not confirmed; operation {}).",
                        key.squad,
                        key.id,
                        sent["dispatch"]["operationId"]
                            .as_str()
                            .unwrap_or(&operation)
                    ));
                }
                Op::Edit {
                    message,
                    schedule,
                    zone,
                } => (
                    Mutation::Edit {
                        message,
                        schedule: schedule
                            .map(|text| service::parse_schedule_text(&text, &zone, now))
                            .transpose()?,
                    },
                    "Edited",
                ),
                Op::Reassign { owner } => (
                    Mutation::Reassign {
                        owner_id: service::member(core, &owner)?.id,
                    },
                    "Reassigned",
                ),
                Op::Pause => (Mutation::Pause, "Paused"),
                Op::Resume => (Mutation::Resume, "Resumed"),
                Op::Remove => (Mutation::Remove, "Removed"),
            };
            (
                actor,
                Change::Existing {
                    key,
                    expected_revision: revision,
                    mutation,
                },
                verb,
            )
        }
    };
    let applied = service::apply(core, config, &actor, change, now)?;
    let job = &applied.job.job;
    let done = if applied.changed {
        let detail = match verb {
            "Added" | "Reassigned" => format!(
                " · owner {}",
                applied.job.owner_name.as_deref().unwrap_or("–")
            ),
            _ => String::new(),
        };
        format!(
            "{verb} {} {} ({}){detail}.",
            job.squad,
            job.id(),
            schedule_text(&job.schedule)
        )
    } else {
        format!(
            "{} {} was already as requested; nothing changed.",
            job.squad,
            job.id()
        )
    };
    Ok(with_warnings(done, &applied.warnings))
}

#[cfg(test)]
mod tests;
