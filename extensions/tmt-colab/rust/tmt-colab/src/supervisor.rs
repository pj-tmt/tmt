//! The Remote door `tmt colab serve` attaches to or starts, through Remote's public CLI only.
//! A started door gets bounded group cleanup on every exit path; an attached door is never touched.
use crate::door::{Door, Lookup, RemoteFailure};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tmt_colab::cloud_sync::{Activation, ActivationFailure, Refusal};

/// How long a starting door may take to print its descriptor.
const READY: Duration = Duration::from_secs(15);
/// How long a stopping door may take to exit after SIGTERM before its group is killed.
const GRACE: Duration = tmt_colab::limits::REMOTE_STOP_GRACE;
const POLL: Duration = Duration::from_millis(50);
const DESCRIPTOR_BYTES: u64 = 16 * 1024;

/// How `serve` reaches a browser.
pub enum Access {
    /// A door already ran; Colab only learned its address.
    Attached(Door),
    /// Colab started the door; dropping the supervisor stops it.
    Started { door: Door, _supervisor: Supervisor },
    /// No door could be reached, and why.
    Unavailable { reason: Reason },
}
/// Why browser access is missing. Only `Stopped` ever starts a door.
pub enum Reason {
    /// Remote gave no answer at all, which is what a missing extension looks like until core offers
    /// the use-time check (#1575).
    Missing,
    /// Remote status or startup answered with an error envelope; shown as it is.
    Remote(RemoteFailure),
    /// Remote said no door runs, and starting one failed.
    WouldNotStart,
    /// Startup failed and cleanup could not be confirmed within its bound.
    CleanupUnconfirmed(String),
}

impl Access {
    /// What a person is told when browser access is missing: the line, and the next step if any.
    pub fn warning(&self) -> Option<(&str, Option<&'static str>)> {
        match self {
            Self::Unavailable {
                reason: Reason::Missing,
            } => Some((Door::INSTALL_HINT, None)),
            Self::Unavailable {
                reason: Reason::Remote(failure),
            } => Some((failure.text(), None)),
            Self::Unavailable {
                reason: Reason::WouldNotStart,
            } => Some((
                "The Remote door did not start; the local space runs without browser access",
                Some("tmt remote serve shows why"),
            )),
            Self::Unavailable {
                reason: Reason::CleanupUnconfirmed(message),
            } => Some((message, None)),
            _ => None,
        }
    }
    pub fn open(stop: &AtomicBool) -> Self {
        match Door::lookup() {
            Lookup::Running(door) => Self::Attached(door),
            // Only a Remote that says no door runs gets one started. An error envelope (a serve
            // that predates `status`, any other code) is shown as it is: a second door must not
            // race the one that may be running.
            Lookup::Stopped(_) => match Supervisor::start(stop) {
                Ok((door, supervisor)) => Self::Started {
                    door,
                    _supervisor: supervisor,
                },
                Err(reason) => Self::Unavailable { reason },
            },
            Lookup::Failed(failure) => Self::Unavailable {
                reason: Reason::Remote(failure),
            },
            Lookup::Unknown => Self::Unavailable {
                reason: Reason::Missing,
            },
        }
    }
}

