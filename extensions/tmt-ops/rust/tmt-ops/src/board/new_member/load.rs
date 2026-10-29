//! Bounded command work on the ordered action lane, never the frame thread.
use super::super::host_launch::{Loaded, read, unavailable};
use super::*;
use crate::{
    config::Config,
    core::{Core, SquadError},
};
use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum Work {
    Load,
    Save {
        host: String,
        argv: Option<Vec<String>>,
    },
    Launch(model::Review),
}
#[derive(Clone)]
pub(in crate::board) struct Task {
    pub owner: u64,
    pub work: Work,
    pub config: Option<Box<Config>>,
}
impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Task")
            .field("owner", &self.owner)
            .field("work", &self.work)
            .finish_non_exhaustive()
    }
}

// One form generation owns its held Config revision. Work includes all mutable
// review inputs; the configuration is carried only to the CAS publisher.
impl PartialEq for Task {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner && self.work == other.work
    }
}
impl Eq for Task {}
pub(in crate::board) struct Completed {
    pub(super) task: Task,
    pub(super) result: Result<Outcome, SquadError>,
}
pub(super) enum Outcome {
    Loaded(Box<Loaded>),
    Created {
        id: String,
        room: Option<String>,
        message: String,
    },
}
pub(in crate::board) fn complete(core: &Core, task: Task) -> Completed {
    let result = match &task.work {
        Work::Load => read(core).map(|loaded| Outcome::Loaded(Box::new(loaded))),
        Work::Save { .. } => {
            return match task.config.clone() {
                Some(config) => save(core, task, *config),
                None => Completed {
                    task,
                    result: Err(unavailable("Template save requires its held Config.")),
                },
            };
        }
        Work::Launch(review) => launch(core, review),
    };
    Completed { task, result }
}
pub(in crate::board) fn save(core: &Core, task: Task, mut config: Config) -> Completed {
    let result = match &task.work {
        Work::Save { host, argv } => config
            .set_launch(host, argv.as_deref())
            .and_then(|()| read(core))
            .map(|loaded| Outcome::Loaded(Box::new(loaded))),
        _ => Err(unavailable("Invalid template save.")),
    };
    Completed { task, result }
}
fn launch(core: &Core, review: &model::Review) -> Result<Outcome, SquadError> {
    let before = core.json_within(&["ls"], Duration::from_secs(5))?;
    let rows = before["identities"]
        .as_array()
        .ok_or_else(|| unavailable("Identity list unavailable; nothing launched."))?;
    if rows
        .iter()
        .any(|row| row["name"] == review.name || row["canonicalName"] == review.name)
    {
        return Err(unavailable(
            "That identity already exists; nothing launched.",
        ));
    }
    let ids = rows
        .iter()
        .filter_map(|r| r["id"].as_str())
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    let outer = review
        .outer
        .as_ref()
        .ok_or_else(|| unavailable("Copy only; nothing created."))?;
    // Re-read the public template before executing the exact reviewed argv. A
    // changed built-in/driver template cannot silently replace the review.
    super::super::host_launch::recheck(
        core,
        &review.host,
        &review.session,
        &review.command,
        outer,
    )?;
    // Core owns Unicode canonicalization. A public lookup refuses aliases too;
    // matching only the list's displayed name would let run reuse an identity.
    match core.json_with_operands(&["identity", "show"], &[&review.name]) {
        Err(error) if error.code == "NAME_NOT_FOUND" => {}
        Err(error) => return Err(error),
        Ok(_) => {
            return Err(unavailable(
                "That identity already exists; nothing launched.",
            ));
        }
    }
    let args: Vec<OsString> = outer[1..].iter().map(OsString::from).collect();
    let result = crate::runner::run(
        Path::new(&outer[0]),
        &args,
        b"",
        Duration::from_secs(10),
        65536,
    );
    // A launch may have partially succeeded despite its exit status. Observe
    // before reporting it; never automatically launch again or close a pane. A
    // command that already failed, such as a provider ending during startup, gets
    // a short look rather than the full wait.
    let failed = matches!(&result, Ok(done) if !done.success);
    let observed_until = Instant::now()
        + if failed {
            Duration::from_secs(2)
        } else {
            Duration::from_secs(30)
        };
    loop {
        let listed=core.json_within(&["ls"],observed_until.saturating_duration_since(Instant::now())).map_err(|e|unavailable(format!("Launch outcome unknown: {} Draft kept; check public state before trying again.",e.message)))?;
        let found: Vec<_> = listed["identities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| {
                row["name"] == review.name
                    && row["id"].as_str().is_some_and(|id| !ids.contains(id))
                    && row["location"]["host"]
                        .as_str()
                        .is_some_and(|host| !host.is_empty())
            })
            .collect();
        if found.len() == 1 {
            let id = found[0]["id"].as_str().unwrap().to_owned();
            let mut room = None;
            let mut message = format!("Created {} in Common pool.", review.name);
            // Keep applies only now that the member started; promoting keeps its UUID.
            let keep_failure = review.save_line().and_then(|line| {
                core.json_with_operands(&["identity", "create"], &[&review.name])
                    .err()
                    .map(|error| {
                        // The squad step needs a kept identity, so it waits for the keep.
                        let then = review
                            .squad_lines()
                            .iter()
                            .map(|squad| format!("\nThen: {squad}"))
                            .collect::<String>();
                        format!(
                            "It started as temporary; keeping it failed: {}\nKeep it with: {line}{then}",
                            error.message
                        )
                    })
            });
            let joined = match (&review.room, &review.new_squad) {
                _ if keep_failure.is_some() => None,
                (Some(target), _) => Some((
                    target.name.clone(),
                    Config::load(core).and_then(|mut config| {
                        crate::membership::workspace::apply(
                            core,
                            &mut config,
                            crate::membership::workspace::Operation::Add {
                                room: target.clone(),
                                member: id.clone(),
                            },
                        )
                    }),
                )),
                // A typed squad is created with the kept member in it.
                (None, Some(name)) => Some((
                    name.clone(),
                    Config::load(core).and_then(|mut config| {
                        crate::membership::workspace::apply(
                            core,
                            &mut config,
                            crate::membership::workspace::Operation::Create {
                                name: name.clone(),
                                lead: None,
                                members: vec![id.clone()],
                            },
                        )
                    }),
                )),
                (None, None) => None,
            };
            if let Some((squad, joined)) = joined {
                match joined {
                    Ok(outcome) if outcome.complete => {
                        room = review.room.as_ref().map(|r| r.id.clone()).or_else(|| {
                            outcome.document["created"]["squad"]["roomId"]
                                .as_str()
                                .map(str::to_owned)
                        });
                        message = format!("Created {} in {squad}.", review.name);
                    }
                    other => {
                        let reason = match other {
                            Err(e) => e.message,
                            Ok(_) => "Squad add was not confirmed.".into(),
                        };
                        let tmt = review.display.command[0].clone();
                        let repair = review
                            .new_squad
                            .iter()
                            .map(|name| {
                                crate::launch::line(&[
                                    tmt.clone(),
                                    "ops".into(),
                                    "sq".into(),
                                    "init".into(),
                                    name.clone(),
                                ])
                            })
                            .chain([crate::launch::line(&[
                                tmt.clone(),
                                "ops".into(),
                                "sq".into(),
                                "add".into(),
                                "--squad".into(),
                                squad.clone(),
                                "--".into(),
                                id.clone(),
                            ])])
                            .collect::<Vec<_>>()
                            .join("\n");
                        message = format!(
                            "Created {} in Common pool; squad add failed: {reason}\nRepair after checking: {repair}",
                            review.name
                        );
                    }
                }
            }
            if let Some(failure) = keep_failure {
                message = format!("{message}\n{failure}");
            }
            return Ok(Outcome::Created { id, room, message });
        }
        if found.len() > 1 || Instant::now() >= observed_until {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let detail = match result {
        Ok(done) if done.success => {
            "Host command exited successfully, but no new bound UUID was observed."
        }
        Ok(_) => "Host command failed; a partial launch may exist.",
        Err(_) => "Host command timed out or failed; a partial launch may exist.",
    };
    Err(unavailable(format!(
        "{detail} Draft kept. Check public state before retrying; nothing was relaunched or closed."
    )))
}
