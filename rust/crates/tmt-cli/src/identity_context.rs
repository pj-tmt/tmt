//! Durable selectors are not pane routing or identity creation.

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    caller_context::SelectCaller,
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
    required_with(explicit, &Host::for_caller)
}

/// [`required`] with the caller selection a command was given.
pub fn required_with(
    explicit: Option<&str>,
    select: SelectCaller<'_>,
) -> Result<Selector, Failure> {
    if let Some(name) = explicit {
        return Ok(Selector::Explicit(name.to_owned()));
    }
    caller_pane(select, false)?
        .map(|(_, pane)| Selector::Pane(pane))
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

/// An optional identity's source. Only an implicit sender selects the
/// caller's host, before storage opens; a durable name reads storage alone.
pub enum Sender {
    Explicit(String),
    Caller(Box<Host>, CallerEnvironment),
}

pub fn sender(explicit: Option<&str>, select: SelectCaller<'_>) -> Result<Sender, Failure> {
    if let Some(name) = explicit {
        return Ok(Sender::Explicit(name.to_owned()));
    }
    let environment = CallerEnvironment::current();
    let host = select(&environment).map_err(crate::caller_context::selection_failed)?;
    Ok(Sender::Caller(Box::new(host), environment))
}

/// The sender's identity. An implicit sender's pane is read only here, after
/// the command has resolved its target, so a lookup miss reads no pane.
pub fn optional(storage: &mut Storage, sender: Sender) -> Result<Option<Identity>, Failure> {
    match sender {
        Sender::Explicit(name) => explicit(storage, &name).map(Some),
        Sender::Caller(host, environment) => match pane_of(*host, &environment, true)? {
            Some((host, pane)) => selected(storage, || Ok(&host), Selector::Pane(pane)),
            None => Ok(None),
        },
    }
}

/// The caller's host and its pane, or no pane.
fn caller_pane(
    select: SelectCaller<'_>,
    allow_anonymous: bool,
) -> Result<Option<(Host, String)>, Failure> {
    let environment = CallerEnvironment::current();
    let host = select(&environment).map_err(crate::caller_context::selection_failed)?;
    pane_of(host, &environment, allow_anonymous)
}

/// The selected host's caller pane, unless the host is a shared runtime host.
fn pane_of(
    host: Host,
    environment: &CallerEnvironment,
    allow_anonymous: bool,
) -> Result<Option<(Host, String)>, Failure> {
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
    let pane = host.caller_pane(environment).map_err(endpoint_failure)?;
    Ok(pane.map(|pane| (host, pane)))
}

fn explicit(storage: &mut Storage, name: &str) -> Result<Identity, Failure> {
    let selected = storage.resolve_identity(name).map_err(|error| {
        Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1).caused_by(error)
    })?;
    selected.ok_or_else(|| identity_missing(name))
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
        Selector::Explicit(name) => explicit(storage, &name).map(Some),
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
