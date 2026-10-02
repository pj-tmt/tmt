//! Current-server pane-first routing shared by diagnostic and delivery commands.

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    output::Failure,
};
use tmt_adapters::{
    host::{Host, OperationOptions},
    storage::Storage,
};
use tmt_core::{
    binding::{self, PaneIdentity},
    endpoint::EndpointSnapshot,
    request::RequestEndpoint,
};

#[cfg(test)]
mod tests;

pub fn resolve(storage: &mut Storage, host: &Host, input: &str) -> Result<PaneIdentity, Failure> {
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
    // A name routes only to a binding on the caller's own server; one on
    // another host or socket reads as not active. That server is resolved
    // first, as for a pane (a no-op on tmux).
    host.resolve_servers(storage).map_err(endpoint_failure)?;
    binding::current_name_presence(storage, &mut host.session(), input)
        .map_err(binding_failure)?
        .ok_or_else(|| {
            Failure::new(
                "NAME_NOT_FOUND",
                format!("Identity '{input}' is not active."),
                3,
            )
        })
}

/// A fresh observation before preparation is not a lease on later processing.
pub fn refresh(observed: &PaneIdentity) -> Result<RequestEndpoint, Failure> {
    let scope = [observed.pane.id.clone()];
    let snapshot = Host::for_server(&observed.server)
        .snapshot(OperationOptions {
            pane_ids: Some(&scope),
            ..Default::default()
        })
        .map_err(endpoint_failure)?;
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
