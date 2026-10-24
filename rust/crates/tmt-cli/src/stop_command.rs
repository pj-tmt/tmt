//! `tmt stop`: stop a bound member's agent with the verified stop that
//! `resume --move` uses. The binding and the remembered session stay, so
//! `tmt resume` works afterwards.

use crate::{
    invocation::OutputMode,
    output::{Failure, identity_missing},
    verified_stop::{self, Recheck, refusal, stop_failed},
};
use std::{
    io::{self, Write},
    time::Instant,
};
use tmt_adapters::{
    config::ConfigPaths,
    host::{Host, Kept, PaneClose},
    process::UnixCommandRunner,
    runtime::RuntimeRegistry,
    storage::{IdentityContextSnapshot, Storage},
};
use tmt_core::{binding::BindingEvidence, identity::Identity};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The agent was running and exited after the one TERM.
    Stopped,
    /// The recorded agent process had already ended; nothing was signalled.
    AlreadyEnded,
    /// The member has no binding, so no agent is running anywhere tmt knows of.
    NotRunning,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::AlreadyEnded => "already_ended",
            Self::NotRunning => "not_running",
        }
    }
}

/// What `--close` did about the member's pane.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    /// tmt opened the pane and it was closed.
    Closed,
    /// The pane was already gone, so there was nothing to close.
    Gone,
    /// The member has no binding, so there is no pane.
    None,
}

impl Pane {
    fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Gone => "gone",
            Self::None => "none",
        }
    }
}

struct Stopped {
    identity: Identity,
    outcome: Outcome,
    /// Present only when the pane was asked to be closed.
    pane: Option<Pane>,
}

fn kept(name: &str, why: Kept) -> Failure {
    let reason = match why {
        Kept::NotOpenedByTmt => "TMT did not open that pane",
        Kept::Unknown => "its host cannot say whether TMT opened that pane",
        Kept::Unsupported => "its host cannot close a pane",
    };
    refusal(
        "PANE_NOT_CLOSED",
        format!(
            "{name}'s agent was stopped, but its pane was left open: {reason}. Close the pane yourself if you no longer need it."
        ),
    )
}

fn context(paths: &ConfigPaths, identity: &Identity) -> Result<IdentityContextSnapshot, Failure> {
    Storage::context_by_identity(&paths.database, &identity.id, 0)
        .map_err(|error| {
            Failure::new("IDENTITY_ERROR", "Could not access member state.", 1).caused_by(error)
        })?
        .filter(|stored| stored.entry.identity == *identity)
        .ok_or_else(|| {
            refusal(
                "IDENTITY_CONFLICT",
                "The identity changed before it was stopped.",
            )
        })
}

fn run(selector: &str, close: bool) -> Result<Stopped, Failure> {
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No agent was stopped.",
            "IDENTITY_ERROR",
            "Could not access member state.",
        )
    })?;
    let stopped = stop(&mut storage, &paths, selector, close);
    if storage.close().is_err() {
        let mut stderr = tmt_cli_style::stream::stderr();
        let terminal = stderr.terminal();
        let _ = tmt_cli_style::message::warning(
            &mut stderr,
            terminal,
            "could not close member state cleanly; inspect identity status before retrying.",
            None,
        );
    }
    stopped
}

fn stop(
    storage: &mut Storage,
    paths: &ConfigPaths,
    selector: &str,
    close: bool,
) -> Result<Stopped, Failure> {
    let identity = storage
        .resolve_identity(selector)
        .map_err(|error| {
            Failure::new("IDENTITY_ERROR", "Could not access member state.", 1).caused_by(error)
        })?
        .ok_or_else(|| identity_missing(selector))?;
    let captured = context(paths, &identity)?;
    let Some(old) = captured.entry.binding.as_ref() else {
        return Ok(Stopped {
            identity,
            outcome: Outcome::NotRunning,
            pane: close.then_some(Pane::None),
        });
    };
    let session = captured.preferences.remembered.as_ref().ok_or_else(|| {
        refusal(
            "RESUME_UNAVAILABLE",
            "The member has no recorded agent session, so its process cannot be proven; no agent was stopped.",
        )
    })?;
    let registry = RuntimeRegistry::first_party();
    let lifecycle = registry.lifecycle(&session.harness).ok_or_else(|| {
        refusal(
            "RESUME_UNSUPPORTED",
            format!(
                "The {} driver is not registered; no agent was stopped.",
                session.harness.as_str()
            ),
        )
    })?;
    let host = Host::for_server(&old.server);
    let source = verified_stop::prove(
        &host,
        &captured,
        old,
        session,
        lifecycle,
        Instant::now() + verified_stop::STOP_ALLOWANCE,
    )?;
    let outcome = if source.agent_alive() {
        let runner = UnixCommandRunner;
        verified_stop::request(
            storage,
            &host,
            &runner,
            &source,
            lifecycle,
            "The member's ownership changed before the stop.",
            |_, _| Ok(Recheck::Holds),
        )?
        .wait()
        .map_err(stop_failed)?;
        Outcome::Stopped
    } else {
        Outcome::AlreadyEnded
    };
    // The pane is closed only after the stop succeeded or the agent had already
    // ended, on a fresh host session so the stop's wait does not eat its budget.
    let pane = if !close {
        None
    } else if matches!(source.evidence, BindingEvidence::EndpointLost) {
        Some(Pane::Gone)
    } else {
        match host
            .session()
            .close_opened_pane(&captured.entry)
            .map_err(|error| {
                refusal(
                    "PANE_NOT_CLOSED",
                    format!(
                        "{}'s agent was stopped, but its pane could not be closed: {error}. Close the pane yourself if you no longer need it.",
                        identity.name
                    ),
                )
            })? {
            PaneClose::Closed => Some(Pane::Closed),
            PaneClose::Kept(why) => return Err(kept(&identity.name, why)),
        }
    };
    Ok(Stopped {
        identity,
        outcome,
        pane,
    })
}

pub fn execute(selector: String, close: bool, mode: OutputMode) -> io::Result<u8> {
    let stopped = match run(&selector, close) {
        Ok(stopped) => stopped,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    let terminal = stdout.terminal();
    if mode.json {
        let mut document = serde_json::json!({
            "identity": {"id": stopped.identity.id, "name": stopped.identity.name},
            "outcome": stopped.outcome.as_str(),
        });
        if let Some(pane) = stopped.pane {
            document["pane"] = pane.as_str().into();
        }
        writeln!(stdout, "{document}")?;
        return Ok(0);
    }
    let name = &stopped.identity.name;
    let kept_state = "its binding and remembered session are kept";
    let message = match (stopped.outcome, stopped.pane) {
        (Outcome::Stopped, Some(Pane::Closed)) => {
            format!("Stopped {name}'s agent and closed its pane; {kept_state}")
        }
        (Outcome::Stopped, _) => format!("Stopped {name}'s agent; {kept_state}"),
        (Outcome::AlreadyEnded, Some(Pane::Closed)) => {
            format!("{name}'s agent had already ended; closed its pane, nothing was signalled")
        }
        (Outcome::AlreadyEnded, _) => {
            format!("{name}'s agent had already ended; nothing was signalled")
        }
        (Outcome::NotRunning, _) => format!("{name} has no running agent; nothing to stop"),
    };
    tmt_cli_style::message::success(&mut stdout, terminal, &message)?;
    Ok(0)
}
