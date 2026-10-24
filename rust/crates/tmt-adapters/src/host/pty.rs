//! In-process built-in host over one member supervisor. The ordinary HostDriver
//! policy remains the only binding/delivery admission owner.

use super::{
    ActionError, AgentNaming, CallerEnvironment, DeliveryError, DeliveryStage, HostError,
    driver::HostDriver, external::ExternalCaller,
};
use crate::{
    process::{CommandError, CommandRunner, ancestry, runtime},
    pty::{
        Connection,
        protocol::{Control, ControlReply},
    },
};
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::{Binding, session::RuntimeState},
    driver::{ActionResult, DeliveryAcceptance, Focused, InputActivity, SendFailure},
    endpoint::{
        BindingMarker, EndpointProbe, EndpointSnapshot, PaneLocation, PaneObservation,
        ServerEvidence,
    },
    host::{HostKind, HostServerIds, HostServerIncarnation},
    identity::Identity,
};

pub(crate) const PANE_ENV: &str = "TMT_PTY_PANE";
pub(crate) const SOCKET_ENV: &str = "TMT_PTY_SOCKET";
const BUDGET: Duration = Duration::from_secs(3);

fn error(reason: &'static str) -> HostError {
    HostError::Evidence {
        driver: HostKind::pty().as_str().into(),
        reason,
    }
}
fn io_error(cause: io::Error) -> HostError {
    HostError::Pty(cause)
}

fn connection(socket: &str, pane: &str, deadline: Instant) -> Result<Connection, HostError> {
    let path = Path::new(socket);
    if !HostKind::pty().is_pane_id(pane)
        || path.file_stem().and_then(|s| s.to_str()) != Some(pane)
        || path.extension().and_then(|s| s.to_str()) != Some("sock")
    {
        return Err(error("the pane and socket do not match"));
    }
    let directory = path
        .parent()
        .ok_or_else(|| error("the socket has no directory"))?;
    if !crate::private_file::directory_is_private(directory) {
        return Err(error("the socket directory is not private"));
    }
    Connection::connect_until(directory, pane, deadline).map_err(io_error)
}

fn call(
    socket: &str,
    pane: &str,
    request: Control,
    deadline: Instant,
) -> Result<ControlReply, HostError> {
    let mut client = connection(socket, pane, deadline)?;
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(io_error(io::ErrorKind::TimedOut.into()));
    }
    client.control(request, left).map_err(io_error)
    // Drop disconnects immediately on success and error; no unread output client survives.
}

struct State {
    server_pid: u64,
    pane_pid: u64,
    marker: Option<BindingMarker>,
    idle: Option<u64>,
}
fn state(socket: &str, pane: &str, deadline: Instant) -> Result<State, HostError> {
    let ControlReply::State {
        supervisor_pid,
        pane_pid,
        key_idle_ms,
        marker,
    } = call(socket, pane, Control::Inspect, deadline)?
    else {
        return Err(error("the supervisor did not return state"));
    };
    let marker = marker
        .map(|text| {
            serde_json::from_str::<tmt_driver_protocol::Marker>(&text)
                .map(|m| BindingMarker {
                    name: m.name,
                    canonical_name: m.canonical_name,
                    identity_id: m.identity_id,
                    binding_id: m.binding_id,
                    server_id: m.server_id,
                    pane_pid: m.pane_pid,
                })
                .map_err(|_| error("the supervisor returned an invalid marker"))
        })
        .transpose()?;
    Ok(State {
        server_pid: u64::from(supervisor_pid),
        pane_pid: u64::from(pane_pid),
        marker,
        idle: key_idle_ms,
    })
}

fn verify_pair<R: CommandRunner>(
    runner: &R,
    observed: &State,
    deadline: Instant,
) -> Result<(), HostError> {
    let chain = ancestry::pinned_chain(runner, observed.pane_pid, deadline)
        .map_err(|_| error("the pane ancestry is unverified"))?;
    if !chain
        .links()
        .get(1)
        .is_some_and(|parent| parent.process.pid() == observed.server_pid)
    {
        return Err(error("the supervisor is not the pane process owner"));
    }
    Ok(())
}

