//! Durable selectors are not pane routing or identity creation.

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    output::Failure,
};
use std::io::Write;
use tmt_adapters::{
    storage::Storage,
    tmux::{BindingSession, CallerEnvironment, Tmux},
};
use tmt_core::{
    binding,
    dispatch::canonical_id,
    identity::{Identity, IdentityReader},
    names::normalize_name,
};

pub fn missing(name: &str) -> Failure {
    Failure::new(
        "NAME_NOT_FOUND",
        format!("Identity '{name}' was not found."),
        3,
    )
}

pub enum Selector {
    Explicit(String),
    Pane(String),
}

fn required_failure() -> Failure {
    Failure::new(
        "IDENTITY_REQUIRED",
        "An identity is required; use --identity or run from a verified bound pane.",
        1,
    )
}

/// Reject an unavailable implicit caller before opening or migrating storage.
pub fn required(explicit: Option<&str>) -> Result<Selector, Failure> {
    select(&Tmux::default(), explicit, false)?.ok_or_else(required_failure)
}

pub fn resolve(storage: &mut Storage, selector: Selector) -> Result<Identity, Failure> {
    selected(storage, &Tmux::default(), selector)?.ok_or_else(required_failure)
}

pub fn optional(
    storage: &mut Storage,
    tmux: &Tmux,
    explicit: Option<&str>,
) -> Result<Option<Identity>, Failure> {
    match select(tmux, explicit, true)? {
        Some(selector) => selected(storage, tmux, selector),
        None => Ok(None),
    }
}

fn select(
    tmux: &Tmux,
    explicit: Option<&str>,
    allow_anonymous: bool,
) -> Result<Option<Selector>, Failure> {
    if let Some(name) = explicit {
        return Ok(Some(Selector::Explicit(name.to_owned())));
    }
    if let Err(error) = crate::caller_context::require_independent_host() {
        // Anonymous delivery remains valid, but must not acquire the identity
        // of an unrelated conversation through the shared host's pane.
        return if allow_anonymous {
            // This is an attribution notice, not a delivery-success claim.
            // Emit in JSON mode too without changing the stdout document.
            let _ = writeln!(
                std::io::stderr(),
                "tmt: sender identity not established on a shared runtime host; using anonymous sender. Use --identity <name>."
            );
            Ok(None)
        } else {
            Err(error)
        };
    }
    tmux.caller_pane(&CallerEnvironment::current())
        .map(|pane| pane.map(Selector::Pane))
        .map_err(endpoint_failure)
}

fn selected(
    storage: &mut Storage,
    tmux: &Tmux,
    selector: Selector,
) -> Result<Option<Identity>, Failure> {
    match selector {
        Selector::Explicit(name) => {
            // A canonical UUID identifies the accepted recipient even if a
            // different identity later takes a UUID-shaped display name.
            // Unknown or retired IDs retain the existing name lookup.
            let by_id = if canonical_id(&name) {
                storage.find_active_identity_by_id(&name).map_err(|error| {
                    Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1)
                        .caused_by(error)
                })?
            } else {
                None
            };
            let selected = match by_id {
                Some(identity) => Some(identity),
                None => storage
                    .find_identity(&normalize_name(&name))
                    .map_err(|error| {
                        Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1)
                            .caused_by(error)
                    })?,
            };
            selected.ok_or_else(|| missing(&name)).map(Some)
        }
        Selector::Pane(pane) => {
            let observed = binding::pane_presence(storage, &mut BindingSession::new(tmux), &pane)
                .map_err(binding_failure)?;
            Ok(observed.identity)
        }
    }
}
