//! The caller's own pane and explicit targets on an external host (#570
//! slice 3b-2b-1). A driver names a pane; core counts it only when the pane's
//! shell is an ancestor of the caller, which it checks itself, the same proof
//! a tmux caller needs.

use super::{
    Drivers,
    session::{driver_process, refused},
};
use crate::{
    host::{CallerEnvironment, HostError},
    process::{CommandRunner, ancestry},
};
use std::{collections::BTreeMap, time::Instant};
use tmt_core::host::HostName;
use tmt_driver_protocol::{
    CallerRequest, CallerResponse, ErrorCode, ResolveTargetRequest, ResolveTargetResponse,
    ServerRequest, ServerResponse, SnapshotRequest, SnapshotResponse,
};

/// A caller's pane on an external host, verified by core: `depth` is where
/// its shell sits in the caller's ancestry (0 is the caller itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCaller {
    pub host: HostName,
    pub pane_id: String,
    pub socket: String,
    pub depth: usize,
}

impl<R: CommandRunner> Drivers<R> {
    /// Every approved host whose declared variables the caller's environment
    /// sets and whose pane core verifies, nearest first. A driver that fails
    /// or names a pane outside the caller's ancestry proves nothing; only a
    /// failed cleanup is an error.
    pub(crate) fn callers(
        &self,
        environment: &CallerEnvironment,
        deadline: Instant,
    ) -> Result<Vec<ExternalCaller>, HostError> {
        let mut chain = None;
        let mut found = Vec::new();
        for record in self.records() {
            let Some(capabilities) = record.capabilities.host() else {
                continue;
            };
            let env: BTreeMap<String, String> = capabilities
                .caller_env
                .iter()
                .filter_map(|name| {
                    let value = environment.driver_env.get(name)?.to_str()?;
                    Some((name.clone(), value.to_owned()))
                })
                .collect();
            let Some(host) = HostName::new(&record.name).filter(|_| !env.is_empty()) else {
                continue;
            };
            let Some(process) = self.open(host) else {
                continue;
            };
            let pane = match process.call::<CallerResponse>(CallerRequest { env }, deadline) {
                Ok(Ok(CallerResponse { pane: Some(pane) })) => pane,
                Err(error) => {
                    unless_cleanup_failed(error)?;
                    continue;
                }
                Ok(_) => continue,
            };
            if chain.is_none() {
                chain = match ancestry::chain(self.runner(), environment.process_id, deadline) {
                    Ok(chain) => Some(chain),
                    Err(ancestry::AncestryError::Command(error)) if error.cleanup_failed() => {
                        return Err(driver_process(&process, error));
                    }
                    Err(_) => return Ok(Vec::new()),
                };
            }
            let Some(depth) = chain
                .as_ref()
                .and_then(|chain| chain.iter().position(|pid| *pid == pane.shell_pid))
            else {
                continue;
            };
            found.push(ExternalCaller {
                host,
                pane_id: pane.id,
                socket: pane.socket,
                depth,
            });
        }
        found.sort_by_key(|caller| caller.depth);
        Ok(found)
    }

    /// An explicit target's pane ID on the caller's server of `host`, or on
    /// the driver's default server. `None` only when no driver is approved,
    /// no server runs, or the driver answers that no such pane exists; a
    /// driver that fails or runs late is an error, never "not found".
    pub(crate) fn resolve_target(
        &self,
        host: HostName,
        target: &str,
        deadline: Instant,
    ) -> Result<Option<String>, HostError> {
        let Some(process) = self.open(host) else {
            return Ok(None);
        };
        let socket = match self.caller().filter(|caller| caller.host == host) {
            Some(caller) => caller.socket.clone(),
            None => {
                let answer = process
                    .call::<ServerResponse>(ServerRequest { socket: None }, deadline)
                    .map_err(HostError::Driver)?
                    .map_err(|error| refused(&process, error))?;
                let Some(server) = answer.server else {
                    return Ok(None);
                };
                server.socket
            }
        };
        let request = ResolveTargetRequest {
            socket,
            target: target.to_owned(),
        };
        match process
            .call::<ResolveTargetResponse>(request, deadline)
            .map_err(HostError::Driver)?
        {
            Ok(answer) => Ok(answer.pane_id),
            // The pane or its server is gone: as definite as `null`.
            Err(error) if error.code == ErrorCode::NotFound => Ok(None),
            Err(error) => Err(refused(&process, error)),
        }
    }

    /// The public target of the caller's verified pane, for presentation
    /// only: one `snapshot` scoped to that pane on the caller's server, with
    /// no server resolved or recorded. A driver that can't say is `None`;
    /// only a failed cleanup is an error.
    pub(crate) fn caller_target(&self, deadline: Instant) -> Result<Option<String>, HostError> {
        let Some(caller) = self.caller() else {
            return Ok(None);
        };
        let Some(process) = self.open(caller.host) else {
            return Ok(None);
        };
        let request = SnapshotRequest {
            socket: caller.socket.clone(),
            panes: Some(vec![caller.pane_id.clone()]),
        };
        match process.call::<SnapshotResponse>(request, deadline) {
            Ok(Ok(snapshot)) => Ok(snapshot
                .panes
                .into_iter()
                .find(|pane| pane.id == caller.pane_id)
                .and_then(|pane| pane.target)),
            Ok(Err(_)) => Ok(None),
            Err(error) => unless_cleanup_failed(error).map(|()| None),
        }
    }
}

/// A failed driver call proves nothing here, unless its process could not
/// be cleaned up.
fn unless_cleanup_failed(error: super::CallError) -> Result<(), HostError> {
    let error = HostError::Driver(error);
    if error.cleanup_failed() {
        Err(error)
    } else {
        Ok(())
    }
}
