//! Stored bindings on an external host, through its approved driver (#570
//! slice 3b-2a-2; `contracts/driver-protocol-v1.md`).
//!
//! Core leads every decision. The driver reports panes and keeps markers;
//! core observes the server and pane processes itself (one batched `ps`),
//! and only core's own observation proves a server gone. The driver's
//! `startTime` is advisory and never stored, and core's start token is never
//! sent to a driver. Input, focus and capture arrive in slice 3b-2b: until
//! then a send is `Unsupported`, so core uses the inbox.

use super::{CallError, DriverProcess, registry::DriverRecord};
use crate::{
    host::{ActionError, DeliveryError, HostError, driver::HostDriver, driver::Unavailable},
    process::{
        CommandError, CommandRunner,
        runtime::{self, observe_starts},
    },
};
use std::{
    cell::OnceCell,
    collections::HashMap,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::{Binding, session::RuntimeState},
    driver::{ActionResult, DeliveryAcceptance, Focused, SendFailure},
    endpoint::{
        BindingMarker, EndpointProbe, EndpointSnapshot, PaneObservation, ProcessIncarnation,
        ServerEvidence,
    },
    host::{HostKind, HostName, HostServerIds, HostServerIncarnation},
    identity::Identity,
};
use tmt_driver_protocol::{
    ClearRequest, ClearResponse, ErrorCode, Marker, Op, Pane, PublishRequest, ServerRequest,
    ServerResponse, SnapshotRequest, SnapshotResponse,
};

/// A handle's approved drivers and, when its primary host is external, that
/// host's resolved server.
pub struct Drivers<R> {
    runner: R,
    records: Vec<DriverRecord>,
    server: OnceCell<ServerEvidence>,
}

impl<R: CommandRunner> Drivers<R> {
    pub fn new(runner: R, records: Vec<DriverRecord>) -> Self {
        Self {
            runner,
            records,
            server: OnceCell::new(),
        }
    }

    /// The first resolution stands for the handle's lifetime.
    pub(crate) fn set_resolved(&self, server: ServerEvidence) {
        let _ = self.server.set(server);
    }

    pub(crate) fn resolved(&self) -> Option<&ServerEvidence> {
        self.server.get()
    }

    /// The approved driver for `host`, checked again before use: `None` when
    /// none is approved or it is no longer the approved executable.
    fn open(&self, host: HostName) -> Option<DriverProcess<&R>> {
        let record = self
            .records
            .iter()
            .find(|record| record.name == host.as_str())?;
        DriverProcess::open(record.clone(), &self.runner).ok()
    }

    /// TMT's UUID for this handle's external server, before any binding
    /// transaction. Its identity is core's own observation of the process the
    /// driver names; a server core can't observe stays unresolved.
    pub fn resolve_server<I: HostServerIds>(
        &self,
        host: HostName,
        ids: &mut I,
    ) -> Result<(), HostError> {
        if self.server.get().is_some() {
            return Ok(());
        }
        let Some(process) = self.open(host) else {
            return Err(HostError::Unavailable(host.as_str().to_owned()));
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        let answer = process
            .call::<ServerResponse>(ServerRequest { socket: None }, deadline)
            .map_err(HostError::Driver)?;
        let Some(server) = answer.map_err(|error| refused(&process, error))?.server else {
            return Ok(());
        };
        let Some(observed) = observe_starts(&self.runner, &[server.pid], deadline)
            .map_err(|error| driver_process(&process, error))?
            .remove(&server.pid)
        else {
            return Ok(());
        };
        let server_id = ids
            .server_id(&HostServerIncarnation {
                host: HostKind::External(host),
                socket_path: &server.socket,
                server_pid: observed.pid(),
                server_start_time: observed.start_identity(),
            })
            .map_err(|error| HostError::ServerIds(Box::new(error)))?;
        if !tmt_core::endpoint::valid_server_id(&server_id) {
            return Err(HostError::ServerIds("invalid server ID".into()));
        }
        self.set_resolved(ServerEvidence {
            host: HostKind::External(host),
            server_id,
            socket_path: server.socket,
            server_pid: observed.pid(),
            server_start_time: observed.start_identity().to_owned(),
        });
        Ok(())
    }
}

fn refused<R>(process: &DriverProcess<R>, error: tmt_driver_protocol::DriverError) -> HostError
where
    R: CommandRunner,
{
    HostError::Refused {
        driver: process.name().to_owned(),
        code: error.code,
        message: error.message,
    }
}

fn driver_process<R: CommandRunner>(process: &DriverProcess<R>, error: CommandError) -> HostError {
    HostError::Driver(CallError::Process {
        driver: process.name().to_owned(),
        error,
    })
}

/// One binding session's external drivers, one per host, opened on first
/// use. A host without an approved, unchanged driver gets an `Unavailable`
/// that names it.
pub struct Session<'a, R> {
    drivers: &'a Drivers<R>,
    deadline: Instant,
    slots: HashMap<HostName, DriverSlot<'a, R>>,
}