pub(super) fn caller<R: CommandRunner>(
    runner: &R,
    env: &CallerEnvironment,
    pinned: Option<&ancestry::PinnedChain>,
    deadline: Instant,
) -> Result<Option<ExternalCaller>, HostError> {
    let Some(pane) = env.driver_env.get(PANE_ENV).and_then(|v| v.to_str()) else {
        return Ok(None);
    };
    let socket = env
        .driver_env
        .get(SOCKET_ENV)
        .and_then(|v| v.to_str())
        .ok_or_else(|| error("the caller socket locator is missing"))?;
    let observed = state(socket, pane, deadline)?;
    verify_pair(runner, &observed, deadline)?;
    let owned;
    let chain = match pinned {
        Some(chain) => chain,
        None => {
            owned = ancestry::pinned_chain(runner, env.process_id, deadline)
                .map_err(|_| error("the caller ancestry is unverified"))?;
            &owned
        }
    };
    let Some(depth) = chain.position(observed.pane_pid) else {
        return Ok(None);
    };
    let segment = &chain.links()[..=depth];
    let pids: Vec<_> = segment
        .iter()
        .map(|link| link.process.pid())
        .chain(std::iter::once(observed.server_pid))
        .collect();
    let now = runtime::observe_starts(runner, &pids, deadline).map_err(HostError::Process)?;
    if !now.contains_key(&observed.server_pid)
        || !segment
            .iter()
            .all(|link| now.get(&link.process.pid()) == Some(&link.process))
    {
        return Err(error("the caller process chain changed"));
    }
    let HostKind::External(host) = HostKind::pty() else {
        unreachable!()
    };
    Ok(Some(ExternalCaller {
        host,
        pane_id: pane.into(),
        socket: socket.into(),
        depth,
    }))
}

pub(super) fn target_socket(pane: &str) -> Result<String, HostError> {
    if !HostKind::pty().is_pane_id(pane) {
        return Err(error("invalid built-in pane target"));
    }
    let paths =
        crate::config::ConfigPaths::discover().map_err(|e| HostError::ServerIds(Box::new(e)))?;
    Ok(crate::pty::socket_path(&paths.pty_directory(), pane)
        .to_string_lossy()
        .into_owned())
}

pub(super) fn socket_path_exists(socket: &str) -> Result<bool, HostError> {
    match std::fs::symlink_metadata(socket) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(error)),
        Ok(_) => Ok(true),
    }
}

pub(super) fn target_exists<R: CommandRunner>(
    runner: &R,
    socket: &str,
    pane: &str,
    deadline: Instant,
) -> Result<bool, HostError> {
    if !socket_path_exists(socket)? {
        return Ok(false);
    }
    let answer = state(socket, pane, deadline)?;
    verify_pair(runner, &answer, deadline)?;
    Ok(true)
}

pub(super) fn resolve<R: CommandRunner, I: HostServerIds>(
    runner: &R,
    socket: &str,
    pane: &str,
    ids: &mut I,
    deadline: Instant,
) -> Result<ServerEvidence, HostError> {
    let answer = state(socket, pane, deadline)?;
    verify_pair(runner, &answer, deadline)?;
    let mut starts =
        runtime::observe_starts(runner, &[answer.server_pid, answer.pane_pid], deadline)
            .map_err(HostError::Process)?;
    let supervisor = starts
        .remove(&answer.server_pid)
        .ok_or_else(|| error("the supervisor process is unverified"))?;
    if !starts.contains_key(&answer.pane_pid) {
        return Err(error("the pane process is unverified"));
    }
    let server_id = ids
        .server_id(&HostServerIncarnation {
            host: HostKind::pty(),
            socket_path: socket,
            server_pid: supervisor.pid(),
            server_start_time: supervisor.start_identity(),
        })
        .map_err(|e| HostError::ServerIds(Box::new(e)))?;
    if !tmt_core::endpoint::valid_server_id(&server_id) {
        return Err(error("the server UUID is invalid"));
    }
    Ok(ServerEvidence {
        host: HostKind::pty(),
        server_id,
        socket_path: socket.into(),
        server_pid: supervisor.pid(),
        server_start_time: supervisor.start_identity().into(),
    })
}

pub(super) fn wait_for_launcher(env: &CallerEnvironment) -> Result<(), HostError> {
    let pane = env
        .driver_env
        .get(PANE_ENV)
        .and_then(|v| v.to_str())
        .ok_or_else(|| error("no pane locator"))?;
    let socket = env
        .driver_env
        .get(SOCKET_ENV)
        .and_then(|v| v.to_str())
        .ok_or_else(|| error("no socket locator"))?;
    match call(socket, pane, Control::WaitLauncher, Instant::now() + BUDGET)? {
        ControlReply::Applied(true) => Ok(()),
        _ => Err(error("the launcher is not listening")),
    }
}