/// Called by the generation worker only after Colab's listener is accepting.
/// The running Remote owns startup selection; no private records or fallback.
pub(crate) fn activate_sync(deadline: Instant) -> Result<Activation, ActivationFailure> {
    let invoke = |words: &[&str]| -> Result<String, ActivationFailure> {
        let (success, reply) =
            crate::door::remote_call_until(words, deadline).ok_or(ActivationFailure::Local)?;
        if success {
            Ok(reply)
        } else {
            Err(
                RemoteFailure::parse(&reply).map_or(ActivationFailure::Local, |failure| {
                    ActivationFailure::Remote(Refusal {
                        // The shared wording for a door that predates the control exchange.
                        message: failure.text().to_owned(),
                        code: failure.code,
                        state: failure.state,
                    })
                }),
            )
        }
    };
    let left = deadline
        .checked_duration_since(Instant::now())
        .ok_or(ActivationFailure::Local)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ActivationFailure::Local)?;
    let milliseconds = now
        .checked_add(left)
        .and_then(|n| u64::try_from(n.as_millis()).ok())
        .ok_or(ActivationFailure::Local)?
        .to_string();
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Activated {
        generation: String,
    }
    let activated: Activated = serde_json::from_str(&invoke(&[
        "__activate",
        "--extension",
        "colab",
        "--deadline-ms",
        &milliseconds,
        "--json",
    ])?)
    .map_err(|_| ActivationFailure::Local)?;
    let identity: serde_json::Value =
        serde_json::from_str(&invoke(&["status", "--layers", "--json"])?)
            .map_err(|_| ActivationFailure::Local)?;
    let project = identity["firestoreDeployment"]["project"]
        .as_str()
        .filter(|p| {
            !p.is_empty()
                && p.len() <= 64
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        .ok_or(ActivationFailure::Local)?
        .to_owned();
    Activation::from_reply(project, &activated.generation).ok_or(ActivationFailure::Local)
}

/// Owns one started door until dropped. Remote owns the port policy.
pub struct Supervisor {
    done: Arc<AtomicBool>,
    watcher: Option<JoinHandle<Vec<String>>>,
}

impl Supervisor {
    /// Starts `tmt remote serve --json` in its own process group and waits for its descriptor.
    /// A failed start preserves Remote's error envelope when present, otherwise the generic
    /// reason. Cancellation, deadlines and every failed start terminate and reap the child.
    fn start(stop: &AtomicBool) -> Result<(Door, Self), Reason> {
        let executable = tmt_invoke::invoking_tmt().map_err(|_| Reason::WouldNotStart)?;
        let mut child = Command::new(executable)
            .args(["remote", "serve", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Ctrl-C reaches Colab only; Colab stops this door itself, after its own socket.
            .process_group(0)
            .spawn()
            .map_err(|_| Reason::WouldNotStart)?;
        let (sender, receiver) = mpsc::channel();
        let (failure_sender, failure_receiver) = mpsc::channel();
        let pipe = child.stdout.take().expect("piped stdout");
        let reader = thread::spawn(move || {
            let mut pipe = BufReader::new(pipe);
            let mut line = String::new();
            let _ = pipe.by_ref().take(DESCRIPTOR_BYTES).read_line(&mut line);
            let failure = RemoteFailure::parse(&line);
            let _ = sender.send(line);
            // Keep draining so a later write never blocks or breaks the door.
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            failure
        });
        let pipe = child.stderr.take().expect("piped stderr");
        let diagnostics = thread::spawn(move || {
            let mut pipe = BufReader::new(pipe);
            let mut line = String::new();
            let _ = pipe.by_ref().take(DESCRIPTOR_BYTES).read_line(&mut line);
            let failure = RemoteFailure::parse(&line);
            let reported = failure.is_some();
            if let Some(failure) = failure {
                let _ = failure_sender.send(failure);
            }
            // The shared warning owns an error envelope; keep other diagnostics visible and
            // drain the pipe throughout the child's lifetime so later writes never block.
            if !reported {
                let _ = tmt_cli_style::stream::stderr().write_all(line.as_bytes());
            }
            // A style stream owns the stderr lock: never hold it while waiting on the child,
            // or Colab's own warnings could block until the door exits.
            let mut buffer = [0; 8 * 1024];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = tmt_cli_style::stream::stderr().write_all(&buffer[..n]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            None::<RemoteFailure>
        });
        let deadline = Instant::now() + READY;
        let mut startup_failure = None;
        let door = loop {
            if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                break None;
            }
            match receiver.recv_timeout(POLL) {
                Ok(line) => {
                    startup_failure = RemoteFailure::parse(&line);
                    break Door::from_descriptor(&line);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
            }
        };
        let Some(door) = door else {
            let deadline = Instant::now() + tmt_colab::limits::REMOTE_CLEANUP;
            let mut failures = abort(&mut child, deadline);
            let failure = join_until(reader, deadline, &mut failures).flatten();
            let failure = failure.or(join_until(diagnostics, deadline, &mut failures).flatten());
            let failure = startup_failure
                .or(failure)
                .or_else(|| failure_receiver.try_recv().ok());
            let reason = failure.map_or(Reason::WouldNotStart, Reason::Remote);
            return Err(if failures.is_empty() {
                reason
            } else {
                let original = match &reason {
                    Reason::Remote(failure) => failure.text(),
                    _ => "The Remote door did not start",
                };
                Reason::CleanupUnconfirmed(format!(
                    "{original}; cleanup unconfirmed: {}",
                    failures.join("; ")
                ))
            });
        };
        let done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let done = Arc::clone(&done);
            thread::spawn(move || {
                let (deadline, mut failures) = watch(child, &done);
                let _ = join_until(reader, deadline, &mut failures);
                let _ = join_until(diagnostics, deadline, &mut failures);
                failures
            })
        };
        Ok((
            door,
            Self {
                done,
                watcher: Some(watcher),
            },
        ))
    }
}

/// Cleanup uncertainty preserves the original serve failure when there is one.
#[derive(Debug)]
pub struct CleanupUnconfirmed {
    pub detail: String,
    pub original: Option<Box<dyn std::error::Error + Send + Sync>>,
}
impl std::fmt::Display for CleanupUnconfirmed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(original) = &self.original {
            write!(f, "{original}; ")?;
        }
        write!(f, "Remote cleanup unconfirmed: {}", self.detail)
    }
}
impl std::error::Error for CleanupUnconfirmed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.original
            .as_ref()
            .map(|error| error.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl Supervisor {
    fn close(&mut self) -> Result<(), CleanupUnconfirmed> {
        self.done.store(true, Ordering::Release);
        let mut failures = Vec::new();
        if let Some(watcher) = self.watcher.take()
            && let Some(worker_failures) = join_until(
                watcher,
                Instant::now() + tmt_colab::limits::REMOTE_CLEANUP + POLL,
                &mut failures,
            )
        {
            failures.extend(worker_failures);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(CleanupUnconfirmed {
                detail: failures.join("; "),
                original: None,
            })
        }
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            let mut output = tmt_cli_style::stream::stderr();
            let terminal = output.terminal();
            let _ = tmt_cli_style::message::warning(
                &mut output,
                terminal,
                &error.to_string(),
                Some("tmt remote status --json"),
            );
        }
    }
}
impl Access {
    /// Close only a door this serve started, with truthful bounded completion.
    pub fn close(mut self) -> Result<(), CleanupUnconfirmed> {
        match &mut self {
            Self::Started { _supervisor, .. } => _supervisor.close(),
            Self::Unavailable {
                reason: Reason::CleanupUnconfirmed(detail),
            } => Err(CleanupUnconfirmed {
                detail: detail.clone(),
                original: None,
            }),
            _ => Ok(()),
        }
    }
}

/// Reports a door that dies on its own, then stops; stops and reaps it once `done` is set.
fn watch(mut child: Child, done: &AtomicBool) -> (Instant, Vec<String>) {
    loop {
        if done.load(Ordering::Acquire) {
            let deadline = Instant::now() + tmt_colab::limits::REMOTE_CLEANUP;
            return (deadline, terminate(&mut child, deadline));
        }
        match observe_exit(&child) {
            Ok(false) => thread::sleep(POLL),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                // ECHILD can mean the leader no longer reserves its numeric identity.
                // Failed observation is not permission to signal a potentially reused group.
                return (
                    Instant::now() + tmt_colab::limits::REMOTE_CLEANUP,
                    vec![format!("leader exit observation failed: {error}")],
                );
            }
            Ok(true) => {
                let deadline = Instant::now() + tmt_colab::limits::REMOTE_CLEANUP;
                let failures = abort(&mut child, deadline);
                let status = child.try_wait().ok().flatten();
                let why = status.map(status_text).unwrap_or_else(|| "stopped".into());
                let mut output = tmt_cli_style::stream::stderr();
                let terminal = output.terminal();
                let _ = tmt_cli_style::message::warning(
                    &mut output,
                    terminal,
                    &format!(
                        "The Remote door {why}; the local space keeps running without browser access"
                    ),
                    Some("tmt remote serve"),
                );
                return (deadline, failures);
            }
        }
    }
}

