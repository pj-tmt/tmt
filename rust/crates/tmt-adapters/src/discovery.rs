//! Finite read-only candidate acquisition and annotation through owning ports.

use crate::{
    host::{Host, HostError},
    process::CommandRunner,
    storage::{Storage, StorageError},
};
use std::{path::Path, time::Instant};
use tmt_core::{
    binding::{self, BindingEvidence},
    discovery::{DiscoveryCandidate, DiscoveryObservation, DiscoverySource},
    endpoint::EndpointProbe,
    host::{HostKind, HostServerIncarnation},
};

pub fn annotate(
    observation: &mut DiscoveryObservation,
    database: &Path,
    deadline: Instant,
) -> Result<(), StorageError> {
    // A fresh installation has no stored identities. Never open/create a
    // database. Each read starts only within the scan's allowance; a spent
    // allowance truncates the scan instead.
    let mut entries = Vec::new();
    if Instant::now() < deadline {
        let truncated;
        (entries, truncated) =
            Storage::discovery_entries(database, observation.host, &observation.socket, deadline)?;
        observation.truncated |= truncated;
    }
    // An external host keeps no server ID of its own: a bind recorded it.
    // The fences of one scan name the one server it observed, read once.
    let server = observation
        .candidates
        .iter()
        .find_map(|candidate| candidate.fence.as_ref())
        .filter(|fence| fence.host != HostKind::Tmux)
        .map(|fence| (fence.host, fence.socket.clone(), fence.server.clone()));
    if let Some((host, socket, server)) = server
        && Instant::now() < deadline
    {
        let id =
            crate::storage::RecordedHostServers::new(database).lookup(&HostServerIncarnation {
                host,
                socket_path: &socket,
                server_pid: server.pid(),
                server_start_time: server.start_identity(),
            })?;
        for fence in observation
            .candidates
            .iter_mut()
            .filter_map(|candidate| candidate.fence.as_mut())
            .filter(|fence| fence.socket == socket && fence.server == server)
        {
            fence.server_id.clone_from(&id);
        }
    }
    // Past the deadline no expectation is usable, whatever was read.
    if Instant::now() >= deadline {
        observation.truncated = true;
    }
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
    let mut observation = host.discover(socket, Some(pane), deadline)?;
    if let DiscoverySource::Unsupported(reason) = observation.source {
        return Err(HostError::Refused {
            driver: observation.host.as_str().into(),
            code: tmt_driver_protocol::ErrorCode::Unsupported,
            message: format!("the host cannot list its panes ({reason})"),
        });
    }
    annotate(&mut observation, database, deadline)
        .map_err(|e| HostError::ServerIds(Box::new(e)))?;
    let driver = observation.host.as_str().into();
    observation
        .candidates
        .into_iter()
        .find(|c| {
            c.pane == pane
                && c.adoptable()
                && c.fence.as_ref().is_some_and(|f| f.digest() == expect)
        })
        .ok_or_else(|| HostError::Evidence {
            driver,
            reason: "discovery target changed or cannot be verified",
        })
}

/// A failed tmux observation's source status; it does not infer endpoint
/// absence from diagnostic text. An external host's failure is always
/// `unknown`.
pub fn tmux_source_status(error: &HostError, socket: Option<&str>) -> &'static str {
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

#[cfg(test)]
mod tests {
    use tmt_core::{
        discovery::DiscoveryTarget,
        endpoint::{BindingMarker, ProcessIncarnation},
        host::HostKind,
    };

