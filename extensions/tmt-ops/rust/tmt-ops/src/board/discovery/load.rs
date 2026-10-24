//! One finite public discovery or guarded binding command on the existing lane.
use super::model::{BindReport, BindSelection, ScanObservation};
use crate::core::{Core, SquadError};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Job {
    /// No host or socket option: Core's caller selection picks the host and
    /// server, and reports a gap host beside tmux.
    Scan,
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
            Self::Scan => (vec!["discover".into()], vec![]),
            Self::Bind(selection) => {
                // The row's own socket: an adoptable row always has one.
                let mut args = vec!["add".into(), "--expect".into(), selection.expect.clone()];
                if let Some(socket) = &selection.key.socket {
                    args.extend(["--socket".into(), socket.clone()]);
                }
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
        Job::Scan => ScanObservation::decode(&core.json(&args)?).map(Outcome::Scan),
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
