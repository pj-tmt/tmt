//! Binary-private lifecycle composition. Public Remote owners retain all authority.
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsFd, OwnedFd},
        unix::net::UnixStream,
    },
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tmt_remote::{
    approval::Approval,
    control::{self, Control},
    core::CoreClient,
    devices::Devices,
    error::RemoteError,
    http::{Door, Handler},
    mount::Mounts,
    operations::Operations,
    pages::Pages,
    pairing::{Pairing, Timing},
    routes::Routes,
    session::{self, DoorSessions},
    site::Site,
    state::{Layout, MachineKey},
    store::{Store, uuid_v4},
};

use tmt_remote::limits::{SERVE_RECORD_BYTES as FRAME_BYTES, SERVE_STARTUP as STARTUP};
const PULSE: Duration = Duration::from_millis(20);
const READY: u8 = 1;
const FAILED: u8 = 2;
const ACCEPT: u8 = 3;
const ACCEPTED: u8 = 4;
const CANCEL: u8 = 5;

fn startup_error() -> RemoteError {
    RemoteError::new(
        "REMOTE_STARTUP_UNCONFIRMED",
        "Remote startup could not be confirmed.",
    )
    .with_hint("inspect tmt remote status; use tmt remote stop before starting again")
}
fn cancelled() -> RemoteError {
    RemoteError::new(
        "REMOTE_STARTUP_CANCELLED",
        "Remote startup was cancelled before handoff.",
    )
}
fn fence(stop: &AtomicBool) -> Result<(), RemoteError> {
    if stop.load(Ordering::SeqCst) {
        Err(cancelled())
    } else {
        Ok(())
    }
}

