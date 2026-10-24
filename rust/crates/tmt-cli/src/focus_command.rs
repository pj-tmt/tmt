//! `tmt focus`: show a verified identity's pane, or a pane (for example to
//! return), in the invoking user's tmux client. It changes only the view.
//! `tmt focus --client` only names that client and the pane it shows.

use crate::{
    binding_error::endpoint_failure,
    invocation::OutputMode,
    output::{Failure, after_cleanup},
    target,
};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    host::{
        ActionError, CallerEnvironment, FocusError, Host, Invoker, OperationOptions, PaneSwap,
        SwapDirection,
    },
    storage::Storage,
};
use tmt_core::{
    binding::BindingEntry,
    driver::{ActionResult, Driver, Focused},
};

fn host_unsupported() -> Failure {
    Failure::new(
        "HOST_UNSUPPORTED",
        "No tmux client can be focused for this invocation: run it inside tmux, from a pane with an attached client. Nothing changed.",
        1,
    )
}

fn pane_missing(pane: &str) -> Failure {
    Failure::new("PANE_NOT_FOUND", format!("Pane '{pane}' was not found."), 3)
}

/// The invoker is only what its own environment reports; no client is guessed.
fn invoker() -> Result<Invoker, Failure> {
    Invoker::from_environment(&CallerEnvironment::current()).ok_or_else(host_unsupported)
}

fn pane_failure(error: FocusError, pane: &str) -> Failure {
    match error {
        FocusError::HostUnsupported => host_unsupported(),
        FocusError::PaneNotFound => pane_missing(pane),
        FocusError::NoNeighbour => no_neighbour(),
        FocusError::Evidence(error) => endpoint_failure(error.into()),
    }
}

fn identity_failure(error: ActionError) -> Failure {
    match error {
        ActionError::HostUnsupported => host_unsupported(),
        ActionError::NoNeighbour => no_neighbour(),
        ActionError::Evidence(error) => endpoint_failure(error),
        other => Failure::new(
            "RECONCILIATION_FAILED",
            "The identity's pane changed before it could be focused; nothing changed.",
            1,
        )
        .caused_by(other),
    }
}

fn no_neighbour() -> Failure {
    Failure::new(
        "PANE_NOT_FOUND",
        "No visible neighbouring pane in that direction; nothing changed.",
        3,
    )
}

enum FocusResult {
    Attached(crate::attach_command::Prepared),
    Focused(Focused),
    Swapped(PaneSwap),
}

/// `target` is an identity UUID, name or pane target (such as a previous
/// `from`); Focus resolves UUIDs first through their recorded binding.
fn run(
    target: String,
    into: Option<SwapDirection>,
    mode: OutputMode,
) -> Result<FocusResult, Failure> {
    // A focus changes the invoking user's own view: the caller's host.
    let host = Host::for_caller(&CallerEnvironment::current());
    // A tmux view needs the invoking client; an external host focuses
    // through its driver, which owns which view moves.
    let swap_invoker = if into.is_some() {
        match host.kind() {
            tmt_core::host::HostKind::External(_) => None,
            _ => Some(invoker()?),
        }
    } else {
        None
    };
    if into.is_some() {
        let caller = swap_invoker
            .as_ref()
            .and_then(|invoker| invoker.pane.as_deref())
            .ok_or_else(host_unsupported)?;
        let verified = host
            .caller_pane(&CallerEnvironment::current())
            .map_err(endpoint_failure)?;
        if verified.as_deref() != Some(caller) {
            return Err(host_unsupported());
        }
    }
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "Nothing changed; retrying the identical command is safe.",
            "IDENTITY_ERROR",
            "Could not open identity storage.",
        )
    })?;
    let pending = target::resolve_focus(&mut storage, &target).and_then(|observed| {
        if let Some(direction) = into {
            let invoker = swap_invoker.as_ref().ok_or_else(host_unsupported)?;
            if observed.server.host != host.kind() || observed.server.socket_path != invoker.socket
            {
                return Err(host_unsupported());
            }
            if let (Some(identity), Some(binding)) = (observed.identity, observed.binding) {
                let entry = BindingEntry {
                    identity,
                    binding: Some(binding),
                };
                return host
                    .session()
                    .with_invoker(invoker.clone())
                    .swap_into(&entry, direction)
                    .map(FocusResult::Swapped)
                    .map_err(identity_failure);
            }
            return host
                .swap_pane(
                    invoker,
                    &observed.pane.id,
                    direction,
                    OperationOptions::default(),
                )
                .map(FocusResult::Swapped)
                .map_err(|error| pane_failure(error, &observed.pane.id));
        }
        if Host::for_server(&observed.server).has_terminal_stream() {
            crate::attach_command::ensure_interactive(mode)?;
            return crate::attach_command::prepare_observed(observed).map(FocusResult::Attached);
        }
        let invoker = match host.kind() {
            tmt_core::host::HostKind::External(_) => None,
            _ => Some(invoker()?),
        };
        let (Some(identity), binding @ Some(_)) = (observed.identity, observed.binding) else {
            // An unbound pane target has no identity evidence to verify.
            let pane = observed.pane.id;
            let Some(invoker) = &invoker else {
                return Err(host_unsupported());
            };
            return host
                .focus_pane(invoker, &pane, OperationOptions::default())
                .map(|before| Focused {
                    interface: pane.clone(),
                    previous: before.pane,
                    viewer: before.client,
                })
                .map(FocusResult::Focused)
                .map_err(|error| pane_failure(error, &pane));
        };
        let entry = BindingEntry { identity, binding };
        let session = host.session();
        let mut session = match invoker {
            Some(invoker) => session.with_invoker(invoker),
            None => session,
        };
        match session.focus(&entry) {
            ActionResult::Completed(focused) => Ok(FocusResult::Focused(focused)),
            ActionResult::Failed(error) => Err(identity_failure(error)),
            _ => Err(host_unsupported()),
        }
    });
    after_cleanup(pending, || storage.close())
}

