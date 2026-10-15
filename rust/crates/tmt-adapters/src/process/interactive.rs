//! Direct-terminal child ownership for an invocation, not a detached service.
//!
//! The shell owns foreground job control. Never signal this shared process
//! group: only the directly owned child may be terminated during cleanup.

use super::{CLEANUP_TIMEOUT, CommandError, CommandFailure};
use nix::{
    errno::Errno,
    poll::{PollFd, PollFlags, PollTimeout, poll},
    sys::signal::{SigSet, SigmaskHow, Signal, pthread_sigmask},
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
    time::{Duration, Instant},
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
        Self::start_with(program, args, &[])
    }

    /// Like `start`, with extra environment for the child only: this process's
    /// own environment is never touched.
    pub fn start_with(
        program: &OsStr,
        args: &[OsString],
        environment: &[(OsString, OsString)],
    ) -> Result<Self, CommandError> {
        // Catch rather than ignore terminal interrupts. Exec resets caught
        // handlers, so the harness receives normal terminal signal behavior.
        let signals =
            ChildSignals::install().map_err(|cause| CommandError::io(CommandFailure::Io, cause))?;
        let job = environment
            .iter()
            .fold(Exec::cmd(program), |exec, (key, value)| {
                exec.env(key, value)
            })
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

    /// Finish the launch owner's storage close before a pending terminal stop.
    /// Called only after spawn, so the provider never inherits this mask. The
    /// closure still runs if masking fails; signal errors do not undo a launch.
    /// Dispositions and every signal other than SIGTSTP remain untouched.
    pub fn with_deferred_suspend<T>(&self, close: impl FnOnce() -> T) -> (T, io::Result<()>) {
        match SuspendDeferral::block() {
            Ok(mut guard) => {
                let result = close();
                // A pending Ctrl-Z takes effect here, after the close returns.
                let restored = guard.restore();
                (result, restored)
            }
            Err(error) => (close(), Err(error)),
        }
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
    pub fn wait(self, on_degraded: impl FnOnce(&io::Error)) -> Result<u32, CommandError> {
        self.wait_observing(None, on_degraded)
    }

    /// Invocation-owned bounded observations while the original child runs.
    /// The callback must return within its own supervised budget. Exit, signals
    /// and cleanup retain this owner's ordering; degraded waiting stops ticks.
    pub fn wait_with_ticks(
        self,
        every: Duration,
        mut on_tick: impl FnMut(),
        on_degraded: impl FnOnce(&io::Error),
    ) -> Result<u32, CommandError> {
        assert!(!every.is_zero(), "observation cadence must be positive");
        self.wait_observing(Some((every, &mut on_tick)), on_degraded)
    }

    fn wait_observing(
        mut self,
        tick: Option<(Duration, &mut dyn FnMut())>,
        on_degraded: impl FnOnce(&io::Error),
    ) -> Result<u32, CommandError> {
        let result = self.wait_for_exit(tick).or_else(|cause| {
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

    fn wait_for_exit(&mut self, mut tick: Option<(Duration, &mut dyn FnMut())>) -> io::Result<u32> {
        let mut next = tick.as_ref().map(|(every, _)| Instant::now() + *every);
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
            if let Some((every, callback)) = tick.as_mut()
                && next.is_some_and(|deadline| Instant::now() >= deadline)
            {
                let started = Instant::now();
                callback();
                // Skip missed deadlines instead of issuing a burst of reads.
                next = Some(if started + *every > Instant::now() {
                    started + *every
                } else {
                    Instant::now() + *every
                });
                continue;
            }
            self.signals.wait(next)?;
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

/// Thread-local scope: preserve the entire inherited mask, including an already
/// blocked SIGTSTP. No signal handler is installed or removed.
struct SuspendDeferral {
    previous: SigSet,
    restored: bool,
    thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl SuspendDeferral {
    fn block() -> io::Result<Self> {
        let mut blocked = SigSet::empty();
        blocked.add(Signal::SIGTSTP);
        let mut previous = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&blocked), Some(&mut previous))?;
        Ok(Self {
            previous,
            restored: false,
            thread: std::marker::PhantomData,
        })
    }

    fn restore(&mut self) -> io::Result<()> {
        pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&self.previous), None)?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for SuspendDeferral {
    fn drop(&mut self) {
        if !self.restored {
            // Also restore on unwinding, or retry a failed explicit restoration.
            let _ = self.restore();
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

    fn wait(&self, deadline: Option<Instant>) -> io::Result<()> {
        loop {
            let mut events = [PollFd::new(self.reader.as_fd(), PollFlags::POLLIN)];
            let timeout = deadline.map_or(PollTimeout::NONE, |deadline| {
                PollTimeout::try_from(deadline.saturating_duration_since(Instant::now()))
                    .unwrap_or(PollTimeout::MAX)
            });
            match poll(&mut events, timeout) {
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
    fn suspension_scope_restores_the_entire_mask_on_success_error_and_unwind() {
        let original = SigSet::thread_get_mask().unwrap();
        // Restore the test thread even if an assertion panics.
        let mut cleanup = SuspendDeferral {
            previous: original,
            restored: false,
            thread: std::marker::PhantomData,
        };
        for inherited_stop in [false, true] {
            let mut inherited = original;
            inherited.add(Signal::SIGUSR1);
            if inherited_stop {
                inherited.add(Signal::SIGTSTP);
            } else {
                inherited.remove(Signal::SIGTSTP);
            }
            inherited.thread_set_mask().unwrap();
            for outcome in [0, 1, 2] {
                let result = std::panic::catch_unwind(|| {
                    let mut guard = SuspendDeferral::block().unwrap();
                    let during = SigSet::thread_get_mask().unwrap();
                    for signal in Signal::iterator() {
                        assert_eq!(
                            during.contains(signal),
                            signal == Signal::SIGTSTP || inherited.contains(signal),
                        );
                    }
                    match outcome {
                        0 => guard.restore(),
                        1 => Err(io::Error::other("storage close failed")),
                        _ => panic!("storage close unwound"),
                    }
                });
                assert_eq!(result.is_ok(), outcome != 2);
                let after = SigSet::thread_get_mask().unwrap();
                for signal in Signal::iterator() {
                    assert_eq!(after.contains(signal), inherited.contains(signal));
                }
            }
        }
        cleanup.restore().unwrap();
    }

    #[test]
    fn a_failed_close_still_restores_the_launchers_mask_and_preserves_the_child() {
        let _invocation = INVOCATION.lock().unwrap();
        let child =
            InteractiveChild::start(OsStr::new("/bin/sh"), &["-c".into(), "exit 23".into()])
                .unwrap();
        let before = SigSet::thread_get_mask().unwrap();
        let (closed, restored) = child.with_deferred_suspend(|| {
            assert!(SigSet::thread_get_mask().unwrap().contains(Signal::SIGTSTP));
            Err::<(), _>(io::Error::other("storage close failed"))
        });
        assert!(closed.is_err());
        restored.unwrap();
        let after = SigSet::thread_get_mask().unwrap();
        for signal in Signal::iterator() {
            assert_eq!(after.contains(signal), before.contains(signal));
        }
        assert_eq!(
            child.wait(|_| panic!("unexpected degradation")).unwrap(),
            23
        );
    }

    #[test]
    fn a_child_stopped_before_admission_retains_real_start_evidence() {
        use crate::process::runtime::ProcessObservation;
        use nix::sys::wait::{WaitPidFlag, WaitStatus};

        let _invocation = INVOCATION.lock().unwrap();
        let child = InteractiveChild::start(
            OsStr::new("/bin/sh"),
            &["-c".into(), "kill -STOP $$; exit 23".into()],
        )
        .unwrap();
        let pid = Pid::from_raw(i32::try_from(child.pid()).unwrap());
        // Consume only the stop notification, never reap the owned child.
        assert_eq!(
            waitpid(pid, Some(WaitPidFlag::WUNTRACED)).unwrap(),
            WaitStatus::Stopped(pid, Signal::SIGSTOP)
        );
        let observation = child
            .observe_runtime(Instant::now() + Duration::from_secs(3))
            .unwrap();
        let ProcessObservation::Stopped(incarnation) = &observation else {
            panic!("expected a real stopped incarnation, got {observation:?}");
        };
        assert_eq!(incarnation.pid(), u64::from(child.pid()));
        assert_eq!(
            observation.matches(incarnation),
            tmt_core::binding::session::RuntimeLiveness::Unknown
        );
        nix::sys::signal::kill(pid, Signal::SIGCONT).unwrap();
        assert_eq!(
            child.wait(|_| panic!("unexpected degradation")).unwrap(),
            23
        );
        assert_eq!(waitpid(pid, None), Err(Errno::ECHILD));
    }

    #[test]
    fn fast_owned_exit_retains_start_evidence_without_authorizing_delivery() {
        use crate::process::{
            CommandError, CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner,
            runtime::{ProcessObservation, observe_owned_child, observe_runtime_process},
        };
        use std::{cell::RefCell, time::Instant};

        // Record only this observation's bounded ps attempts. Forward the
        // original request unchanged so diagnostics cannot hide uncertainty.
        #[derive(Default)]
        struct RecordingRunner {
            attempts: RefCell<Vec<(Duration, Duration, String)>>,
        }
        impl CommandRunner for RecordingRunner {
            fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
                let started = Instant::now();
                let remaining = request.deadline.saturating_duration_since(started);
                let result = UnixCommandRunner.execute(request);
                let elapsed = started.elapsed();
                let evidence = match &result {
                    Ok(output) => format!(
                        "stdout={:?}, stderr={:?}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr),
                    ),
                    Err(error) => format!("{error:?}"),
                };
                self.attempts
                    .borrow_mut()
                    .push((remaining, elapsed, evidence));
                result
            }
        }

        let _invocation = INVOCATION.lock().unwrap();
        let child =
            InteractiveChild::start(OsStr::new("/bin/sh"), &["-c".into(), "exit 23".into()])
                .unwrap();
        let pid = child.pid();
        let deadline = Instant::now() + Duration::from_secs(3);
        let runner = RecordingRunner::default();
        let incarnation = loop {
            runner.attempts.borrow_mut().clear();
            match observe_owned_child(&runner, &child, deadline).unwrap_or_else(|error| {
                panic!(
                    "owned runtime probe failed: {error:?}; ps attempts (remaining, elapsed, result): {:?}",
                    runner.attempts.borrow(),
                )
            }) {
                ProcessObservation::UnreapedZombie(incarnation) => break incarnation,
                // macOS can report ?E while the owned child is still exiting.
                // Uncertainty is not exit evidence: wait within the original
                // budget, then require UnreapedZombie before using its identity.
                ProcessObservation::Live(_) | ProcessObservation::Unknown
                    if Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                other => panic!(
                    "expected an owned unreaped exit, received {other:?}; ps attempts (remaining, elapsed, result): {:?}",
                    runner.attempts.borrow(),
                ),
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
    fn foreground_ticks_release_only_the_owned_child_and_preserve_exit() {
        let _invocation = INVOCATION.lock().unwrap();
        let directory = crate::test_support::TestDirectory::new();
        let release = directory.path.join("release");
        let child = InteractiveChild::start(
            OsStr::new("/bin/sh"),
            &[
                "-c".into(),
                "while [ ! -f \"$1\" ]; do sleep 0.001; done; exit 37".into(),
                "tick-fixture".into(),
                release.clone().into_os_string(),
            ],
        )
        .unwrap();
        let pid = Pid::from_raw(i32::try_from(child.pid()).unwrap());
        let mut ticks = 0;
        let code = child
            .wait_with_ticks(
                Duration::from_millis(20),
                || {
                    ticks += 1;
                    std::fs::write(&release, "released").unwrap();
                },
                |cause| panic!("unexpected degradation: {cause}"),
            )
            .unwrap();
        assert_eq!(code, 37);
        assert!(ticks > 0);
        assert_eq!(waitpid(pid, None), Err(Errno::ECHILD));
        let fast = InteractiveChild::start(OsStr::new("/bin/sh"), &["-c".into(), "exit 23".into()])
            .unwrap();
        assert_eq!(
            fast.wait_with_ticks(
                Duration::from_secs(1),
                || panic!("tick after fast exit"),
                |cause| panic!("{cause}")
            )
            .unwrap(),
            23
        );
    }

    #[test]
    fn degraded_wait_stops_sampling_and_reaps_original_child() {
        let _invocation = INVOCATION.lock().unwrap();
        let directory = crate::test_support::TestDirectory::new();
        let release = directory.path.join("release");
        let child = InteractiveChild::start(
            OsStr::new("/bin/sh"),
            &[
                "-c".into(),
                "while [ ! -f \"$1\" ]; do sleep 0.001; done; exit 37".into(),
                "degraded-tick-fixture".into(),
                release.clone().into_os_string(),
            ],
        )
        .unwrap();
        let pid = Pid::from_raw(i32::try_from(child.pid()).unwrap());
        child
            .signals
            .reader
            .shutdown(std::net::Shutdown::Read)
            .unwrap();
        let mut degraded = false;
        assert_eq!(
            child
                .wait_with_ticks(
                    Duration::from_millis(20),
                    || panic!("degraded sampling"),
                    |_| {
                        degraded = true;
                        std::fs::write(&release, "released").unwrap();
                    }
                )
                .unwrap(),
            37
        );
        assert!(degraded);
        assert_eq!(waitpid(pid, None), Err(Errno::ECHILD));
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
    fn extra_environment_reaches_only_the_child() {
        let _invocation = INVOCATION.lock().unwrap();
        let key = "TMT_INTERACTIVE_TEST_CHILD_ONLY";
        let child = InteractiveChild::start_with(
            OsStr::new("/bin/sh"),
            &[
                OsString::from("-c"),
                OsString::from(format!("test \"${key}\" = child-value")),
            ],
            &[(OsString::from(key), OsString::from("child-value"))],
        )
        .unwrap();
        assert_eq!(
            child
                .wait(|cause| panic!("unexpected degradation: {cause}"))
                .unwrap(),
            0
        );
        assert!(std::env::var_os(key).is_none(), "this process is untouched");
        // Without it the child does not see the variable at all.
        let child = InteractiveChild::start(
            OsStr::new("/bin/sh"),
            &[
                OsString::from("-c"),
                OsString::from(format!("test -z \"${key}\"")),
            ],
        )
        .unwrap();
        assert_eq!(
            child
                .wait(|cause| panic!("unexpected degradation: {cause}"))
                .unwrap(),
            0
        );
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
