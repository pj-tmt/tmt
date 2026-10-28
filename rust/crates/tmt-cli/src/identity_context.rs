//! Durable selectors are not pane routing or identity creation.

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    output::{Failure, identity_missing},
};
use tmt_adapters::{
    host::{CallerEnvironment, Host},
    storage::Storage,
};
use tmt_core::{binding, identity::Identity};

pub fn expect_name(identity: &Identity, expected: &str) -> Result<(), Failure> {
    if identity.canonical_name != tmt_core::names::normalize_name(expected) {
        return Err(Failure::new(
            "IDENTITY_CHANGED",
            format!(
                "The identity {} is now named '{}', not '{expected}'.",
                identity.id, identity.name
            ),
            5,
        ));
    }
    Ok(())
}

/// A name pin reads the UUID directly; retired IDs never fall back to names.
pub fn pinned_identity(
    storage: &mut Storage,
    id: &str,
    expected: &str,
) -> Result<Identity, Failure> {
    let unavailable = |error| {
        Failure::new(
            "IDENTITY_ERROR",
            "Could not complete the identity operation.",
            1,
        )
        .caused_by(error)
    };
    let identity = storage
        .find_active_identity_by_id(id)
        .map_err(unavailable)?;
    let Some(identity) = identity else {
        return if storage
            .find_identity_by_id(id)
            .map_err(unavailable)?
            .is_some()
        {
            Err(Failure::new(
                "IDENTITY_CHANGED",
                format!("The identity {id} has been retired."),
                5,
            ))
        } else {
            Err(identity_missing(id))
        };
    };
    expect_name(&identity, expected)?;
    Ok(identity)
}

pub enum Selector {
    Explicit(String),
    /// A session-pinned UUID; never fall back to a display name.
    SavedId(String),
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
/// A durable selector never discovers the caller's host.
pub fn required(explicit: Option<&str>) -> Result<Selector, Failure> {
    if let Some(name) = explicit {
        return Ok(Selector::Explicit(name.to_owned()));
    }
    select(
        &crate::caller_context::host(&CallerEnvironment::current())?,
        false,
    )?
    .ok_or_else(required_failure)
}

/// Only a pane selector discovers the caller's host; durable selectors read
/// storage alone.
pub fn resolve(storage: &mut Storage, selector: Selector) -> Result<Identity, Failure> {
    selected(
        storage,
        || crate::caller_context::host(&CallerEnvironment::current()),
        selector,
    )?
    .ok_or_else(required_failure)
}

pub fn optional(
    storage: &mut Storage,
    host: impl FnOnce() -> Result<Host, Failure>,
    explicit: Option<&str>,
) -> Result<Option<Identity>, Failure> {
    if let Some(name) = explicit {
        return selected(storage, host, Selector::Explicit(name.to_owned()));
    }
    let host = host()?;
    match select(&host, true)? {
        Some(selector) => selected(storage, || Ok(&host), selector),
        None => Ok(None),
    }
}

fn select(host: &Host, allow_anonymous: bool) -> Result<Option<Selector>, Failure> {
    if let Err(error) = crate::caller_context::require_independent_host() {
        // Anonymous delivery remains valid, but must not acquire the identity
        // of an unrelated conversation through the shared host's pane.
        return if allow_anonymous && !error.has_runtime_refusal() {
            // This is an attribution notice, not a delivery-success claim.
            // Emit in JSON mode too without changing the stdout document.
            let mut stderr = tmt_cli_style::stream::stderr();
            let terminal = stderr.terminal();
            let _ = tmt_cli_style::message::warning(
                &mut stderr,
                terminal,
                "Sender identity not established on a shared runtime host; using an anonymous sender.",
                Some("pass --identity <name>"),
            );
            Ok(None)
        } else {
            Err(error)
        };
    }
    host.caller_pane(&CallerEnvironment::current())
        .map(|pane| pane.map(Selector::Pane))
        .map_err(endpoint_failure)
}

fn selected<H: std::borrow::Borrow<Host>>(
    storage: &mut Storage,
    host: impl FnOnce() -> Result<H, Failure>,
    selector: Selector,
) -> Result<Option<Identity>, Failure> {
    match selector {
        Selector::SavedId(id) => {
            let identity = storage
                .find_active_identity_by_id(&id)
                .map_err(|error| {
                    Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1)
                        .caused_by(error)
                })?
                .ok_or_else(|| identity_missing(&id))?;
            if identity.lifetime != tmt_core::identity::Lifetime::Saved {
                return Err(Failure::new(
                    "MCP_SAVED_IDENTITY_REQUIRED",
                    "MCP requires an existing saved identity.",
                    1,
                ));
            }
            Ok(Some(identity))
        }
        Selector::Explicit(name) => {
            let selected = storage.resolve_identity(&name).map_err(|error| {
                Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1)
                    .caused_by(error)
            })?;
            selected.ok_or_else(|| identity_missing(&name)).map(Some)
        }
        Selector::Pane(pane) => {
            let host = host()?;
            let host = host.borrow();
            host.resolve_servers(storage).map_err(endpoint_failure)?;
            let observed = binding::pane_presence(storage, &mut host.session(), &pane)
                .map_err(binding_failure)?;
            Ok(observed.identity)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caller_context::test_support::{World, cleanup_failure};
    use std::cell::Cell;
    use tmt_core::identity::{Lifetime, create_or_resolve};

    #[test]
    fn an_explicit_optional_identity_does_not_acquire_the_callers_host() {
        let world = World::new();
        let mut storage = world.storage();
        let expected = create_or_resolve(&mut storage, "Sender", Lifetime::Saved)
            .unwrap()
            .identity;
        let calls = Cell::new(0);
        let selected = optional(
            &mut storage,
            || {
                calls.set(calls.get() + 1);
                Err(cleanup_failure())
            },
            Some("Sender"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.id, expected.id);
        assert_eq!(calls.get(), 0);
        storage.close().unwrap();
    }

    #[test]
    fn a_missing_explicit_identity_keeps_its_error_without_caller_discovery() {
        let world = World::new();
        let mut storage = world.storage();
        let missing = optional(
            &mut storage,
            || panic!("explicit name discovered caller"),
            Some("Missing"),
        )
        .unwrap_err();
        assert_eq!(
            (missing.code.as_str(), missing.status),
            ("NAME_NOT_FOUND", 3)
        );
        storage.close().unwrap();
    }

    #[test]
    fn an_implicit_optional_identity_preserves_the_selection_cleanup_failure() {
        let world = World::new();
        let mut storage = world.storage();
        let expected = create_or_resolve(&mut storage, "Sender", Lifetime::Saved)
            .unwrap()
            .identity;
        let failure = optional(&mut storage, || Err(cleanup_failure()), None).unwrap_err();
        assert_eq!(failure.document(), cleanup_failure().document());
        assert_eq!(failure.status, 1);
        assert_eq!(
            storage.resolve_identity("Sender").unwrap().unwrap(),
            expected
        );
        storage.close().unwrap();
    }
}