/// Resolves the invoker's client exactly as a focus would, without switching
/// anything or opening storage.
pub fn client(mode: OutputMode) -> io::Result<u8> {
    let view = match invoker().and_then(|invoker| {
        Host::for_caller(&CallerEnvironment::current())
            .invoker_client(&invoker, OperationOptions::default())
            .map_err(|error| pane_failure(error, invoker.pane.as_deref().unwrap_or_default()))
    }) {
        Ok(view) => view,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        let document = serde_json::json!({"client": view.client, "pane": view.pane});
        writeln!(stdout, "{document}")?;
    } else {
        writeln!(
            stdout,
            "Client {} shows {}.",
            view.client,
            view.pane.as_deref().unwrap_or("no pane")
        )?;
    }
    Ok(0)
}

pub fn execute(target: String, into: Option<SwapDirection>, mode: OutputMode) -> io::Result<u8> {
    let result = match run(target, into, mode) {
        Ok(result) => result,
        Err(mut error) => {
            if into.is_some() && error.code == "HOST_UNSUPPORTED" {
                error.message = "This host cannot swap panes for this invocation: use a verified caller pane and another target on the same server. Nothing changed.".into();
            }
            return error.publish(mode);
        }
    };
    if let FocusResult::Attached(prepared) = result {
        return crate::attach_command::execute_prepared(prepared, false, mode);
    }
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    let terminal = stdout.terminal();
    let focused = match result {
        FocusResult::Attached(_) => unreachable!("attachment returned before text output"),
        FocusResult::Swapped(swapped) => {
            if mode.json {
                let document = if swapped.already {
                    serde_json::json!({"swapped": null, "with": {"pane": swapped.neighbour}, "already": true})
                } else {
                    serde_json::json!({"swapped": {"pane": swapped.target}, "with": {"pane": swapped.neighbour}})
                };
                writeln!(stdout, "{document}")?;
            } else {
                tmt_cli_style::message::success(
                    &mut stdout,
                    terminal,
                    &if swapped.already {
                        format!("{} is already beside this pane", swapped.target)
                    } else {
                        format!("Swapped {} with {}", swapped.target, swapped.neighbour)
                    },
                )?;
            }
            return Ok(0);
        }
        FocusResult::Focused(focused) => focused,
    };
    if mode.json {
        let document = serde_json::json!({
            "focused": {"pane": focused.interface},
            "from": focused.previous.map(|pane| serde_json::json!({"pane": pane})),
            "client": focused.viewer,
        });
        writeln!(stdout, "{document}")?;
    } else {
        tmt_cli_style::message::success(
            &mut stdout,
            terminal,
            &format!("Focused {}", focused.interface),
        )?;
    }
    Ok(0)
}
