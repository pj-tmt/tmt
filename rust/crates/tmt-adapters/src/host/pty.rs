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
use tmt_driver_protocol::LaunchTemplate;

/// Starts a member in the background, with no pane to open: `$1` is the name it is bound as and
/// `$2` the command line. Nothing is interpolated. `tmt run` returns once the member's launcher
/// is admitted and prints the host and the bound name; the member keeps running when the
/// caller's terminal closes. An empty name or command is refused first, as for tmux's template.
const LAUNCH_SCRIPT: &str = "[ -n \"$1\" ] && [ -n \"$2\" ] || { echo 'tmt-launch: a session and a command are required' >&2; exit 2; }; \
exec tmt run --host pty \"$1\" sh -c \"$2\"";

pub fn launch_template() -> LaunchTemplate {
    let argv: Vec<String> = [
        "sh",
        "-c",
        LAUNCH_SCRIPT,
        "tmt-launch",
        "{session}",
        "{command}",
    ]
    .map(str::to_owned)
    .to_vec();
    LaunchTemplate::new(&argv).expect("the shipped built-in launch template is valid")
}

pub(crate) const PANE_ENV: &str = "TMT_PTY_PANE";
pub(crate) const SOCKET_ENV: &str = "TMT_PTY_SOCKET";
const BUDGET: Duration = Duration::from_secs(3);
/// How long closing a member waits for its supervisor to end after the program did.
const CLOSE_WAIT: Duration = Duration::from_secs(3);
const CLOSE_POLL: Duration = Duration::from_millis(50);

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
    decode_state(call(socket, pane, Control::Inspect, deadline)?)
}

