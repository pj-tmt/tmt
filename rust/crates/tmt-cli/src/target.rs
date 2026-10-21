//! Pane syntax and recorded-recipient routing shared by diagnostics and delivery.

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    output::Failure,
};
use tmt_adapters::{
    host::{Host, OperationOptions},
    storage::Storage,
};
use tmt_core::{
    binding::{self, BindingRepository, PaneIdentity},
    endpoint::EndpointSnapshot,
    identity::IdentityReader,
    request::RequestEndpoint,
};

#[cfg(test)]
mod tests;

/// Focus alone accepts a canonical UUIDv4 before considering names or panes.
pub fn resolve_focus(storage: &mut Storage, input: &str) -> Result<PaneIdentity, Failure> {
    if !focus_identity_id(input) {
        return resolve(storage, input);
    }
    resolve_identity(storage, input, input)
}

fn focus_identity_id(input: &str) -> bool {
    // The existing predicates jointly require canonical lowercase spelling,
    // version 4 and the RFC4122 variant; no UUID dependency is needed here.
    tmt_core::dispatch::canonical_id(input) && tmt_core::endpoint::valid_server_id(input)
}

pub fn resolve(storage: &mut Storage, input: &str) -> Result<PaneIdentity, Failure> {
    let pane = tmt_core::identity::addresses_pane(storage, input).map_err(|error| {
        Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1).caused_by(error)
    })?;
    if pane {
        let host = Host::for_target(input);
        host.resolve_servers(storage).map_err(endpoint_failure)?;
        let mut endpoint = host.session();
        let pane = host
            .resolve_target(input, OperationOptions::default())
            .map_err(endpoint_failure)?
            .ok_or_else(|| {
                Failure::new(
                    "PANE_NOT_FOUND",
                    format!("Pane target '{input}' was not found."),
                    3,
                )
            })?;
        return binding::pane_presence(storage, &mut endpoint, &pane).map_err(binding_failure);
    }
    let identity = storage
        .find_identity(&tmt_core::names::normalize_name(input))
        .map_err(|error| {
            Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1).caused_by(error)
        })?;
    let Some(identity) = identity else {
        return Err(missing_identity(input, false));
    };
    resolve_identity(storage, &identity.id, input)
}

fn missing_identity(input: &str, exists: bool) -> Failure {
    let message = if exists {
        format!("Identity '{input}' is not active.")
    } else {
        format!("Identity '{input}' was not found.")
    };
    Failure::new("NAME_NOT_FOUND", message, 3)
}

fn resolve_identity(storage: &mut Storage, id: &str, input: &str) -> Result<PaneIdentity, Failure> {
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(id))
        .map_err(|error| {
            Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1).caused_by(error)
        })?;
    let Some(entry) = entry else {
        return Err(missing_identity(input, false));
    };
    let Some(binding) = entry.binding else {
        return Err(missing_identity(input, true));
    };
    let host = Host::for_server(&binding.server);
    // Core captures the current record again and reconciles only unchanged
    // ownership against fresh evidence. A locator alone never authorizes input.
    let presence =
        binding::current_identity_presence(storage, &mut host.session(), id).map_err(|error| {
            match error {
                binding::BindingError::Endpoint(error) => recipient_failure(&binding.server, error),
                error => binding_failure(error),
            }
        })?;
    if let Some(presence) = presence {
        return Ok(presence);
    }
    // Reconciliation can retire a temporary identity. Classify the refusal
    // from the committed record, never the entry captured before observation.
    let exists = storage
        .with_binding_transaction(|records| records.entry_by_id(id).map(|entry| entry.is_some()))
        .map_err(|error| {
            Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1).caused_by(error)
        })?;
    Err(missing_identity(input, exists))
}

fn recipient_failure(
    server: &tmt_core::endpoint::ServerEvidence,
    error: tmt_adapters::host::HostError,
) -> Failure {
    use tmt_adapters::host::HostError;
    if let (tmt_core::host::HostKind::External(name), HostError::Unavailable(_)) =
        (server.host, &error)
    {
        return Failure::new(
            "RECONCILIATION_FAILED",
            format!(
                "Host driver '{}' is unavailable or not approved. No pane input occurred.",
                name.as_str()
            ),
            1,
        )
        .caused_by(error)
        .suggestion("Run tmt driver install <driver-path> to review and approve the driver, then retry this command.".into());
    }
    if error.socket_permission_denied() {
        return endpoint_failure(error);
    }
    endpoint_failure(error).suggestion(format!(
        "Restore access to the {} server at '{}' and retry this command.",
        server.host.as_str(),
        server.socket_path
    ))
}

/// A fresh observation before preparation is not a lease on later processing.
pub fn refresh(observed: &PaneIdentity) -> Result<RequestEndpoint, Failure> {
    let scope = [observed.pane.id.clone()];
    let snapshot = Host::for_server(&observed.server)
        .snapshot(OperationOptions {
            pane_ids: Some(&scope),
            ..Default::default()
        })
        .map_err(|error| recipient_failure(&observed.server, error))?;
    refreshed_endpoint(observed, snapshot)
}

fn refreshed_endpoint(
    observed: &PaneIdentity,
    snapshot: EndpointSnapshot,
) -> Result<RequestEndpoint, Failure> {
    use tmt_core::{
        binding::{BindingEntry, BindingEvidence, evaluate_binding},
        endpoint::EndpointProbe,
    };
    let pane = if let Some(identity) = &observed.identity {
        let entry = BindingEntry {
            identity: identity.clone(),
            binding: observed.binding.clone(),
        };
        match evaluate_binding(&entry, &EndpointProbe::Live(snapshot.clone())) {
            BindingEvidence::Active(pane) => *pane,
            _ => {
                return Err(Failure::new(
                    "RECONCILIATION_FAILED",
                    "Recipient identity changed before request preparation.",
                    1,
                ));
            }
        }
    } else {
        snapshot
            .panes
            .iter()
            .find(|pane| pane.id == observed.pane.id)
            .cloned()
            .ok_or_else(|| {
                Failure::new(
                    "PANE_NOT_FOUND",
                    "Recipient pane disappeared before request preparation.",
                    3,
                )
            })?
    };
    Ok(RequestEndpoint {
        server: snapshot.server,
        pane_id: pane.id,
        pane_pid: pane.pane_pid,
    })
}

// Source-checked command sample for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Run tmt driver install <driver-path> to review and approve the driver, then retry this command.",
        &[" to review"],
        &[],
    ),
];
