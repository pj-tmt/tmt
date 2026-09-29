use super::*;
use crate::{
    host::{HostKind, ServerSelector},
    identity::{self, IdentityError, Lifetime},
    names::{normalize_name, validate_existing_name},
};
use std::collections::BTreeMap;

// Presentation priority followed by complete endpoint evidence. Keep grouping
// and lookup keyed identically without allocating a second server registry.
fn server_key(
    server: &ServerEvidence,
    current: Option<ServerSelector<'_>>,
) -> (bool, HostKind, String, String, u64, String) {
    let selector = ServerSelector {
        host: server.host,
        socket: &server.socket_path,
    };
    (
        current != Some(selector),
        server.host,
        server.socket_path.clone(),
        server.server_id.clone(),
        server.server_pid,
        server.server_start_time.clone(),
    )
}

/// A malformed marker is not proof that a process died. A changed server ID
/// alone likewise cannot prove that the recorded server process was replaced.
pub fn evaluate_binding(entry: &BindingEntry, probe: &EndpointProbe) -> BindingEvidence {
    let Some(binding) = &entry.binding else {
        return BindingEvidence::Unknown;
    };
    let snapshot = match probe {
        EndpointProbe::Dead => return BindingEvidence::EndpointLost,
        EndpointProbe::Unknown => return BindingEvidence::Unknown,
        EndpointProbe::Live(snapshot) => snapshot,
    };
    // Another host, or another socket, cannot speak for this binding.
    if snapshot.server.host != binding.server.host
        || snapshot.server.socket_path != binding.server.socket_path
    {
        return BindingEvidence::Unknown;
    }
    if snapshot.server.server_pid != binding.server.server_pid
        || snapshot.server.server_start_time != binding.server.server_start_time
    {
        return BindingEvidence::EndpointLost;
    }
    if snapshot.server.server_id != binding.server.server_id {
        return BindingEvidence::Unknown;
    }
    let Some(pane) = snapshot
        .panes
        .iter()
        .find(|pane| pane.id == binding.pane_id)
    else {
        return BindingEvidence::EndpointLost;
    };
    if pane.pane_pid != binding.pane_pid {
        return BindingEvidence::EndpointLost;
    }
    let Some(marker) = &pane.marker else {
        return BindingEvidence::MarkerMismatch;
    };
    // The IDs prove ownership. The name is informational: it must be a
    // well-formed, self-consistent name, but a rename leaves it stale until the
    // next cosmetic refresh rewrites it, and a stale name never detaches.
    let valid = validate_existing_name(&marker.name)
        .is_ok_and(|name| name.canonical_name() == marker.canonical_name);
    if !valid
        || binding.identity_id != entry.identity.id
        || marker.identity_id != entry.identity.id
        || marker.binding_id != binding.id
        || marker.server_id != binding.server.server_id
        || marker.pane_pid != binding.pane_pid
    {
        return BindingEvidence::MarkerMismatch;
    }
    BindingEvidence::Active(Box::new(pane.clone()))
}

pub(super) fn named_entry<R, O>(
    records: &dyn BindingRecords<Error = R>,
    name: &str,
) -> Result<Option<BindingEntry>, BindingError<R, O>> {
    let identity = identity::find_by_name(records, name).map_err(|error| match error {
        IdentityError::InvalidName(error) => BindingError::InvalidName(error),
        IdentityError::Repository(error) => BindingError::Repository(error),
    })?;
    identity
        .map(|identity| records.entry_by_id(&identity.id))
        .transpose()
        .map(Option::flatten)
        .map_err(BindingError::Repository)
}

pub(super) fn reconcile<R, O>(
    records: &mut dyn BindingRecords<Error = R>,
    entry: BindingEntry,
    probe: &EndpointProbe,
) -> Result<Option<IdentityPresence>, BindingError<R, O>> {
    let Some(binding) = &entry.binding else {
        return Ok(Some(IdentityPresence {
            identity: entry.identity,
            presence: Presence::Offline,
            pane: None,
            binding: None,
        }));
    };
    let (presence, pane) = match evaluate_binding(&entry, probe) {
        BindingEvidence::Active(pane) => {
            records.touch_binding(&binding.id)?;
            (Presence::Active, Some(*pane))
        }
        BindingEvidence::EndpointLost if entry.identity.lifetime == Lifetime::Temporary => {
            records.retire_identity(&entry.identity, false)?;
            return Ok(None);
        }
        BindingEvidence::EndpointLost | BindingEvidence::MarkerMismatch => {
            records.detach_binding(&binding.id)?;
            (Presence::Offline, None)
        }
        BindingEvidence::Unknown => (Presence::Unknown, None),
    };
    let binding = (presence == Presence::Active).then_some(binding.clone());
    Ok(Some(IdentityPresence {
        identity: entry.identity,
        presence,
        pane,
        binding,
    }))
}