pub(super) fn launch_ready(binding: &Binding) -> Result<(), HostError> {
    let pane_pid = u32::try_from(binding.pane_pid).map_err(|_| error("invalid launch owner"))?;
    match call(
        &binding.server.socket_path,
        &binding.pane_id,
        Control::LaunchReady { pane_pid },
        Instant::now() + BUDGET,
    )? {
        ControlReply::Applied(true) => Ok(()),
        _ => Err(error("the supervisor did not confirm launch readiness")),
    }
}

pub(super) struct PtyDriver<'a, R> {
    pub runner: &'a R,
    pub server: Option<&'a ServerEvidence>,
    pub deadline: Instant,
    pub enter_delay: Duration,
}
impl<R: CommandRunner> PtyDriver<'_, R> {
    fn snapshot_at(
        &self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointSnapshot, HostError> {
        let pane = Path::new(&server.socket_path)
            .file_stem()
            .and_then(|v| v.to_str())
            .ok_or_else(|| error("invalid socket path"))?;
        let answer = state(&server.socket_path, pane, self.deadline)?;
        verify_pair(self.runner, &answer, self.deadline)?;
        let starts = runtime::observe_starts(
            self.runner,
            &[answer.server_pid, answer.pane_pid],
            self.deadline,
        )
        .map_err(HostError::Process)?;
        if answer.server_pid != server.server_pid
            || !starts
                .get(&answer.server_pid)
                .is_some_and(|p| p.start_identity() == server.server_start_time)
        {
            return Err(error("the supervisor incarnation changed"));
        }
        let mut found = Vec::new();
        if panes.is_empty() || panes.iter().any(|id| id == pane) {
            let incarnation = starts
                .get(&answer.pane_pid)
                .ok_or_else(|| error("the pane process is unverified"))?;
            found.push(PaneObservation {
                id: pane.into(),
                target: None,
                cwd: None,
                command: String::new(),
                pane_pid: answer.pane_pid,
                pane_incarnation: Some(incarnation.start_identity().into()),
                suggested_name: None,
                marker: answer.marker,
                location: Some(PaneLocation {
                    host: server.host.as_str().into(),
                    session: None,
                    window: None,
                }),
                agent_name: None,
            });
        }
        Ok(EndpointSnapshot {
            server: server.clone(),
            panes: found,
        })
    }
}

pub(super) fn input(
    socket: &str,
    pane: &str,
    message: &str,
    enter_delay: Duration,
) -> Result<(), DeliveryError> {
    if message.len() > crate::pty::protocol::MAX_PAYLOAD {
        return Err(DeliveryError::new(
            DeliveryStage::Prepare,
            error("the message exceeds the paste byte bound"),
        ));
    }
    let deadline = Instant::now() + BUDGET;
    let mut client = connection(socket, pane, deadline)
        .map_err(|e| DeliveryError::new(DeliveryStage::Prepare, e))?;
    if !message.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(DeliveryError::new(
                DeliveryStage::Prepare,
                io_error(io::ErrorKind::TimedOut.into()),
            ));
        }
        match client.control(Control::Paste(message.as_bytes().to_vec()), remaining) {
            Ok(ControlReply::Applied(true)) => {}
            Ok(_) => {
                return Err(DeliveryError::new(
                    DeliveryStage::Paste,
                    error("the supervisor refused paste"),
                ));
            }
            Err(e) => return Err(DeliveryError::new(DeliveryStage::Paste, io_error(e))),
        }
    }
    // Keep no idle output subscriber during the configured delay.
    drop(client);
    std::thread::sleep(enter_delay);
    let reply = call(socket, pane, Control::Enter, Instant::now() + BUDGET)
        .map_err(|e| DeliveryError::new(DeliveryStage::Submit, e))?;
    match reply {
        ControlReply::Applied(true) => Ok(()),
        _ => Err(DeliveryError::new(
            DeliveryStage::Submit,
            error("the supervisor refused Enter"),
        )),
    }
}

