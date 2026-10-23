//! Bounded command work on the ordered action lane, never the frame thread.
use super::*;
use crate::launch::display_executable;
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
pub(super) struct Loaded {
    pub hosts: Vec<crate::launch::Host>,
    pub config: Config,
    pub executable: String,
    pub display_executable: String,
}
pub(super) enum Outcome {
    Loaded(Box<Loaded>),
    Created {
        id: String,
        room: Option<String>,
        message: String,
    },
}
fn unavailable(text: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_LAUNCH_UNAVAILABLE", text)
}
fn read(core: &Core) -> Result<Loaded, SquadError> {
    let hosts = crate::launch::hosts(&core.json(&["host", "templates"])?).map_err(unavailable)?;
    let executable = core
        .executable()
        .to_str()
        .filter(|s| !s.chars().any(char::is_control))
        .ok_or_else(|| unavailable("Core executable cannot be copied."))?
        .into();
    Ok(Loaded {
        hosts,
        config: Config::load(core)?,
        executable,
        display_executable: display_executable(
            core.executable(),
            std::env::var_os("PATH").as_deref(),
        ),
    })
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
    let loaded = read(core)?;
    let host = loaded
        .hosts
        .iter()
        .find(|h| h.name == review.host && matches!(h.state.as_str(), "builtin" | "ok"))
        .ok_or_else(|| unavailable("Host changed; review again."))?;
    let argv = loaded
        .config
        .launch_override(&review.host)?
        .or_else(|| host.argv.clone())
        .ok_or_else(|| unavailable("Host launch is now unavailable."))?;
    if crate::launch::fill(
        &argv,
        &review.session,
        &crate::launch::line(&review.command),
    )
    .map_err(unavailable)?
        != *outer
    {
        return Err(unavailable("Host launch changed; review again."));
    }
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
    // before reporting it; never automatically launch again or close a pane.
    let observed_until = Instant::now() + Duration::from_secs(30);
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
            if let Some(target) = &review.room {
                let joined = Config::load(core).and_then(|mut config| {
                    crate::membership::workspace::apply(
                        core,
                        &mut config,
                        crate::membership::workspace::Operation::Add {
                            room: target.clone(),
                            member: id.clone(),
                        },
                    )
                });
                match joined {
                    Ok(outcome) if outcome.complete => {
                        room = Some(target.id.clone());
                        message = format!("Created {} in {}.", review.name, target.name);
                    }
                    other => {
                        let reason = match other {
                            Err(e) => e.message,
                            Ok(_) => "Squad add was not confirmed.".into(),
                        };
                        message = format!(
                            "Created {} in Common pool; squad add failed: {reason}\nRepair after checking: {}",
                            review.name,
                            crate::launch::line(&[
                                review.display.command[0].clone(),
                                "ops".into(),
                                "sq".into(),
                                "add".into(),
                                "--squad".into(),
                                target.name.clone(),
                                "--".into(),
                                id.clone()
                            ])
                        );
                    }
                }
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