fn status_text(status: ExitStatus) -> String {
    status.code().map_or_else(
        || "was signalled".into(),
        |code| format!("exited with status {code}"),
    )
}

/// Observe without reaping: the unreaped leader reserves the numeric group identity.
fn observe_exit(child: &Child) -> std::io::Result<bool> {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let pid = Pid::from_raw(child.id() as i32)
        .ok_or_else(|| std::io::Error::other("Invalid owned Remote PID"))?;
    waitid(
        WaitId::Pid(pid),
        WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
    )
    .map(|status| status.is_some())
    .map_err(std::io::Error::from)
}

trait ProcessCleanup {
    fn signal(&mut self, signal: Signal) -> std::io::Result<()>;
    fn reap(&mut self) -> std::io::Result<bool>;
    fn absent(&mut self) -> std::io::Result<bool>;
}
struct OwnedChild<'a>(&'a mut Child);
impl ProcessCleanup for OwnedChild<'_> {
    fn signal(&mut self, signal: Signal) -> std::io::Result<()> {
        killpg(Pid::from_raw(self.0.id() as i32), signal).map_err(std::io::Error::from)
    }
    fn reap(&mut self) -> std::io::Result<bool> {
        self.0.try_wait().map(|status| status.is_some())
    }
    fn absent(&mut self) -> std::io::Result<bool> {
        match killpg(Pid::from_raw(self.0.id() as i32), None) {
            Err(nix::errno::Errno::ESRCH) => Ok(true),
            Ok(()) => Ok(false),
            Err(error) => Err(std::io::Error::from(error)),
        }
    }
}

