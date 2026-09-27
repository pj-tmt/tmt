//! Direct-terminal child ownership for an invocation, not a detached service.
//!
//! The shell owns foreground job control. Never signal this shared process
//! group: only the directly owned child may be terminated during cleanup.

use super::{CLEANUP_TIMEOUT, CommandError, CommandFailure};
use nix::{
    errno::Errno,
    poll::{PollFd, PollFlags, PollTimeout, poll},
};
use signal_hook::{
    SigId,
    consts::{SIGCHLD, SIGHUP, SIGINT, SIGQUIT, SIGTERM},
    flag, low_level,
};
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use subprocess::{Exec, Job, JobExt};

/// Invocation-exclusive signal registrations and the one child they protect.
/// The caller records the returned PID before waiting, and persists completion
/// after wait. Dropping early terminates and reaps only the direct child.
/// Signal-hook unregisters callbacks but does not restore default dispositions;
/// use this only in a short-lived CLI invocation, not a long-lived service.
pub struct InteractiveChild {
    job: Job,
    signals: ChildSignals,
    finished: bool,
}

impl InteractiveChild {
    pub fn start(program: &OsStr, args: &[OsString]) -> Result<Self, CommandError> {
        // Catch rather than ignore terminal interrupts. Exec resets caught
        // handlers, so the harness receives normal terminal signal behavior.
        let signals =
            ChildSignals::install().map_err(|cause| CommandError::io(CommandFailure::Io, cause))?;
        let job = Exec::cmd(program)
            .args(args.iter().cloned())
            // Defaults inherit all three streams and the parent's process group.
            .start()
            .map_err(|cause| CommandError::io(CommandFailure::Spawn, cause))?;
        Ok(Self {
            job,
            signals,
            finished: false,
        })
    }

    pub fn pid(&self) -> u32 {
        self.job.pid()
    }

    /// Retain a real start identity even if this directly owned child has
    /// already exited. No wait/reap occurs before this observation returns.
    pub fn observe_runtime(
        &self,
        deadline: std::time::Instant,
    ) -> Result<super::runtime::ProcessObservation, CommandError> {
        super::runtime::observe_owned_child(&super::UnixCommandRunner, self, deadline)
    }

    /// Return the shell-compatible exit code, preserving nonzero harness exits.
    pub fn wait(mut self, on_degraded: impl FnOnce(&io::Error)) -> Result<u32, CommandError> {
        let result = self.wait_for_exit().or_else(|cause| {
            // Broken wrapper notification plumbing must not kill a user's live
            // agent mid-turn. Terminal job control still works; only forwarding
            // wrapper-directed signals is degraded during this plain wait.
            // TERM/HUP callbacks remain installed and absorb those signals;
            // direct terminal-group delivery to the harness still applies.
            on_degraded(&cause);
            self.job.wait().map(exit_code)
        });
        match result {
            Ok(code) => {
                self.finished = true;
                Ok(code)
            }
            Err(cause) => {
                let mut error = CommandError::io(CommandFailure::Io, cause);
                error.cleanup_error = self.cleanup().err();
                Err(error)
            }
        }
    }

    fn wait_for_exit(&mut self) -> io::Result<u32> {
        loop {
            // Drain before checking completion: an exit racing with the check
            // leaves a readable notification, so the following wait cannot miss it.
            self.signals.drain()?;
            if let Some(status) = self.job.wait_timeout(Duration::ZERO)? {
                return Ok(exit_code(status));
            }
            for (signal, pending) in [
                (SIGTERM, &self.signals.terminate),
                (SIGHUP, &self.signals.hangup),
            ] {
                if pending.swap(false, Ordering::SeqCst) {
                    // The child has not been reaped, so its PID cannot be reused.
                    // A simultaneous exit is handled by the next completion check.
                    match self.job.send_signal(signal) {
                        Err(error) if error.raw_os_error() == Some(Errno::ESRCH as i32) => {}
                        result => result?,
                    }
                }
            }
            self.signals.wait()?;
        }
    }

    fn cleanup(&mut self) -> io::Result<()> {
        // Give the harness a bounded opportunity to save before forced cleanup.
        let terminated = self.job.terminate();
        if matches!(self.job.wait_timeout(CLEANUP_TIMEOUT), Ok(Some(_))) {
            self.finished = true;
            return terminated;
        }
        let killed = self.job.kill();
        let waited = self.job.wait_timeout(CLEANUP_TIMEOUT);
        self.finished = true;
        if !matches!(waited, Ok(Some(_))) {
            self.job.detach();
        }
        match waited {
            Ok(Some(_)) => killed,
            Ok(None) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Interactive child could not be reaped within the cleanup budget",
            )),
            Err(error) => Err(error),
        }
    }
}

fn exit_code(status: subprocess::ExitStatus) -> u32 {
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0).unsigned_abs())
}

impl Drop for InteractiveChild {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.cleanup();
        }
    }
}

struct ChildSignals {
    reader: UnixStream,
    terminate: Arc<AtomicBool>,
    hangup: Arc<AtomicBool>,
    registrations: Vec<SigId>,
}

