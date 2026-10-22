//! First-run inventory of an external host through driver protocol v1
//! `server` and `snapshot` (`contracts/discovery-v1.md`). It records nothing:
//! core reads the server and pane start tokens itself, and only a guarded
//! bind records a server, after checking it again
//! ([`Drivers::resolve_fenced_server`]).

use super::{
    Drivers,
    session::{driver_process, observation, refused},
};
use crate::{
    host::HostError,
    process::{CommandRunner, runtime::observe_starts},
};
use std::{collections::BTreeMap, time::Instant};
use tmt_core::{
    discovery::{
        DiscoveryCandidate, DiscoveryObservation, DiscoverySource, DiscoveryTarget, MAX_CANDIDATES,
        display_text,
    },
    endpoint::{ProcessIncarnation, ServerEvidence, valid_server_id},
    host::{HostKind, HostName, HostServerIds, HostServerIncarnation},
};
use tmt_driver_protocol::{Op, ServerRequest, ServerResponse, SnapshotRequest, SnapshotResponse};

impl<R: CommandRunner> Drivers<R> {
    /// The panes of one server of `host`: `socket`, else the caller's own
    /// server on this host, else the driver's default. A host without an
    /// approved driver, or whose driver lists no panes, is `Unsupported`, and
    /// no server is `Offline`; both run nothing more. Any other failure is an
    /// error, never evidence that the server is gone.
    pub(crate) fn discover(
        &self,
        host: HostName,
        socket: Option<&str>,
        pane: Option<&str>,
        deadline: Instant,
    ) -> Result<DiscoveryObservation, HostError> {
        let requested = socket.map(str::to_owned).or_else(|| {
            self.caller()
                .filter(|caller| caller.host == host)
                .map(|caller| caller.socket.clone())
        });
        let answered = |source, socket: Option<&str>| DiscoveryObservation {
            host: HostKind::External(host),
            source,
            socket: socket.unwrap_or_default().into(),
            candidates: Vec::new(),
            truncated: false,
        };
        if !self
            .records()
            .iter()
            .any(|record| record.name == host.as_str())
        {
            return Ok(answered(
                DiscoverySource::Unsupported("driver_not_approved"),
                requested.as_deref(),
            ));
        }
        let process = self
            .open(host)
            .ok_or_else(|| HostError::Unavailable(host.as_str().to_owned()))?;
        if !process.supports(Op::Server) || !process.supports(Op::Snapshot) {
            return Ok(answered(
                DiscoverySource::Unsupported("driver_lacks_inventory"),
                requested.as_deref(),
            ));
        }
        let changed = |reason| HostError::Evidence {
            driver: host.as_str().to_owned(),
            reason,
        };
        let answer = process
            .call::<ServerResponse>(
                ServerRequest {
                    socket: requested.clone(),
                },
                deadline,
            )
            .map_err(HostError::Driver)?
            .map_err(|error| refused(&process, error))?;
        let Some(server) = answer.server else {
            return Ok(answered(DiscoverySource::Offline, requested.as_deref()));
        };
        if requested
            .as_ref()
            .is_some_and(|socket| *socket != server.socket)
        {
            return Err(changed("the server answered for another socket"));
        }
        let answer = process
            .call::<SnapshotResponse>(
                SnapshotRequest {
                    socket: server.socket.clone(),
                    panes: pane.map(|pane| vec![pane.to_owned()]),
                },
                deadline,
            )
            .map_err(HostError::Driver)?
            .map_err(|error| refused(&process, error))?;
        // The decoder already refused a pane listed twice and a malformed
        // marker: either fails the whole answer, never one row.
        let mut rows = BTreeMap::new();
        for row in answer.panes {
            if pane.is_some_and(|pane| pane != row.id) {
                return Err(changed("the driver answered for another pane"));
            }
            rows.insert(row.id.clone(), row);
        }
        let mut truncated = rows.len() > MAX_CANDIDATES;
        let rows: Vec<_> = rows.into_values().take(MAX_CANDIDATES).collect();
        let pids: Vec<u64> = std::iter::once(server.pid)
            .chain(rows.iter().map(|row| row.pane_pid))
            .collect();
        let mut starts = observe_starts(self.runner(), &pids, deadline)
            .map_err(|error| driver_process(&process, error))?;
        if Instant::now() >= deadline {
            truncated = true;
        }
        let server_start = starts.get(&server.pid).cloned();
        let candidates = rows
            .into_iter()
            .map(|row| {
                let start = starts.remove(&row.pane_pid);
                let observed = observation(&host, row, start.clone());
                let fence = server_start
                    .clone()
                    .zip(start)
                    .map(|(server_process, process)| DiscoveryTarget {
                        host: HostKind::External(host),
                        socket: server.socket.clone(),
                        server_start_time: server_process.start_identity().to_owned(),
                        server: server_process,
                        server_id: None,
                        pane: observed.id.clone(),
                        process,
                        marker: observed.marker.clone(),
                        runtime: observed.suggested_name.clone(),
                    });
                let reason = if truncated {
                    Some("scan_truncated")
                } else if fence.is_none() {
                    Some("native_incarnation_unknown")
                } else {
                    None
                };
                let (command, a) = display_text(&observed.command);
                let (cwd, b) = display_text(observed.cwd.as_deref().unwrap_or_default());
                let agent_name = observed.agent_name.as_deref().map(display_text);
                DiscoveryCandidate {
                    pane: observed.id,
                    session: None,
                    window: None,
                    target: observed.target,
                    title: String::new(),
                    command,
                    cwd,
                    text_truncated: a || b || agent_name.as_ref().is_some_and(|(_, cut)| *cut),
                    agent_name: agent_name.map(|(name, _)| name),
                    fence,
                    identity: None,
                    state: "unknown",
                    reason,
                    duplicate: false,
                }
            })
            .collect();
        Ok(DiscoveryObservation {
            host: HostKind::External(host),
            source: DiscoverySource::Ok,
            socket: server.socket,
            candidates,
            truncated,
        })
    }

