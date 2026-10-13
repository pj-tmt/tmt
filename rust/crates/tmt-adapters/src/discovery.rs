//! Finite read-only candidate acquisition and annotation through owning ports.

use crate::{
    host::{DiscoveryObservation, Host, HostError},
    process::CommandRunner,
    storage::{Storage, StorageError},
};
use std::{path::Path, time::Instant};
use tmt_core::{
    binding::{self, BindingEvidence},
    discovery::DiscoveryCandidate,
    endpoint::EndpointProbe,
};

pub fn annotate(
    observation: &mut DiscoveryObservation,
    database: &Path,
    deadline: Instant,
) -> Result<(), StorageError> {
    // A fresh installation has no stored identities. Never open/create a database.
    let (entries, truncated) = Storage::discovery_entries(database, &observation.socket, deadline)?;
    observation.truncated |= truncated;
    for candidate in &mut observation.candidates {
        let owners: Vec<_> = entries
            .iter()
            .filter(|e| {
                e.binding
                    .as_ref()
                    .is_some_and(|b| b.pane_id == candidate.pane)
            })
            .collect();
        if let [owner] = owners.as_slice() {
            if let Some(fence) = &candidate.fence {
                let probe = fence
                    .endpoint_snapshot()
                    .map_or(EndpointProbe::Unknown, EndpointProbe::Live);
                match binding::evaluate_binding(owner, &probe) {
                    BindingEvidence::Active(_) => {
                        candidate.identity = Some((*owner).clone());
                        candidate.state = "known";
                    }
                    _ => {
                        candidate.state = "stale";
                        candidate.reason = Some("stored_binding_stale");
                    }
                }
            }
        } else if owners.len() > 1 {
            candidate.state = "duplicate";
            candidate.duplicate = true;
            candidate.reason = Some("conflicting_bindings");
        }
        if owners.is_empty()
            && candidate.identity.is_none()
            && candidate.fence.as_ref().is_some_and(|f| f.marker.is_some())
        {
            candidate.state = "hook_only";
            candidate.reason = Some("unverified_marker");
        }
        if observation.truncated {
            candidate.reason = Some("scan_truncated");
        }
    }
    Ok(())
}

/// One explicit target. No effects, and refusal is never evidence of absence.
pub fn guarded_candidate<R: CommandRunner>(
    host: &Host<R>,
    socket: Option<&str>,
    pane: &str,
    database: &Path,
    expect: &str,
    deadline: Instant,
) -> Result<DiscoveryCandidate, HostError> {
    let mut observation = host
        .discover(socket, Some(pane), deadline)?
        .ok_or_else(|| HostError::Unavailable("discovery is supported only on tmux".into()))?;
    annotate(&mut observation, database, deadline)
        .map_err(|e| HostError::ServerIds(Box::new(e)))?;
    observation
        .candidates
        .into_iter()
        .find(|c| {
            c.pane == pane
                && c.adoptable()
                && c.fence.as_ref().is_some_and(|f| f.digest() == expect)
        })
        .ok_or_else(|| HostError::Evidence {
            driver: tmt_core::host::HostKind::Tmux.as_str().into(),
            reason: "discovery target changed or cannot be verified",
        })
}

/// Observation failure does not infer endpoint absence from diagnostic text.
pub fn source_status(error: &HostError, socket: Option<&str>) -> &'static str {
    if error.socket_permission_denied() {
        return "permission_denied";
    }
    if socket.is_some_and(|path| {
        std::fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    }) {
        return "offline";
    }
    "unknown"
}