fn decode_state(reply: ControlReply) -> Result<State, HostError> {
    let ControlReply::State {
        supervisor_pid,
        pane_pid,
        key_idle_ms,
        marker,
    } = reply
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
        self.snapshot_from(server, pane, panes, answer)
    }

    fn snapshot_from(
        &self,
        server: &ServerEvidence,
        pane: &str,
        panes: &[String],
        answer: State,
    ) -> Result<EndpointSnapshot, HostError> {
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

/// Admits the interactive connection itself, rather than verifying a separate
/// socket and assuming its path is still the same member on reconnect.
pub(super) fn attach<R: CommandRunner>(
    runner: &R,
    entry: &tmt_core::binding::BindingEntry,
) -> Result<Connection, ActionError> {
    use tmt_core::binding::{BindingEvidence, evaluate_binding};
    let binding = entry.binding.as_ref().ok_or(ActionError::Unverified)?;
    let deadline = Instant::now() + BUDGET;
    let mut client = connection(&binding.server.socket_path, &binding.pane_id, deadline)
        .map_err(ActionError::Evidence)?;
    let answer = client
        .control(
            Control::Inspect,
            deadline.saturating_duration_since(Instant::now()),
        )
        .map_err(|e| ActionError::Evidence(io_error(e)))?;
    let driver = PtyDriver {
        runner,
        server: Some(&binding.server),
        deadline,
        enter_delay: Duration::ZERO,
    };
    let snapshot = driver
        .snapshot_from(
            &binding.server,
            &binding.pane_id,
            std::slice::from_ref(&binding.pane_id),
            decode_state(answer).map_err(ActionError::Evidence)?,
        )
        .map_err(ActionError::Evidence)?;
    if Instant::now() >= deadline
        || !matches!(
            evaluate_binding(entry, &EndpointProbe::Live(snapshot)),
            BindingEvidence::Active(_)
        )
    {
        return Err(ActionError::Unverified);
    }
    Ok(client)
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

/// Waits up to `wait` for the supervisor incarnation `pid`/`start` to be gone (or replaced by
/// another process of that pid), which is what closing a built-in member means. It signals
/// nothing; a program that still runs when the time is up is refused, not ended.
fn wait_for_supervisor_end<R: CommandRunner>(
    runner: &R,
    pid: u64,
    start: &str,
    wait: Duration,
) -> Result<(), ActionError> {
    let deadline = Instant::now() + wait;
    loop {
        match runtime::observe_runtime_process(runner, pid, deadline)
            .map_err(|e| ActionError::Evidence(HostError::Process(e)))?
        {
            runtime::ProcessObservation::Gone => return Ok(()),
            runtime::ProcessObservation::Live(process)
            | runtime::ProcessObservation::Stopped(process)
                if process.start_identity() != start =>
            {
                return Ok(());
            }
            _ => {}
        }
        if Instant::now() + CLOSE_POLL >= deadline {
            return Err(ActionError::Evidence(error(
                "the member's program is still running; nothing was closed",
            )));
        }
        std::thread::sleep(CLOSE_POLL);
    }
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
        if self.publish_if(binding, identity, None)? {
            Ok(())
        } else {
            Err(error("the pane changed before marker publication"))
        }
    }
    fn publish_if(
        &mut self,
        binding: &Binding,
        identity: &Identity,
        expected: Option<&str>,
    ) -> Result<bool, HostError> {
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
        let socket = &binding.server.socket_path;
        let pane = &binding.pane_id;
        let conditional = match call(socket, pane, Control::Capabilities, self.deadline) {
            Ok(ControlReply::Applied(value)) => value,
            // Version 1 supervisors close a connection on an unknown control.
            Err(HostError::Pty(cause)) if cause.kind() == io::ErrorKind::UnexpectedEof => false,
            Err(cause) => return Err(cause),
            _ => return Err(error("the supervisor returned invalid capabilities")),
        };
        let request = if conditional {
            Control::PublishIf {
                pane_pid,
                key: binding.id.clone(),
                expected_key: expected.map(str::to_owned),
                marker: text,
            }
        } else {
            // Re-observe the same supervisor and pane after the capability connection
            // closes. Core verifies the published marker and compensates on mismatch.
            let observed = state(socket, pane, self.deadline)?;
            verify_pair(self.runner, &observed, self.deadline)?;
            let start = runtime::observe_start(self.runner, observed.server_pid, self.deadline)
                .map_err(HostError::Process)?;
            if observed.server_pid != binding.server.server_pid
                || observed.pane_pid != binding.pane_pid
                || start.as_deref() != Some(binding.server.server_start_time.as_str())
            {
                return Err(error("the supervisor changed during capability detection"));
            }
            Control::Publish {
                pane_pid,
                key: binding.id.clone(),
                marker: text,
            }
        };
        match call(socket, pane, request, self.deadline)? {
            ControlReply::Applied(applied) => Ok(applied),
            _ => Err(error("the supervisor did not confirm marker publication")),
        }
    }
    fn conditional_clear(&mut self, binding: &Binding) -> Result<bool, HostError> {
        match call(
            &binding.server.socket_path,
            &binding.pane_id,
            Control::Capabilities,
            self.deadline,
        ) {
            Ok(ControlReply::Applied(value)) => Ok(value),
            Err(HostError::Pty(cause)) if cause.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
            Err(cause) => Err(cause),
            _ => Err(error("the supervisor returned invalid capabilities")),
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
    fn close_preflight(&self) -> Result<(), ActionError> {
        Ok(())
    }
    /// A supervisor only starts through tmt's launcher, so every built-in member was opened by
    /// tmt; the binding must still name a built-in supervisor and pane.
    fn opened_by_tmt(&mut self, binding: &Binding) -> Result<Option<bool>, HostError> {
        Ok(Some(
            binding.server.host == HostKind::pty() && HostKind::pty().is_pane_id(&binding.pane_id),
        ))
    }
    /// Closing a built-in member means its program has exited and its supervisor has ended and
    /// removed its socket and lock. Nothing is signalled and no control is sent: the verified
    /// stop already ended the agent, and a supervisor ends by itself with its program. This waits
    /// a bounded time for that exact supervisor incarnation to be gone, and refuses while its
    /// program still runs.
    fn close_pane(&mut self, binding: &Binding) -> Result<(), ActionError> {
        wait_for_supervisor_end(
            self.runner,
            binding.server.server_pid,
            &binding.server.server_start_time,
            CLOSE_WAIT,
        )
    }
    fn name_agent(&mut self, _: &Binding, _: &str) -> Result<AgentNaming, HostError> {
        Ok(AgentNaming::NotLinked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::UnixCommandRunner;
    use std::process::{Child, Command, Stdio};

    fn spawn(script: &str) -> (Child, u64, String) {
        let child = Command::new("/bin/sh")
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = u64::from(child.id());
        let start = runtime::observe_start(
            &UnixCommandRunner,
            pid,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap()
        .expect("a live child has a start identity");
        (child, pid, start)
    }

    #[test]
    fn old_capability_supervisor_uses_legacy_publish_and_verifies_marker() {
        use crate::test_support::{TestChild, TestDirectory};
        use std::{
            io::{Read, Write},
            os::unix::net::UnixListener,
        };
        use tmt_core::identity::Lifetime;
        use tmt_pty_protocol::{ClientMessage, Decoder, Frame, ServerMessage, Size, VERSION};

        let root = TestDirectory::new();
        let directory = root.path.join("pty");
        crate::private_file::ensure_private_directory(&directory).unwrap();
        let socket = directory.join("pty_test.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let child = Command::new("/bin/sh")
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let child = TestChild::new(child);
        let pane_pid = child.child.id();
        let supervisor_pid = std::process::id();
        let server_start = runtime::observe_start(
            &UnixCommandRunner,
            u64::from(supervisor_pid),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap()
        .unwrap();
        let handle = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut marker = None;
            let mut seen_legacy = false;
            for step in 0..5 {
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(value) => break value,
                        Err(e)
                            if e.kind() == io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("fake old supervisor accept: {e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut decoder = Decoder::new();
                let hello = loop {
                    if let Some(frame) = decoder.frame::<ClientMessage>().unwrap() {
                        break frame;
                    }
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    decoder.push(&buf[..n]).unwrap();
                };
                assert!(matches!(hello, ClientMessage::Hello { version: VERSION }));
                stream
                    .write_all(
                        &ServerMessage::Welcome {
                            version: VERSION,
                            size: Size::new(80, 24).unwrap(),
                        }
                        .encode(),
                    )
                    .unwrap();
                let request = loop {
                    if let Some(frame) = decoder.frame::<ClientMessage>().unwrap() {
                        break frame;
                    }
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    decoder.push(&buf[..n]).unwrap();
                };
                let answer = match request {
                    ClientMessage::Control(Control::Capabilities) if step == 0 || step == 4 => {
                        continue;
                    }
                    ClientMessage::Control(Control::Inspect) if step == 1 || step == 3 => {
                        ControlReply::State {
                            supervisor_pid,
                            pane_pid,
                            key_idle_ms: None,
                            marker: marker.clone(),
                        }
                    }
                    ClientMessage::Control(Control::Publish {
                        pane_pid: pid,
                        key,
                        marker: value,
                    }) if step == 2 => {
                        assert_eq!(pid, pane_pid);
                        assert_eq!(key, "binding");
                        marker = Some(value);
                        seen_legacy = true;
                        ControlReply::Applied(true)
                    }
                    other => panic!("unexpected old supervisor request: {other:?}"),
                };
                stream
                    .write_all(&ServerMessage::Control(answer).encode())
                    .unwrap();
            }
            assert!(seen_legacy);
        });
        let server = ServerEvidence {
            host: HostKind::pty(),
            server_id: "server".into(),
            socket_path: socket.to_string_lossy().into_owned(),
            server_pid: u64::from(supervisor_pid),
            server_start_time: server_start,
        };
        let binding = Binding {
            id: "binding".into(),
            identity_id: "identity".into(),
            server,
            pane_id: "pty_test".into(),
            pane_pid: u64::from(pane_pid),
            pane_incarnation: None,
            session: Default::default(),
        };
        let identity = Identity {
            id: "identity".into(),
            name: "Name".into(),
            canonical_name: "name".into(),
            lifetime: Lifetime::Saved,
            created_at: "".into(),
            updated_at: "".into(),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut driver = PtyDriver {
            runner: &UnixCommandRunner,
            server: None,
            deadline,
            enter_delay: Duration::ZERO,
        };
        assert!(driver.publish_if(&binding, &identity, None).unwrap());
        let observed = state(&binding.server.socket_path, &binding.pane_id, deadline).unwrap();
        assert_eq!(observed.marker.unwrap().binding_id, "binding");
        assert!(!driver.conditional_clear(&binding).unwrap());
        handle.join().unwrap();
    }

    #[test]
    fn a_supervisor_that_ends_within_the_bound_closes_the_member() {
        let (mut child, pid, start) = spawn("sleep 0.3");
        let reaper = std::thread::spawn(move || child.wait());
        wait_for_supervisor_end(&UnixCommandRunner, pid, &start, Duration::from_secs(10))
            .expect("the supervisor ended");
        reaper.join().unwrap().unwrap();
    }

    #[test]
    fn a_running_program_is_refused_after_the_bound_and_left_alone() {
        let (mut child, pid, start) = spawn("sleep 30");
        let started = Instant::now();
        let refused =
            wait_for_supervisor_end(&UnixCommandRunner, pid, &start, Duration::from_millis(400));
        assert!(
            matches!(refused, Err(ActionError::Evidence(_))),
            "{refused:?}"
        );
        assert!(started.elapsed() >= Duration::from_millis(300));
        // Nothing was signalled: it is still the same live process.
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn another_incarnation_of_the_pid_counts_as_the_supervisor_having_ended() {
        let (mut child, pid, _) = spawn("sleep 30");
        wait_for_supervisor_end(&UnixCommandRunner, pid, "some other start", BUDGET)
            .expect("a different start identity is a different process");
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