struct Signals {
    ids: Vec<signal_hook::SigId>,
}
impl Signals {
    fn register(stop: &Arc<AtomicBool>, launcher: bool) -> Result<Self, RemoteError> {
        let mut signals = Self { ids: Vec::new() };
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM]
            .into_iter()
            .chain(launcher.then_some(signal_hook::consts::SIGHUP))
        {
            signals.ids.push(
                signal_hook::flag::register(signal, Arc::clone(stop)).map_err(|_| {
                    RemoteError::new("REMOTE_SIGNAL", "Could not register serve shutdown.")
                })?,
            );
        }
        Ok(signals)
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        for id in self.ids.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

/// Absolute frame deadline, including partial headers/payloads. Read timeouts
/// alone would let a byte-at-a-time peer renew the startup budget indefinitely.
fn read_exact_until(
    stream: &mut UnixStream,
    bytes: &mut [u8],
    deadline: Instant,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .min(PULSE);
        let mut descriptors = [PollFd::new(stream.as_fd(), PollFlags::POLLIN)];
        match poll(
            &mut descriptors,
            PollTimeout::try_from(remaining).unwrap_or(PollTimeout::MAX),
        ) {
            Ok(0) | Err(nix::errno::Errno::EINTR) => continue,
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        match stream.read(&mut bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => offset += n,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn write_frame(stream: &mut UnixStream, tag: u8, value: &Value) -> Result<(), RemoteError> {
    let payload = serde_json::to_vec(value).map_err(|_| startup_error())?;
    if payload.len() > FRAME_BYTES {
        return Err(startup_error());
    }
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(tag);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    stream.write_all(&frame).map_err(|_| startup_error())
}
fn read_frame(
    stream: &mut UnixStream,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<(u8, Value), RemoteError> {
    let mut header = [0; 5];
    read_exact_until(stream, &mut header, deadline, stop).map_err(|_| startup_error())?;
    if !matches!(header[0], READY | FAILED) {
        return Err(startup_error());
    }
    let length = u32::from_be_bytes(header[1..].try_into().expect("four bytes")) as usize;
    if length == 0 || length > FRAME_BYTES {
        return Err(startup_error());
    }
    let mut payload = vec![0; length];
    read_exact_until(stream, &mut payload, deadline, stop).map_err(|_| startup_error())?;
    Ok((
        header[0],
        serde_json::from_slice(&payload).map_err(|_| startup_error())?,
    ))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ready {
    profile: String,
    binding: String,
    state: String,
    address: String,
    machine_id: String,
    window_id: String,
    startup_core_calls: u8,
}
impl Ready {
    fn validate(value: Value) -> Result<Self, RemoteError> {
        let ready: Self = serde_json::from_value(value).map_err(|_| startup_error())?;
        if ready.profile != "local-v1"
            || ready.binding != "loopback-http"
            || ready.state != "ready"
            || ready.startup_core_calls != 2
            || ready.address.len() > 256
            || !ready.address.starts_with("http://127.0.0.1:")
            || ready.machine_id.len() != 36
            || !ready.machine_id.is_ascii()
            || ready.window_id.len() != 36
            || !ready.window_id.is_ascii()
        {
            return Err(startup_error());
        }
        Ok(ready)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Failed {
    code: String,
    message: String,
    hint: Option<String>,
    cleanup_confirmed: bool,
}
impl Failed {
    fn validate(value: Value) -> Result<Self, RemoteError> {
        let failed: Self = serde_json::from_value(value).map_err(|_| startup_error())?;
        if failed.code.is_empty()
            || failed.code.len() > 64
            || !failed
                .code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit())
            || failed.message.len() > 1024
            || failed.message.chars().any(char::is_control)
            || failed
                .hint
                .as_ref()
                .is_some_and(|hint| hint.len() > 512 || hint.chars().any(char::is_control))
        {
            return Err(startup_error());
        }
        Ok(failed)
    }
}

/// Only one monitor and one private endpoint during startup. Accept is read
/// before EOF; once read, neither later EOF nor lost acknowledgment cancels it.
struct Handoff {
    stream: Option<UnixStream>,
    monitor: Option<JoinHandle<bool>>,
    stop: Arc<AtomicBool>,
}
impl Handoff {
    fn new(stream: UnixStream, stop: Arc<AtomicBool>) -> Result<Self, RemoteError> {
        nix::fcntl::fcntl(
            &stream,
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
        )
        .map_err(|_| startup_error())?;
        stream
            .set_write_timeout(Some(STARTUP))
            .map_err(|_| startup_error())?;
        let mut reader = stream.try_clone().map_err(|_| startup_error())?;
        let monitor_stop = Arc::clone(&stop);
        let deadline = Instant::now() + STARTUP;
        let monitor = thread::Builder::new()
            .name("remote-startup".into())
            .spawn(move || {
                let mut command = [0];
                let accepted = read_exact_until(&mut reader, &mut command, deadline, &monitor_stop)
                    .is_ok()
                    && command[0] == ACCEPT;
                if !accepted {
                    monitor_stop.store(true, Ordering::SeqCst);
                }
                accepted
            })
            .map_err(|_| startup_error())?;
        Ok(Self {
            stream: Some(stream),
            monitor: Some(monitor),
            stop,
        })
    }
    fn ready(&mut self, value: &Value) -> Result<(), RemoteError> {
        fence(&self.stop)?;
        write_frame(
            self.stream.as_mut().expect("startup endpoint"),
            READY,
            value,
        )?;
        let accepted = self
            .monitor
            .take()
            .expect("one startup monitor")
            .join()
            .unwrap_or(false);
        if !accepted {
            return Err(cancelled());
        }
        // The command may have been accepted while a real shutdown signal raced.
        // Preserve that signal. A lost Accepted write never revokes the handoff.
        let _ = self
            .stream
            .take()
            .expect("startup endpoint")
            .write_all(&[ACCEPTED]);
        Ok(())
    }
    fn finish(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            if let Some(stream) = &self.stream {
                let _ = stream.shutdown(std::net::Shutdown::Read);
            }
            let _ = monitor.join();
        }
    }
}
impl Drop for Handoff {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Fixed local diagnostic; acquiring Serving precedes opening or clearing it.
/// File I/O has no wall-time promise. No asynchronous/deferred writer exists.
struct Diagnostic(File);
impl Diagnostic {
    fn clear(layout: &Layout) -> Result<Self, RemoteError> {
        let file = layout.file("serve-error.json")?;
        file.set_len(0)?;
        file.sync_all()?;
        Ok(Self(file))
    }
    fn failure(&mut self, error: &RemoteError) {
        let (phase, code, message) = match error.code.as_str() {
            "REMOTE_PORT_BUSY" => ("bind", "REMOTE_PORT_BUSY", "The requested port is busy."),
            "REMOTE_STATE_UNSAFE" => (
                "state",
                "REMOTE_STATE_UNSAFE",
                "Remote state was refused as unsafe.",
            ),
            "REMOTE_CORE_UNCERTAIN" => (
                "core",
                "REMOTE_CORE_UNCERTAIN",
                "Core invocation cleanup is unconfirmed.",
            ),
            "REMOTE_STARTUP_CANCELLED" => (
                "handoff",
                "REMOTE_STARTUP_CANCELLED",
                "Startup ended before handoff.",
            ),
            _ => (
                "serve",
                "REMOTE_SERVE_FAILED",
                "Remote serving failed; inspect status or use foreground mode.",
            ),
        };
        let bytes =
            serde_json::to_vec(&json!({"version":1,"phase":phase,"code":code,"message":message}))
                .expect("fixed diagnostic");
        debug_assert!(bytes.len() <= FRAME_BYTES);
        let _ = self.0.write_all(&bytes).and_then(|()| self.0.sync_all());
    }
}

pub(super) fn run(arguments: &clap::ArgMatches) -> Result<(), RemoteError> {
    let port = arguments.get_one::<u16>("port").copied();
    let json_output = arguments.get_flag("json");
    let stop = Arc::new(AtomicBool::new(false));
    let launcher = !arguments.get_flag("worker")
        && !arguments.get_flag("foreground")
        && (!json_output || arguments.get_flag("background"));
    let _signals = Signals::register(&stop, launcher)?;
    if arguments.get_flag("worker") {
        // Stdio transfers the private pair safely through exec, without unsafe
        // raw-FD adoption or a workspace lint exception. No process-group change
        // occurs in the parent; the exact worker drops its terminal immediately.
        nix::unistd::setsid().map_err(|_| startup_error())?;
        let input = std::io::stdin();
        nix::fcntl::fcntl(
            &input,
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
        )
        .map_err(|_| startup_error())?;
        let owned = input
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| startup_error())?;
        let stream = UnixStream::from(owned);
        if stream.peer_addr().is_err() {
            return Err(startup_error());
        }
        nix::unistd::close(0).map_err(|_| startup_error())?;
        let mut handoff = Handoff::new(stream, Arc::clone(&stop))?;
        let result = foreground(port, json_output, &stop, Some(&mut handoff));
        handoff.finish();
        if let Err(error) = &result {
            if let Some(stream) = handoff.stream.as_mut() {
                let message = if error.message.len() <= 1024
                    && !error.message.chars().any(char::is_control)
                {
                    error.message.clone()
                } else {
                    "Remote startup failed.".into()
                };
                let _ = write_frame(
                    stream,
                    FAILED,
                    &json!({"code":error.code,
                    "message":message,"hint":error.hint,"cleanupConfirmed":error.code != "REMOTE_CORE_UNCERTAIN"}),
                );
            }
        }
        return result;
    }
    if arguments.get_flag("foreground") || (json_output && !arguments.get_flag("background")) {
        foreground(port, json_output, &stop, None)
    } else {
        background(port, json_output, &stop)
    }
}

fn publish(value: &Value, json_output: bool, detached: bool) -> Result<(), RemoteError> {
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::message::warning(
            &mut output,
            terminal,
            "Door ready; pair a device with tmt remote pair",
            None,
        )?;
        writeln!(
            output,
            "{}",
            value["address"].as_str().expect("ready address")
        )?;
    }
    if detached && !json_output {
        writeln!(
            output,
            "Manage this door: tmt remote status; tmt remote stop"
        )?;
    }
    output.flush()?;
    Ok(())
}

fn background(
    port: Option<u16>,
    json_output: bool,
    stop: &Arc<AtomicBool>,
) -> Result<(), RemoteError> {
    let deadline = Instant::now() + STARTUP;
    let (mut parent, worker) = UnixStream::pair().map_err(|_| startup_error())?;
    parent
        .set_write_timeout(Some(STARTUP))
        .map_err(|_| startup_error())?;
    let mut command = Command::new(std::env::current_exe().map_err(|_| startup_error())?);
    command.args(["serve", "--foreground", "--worker"]);
    if let Some(port) = port {
        command.args(["--port", &port.to_string()]);
    }
    command
        .stdin(Stdio::from(OwnedFd::from(worker)))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| startup_error())?;
    drop(command);

    let mut cleanup_confirmed = false;
    let result = (|| {
        let (tag, value) = read_frame(&mut parent, deadline, stop)?;
        if tag == FAILED {
            let failed = Failed::validate(value)?;
            cleanup_confirmed = failed.cleanup_confirmed;
            let mut error = RemoteError::new(&failed.code, &failed.message);
            error.hint = failed.hint;
            return Err(error);
        }
        let ready = Ready::validate(value)?;
        fence(stop)?;
        // ONE byte: a successful write is the exact irreversible handoff cutoff.
        parent.write_all(&[ACCEPT]).map_err(|_| startup_error())?;
        Ok(ready)
    })();
    let ready = match result {
        Ok(ready) => ready,
        Err(error) => {
            let _ = parent.write_all(&[CANCEL]);
            let _ = parent.shutdown(std::net::Shutdown::Write);
            let cleanup_deadline = Instant::now() + tmt_remote::limits::STOP_WAIT;
            if !cleanup_confirmed {
                let keep_reading = AtomicBool::new(false);
                while let Ok((tag, value)) =
                    read_frame(&mut parent, cleanup_deadline, &keep_reading)
                {
                    if tag == FAILED {
                        cleanup_confirmed =
                            Failed::validate(value).is_ok_and(|failed| failed.cleanup_confirmed);
                        break;
                    }
                }
            }
            let exited = cleanup(&mut child, cleanup_deadline);
            return if exited && cleanup_confirmed {
                Err(if stop.load(Ordering::SeqCst) {
                    cancelled()
                } else {
                    error
                })
            } else {
                Err(startup_error())
            };
        }
    };
    // From here the service may be accepted: no cleanup, kill, or automatic retry.
    let mut acknowledgment = [0];
    read_exact_until(&mut parent, &mut acknowledgment, deadline, stop)
        .map_err(|_| startup_error())?;
    if acknowledgment[0] != ACCEPTED {
        return Err(startup_error());
    }
    publish(
        &serde_json::to_value(ready).expect("ready serialization"),
        json_output,
        true,
    )
    .map_err(|_| {
        RemoteError::new(
            "REMOTE_READY_OUTPUT",
            "Remote readiness output was not completed; startup may have succeeded.",
        )
        .with_hint("inspect tmt remote status; use tmt remote stop before starting again")
    })
}

/// Observe/reap only our own child. A None result leaves its PID reserved even
/// if exit races the subsequent signal. Never signal after a successful reap.
/// Forced termination cannot confirm separately grouped invocation cleanup.
fn cleanup(child: &mut Child, deadline: Instant) -> bool {
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Err(_) => return false,
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(PULSE);
    }
    let pid = child.id() as i32;
    // Before setsid the worker is not a group leader. Signal only this reserved
    // child as a fallback; never the inherited caller's process group.
    let _ = nix::sys::signal::killpg(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    );
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    );
    let reap_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < reap_deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return false;
        }
        thread::sleep(PULSE);
    }
    false
}