enum DriverSlot<'a, R> {
    Ready(Box<ExternalDriver<'a, R>>),
    Missing(Unavailable),
}

impl<'a, R: CommandRunner> Session<'a, R> {
    pub fn new(drivers: &'a Drivers<R>) -> Self {
        Self {
            drivers,
            deadline: Instant::now(),
            slots: HashMap::new(),
        }
    }

    pub fn begin_coordination(&mut self) {
        self.deadline = Instant::now() + Duration::from_secs(3);
        for slot in self.slots.values_mut() {
            if let DriverSlot::Ready(driver) = slot {
                driver.deadline = self.deadline;
            }
        }
    }

    /// A caller's own deadline, when it is sooner.
    pub fn limit(&mut self, deadline: Option<Instant>) {
        if let Some(deadline) = deadline {
            self.deadline = self.deadline.min(deadline);
        }
    }

    pub fn budget_available(&self) -> bool {
        Instant::now() < self.deadline
    }

    pub fn driver(&mut self, host: HostName) -> &mut dyn HostDriver {
        let drivers = self.drivers;
        let deadline = self.deadline;
        let slot = self
            .slots
            .entry(host)
            .or_insert_with(|| match drivers.open(host) {
                Some(process) => DriverSlot::Ready(Box::new(ExternalDriver {
                    host,
                    process,
                    runner: &drivers.runner,
                    server: drivers.server.get(),
                    deadline,
                })),
                None => DriverSlot::Missing(Unavailable::of(HostKind::External(host))),
            });
        match slot {
            DriverSlot::Ready(driver) => driver.as_mut(),
            DriverSlot::Missing(unavailable) => unavailable,
        }
    }

    /// Runtime liveness from core's own process inspection, as on Herdr; a
    /// host without its driver is never present, so this is never asked.
    pub fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, CommandError> {
        runtime::binding_runtime(&self.drivers.runner, binding, self.deadline)
    }
}

/// An approved driver behind the host-driver trait.
pub struct ExternalDriver<'a, R> {
    host: HostName,
    process: DriverProcess<&'a R>,
    runner: &'a R,
    /// The handle's resolved server, when its primary host is this one.
    server: Option<&'a ServerEvidence>,
    deadline: Instant,
}

impl<R: CommandRunner> ExternalDriver<'_, R> {
    fn call<T: tmt_driver_protocol::Answer>(
        &self,
        body: impl serde::Serialize,
    ) -> Result<Result<T, tmt_driver_protocol::DriverError>, HostError> {
        self.process
            .call(body, self.deadline)
            .map_err(HostError::Driver)
    }

    /// The driver's panes on `socket`, scoped to `panes`, with each shell's
    /// incarnation and the server's from one `ps`.
    fn observe(
        &self,
        socket: &str,
        server_pid: u64,
        panes: Option<&[String]>,
    ) -> Result<Observed, HostError> {
        let answer = self.call::<SnapshotResponse>(SnapshotRequest {
            socket: socket.into(),
            panes: panes.map(<[String]>::to_vec),
        })?;
        let panes = match answer {
            Ok(snapshot) => Some(snapshot.panes),
            Err(error) if matches!(error.code, ErrorCode::Unavailable | ErrorCode::NotFound) => {
                None
            }
            Err(error) => return Err(refused(&self.process, error)),
        };
        let pids: Vec<u64> = std::iter::once(server_pid)
            .chain(panes.iter().flatten().map(|pane| pane.pane_pid))
            .collect();
        let mut starts = observe_starts(self.runner, &pids, self.deadline)
            .map_err(|error| driver_process(&self.process, error))?;
        let server = starts.get(&server_pid).cloned();
        let panes = panes.map(|panes| {
            panes
                .into_iter()
                .map(|pane| {
                    let start = starts.remove(&pane.pane_pid);
                    observation(pane, start)
                })
                .collect()
        });
        Ok(Observed { server, panes })
    }
}

struct Observed {
    /// Core's observation of the server process; `None` when it can't tell.
    server: Option<ProcessIncarnation>,
    /// `None` when the driver can't reach the server.
    panes: Option<Vec<PaneObservation>>,
}

fn observation(pane: Pane, start: Option<ProcessIncarnation>) -> PaneObservation {
    PaneObservation {
        suggested_name: crate::drivers::suggested_name(&pane.command),
        id: pane.id,
        target: pane.target,
        cwd: pane.cwd,
        command: pane.command,
        pane_pid: pane.pane_pid,
        pane_incarnation: start.map(|process| process.start_identity().to_owned()),
        marker: pane.marker.map(|marker| BindingMarker {
            name: marker.name,
            canonical_name: marker.canonical_name,
            identity_id: marker.identity_id,
            binding_id: marker.binding_id,
            server_id: marker.server_id,
            pane_pid: marker.pane_pid,
        }),
    }
}