impl ChildSignals {
    fn install() -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        let mut signals = Self {
            reader,
            terminate: Arc::new(AtomicBool::new(false)),
            hangup: Arc::new(AtomicBool::new(false)),
            registrations: Vec::new(),
        };
        for (signal, pending) in [(SIGTERM, &signals.terminate), (SIGHUP, &signals.hangup)] {
            signals
                .registrations
                .push(flag::register(signal, pending.clone())?);
        }
        for signal in [SIGCHLD, SIGTERM, SIGHUP, SIGINT, SIGQUIT] {
            signals
                .registrations
                .push(low_level::pipe::register(signal, writer.try_clone()?)?);
        }
        Ok(signals)
    }

    fn drain(&mut self) -> io::Result<()> {
        let mut bytes = [0; 256];
        loop {
            match self.reader.read(&mut bytes) {
                Ok(0) => return Err(io::Error::other("Child notification channel closed")),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    }

    fn wait(&self) -> io::Result<()> {
        loop {
            let mut events = [PollFd::new(self.reader.as_fd(), PollFlags::POLLIN)];
            match poll(&mut events, PollTimeout::NONE) {
                Err(Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
            let ready = events[0]
                .revents()
                .ok_or_else(|| io::Error::other("Invalid child notification state"))?;
            if ready.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL | PollFlags::POLLHUP) {
                return Err(io::Error::other("Child notification failed"));
            }
            return Ok(());
        }
    }
}

impl Drop for ChildSignals {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..).rev() {
            low_level::unregister(registration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::{sys::wait::waitpid, unistd::Pid};
    use std::sync::Mutex;

    // Signal registrations are invocation-owned, so these in-process fixtures
    // must not overlap one another. Terminal/group signaling uses isolated E2E.
    static INVOCATION: Mutex<()> = Mutex::new(());

    #[test]
    fn fast_owned_exit_retains_start_evidence_without_authorizing_delivery() {
        use crate::process::{
            UnixCommandRunner,
            runtime::{ProcessObservation, observe_runtime_process},
        };
        use std::time::Instant;
        let _invocation = INVOCATION.lock().unwrap();
        let child =
            InteractiveChild::start(OsStr::new("/bin/sh"), &["-c".into(), "exit 23".into()])
                .unwrap();
        let pid = child.pid();
        let deadline = Instant::now() + Duration::from_secs(3);
        let incarnation = loop {
            match child.observe_runtime(deadline).unwrap() {
                ProcessObservation::UnreapedZombie(incarnation) => break incarnation,
                ProcessObservation::Live(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                other => panic!("expected an owned unreaped exit, received {other:?}"),
            }
        };
        assert_eq!(incarnation.pid(), u64::from(pid));
        assert_eq!(
            observe_runtime_process(&UnixCommandRunner, u64::from(pid), deadline).unwrap(),
            ProcessObservation::Gone
        );
        assert_eq!(
            child
                .wait(|cause| panic!("unexpected degradation: {cause}"))
                .unwrap(),
            23
        );
        assert_eq!(
            waitpid(Pid::from_raw(i32::try_from(pid).unwrap()), None),
            Err(Errno::ECHILD)
        );
    }

    #[test]
    fn normal_nonzero_and_signal_exits_keep_the_harness_status() {
        let _invocation = INVOCATION.lock().unwrap();
        for (script, expected) in [("exit 0", 0), ("exit 23", 23), ("kill -TERM $$", 143)] {
            let child = InteractiveChild::start(
                OsStr::new("/bin/sh"),
                &[OsString::from("-c"), OsString::from(script)],
            )
            .unwrap();
            let pid = Pid::from_raw(i32::try_from(child.pid()).unwrap());
            assert_eq!(
                child
                    .wait(|cause| panic!("unexpected degradation: {cause}"))
                    .unwrap(),
                expected
            );
            assert_eq!(waitpid(pid, None), Err(Errno::ECHILD));
        }
    }

    #[test]
    fn abandoned_child_is_killed_and_reaped_without_signaling_its_group() {
        let _invocation = INVOCATION.lock().unwrap();
        let child =
            InteractiveChild::start(OsStr::new("/bin/sleep"), &[OsString::from("60")]).unwrap();
        let pid = Pid::from_raw(i32::try_from(child.pid()).unwrap());
        drop(child);
        assert_eq!(waitpid(pid, None), Err(Errno::ECHILD));
    }

    #[test]
    fn missing_executable_is_a_spawn_error() {
        let _invocation = INVOCATION.lock().unwrap();
        let result =
            InteractiveChild::start(OsStr::new("/nonexistent/tmt-interactive-test-harness"), &[]);
        assert!(matches!(result, Err(error) if error.kind == CommandFailure::Spawn));
    }

    #[test]
    fn notification_failure_preserves_the_child_exit_and_reports_degradation() {
        let _invocation = INVOCATION.lock().unwrap();
        let child = InteractiveChild::start(
            OsStr::new("/bin/sh"),
            &[OsString::from("-c"), OsString::from("exit 37")],
        )
        .unwrap();
        child
            .signals
            .reader
            .shutdown(std::net::Shutdown::Read)
            .unwrap();
        let pid = Pid::from_raw(i32::try_from(child.pid()).unwrap());
        let mut notified = false;
        assert_eq!(child.wait(|_| notified = true).unwrap(), 37);
        assert!(notified);
        assert_eq!(waitpid(pid, None), Err(Errno::ECHILD));
    }

    #[test]
    fn isolated_signals_preserve_wrapper_and_reap_the_harness() {
        use crate::{
            process::{CommandRequest, CommandRunner, UnixCommandRunner},
            test_support::TestDirectory,
        };
        use std::time::Instant;

        // The existing bounded runner owns the entire fixture group. Test
        // failure therefore cannot leave either wrapper or harness behind.
        // These are signal-routing tests, not proof of terminal job control.
        const SCRIPT: &str = r#"
set -eu
ulimit -c 0
export TMT_INTERACTIVE_FIXTURE_DIR="$2"
export TMT_INTERACTIVE_FIXTURE_MODE="$3"
"$1" --exact process::interactive::tests::signal_fixture --nocapture --test-threads=1 &
wrapper=$!
while [ ! -s "$2/ready" ]; do
  kill -0 "$wrapper"
  sleep 0.01
done
read -r harness < "$2/ready"
case "$3" in
  INT|QUIT) kill -"$3" "$wrapper" "$harness" ;;
  TERM|HUP) kill -"$3" "$wrapper" ;;
  DEGRADED)
    while [ ! -f "$2/degraded" ]; do
      kill -0 "$wrapper"
      sleep 0.01
    done
    kill -0 "$wrapper"
    kill -0 "$harness"
    : > "$2/release"
    ;;
  DROP) ;;
esac
wait "$wrapper"
test "$(cat "$2/result")" = "$4"
if [ "$3" = DROP ]; then test "$(cat "$2/graceful")" = saved; fi
if kill -0 "$harness" 2>/dev/null; then exit 91; fi
"#;
        for (signal, expected) in [
            ("INT", "130"),
            ("QUIT", "131"),
            ("TERM", "143"),
            ("HUP", "129"),
            ("DEGRADED", "37"),
            ("DROP", "0"),
        ] {
            let directory = TestDirectory::new();
            let args = vec![
                OsString::from("-c"),
                OsString::from(SCRIPT),
                OsString::from("interactive-signal-fixture"),
                std::env::current_exe().unwrap().into_os_string(),
                directory.path.clone().into_os_string(),
                OsString::from(signal),
                OsString::from(expected),
            ];
            UnixCommandRunner
                .execute(CommandRequest {
                    program: OsStr::new("/bin/sh"),
                    args: &args,
                    input: &[],
                    deadline: Instant::now() + Duration::from_secs(10),
                    max_output_bytes: 16_384,
                })
                .unwrap_or_else(|error| panic!("{signal} fixture failed: {error:?}"));
        }
    }

    #[test]
    fn signal_fixture() {
        let Some(directory) = std::env::var_os("TMT_INTERACTIVE_FIXTURE_DIR") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        let mode = std::env::var("TMT_INTERACTIVE_FIXTURE_MODE").unwrap();
        let degraded = mode == "DEGRADED";
        let child = if mode == "DROP" {
            InteractiveChild::start(OsStr::new("/bin/sh"), &[
                OsString::from("-c"),
                OsString::from("trap 'printf saved > \"$1/graceful\"; exit 0' TERM; : > \"$1/trap-ready\"; while :; do sleep 0.01; done"),
                OsString::from("graceful-harness"),
                directory.clone().into_os_string(),
            ]).unwrap()
        } else if degraded {
            InteractiveChild::start(
                OsStr::new("/bin/sh"),
                &[
                    OsString::from("-c"),
                    OsString::from("while [ ! -f \"$1/release\" ]; do sleep 0.01; done; exit 37"),
                    OsString::from("degraded-harness"),
                    directory.clone().into_os_string(),
                ],
            )
            .unwrap()
        } else {
            InteractiveChild::start(OsStr::new("/bin/sleep"), &[OsString::from("60")]).unwrap()
        };
        let pid = child.pid();
        std::fs::write(directory.join("ready.tmp"), format!("{pid}\n")).unwrap();
        std::fs::rename(directory.join("ready.tmp"), directory.join("ready")).unwrap();
        if mode == "DROP" {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !directory.join("trap-ready").exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "harness trap did not become ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            drop(child);
            assert_eq!(
                waitpid(Pid::from_raw(i32::try_from(pid).unwrap()), None),
                Err(Errno::ECHILD)
            );
            std::fs::write(directory.join("result"), "0").unwrap();
            return;
        }
        if degraded {
            child
                .signals
                .reader
                .shutdown(std::net::Shutdown::Read)
                .unwrap();
        }
        let code = child
            .wait(|cause| {
                assert!(degraded, "unexpected degradation: {cause}");
                std::fs::write(directory.join("degraded"), "degraded").unwrap();
            })
            .unwrap();
        assert_eq!(
            waitpid(Pid::from_raw(i32::try_from(pid).unwrap()), None),
            Err(Errno::ECHILD)
        );
        std::fs::write(directory.join("result"), code.to_string()).unwrap();
    }
}
