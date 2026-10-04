//! Read-only context composition. No binding reconciliation or notebook creation.

mod presentation;

use crate::invocation::OutputMode;
use std::{
    io::{self, Write},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    extension_hooks,
    host::{CallerEnvironment, Host, OperationOptions},
    notes,
    storage::Storage,
};
use tmt_core::{
    binding::{BindingEvidence, evaluate_binding},
    endpoint::EndpointProbe,
    identity::NotesIdentityId,
};

pub fn execute(mode: OutputMode) -> io::Result<u8> {
    let document = observe().unwrap_or_else(presentation::unavailable);
    let output = presentation::bounded(document, mode.json)?;
    tmt_cli_style::stream::stdout(mode.json).write_all(output.as_bytes())?;
    Ok(0)
}

fn observe() -> Option<serde_json::Value> {
    crate::caller_context::require_independent_host().ok()?;
    let environment = CallerEnvironment::current();
    let host = Host::for_caller(&environment);
    let pane = host.caller_pane(&environment).ok()??;
    let snapshot = host
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
    let stored = Storage::context_by_pane(
        &paths.database,
        snapshot.server.host,
        &pane,
        &snapshot.server.server_id,
        now,
    )
    .ok()?;
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
    compacted: bool,
) -> io::Result<String> {
    let saved = NotesIdentityId::try_from(&stored.entry.identity).is_ok();
    let enabled = compacted
        && saved
        && ConfigFiles {
            paths: paths.clone(),
        }
        .notes_compaction_reminder()
        .unwrap_or(false);
    let mut document = verified_document(stored, paths, deadline);
    if enabled {
        presentation::add_compaction_reminder(&mut document);
    }
    presentation::bounded(document, false)
}

/// An already verified prompt carries incoming attention and consented extension
/// lines, with the same escaping and aggregate budget as SessionStart context.
pub(crate) fn render_prompt(
    stored: &tmt_adapters::storage::IdentityContextSnapshot,
    paths: &ConfigPaths,
    deadline: Instant,
    reminder: Option<u64>,
) -> String {
    let reminder = reminder.map(|percent| {
        let path = NotesIdentityId::try_from(&stored.entry.identity)
            .ok()
            .and_then(|id| notes::existing_path(paths, &id).ok().flatten())
            .and_then(|path| path.into_os_string().into_string().ok());
        presentation::threshold_reminder(percent, &stored.entry.identity.id, path.as_deref())
    });
    presentation::bounded_prompt(
        stored.requests.incoming,
        &stored.entry.identity.id,
        &extension_hooks::context_contributions(
            &paths.global_dir,
            &stored.entry.identity.id,
            deadline,
        ),
        reminder.as_deref(),
    )
}

pub(crate) fn unbound_text() -> io::Result<String> {
    presentation::bounded(presentation::unbound(), false)
}

#[cfg(test)]
pub(crate) use presentation::hint_commands;

#[cfg(test)]
pub(crate) use presentation::PRINTED_HINTS as PRESENTATION_HINTS;