pub(super) fn observe<O: BindingEndpoint>(
    endpoint: &mut O,
    entry: &BindingEntry,
) -> Result<EndpointProbe, O::Error> {
    match &entry.binding {
        Some(binding) if endpoint.budget_available() => {
            endpoint.probe_binding(&binding.server, std::slice::from_ref(&binding.pane_id))
        }
        _ => Ok(EndpointProbe::Unknown),
    }
}

pub fn name_presence<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    name: &str,
) -> Result<IdentityPresence, BindingError<R::Error, O::Error>> {
    repository
        .with_binding_transaction(|records| {
            endpoint.begin_coordination();
            let entry = named_entry(records, name)?
                .ok_or_else(|| BindingError::NameNotFound(name.into()))?;
            let probe = observe(endpoint, &entry).map_err(BindingError::Endpoint)?;
            // Commit conclusive retirement even when this read no longer finds a
            // live-name record. Convert absence to the public error after commit.
            reconcile(records, entry, &probe)
        })?
        .ok_or_else(|| BindingError::NameNotFound(name.into()))
}

pub fn pane_presence<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
) -> Result<PaneIdentity, BindingError<R::Error, O::Error>> {
    repository.with_binding_transaction(|records| {
        endpoint.begin_coordination();
        let snapshot = endpoint
            .current_snapshot(&[pane_id.into()])
            .map_err(BindingError::Endpoint)?;
        let pane = snapshot
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)
            .cloned()
            .ok_or_else(|| BindingError::PaneNotFound(pane_id.into()))?;
        let entry =
            records.entry_by_pane(snapshot.server.host, pane_id, &snapshot.server.server_id)?;
        let server = snapshot.server.clone();
        let active = match entry {
            Some(entry) => reconcile(records, entry, &EndpointProbe::Live(snapshot))?
                .filter(|row| row.presence == Presence::Active),
            None => None,
        };
        let (identity, binding) =
            active.map_or((None, None), |row| (Some(row.identity), row.binding));
        Ok(PaneIdentity {
            server,
            pane,
            identity,
            binding,
        })
    })
}

/// Routing is scoped to the current server, unlike global presence reporting.
/// Lookup does not impose creation-name validation or probe a foreign server.
pub fn current_name_presence<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    name: &str,
) -> Result<Option<PaneIdentity>, BindingError<R::Error, O::Error>> {
    repository.with_binding_transaction(|records| {
        endpoint.begin_coordination();
        let Some(identity) = records.find_identity(&normalize_name(name))? else {
            return Ok(None);
        };
        let Some(entry) = records.entry_by_id(&identity.id)? else {
            return Ok(None);
        };
        let Some(binding) = &entry.binding else {
            return Ok(None);
        };
        let snapshot = endpoint
            .current_snapshot(std::slice::from_ref(&binding.pane_id))
            .map_err(BindingError::Endpoint)?;
        let server = snapshot.server.clone();
        let active = reconcile(records, entry, &EndpointProbe::Live(snapshot))?
            .filter(|row| row.presence == Presence::Active);
        Ok(active.and_then(|row| {
            row.pane.map(|pane| PaneIdentity {
                server,
                pane,
                identity: Some(row.identity),
                binding: row.binding,
            })
        }))
    })
}

/// One joined repository read and one bounded observation per recorded server,
/// never one process per identity. Unknown or exhausted observations preserve
/// their rows; output ordering stays the repository's canonical BINARY order.
pub fn list_presence<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    current_server: Option<ServerSelector<'_>>,
) -> Result<Vec<IdentityPresence>, BindingError<R::Error, O::Error>> {
    repository.with_binding_transaction(|records| {
        endpoint.begin_coordination();
        let entries = records.binding_entries()?;
        let mut groups = BTreeMap::<_, (ServerEvidence, Vec<String>)>::new();
        for entry in &entries {
            if let Some(binding) = &entry.binding {
                let server = &binding.server;
                let key = server_key(server, current_server);
                groups
                    .entry(key)
                    .or_insert_with(|| (server.clone(), Vec::new()))
                    .1
                    .push(binding.pane_id.clone());
            }
        }
        let mut probes = BTreeMap::new();
        for (key, (server, panes)) in groups {
            let probe = if endpoint.budget_available() {
                endpoint
                    .probe_binding(&server, &panes)
                    .map_err(BindingError::Endpoint)?
            } else {
                EndpointProbe::Unknown
            };
            probes.insert(key, probe);
        }
        let mut result = Vec::new();
        for entry in entries {
            let probe = entry
                .binding
                .as_ref()
                .and_then(|binding| probes.get(&server_key(&binding.server, current_server)))
                .unwrap_or(&EndpointProbe::Unknown);
            if let Some(row) = reconcile(records, entry, probe)? {
                result.push(row);
            }
        }
        Ok(result)
    })
}