pub(super) fn capture(socket: &str, pane: &str, lines: u32) -> Result<String, HostError> {
    let ControlReply::Screen(text) = call(socket, pane, Control::Capture, Instant::now() + BUDGET)?
    else {
        return Err(error("the supervisor refused capture"));
    };
    if lines == 0 {
        return Ok(text);
    }
    let all: Vec<_> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines as usize)..].join("\n"))
}

impl<R: CommandRunner> HostDriver for PtyDriver<'_, R> {
    fn begin_coordination(&mut self) {
        self.deadline = Instant::now() + BUDGET;
    }
    fn budget_available(&self) -> bool {
        Instant::now() < self.deadline
    }
    fn snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, HostError> {
        self.snapshot_at(
            self.server
                .ok_or_else(|| error("no supervisor was resolved"))?,
            panes,
        )
    }
    fn probe(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, HostError> {
        if server.host != HostKind::pty() || !self.budget_available() {
            return Ok(EndpointProbe::Unknown);
        }
        let start = runtime::observe_start(self.runner, server.server_pid, self.deadline)
            .map_err(HostError::Process)?;
        match start {
            Some(start) if start != server.server_start_time => return Ok(EndpointProbe::Dead),
            None if runtime::recorded_process_gone(server.server_pid) => {
                return Ok(EndpointProbe::Dead);
            }
            None => return Ok(EndpointProbe::Unknown),
            _ => {}
        }
        match self.snapshot_at(server, panes) {
            Ok(snapshot) if self.budget_available() => Ok(EndpointProbe::Live(snapshot)),
            Err(e) if e.cleanup_failed() => Err(e),
            _ => Ok(EndpointProbe::Unknown),
        }
    }
    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), HostError> {
        let marker = binding.marker(identity);
        let text = serde_json::to_string(&tmt_driver_protocol::Marker {
            name: marker.name,
            canonical_name: marker.canonical_name,
            identity_id: marker.identity_id,
            binding_id: marker.binding_id,
            server_id: marker.server_id,
            pane_pid: marker.pane_pid,
        })
        .map_err(|_| error("the marker could not be encoded"))?;
        let pane_pid = u32::try_from(binding.pane_pid).map_err(|_| error("invalid pane PID"))?;
        match call(
            &binding.server.socket_path,
            &binding.pane_id,
            Control::Publish {
                pane_pid,
                key: binding.id.clone(),
                marker: text,
            },
            self.deadline,
        )? {
            ControlReply::Applied(true) => Ok(()),
            _ => Err(error("the pane changed before marker publication")),
        }
    }
    fn clear(&mut self, binding: &Binding, _: &Identity) -> Result<bool, HostError> {
        match call(
            &binding.server.socket_path,
            &binding.pane_id,
            Control::Clear {
                key: binding.id.clone(),
            },
            self.deadline,
        )? {
            ControlReply::Applied(answer) => Ok(answer),
            _ => Err(error("the supervisor did not confirm marker removal")),
        }
    }
    fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, CommandError> {
        runtime::binding_runtime(self.runner, binding, self.deadline)
    }
    fn pane_incarnation(&mut self, pid: u64) -> Result<Option<String>, HostError> {
        runtime::observe_start(self.runner, pid, self.deadline).map_err(HostError::Process)
    }
    fn input_activity(&mut self, binding: &Binding) -> Result<InputActivity, HostError> {
        Ok(
            state(&binding.server.socket_path, &binding.pane_id, self.deadline)?
                .idle
                .map_or(InputActivity::Unknown, InputActivity::ElapsedMs),
        )
    }
    fn has_input(&self) -> bool {
        true
    }
    fn prompt(
        &mut self,
        _: &Binding,
        _: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
        ActionResult::Unsupported
    }
    fn input(&mut self, binding: &Binding, message: &str) -> Result<(), DeliveryError> {
        input(
            &binding.server.socket_path,
            &binding.pane_id,
            message,
            self.enter_delay,
        )
    }
    fn focus_preflight(&self, _: Option<&Binding>) -> Result<(), ActionError> {
        Err(ActionError::HostUnsupported)
    }
    fn focus(&mut self, _: &Binding) -> Result<Focused, ActionError> {
        Err(ActionError::HostUnsupported)
    }
    fn name_agent(&mut self, _: &Binding, _: &str) -> Result<AgentNaming, HostError> {
        Ok(AgentNaming::NotLinked)
    }
}