/// Destructive signals precede the only reap; subsequent probes are read-only.
fn cleanup(
    process: &mut impl ProcessCleanup,
    deadline: Instant,
    grace: Option<Duration>,
) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(grace) = grace {
        if let Err(error) = process.signal(Signal::SIGTERM)
            && error.raw_os_error() != Some(nix::libc::ESRCH)
        {
            failures.push(format!("TERM failed: {error}"));
        }
        let grace_end = (Instant::now() + grace).min(deadline);
        while Instant::now() < grace_end {
            match process.absent() {
                Ok(true) | Err(_) => break,
                Ok(false) => {
                    thread::sleep(POLL.min(grace_end.saturating_duration_since(Instant::now())))
                }
            }
        }
        // A pre-reap probe cannot distinguish Darwin's zombie leader from a denied group.
        // It only ends the grace; the post-reap probe below decides cleanup completion.
    }
    // This owner has never reaped the leader. Its identity remains reserved, even after exit.
    let kill_error = process.signal(Signal::SIGKILL).err();
    let mut reaped = false;
    while Instant::now() < deadline {
        match process.reap() {
            Ok(true) => {
                reaped = true;
                break;
            }
            Ok(false) => {
                thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())))
            }
            Err(error) => {
                failures.push(format!("reap failed: {error}"));
                break;
            }
        }
    }
    if !reaped {
        failures.push("leader reap unconfirmed".into());
    }
    let mut absent = false;
    while Instant::now() < deadline {
        match process.absent() {
            Ok(true) => {
                absent = true;
                break;
            }
            Ok(false) => {
                thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())))
            }
            Err(error) => {
                failures.push(format!("group probe failed: {error}"));
                break;
            }
        }
    }
    if !absent {
        failures.push("process group absence unconfirmed".into());
    }
    if let Some(error) = kill_error {
        // Mirror Core's process::confirm_group_termination: Darwin can deny KILL for an
        // unreaped zombie leader. Only successful reap AND a read-only ESRCH clear EPERM.
        let disappeared = error.raw_os_error() == Some(nix::libc::ESRCH)
            || (error.raw_os_error() == Some(nix::libc::EPERM) && reaped && absent);
        if !disappeared {
            failures.push(format!("KILL failed: {error}"));
        }
    }
    failures
}
fn terminate(child: &mut Child, deadline: Instant) -> Vec<String> {
    cleanup(&mut OwnedChild(child), deadline, Some(GRACE))
}
/// A failed startup or an unexpected exit has no graceful service left to stop.
fn abort(child: &mut Child, deadline: Instant) -> Vec<String> {
    cleanup(&mut OwnedChild(child), deadline, None)
}
fn join_until<T>(
    handle: JoinHandle<T>,
    deadline: Instant,
    failures: &mut Vec<String>,
) -> Option<T> {
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
    if !handle.is_finished() {
        failures.push("worker join timed out".into());
        return None;
    }
    match handle.join() {
        Ok(value) => Some(value),
        Err(_) => {
            failures.push("worker panicked".into());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_start_cleanup_remains_unconfirmed_at_close() {
        let access = Access::Unavailable {
            reason: Reason::CleanupUnconfirmed("startup reader join timed out".into()),
        };
        let error = access.close().unwrap_err();
        assert!(error.to_string().contains("startup reader join timed out"));
        assert!(error.original.is_none());
    }

    struct FakeProcess {
        events: Vec<&'static str>,
        reaped: bool,
        denied: bool,
        zombie_kill: bool,
        reapable: bool,
    }
    impl ProcessCleanup for FakeProcess {
        fn signal(&mut self, signal: Signal) -> std::io::Result<()> {
            assert!(!self.reaped, "numeric identity was reused after reap");
            self.events.push(if signal == Signal::SIGTERM {
                "TERM"
            } else {
                "KILL"
            });
            if self.denied || (self.zombie_kill && signal == Signal::SIGKILL) {
                Err(std::io::Error::from_raw_os_error(nix::libc::EPERM))
            } else {
                Ok(())
            }
        }
        fn reap(&mut self) -> std::io::Result<bool> {
            self.events.push("reap");
            self.reaped = self.reapable;
            Ok(self.reaped)
        }
        fn absent(&mut self) -> std::io::Result<bool> {
            self.events.push("probe");
            Ok(self.reaped && !self.denied)
        }
    }
    fn fake() -> FakeProcess {
        FakeProcess {
            events: Vec::new(),
            reaped: false,
            denied: false,
            zombie_kill: false,
            reapable: true,
        }
    }

    #[test]
    fn exit_and_stop_signal_before_reap_and_only_probe_reused_identity() {
        let mut process = fake();
        assert!(
            cleanup(
                &mut process,
                Instant::now() + Duration::from_secs(1),
                Some(Duration::ZERO)
            )
            .is_empty()
        );
        assert_eq!(process.events, ["TERM", "KILL", "reap", "probe"]);
    }

    #[test]
    fn denied_signals_and_unreaped_leader_are_bounded_and_unconfirmed() {
        let mut process = fake();
        process.denied = true;
        let errors = cleanup(
            &mut process,
            Instant::now() + Duration::from_secs(1),
            Some(Duration::ZERO),
        );
        assert!(errors.iter().any(|error| error.contains("TERM failed")));
        assert!(errors.iter().any(|error| error.contains("KILL failed")));
        let mut process = fake();
        process.reapable = false;
        let start = Instant::now();
        let errors = cleanup(
            &mut process,
            start + Duration::from_millis(10),
            Some(Duration::ZERO),
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("reap unconfirmed"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("absence unconfirmed"))
        );
    }

    #[test]
    fn darwin_zombie_kill_is_cleared_only_after_reap_and_absence() {
        let mut process = fake();
        process.zombie_kill = true;
        assert!(
            cleanup(
                &mut process,
                Instant::now() + Duration::from_secs(1),
                Some(Duration::ZERO)
            )
            .is_empty()
        );
        assert_eq!(process.events, ["TERM", "KILL", "reap", "probe"]);
        let mut process = fake();
        process.zombie_kill = true;
        process.reapable = false;
        let errors = cleanup(
            &mut process,
            Instant::now() + Duration::from_millis(10),
            Some(Duration::ZERO),
        );
        assert!(errors.iter().any(|error| error.contains("KILL failed")));
    }

    #[test]
    fn retained_reader_returns_unconfirmed_and_can_be_released_and_joined() {
        let (release, receiver) = mpsc::channel();
        let (finished, completion) = mpsc::channel();
        let handle = thread::spawn(move || {
            receiver.recv().unwrap();
            finished.send(()).unwrap();
        });
        let mut failures = Vec::new();
        assert!(
            join_until(
                handle,
                Instant::now() + Duration::from_millis(10),
                &mut failures
            )
            .is_none()
        );
        assert_eq!(failures, ["worker join timed out"]);
        release.send(()).unwrap();
        completion.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn failed_start_and_unexpected_exit_reserve_identity_until_abort_cleanup() {
        for code in [0, 23] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", &format!("exit {code}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !observe_exit(&child).unwrap() {
                assert!(Instant::now() < deadline);
                thread::sleep(POLL);
            }
            let failures = abort(&mut child, deadline);
            assert!(failures.is_empty(), "{failures:?}");
            assert_eq!(child.try_wait().unwrap().unwrap().code(), Some(code));
            assert!(OwnedChild(&mut child).absent().unwrap());
        }
        let mut process = fake();
        assert!(cleanup(&mut process, Instant::now() + Duration::from_secs(1), None).is_empty());
        assert_eq!(process.events, ["KILL", "reap", "probe"]);
    }

    #[test]
    fn private_wrapped_group_is_observed_without_reap_then_stopped() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "trap 'wait; exit 0' TERM; sleep 30 & wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        assert!(!observe_exit(&child).unwrap());
        let failures = cleanup(
            &mut OwnedChild(&mut child),
            Instant::now() + Duration::from_secs(5),
            Some(Duration::from_millis(10)),
        );
        assert!(child.try_wait().unwrap().is_some(), "{failures:?}");
        assert!(failures.is_empty(), "{failures:?}");
        assert!(OwnedChild(&mut child).absent().unwrap());
    }
}
