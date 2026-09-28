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
    storage::Storage,
    tmux::{
        ActionError, BindingSession, CallerEnvironment, FocusError, Invoker, OperationOptions, Tmux,
    },
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
/// Key-binding jobs have `TMUX` but no `TMUX_PANE`; `TMUX`'s session serves.
fn invoker() -> Result<Invoker, Failure> {
    let environment = CallerEnvironment::current();
    let socket = environment
        .selected_server_socket()
        .ok()
        .flatten()
        .ok_or_else(host_unsupported)?;
    let pane = environment
        .pane
        .as_ref()
        .and_then(|pane| pane.to_str())
        .filter(|pane| !pane.is_empty())
        .map(str::to_owned);
    let session = environment.selected_session();
    if pane.is_none() && session.is_none() {
        return Err(host_unsupported());
    }
    Ok(Invoker {
        socket: socket.to_owned(),
        pane,
        session,
    })
}

fn pane_failure(error: FocusError, pane: &str) -> Failure {
    match error {
        FocusError::HostUnsupported => host_unsupported(),
        FocusError::PaneNotFound => pane_missing(pane),
        FocusError::Tmux(error) => endpoint_failure(error),
    }
}

fn identity_failure(error: ActionError) -> Failure {
    match error {
        ActionError::HostUnsupported => host_unsupported(),
        ActionError::Evidence(error) => endpoint_failure(error),
        other => Failure::new(
            "RECONCILIATION_FAILED",
            "The identity's pane changed before it could be focused; nothing changed.",
            1,
        )
        .caused_by(other),
    }
}

/// `target` is an identity name or a pane target (such as a previous
/// `from`); the shared resolver handles both, current server only.
fn run(target: String) -> Result<Focused, Failure> {
    let invoker = invoker()?;
    let tmux = Tmux::default();
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
    let pending = target::resolve(&mut storage, &tmux, &target).and_then(|observed| {
        let (Some(identity), binding @ Some(_)) = (observed.identity, observed.binding) else {
            // An unbound pane target has no identity evidence to verify.
            let pane = observed.pane.id;
            return tmux
                .focus_pane(&invoker, &pane, OperationOptions::default())
                .map(|before| Focused {
                    interface: pane.clone(),
                    previous: before.pane,
                    viewer: before.client,
                })
                .map_err(|error| pane_failure(error, &pane));
        };
        let entry = BindingEntry { identity, binding };
        match BindingSession::new(&tmux)
            .with_invoker(invoker)
            .focus(&entry)
        {
            ActionResult::Completed(focused) => Ok(focused),
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
        Tmux::default()
            .invoker_client(&invoker, OperationOptions::default())
            .map_err(|error| pane_failure(error, invoker.pane.as_deref().unwrap_or_default()))
    }) {
        Ok(view) => view,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = io::stdout().lock();
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

pub fn execute(target: String, mode: OutputMode) -> io::Result<u8> {
    let focused = match run(target) {
        Ok(focused) => focused,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = io::stdout().lock();
    if mode.json {
        let document = serde_json::json!({
            "focused": {"pane": focused.interface},
            "from": focused.previous.map(|pane| serde_json::json!({"pane": pane})),
            "client": focused.viewer,
        });
        writeln!(stdout, "{document}")?;
    } else {
        writeln!(stdout, "Focused {}.", focused.interface)?;
    }
    Ok(0)
}
