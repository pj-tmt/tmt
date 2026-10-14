//! One finite public discovery or guarded binding command on the existing lane.
use super::model::{BindReport, BindSelection, ScanObservation};
use crate::core::{Core, SquadError};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanScope {
    pub host: Option<String>,
    pub socket: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Job {
    Scan(ScanScope),
    Bind(BindSelection),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Task {
    pub owner: u64,
    pub serial: u64,
    pub job: Job,
}
#[derive(Debug)]
pub(super) enum Outcome {
    Scan(ScanObservation),
    Bind(BindReport),
}
pub(crate) struct Completed {
    pub task: Task,
    pub(super) result: Result<Outcome, SquadError>,
}
impl Job {
    pub fn writes(&self) -> bool {
        matches!(self, Self::Bind(_))
    }
    /// Options and operands stay separate; even an option-like name is literal.
    pub(super) fn arguments(&self) -> (Vec<String>, Vec<String>) {
        match self {
            Self::Scan(scope) => {
                let mut args = vec!["discover".into()];
                if let Some(host) = &scope.host {
                    args.extend(["--host".into(), host.clone()]);
                }
                if let Some(socket) = &scope.socket {
                    args.extend(["--socket".into(), socket.clone()]);
                }
                (args, vec![])
            }
            Self::Bind(selection) => {
                let mut args = vec![
                    "add".into(),
                    "--expect".into(),
                    selection.expect.clone(),
                    "--socket".into(),
                    selection.key.socket.clone().expect("reviewed tmux socket"),
                ];
                let mut operands = vec![selection.key.pane.clone()];
                match &selection.selector {
                    super::model::AgentSelector::NewName(name) => operands.push(name.clone()),
                    super::model::AgentSelector::ExistingUuid(id) => {
                        args.extend(["--identity".into(), id.clone()])
                    }
                }
                if selection.save {
                    args.push("--save".into());
                }
                (args, operands)
            }
        }
    }
}
pub(crate) fn complete(core: &Core, task: Task) -> Completed {
    let result = execute(core, &task.job);
    Completed { task, result }
}
fn execute(core: &Core, job: &Job) -> Result<Outcome, SquadError> {
    let (args, operands) = job.arguments();
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    let operands: Vec<_> = operands.iter().map(String::as_str).collect();
    match job {
        Job::Scan(scope) => {
            let value = core.json(&args)?;
            let observation = ScanObservation::decode(&value)?;
            if scope
                .host
                .as_ref()
                .is_some_and(|host| observation.sources.iter().any(|s| s.host != *host))
                || scope.socket.as_ref().is_some_and(|socket| {
                    observation
                        .sources
                        .iter()
                        .any(|s| s.socket.as_ref() != Some(socket))
                })
            {
                return Err(super::model::invalid());
            }
            Ok(Outcome::Scan(observation))
        }
        Job::Bind(selection) => {
            // The common process owner bounds and reaps the command. Guarded add
            // can publish refused/partial evidence on a nonzero exit.
            let finished =
                core.json_finished_with_operands(&args, &operands, Duration::from_secs(15))?;
            let value =
                serde_json::from_slice(&finished.stdout).map_err(|_| super::model::invalid())?;
            BindReport::decode(&value, selection, finished.success).map(Outcome::Bind)
        }
    }
}