    /// Records TMT's UUID for the server of a guarded bind's fence, and
    /// resolves this handle to it, only after the driver names that exact
    /// socket and core sees the fenced server process (PID and start token)
    /// still running it. Any other answer refuses with nothing recorded.
    pub(crate) fn resolve_fenced_server<I: HostServerIds>(
        &self,
        host: HostName,
        fence: &DiscoveryTarget,
        ids: &mut I,
        deadline: Instant,
    ) -> Result<String, HostError> {
        let changed = |reason| HostError::Evidence {
            driver: host.as_str().to_owned(),
            reason,
        };
        if fence.host != HostKind::External(host) {
            return Err(changed("the discovered pane is on another host"));
        }
        let process = self
            .open(host)
            .ok_or_else(|| HostError::Unavailable(host.as_str().to_owned()))?;
        let server = process
            .call::<ServerResponse>(
                ServerRequest {
                    socket: Some(fence.socket.clone()),
                },
                deadline,
            )
            .map_err(HostError::Driver)?
            .map_err(|error| refused(&process, error))?
            .server
            .filter(|server| server.socket == fence.socket)
            .ok_or_else(|| changed("the discovered server is gone"))?;
        let observed: Option<ProcessIncarnation> =
            observe_starts(self.runner(), &[server.pid], deadline)
                .map_err(|error| driver_process(&process, error))?
                .remove(&server.pid);
        if observed.as_ref() != Some(&fence.server) {
            return Err(changed("the discovered server was replaced"));
        }
        let server_id = ids
            .server_id(&HostServerIncarnation {
                host: fence.host,
                socket_path: &fence.socket,
                server_pid: fence.server.pid(),
                server_start_time: fence.server.start_identity(),
            })
            .map_err(|error| HostError::ServerIds(Box::new(error)))?;
        if !valid_server_id(&server_id) {
            return Err(HostError::ServerIds("invalid server ID".into()));
        }
        self.set_resolved(ServerEvidence {
            host: fence.host,
            server_id: server_id.clone(),
            socket_path: fence.socket.clone(),
            server_pid: fence.server.pid(),
            server_start_time: fence.server.start_identity().to_owned(),
        });
        Ok(server_id)
    }
}
