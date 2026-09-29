//! Read-only context composition. No binding reconciliation or notebook creation.

mod presentation;

use crate::invocation::OutputMode;
use std::{
    io::{self, Write},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tmt_adapters::{
    config::ConfigPaths,
    extension_hooks, notes,
    storage::Storage,
    tmux::{CallerEnvironment, OperationOptions, Tmux},
};
use tmt_core::{
    binding::{BindingEvidence, evaluate_binding},
    endpoint::EndpointProbe,
    identity::NotesIdentityId,
};

pub fn execute(mode: OutputMode) -> io::Result<u8> {
    let document = observe().unwrap_or_else(presentation::unavailable);
    let output = presentation::bounded(document, mode.json)?;
    io::stdout().lock().write_all(output.as_bytes())?;
    Ok(0)
}

fn observe() -> Option<serde_json::Value> {
    crate::caller_context::require_independent_host().ok()?;
    let tmux = Tmux::default();
    let pane = tmux.caller_pane(&CallerEnvironment::current()).ok()??;
    let snapshot = tmux
        .observe_snapshot(OperationOptions {
            pane_ids: Some(std::slice::from_ref(&pane)),
            ..OperationOptions::default()
        })
        .ok()?;
    let paths = ConfigPaths::discover().ok()?;
    let now = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis(),
    )
    .ok()?;
    let stored =
        Storage::context_by_pane(&paths.database, &pane, &snapshot.server.server_id, now).ok()?;
    let Some(stored) = stored else {
        // This branch emits only the fixed unbound hint, never identity data.
        // A command-derived suggested name is not a stored binding marker.
        return snapshot
            .panes
            .iter()
            .find(|candidate| candidate.id == pane)
            .filter(|candidate| candidate.marker.is_none())
            .map(|_| presentation::unbound());
    };
    if !matches!(
        evaluate_binding(&stored.entry, &EndpointProbe::Live(snapshot)),
        BindingEvidence::Active(_)
    ) {
        return None;
    }
    Some(verified_document(
        stored,
        &paths,
        Instant::now() + extension_hooks::CONTEXT_DEADLINE,
    ))
}

/// Only a verified, bound identity asks enabled extensions for context.
pub(crate) fn verified_document(
    stored: tmt_adapters::storage::IdentityContextSnapshot,
    paths: &ConfigPaths,
    deadline: Instant,
) -> serde_json::Value {
    let notes = NotesIdentityId::try_from(&stored.entry.identity)
        .ok()
        .and_then(|id| notes::existing_path(paths, &id).ok().flatten())
        .and_then(|path| path.into_os_string().into_string().ok());
    let extensions = extension_hooks::context_contributions(
        &paths.global_dir,
        &stored.entry.identity.id,
        deadline,
    );
    presentation::document(stored, notes, &extensions)
}

pub(crate) fn render_verified(
    stored: tmt_adapters::storage::IdentityContextSnapshot,
    paths: &ConfigPaths,
    deadline: Instant,
) -> io::Result<String> {
    presentation::bounded(verified_document(stored, paths, deadline), false)
}

pub(crate) fn unbound_text() -> io::Result<String> {
    presentation::bounded(presentation::unbound(), false)
}
