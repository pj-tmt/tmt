//! One confirmed host launch; only a same-UUID running-driver observation completes it.
use super::{super::host_launch, model::Review};
use crate::core::{Core, SquadError};
use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum Work {
    Load,
    Launch(Box<Review>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Task {
    pub(super) owner: u64,
    pub(super) work: Work,
}
pub(in crate::board) struct Completed {
    pub(super) task: Task,
    pub(super) result: Result<Outcome, SquadError>,
}
pub(super) enum Outcome {
    Loaded(Box<host_launch::Loaded>),
    Switched(String),
    Unknown(String),
}
pub(in crate::board) fn complete(core: &Core, task: Task) -> Completed {
    let result = match &task.work {
        Work::Load => host_launch::read(core).map(|l| Outcome::Loaded(Box::new(l))),
        Work::Launch(review) => launch(core, review, Duration::from_secs(90)),
    };
    Completed { task, result }
}
fn unknown(detail: &str) -> Outcome {
    Outcome::Unknown(format!(
        "Switch outcome unknown. {detail}\nCheck the old agent and destination before retrying. Nothing was automatically relaunched or closed."
    ))
}
pub(super) fn launch(
    core: &Core,
    review: &Review,
    budget: Duration,
) -> Result<Outcome, SquadError> {
    let before = core.json_within(&["ls"], Duration::from_secs(5))?;
    let rows = before["identities"].as_array().ok_or_else(|| {
        host_launch::unavailable("Member observation unavailable; nothing launched.")
    })?;
    if !rows.iter().any(|r| review.source.matches(r)) {
        return Err(host_launch::unavailable(
            "Selected member or session observation changed. Close and review again; nothing launched.",
        ));
    }
    let outer = review
        .outer
        .as_ref()
        .ok_or_else(|| host_launch::unavailable("Copy only; nothing launched."))?;
    host_launch::recheck(
        core,
        review.source.host(),
        review.source.session(),
        &review.command,
        outer,
    )?;
    let until = Instant::now() + budget;
    let args = outer[1..].iter().map(OsString::from).collect::<Vec<_>>();
    let launched = crate::runner::run(
        Path::new(&outer[0]),
        &args,
        b"",
        budget.min(Duration::from_secs(10)),
        65536,
    );
    // A host failure can follow a partial launch. Always inspect public state;
    // an exit status or pane ID cannot stand in for the requested driver's evidence.
    loop {
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let listed = match core.json_within(&["ls"], remaining.min(Duration::from_secs(5))) {
            Ok(v) => v,
            Err(e) => return Ok(unknown(&e.message)),
        };
        if let Some(row) = listed["identities"]
            .as_array()
            .and_then(|rows| rows.iter().find(|r| review.observed(r)))
        {
            let model = row["resume"]["observedModel"]["model"]
                .as_str()
                .unwrap_or("unavailable");
            return Ok(Outcome::Switched(format!(
                "{} is running {} in {} / {}.\nObserved model: {model}\nName, UUID, notes and open requests stay.",
                review.source.name,
                review.driver,
                review.source.host(),
                review.source.session()
            )));
        }
        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
    let detail = match launched {
        Ok(done) if !done.success => serde_json::from_slice::<serde_json::Value>(&done.stdout)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Host command failed; a partial launch may exist.".into()),
        Ok(_) => {
            "No same-UUID running-driver observation arrived within the observation window.".into()
        }
        Err(_) => "Host command timed out or failed; a partial launch may exist.".into(),
    };
    Ok(unknown(&detail))
}