fn foreground(
    port: Option<u16>,
    json_output: bool,
    stop: &Arc<AtomicBool>,
    mut handoff: Option<&mut Handoff>,
) -> Result<(), RemoteError> {
    let core = CoreClient::discover()?;
    let capabilities = core.capabilities(&stop)?;
    if capabilities["version"] != 1
        || capabilities["limits"]["outputBytes"]
            .as_u64()
            .is_none_or(|n| n == 0 || n > tmt_remote::core::OUTPUT_LIMIT as u64)
    {
        return Err(RemoteError::new(
            "REMOTE_CORE_UNAVAILABLE",
            "Core advertised an unsupported protocol or output bound.",
        ));
    }
    let input_limit = capabilities["limits"]["inputBytes"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= tmt_remote::limits::CORE_INPUT_BYTES as u64)
        .ok_or_else(|| {
            RemoteError::new(
                "REMOTE_CORE_UNAVAILABLE",
                "Core did not advertise a valid input bound.",
            )
        })? as usize;
    fence(stop)?;
    let root = core.storage_root(stop)?;
    fence(stop)?;
    let layout = Layout::open(&root)?;
    fence(stop)?;
    let serving = layout.serve_lock()?;
    let mut diagnostic = if handoff.is_some() {
        Some(Diagnostic::clear(&layout)?)
    } else {
        None
    };
    let result = (|| {
        serving.retain_for_invocations()?;
        fence(stop)?;
        let machine_key = MachineKey::open(&layout)?;
        let mut store = Store::open(&serving)?;
        let machine = store.machine()?;
        let requested = port;
        let remembered = store.remembered_port()?;
        let selected = requested.or(remembered).unwrap_or(0);
        let door = Door::bind(selected).map_err(|error| {
            if requested.is_none() && remembered.is_some() && error.code == "REMOTE_PORT_BUSY" {
                RemoteError::new(
                    "REMOTE_PORT_BUSY",
                    &format!("Remote's port {selected} is in use"),
                )
                .with_hint("stop what is using it to keep this browser paired, or run tmt remote serve --port <n> and pair again")
            } else {
                error
            }
        })?;
        let bound_port = door.socket_addr()?.port();
        let store = Arc::new(Mutex::new(store));
        // Each run is a new window; grants survive it, sessions do not.
        let window_id = uuid_v4()?;
        let pairing = Arc::new(Pairing::new(
            machine.id.clone(),
            window_id.clone(),
            machine_key.public(),
            door.origin.clone(),
            Arc::clone(&store),
            Timing::CONTRACT,
        ));
        let sessions = Arc::new(DoorSessions::new(
            machine.id.clone(),
            window_id.clone(),
            door.origin.clone(),
            format!("{}/x/", machine.route_prefix),
            machine_key,
            Arc::clone(&store),
            session::IDLE,
        ));
        let operations = Arc::new(Operations::new(core, Arc::clone(&stop), input_limit));
        let routes = Routes::new(input_limit, machine.route_prefix.clone())?
            .with_pairing(Arc::clone(&pairing))
            .with_sessions(Arc::clone(&sessions))
            .with_operations(Arc::clone(&operations));
        let address = format!("{}{}", door.origin, routes.prefix());
        let approval = Arc::new(Approval::new(
            Arc::clone(&store),
            Arc::clone(&sessions),
            operations,
        ));
        approval.cancel_pending()?;
        let devices = Arc::new(Devices::new(
            Arc::clone(&store),
            Some(Arc::clone(&sessions)),
        ));
        let control = Control::start(
            &serving,
            Arc::clone(&pairing),
            Arc::clone(&devices),
            control::Door {
                origin: door.origin.clone(),
                prefix: machine.route_prefix.clone(),
            },
            Some(Arc::clone(&approval)),
            Arc::clone(&stop),
        )?;
        let site = Arc::new(Site {
            routes,
            mounts: Arc::new(Mounts::new(
                root,
                &door.origin,
                &machine.route_prefix,
                sessions,
            )),
            pages: Some(
                Pages::new(
                    &door.origin,
                    machine.id.clone(),
                    window_id.clone(),
                    &machine.route_prefix,
                )
                .with_pairing(pairing),
            ),
        });
        let events = devices.start_events(Arc::clone(&site.mounts))?;
        fence(stop)?;
        store
            .lock()
            .expect("store lock")
            .remember_port(bound_port)?;
        fence(stop)?;
        let ready = json!({"profile":"local-v1","binding":"loopback-http","state":"ready","address":address,"machineId":machine.id,"windowId":window_id,"startupCoreCalls":2});
        if let Some(handoff) = handoff.as_mut() {
            handoff.ready(&ready)?;
        } else {
            publish(&ready, json_output, false)?;
        }
        let result = door.run(&stop, site as Arc<dyn Handler>);
        // Stopping cancels any pending pairing before state is released.
        control.stop();
        approval.cancel_pending()?;
        drop(events);
        result
    })();
    if let (Some(diagnostic), Err(error)) = (&mut diagnostic, &result) {
        diagnostic.failure(error);
    }
    // Diagnostic writes/close happen while Serving is still held.
    drop(diagnostic);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_accept_wins_over_later_eof_but_never_erases_shutdown() {
        let (mut parent, worker) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut handoff = Handoff::new(worker, Arc::clone(&stop)).unwrap();
        parent.write_all(&[ACCEPT]).unwrap();
        parent.shutdown(std::net::Shutdown::Write).unwrap();
        handoff.ready(&json!({"fixture":"ready"})).unwrap();
        assert!(!stop.load(Ordering::SeqCst));
        assert!(handoff.monitor.is_none() && handoff.stream.is_none());

        let (mut parent, worker) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut handoff = Handoff::new(worker, Arc::clone(&stop)).unwrap();
        parent.write_all(&[ACCEPT]).unwrap();
        assert!(handoff.monitor.take().unwrap().join().unwrap());
        stop.store(true, Ordering::SeqCst);
        assert!(fence(&stop).is_err());
        handoff.finish();
        assert!(stop.load(Ordering::SeqCst));
    }

    #[test]
    fn eof_cancel_and_failure_wake_and_join_the_only_monitor() {
        for command in [None, Some(CANCEL), Some(99)] {
            let (mut parent, worker) = UnixStream::pair().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let mut handoff = Handoff::new(worker, Arc::clone(&stop)).unwrap();
            if let Some(command) = command {
                parent.write_all(&[command]).unwrap();
            }
            drop(parent);
            assert!(!handoff.monitor.take().unwrap().join().unwrap());
            assert!(stop.load(Ordering::SeqCst));
        }
        let (_parent, worker) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut handoff = Handoff::new(worker, stop).unwrap();
        handoff.finish();
        assert!(handoff.monitor.is_none());
    }

    #[test]
    fn frame_length_and_partial_frame_obey_one_absolute_deadline() {
        let (mut parent, mut worker) = UnixStream::pair().unwrap();
        parent.write_all(&[READY, 0, 0, 16, 1]).unwrap();
        assert!(
            read_frame(
                &mut worker,
                Instant::now() + Duration::from_secs(1),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        parent.write_all(&[READY, 0]).unwrap();
        let start = Instant::now();
        assert!(
            read_frame(
                &mut worker,
                start + Duration::from_millis(30),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