    /// One digest block: each vector overrides its block's `base`.
    fn check(block: &serde_json::Value) -> usize {
        let vectors = block["vectors"].as_array().unwrap();
        for vector in vectors {
            let mut value = block["base"].clone();
            for (key, field) in vector.as_object().unwrap() {
                value[key] = field.clone();
            }
            let marker = &value["marker"];
            let target = DiscoveryTarget {
                host: HostKind::parse(value["host"].as_str().unwrap()).unwrap(),
                socket: value["socket"].as_str().unwrap().into(),
                server: ProcessIncarnation::new(
                    value["serverPid"].as_u64().unwrap(),
                    value["serverStartIdentity"].as_str().unwrap(),
                )
                .unwrap(),
                server_id: value["serverId"].as_str().map(str::to_owned),
                server_start_time: value["serverStartTime"].as_str().unwrap().into(),
                pane: value["pane"].as_str().unwrap().into(),
                process: ProcessIncarnation::new(
                    value["panePid"].as_u64().unwrap(),
                    value["paneStartIdentity"].as_str().unwrap(),
                )
                .unwrap(),
                runtime: value["runtime"].as_str().map(str::to_owned),
                marker: (!marker.is_null()).then(|| {
                    let name = marker["name"].as_str().unwrap_or("cosmetic");
                    BindingMarker {
                        name: name.into(),
                        canonical_name: name.into(),
                        identity_id: marker["identityId"].as_str().unwrap().into(),
                        binding_id: marker["bindingId"].as_str().unwrap().into(),
                        server_id: marker["serverId"].as_str().unwrap().into(),
                        pane_pid: marker["panePid"].as_u64().unwrap(),
                    }
                }),
            };
            if target.host != HostKind::Tmux {
                // An external server's start time is core's own start token.
                assert_eq!(
                    target.server_start_time,
                    target.server.start_identity(),
                    "{}",
                    vector["name"]
                );
            }
            assert_eq!(
                target.digest(),
                vector["expect"].as_str().unwrap(),
                "{}",
                vector["name"]
            );
        }
        vectors.len()
    }

    #[test]
    fn discovery_digest_matches_normative_first_run_and_ownership_vectors() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../../../contracts/discovery-v1.json")).unwrap();
        let fence = &contract["digestFence"];
        assert_eq!(fence["base"]["host"], HostKind::Tmux.as_str());
        assert_eq!(check(fence), 5);
        let external = &fence["externalVectors"];
        assert_ne!(external["base"]["host"], HostKind::Tmux.as_str());
        assert_eq!(check(external), 5);
    }

    /// A scan of three fenced panes on one external server, whose server ID
    /// a bind recorded, in a store with no binding for them.
    #[test]
    fn a_spent_allowance_truncates_annotation_and_leaves_no_expectation() {
        use std::time::{Duration, Instant};
        use tmt_core::{
            discovery::{DiscoveryCandidate, DiscoveryObservation, DiscoverySource},
            host::{HostServerIds, HostServerIncarnation},
        };
        let directory = crate::test_support::TestDirectory::new();
        let database = directory.path.join("state.db");
        let herdr = HostKind::parse("herdr").unwrap();
        let server = ProcessIncarnation::new(41, "Tue 22:09").unwrap();
        let mut storage = crate::storage::Storage::open(&database).unwrap();
        let recorded = storage
            .server_id(&HostServerIncarnation {
                host: herdr,
                socket_path: "/tmp/herdr.sock",
                server_pid: 41,
                server_start_time: "Tue 22:09",
            })
            .unwrap();
        storage.close().unwrap();
        let candidate = |pane: &str, pid: u64| DiscoveryCandidate {
            pane: pane.into(),
            session: None,
            window: None,
            target: None,
            title: String::new(),
            command: String::new(),
            cwd: String::new(),
            agent_name: None,
            fence: Some(DiscoveryTarget {
                host: herdr,
                socket: "/tmp/herdr.sock".into(),
                server: server.clone(),
                server_id: None,
                server_start_time: "Tue 22:09".into(),
                pane: pane.into(),
                process: ProcessIncarnation::new(pid, "Tue 22:10").unwrap(),
                runtime: None,
                marker: None,
            }),
            identity: None,
            state: "unknown",
            reason: None,
            duplicate: false,
            text_truncated: false,
        };
        let observation = || DiscoveryObservation {
            host: herdr,
            source: DiscoverySource::Ok,
            socket: "/tmp/herdr.sock".into(),
            candidates: vec![
                candidate("term_1", 51),
                candidate("term_2", 52),
                candidate("term_3", 53),
            ],
            truncated: false,
        };
        // Within the allowance: one recorded ID for every fence, all adoptable.
        let mut within = observation();
        super::annotate(
            &mut within,
            &database,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert!(!within.truncated);
        for candidate in &within.candidates {
            assert!(candidate.adoptable(), "{}", candidate.pane);
            assert_eq!(
                candidate.fence.as_ref().unwrap().server_id,
                Some(recorded.clone())
            );
        }
        // Spent: nothing more is read, the scan is truncated, and no row is
        // adoptable, so no expectation is offered.
        let mut spent = observation();
        super::annotate(&mut spent, &database, Instant::now()).unwrap();
        assert!(spent.truncated);
        for candidate in &spent.candidates {
            assert!(!candidate.adoptable(), "{}", candidate.pane);
            assert_eq!(candidate.reason, Some("scan_truncated"));
            assert_eq!(candidate.fence.as_ref().unwrap().server_id, None);
        }
    }
}
