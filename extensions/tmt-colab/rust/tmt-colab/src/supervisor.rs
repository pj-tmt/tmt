//! The Remote door `tmt colab serve` attaches to or starts, through Remote's public CLI only.
//! A door Colab started is stopped and reaped on every exit path; an attached door is never touched.
use crate::door::{Door, Lookup, RemoteFailure};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use std::{
    io::{BufRead, BufReader, Read},
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

/// How long a starting door may take to print its descriptor.
const READY: Duration = Duration::from_secs(15);
/// How long a stopping door may take to exit after SIGTERM before its group is killed.
const GRACE: Duration = Duration::from_secs(3);
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
    /// Remote answered with an error envelope; shown as it is, nothing started.
    Remote(RemoteFailure),
    /// Remote said no door runs, and starting one failed.
    WouldNotStart,
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
                Some((door, supervisor)) => Self::Started {
                    door,
                    _supervisor: supervisor,
                },
                None => Self::Unavailable {
                    reason: Reason::WouldNotStart,
                },
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

/// Owns one started door until dropped. Remote chooses the port: it reuses the last one and
/// falls back to a free one itself.
pub struct Supervisor {
    done: Arc<AtomicBool>,
    watcher: Option<JoinHandle<()>>,
}

impl Supervisor {
    /// Starts `tmt remote serve --json` in its own process group and waits for its descriptor.
    /// `None` when it cannot start, exits first, takes too long or `stop` is raised; a child
    /// started by then is terminated and reaped before returning.
    fn start(stop: &AtomicBool) -> Option<(Door, Self)> {
        let mut child = Command::new(tmt_invoke::invoking_tmt().ok()?)
            .args(["remote", "serve", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            // Remote's own diagnostics (a busy port, a stale lock) are the reason a start fails.
            .stderr(Stdio::inherit())
            // Ctrl-C reaches Colab only; Colab stops this door itself, after its own socket.
            .process_group(0)
            .spawn()
            .ok()?;
        let (sender, receiver) = mpsc::channel();
        let pipe = child.stdout.take()?;
        let reader = thread::spawn(move || {
            let mut pipe = BufReader::new(pipe);
            let mut line = String::new();
            let _ = pipe.by_ref().take(DESCRIPTOR_BYTES).read_line(&mut line);
            let _ = sender.send(line);
            // Keep draining so a later write never blocks or breaks the door.
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
        });
        let deadline = Instant::now() + READY;
        let door = loop {
            if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                break None;
            }
            match receiver.recv_timeout(POLL) {
                Ok(line) => break Door::from_descriptor(&line),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
            }
        };
        let Some(door) = door else {
            terminate(&mut child);
            let _ = reader.join();
            return None;
        };
        let done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let done = Arc::clone(&done);
            thread::spawn(move || {
                watch(child, &done);
                let _ = reader.join();
            })
        };
        Some((
            door,
            Self {
                done,
                watcher: Some(watcher),
            },
        ))
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

/// Reports a door that dies on its own, then stops; stops and reaps it once `done` is set.
fn watch(mut child: Child, done: &AtomicBool) {
    loop {
        if done.load(Ordering::Acquire) {
            terminate(&mut child);
            return;
        }
        match child.try_wait() {
            Ok(None) => thread::sleep(POLL),
            Ok(Some(status)) => {
                // The door is gone; anything it left in its group goes too.
                let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
                let mut output = tmt_cli_style::stream::stderr();
                let terminal = output.terminal();
                let _ = tmt_cli_style::message::warning(
                    &mut output,
                    terminal,
                    &format!(
                        "The Remote door {}; the local space keeps running without browser access",
                        status_text(status)
                    ),
                    Some("tmt remote serve"),
                );
                return;
            }
            Err(_) => {
                terminate(&mut child);
                return;
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

/// Stops everything Colab started in the door's group and reaps the door itself. The door is
/// the group leader but may be a wrapper (core resolves `tmt remote` to a separate process), so
/// the whole group gets SIGTERM, then SIGKILL after a short grace; nothing is left behind.
fn terminate(child: &mut Child) {
    let group = Pid::from_raw(child.id() as i32);
    let _ = killpg(group, Signal::SIGTERM);
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        thread::sleep(POLL);
    }
    while Instant::now() < deadline && killpg(group, None).is_ok() {
        thread::sleep(POLL);
    }
    if killpg(group, None).is_ok() {
        let _ = killpg(group, Signal::SIGKILL);
    }
    let _ = child.wait();
}