impl<R: CommandRunner> HostDriver for ExternalDriver<'_, R> {
    fn begin_coordination(&mut self) {
        self.deadline = Instant::now() + Duration::from_secs(3);
    }

    fn budget_available(&self) -> bool {
        Instant::now() < self.deadline
    }

    /// Panes on the handle's resolved server. A server that is not the
    /// resolved incarnation any more is an error, never a substitute.
    fn snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, HostError> {
        let server = self.server.cloned().ok_or_else(|| HostError::Evidence {
            driver: self.process.name().to_owned(),
            reason: "no server was resolved for this command",
        })?;
        let observed = self.observe(&server.socket_path, server.server_pid, Some(panes))?;
        let same = observed.server.as_ref().is_some_and(|process| {
            process.pid() == server.server_pid
                && process.start_identity() == server.server_start_time
        });
        let Some(panes) = observed.panes.filter(|_| same) else {
            return Err(HostError::Evidence {
                driver: self.process.name().to_owned(),
                reason: "the server is no longer the one this command resolved",
            });
        };
        Ok(EndpointSnapshot { server, panes })
    }

    /// Core leads: the recorded server is dead only when core sees its
    /// process gone or replaced, live only when it is the same process and
    /// the driver reaches it, and unknown otherwise.
    fn probe(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, HostError> {
        if server.host != HostKind::External(self.host) || !self.budget_available() {
            return Ok(EndpointProbe::Unknown);
        }
        let observed = match self.observe(&server.socket_path, server.server_pid, Some(panes)) {
            Ok(observed) => observed,
            Err(error) if error.cleanup_failed() => return Err(error),
            Err(_) => return Ok(EndpointProbe::Unknown),
        };
        let probe = match (observed.server, observed.panes) {
            (Some(process), panes) if process.start_identity() == server.server_start_time => {
                match panes {
                    Some(panes) => EndpointProbe::Live(EndpointSnapshot {
                        server: server.clone(),
                        panes,
                    }),
                    // The same server, but the driver can't reach it.
                    None => EndpointProbe::Unknown,
                }
            }
            // Another process now holds the recorded pid.
            (Some(_), _) => EndpointProbe::Dead,
            (None, _) if runtime::recorded_process_gone(server.server_pid) => EndpointProbe::Dead,
            (None, _) => EndpointProbe::Unknown,
        };
        // A spent coordination budget does not authorize loss or retirement.
        Ok(if self.budget_available() {
            probe
        } else {
            EndpointProbe::Unknown
        })
    }

    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), HostError> {
        let marker = binding.marker(identity);
        let answer = self
            .process
            .call_done(
                Op::Publish,
                PublishRequest {
                    socket: binding.server.socket_path.clone(),
                    pane_id: binding.pane_id.clone(),
                    pane_pid: binding.pane_pid,
                    marker: Marker {
                        name: marker.name,
                        canonical_name: marker.canonical_name,
                        identity_id: marker.identity_id,
                        binding_id: marker.binding_id,
                        server_id: marker.server_id,
                        pane_pid: marker.pane_pid,
                    },
                },
                self.deadline,
            )
            .map_err(HostError::Driver)?;
        answer.map_err(|error| refused(&self.process, error))
    }

    fn clear(&mut self, binding: &Binding) -> Result<bool, HostError> {
        let answer = self.call::<ClearResponse>(ClearRequest {
            socket: binding.server.socket_path.clone(),
            pane_id: binding.pane_id.clone(),
            binding_id: binding.id.clone(),
        })?;
        Ok(answer
            .map_err(|error| refused(&self.process, error))?
            .cleared)
    }

    fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, CommandError> {
        runtime::binding_runtime(self.runner, binding, self.deadline)
    }

    fn pane_incarnation(&mut self, pane_pid: u64) -> Result<Option<String>, HostError> {
        runtime::observe_start(self.runner, pane_pid, self.deadline)
            .map_err(|error| driver_process(&self.process, error))
    }

    /// Input arrives in slice 3b-2b; until then core uses the inbox.
    fn has_input(&self) -> bool {
        false
    }

    /// The `prompt` operation arrives with input in slice 3b-2b.
    fn prompt(
        &mut self,
        _: &Binding,
        _: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
        ActionResult::Unsupported
    }

    fn input(&mut self, _: &Binding, _: &str) -> Result<(), DeliveryError> {
        Err(DeliveryError::unsupported())
    }

    fn focus_preflight(&self, _: Option<&Binding>) -> Result<(), ActionError> {
        Err(ActionError::HostUnsupported)
    }

    fn focus(&mut self, _: &Binding) -> Result<Focused, ActionError> {
        Err(ActionError::HostUnsupported)
    }
}
