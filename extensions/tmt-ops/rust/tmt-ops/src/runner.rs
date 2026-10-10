//! Squad's command/error mapping over the neutral bounded process owner.

use std::{
    ffi::OsString,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tmt_invoke::{Cleanup, EnvironmentPolicy, FailureKind, LaunchOptions, ProcessGroup, Request};

thread_local! {
    static NO_COMMANDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks the calling thread as one that must never wait on a command. The board
/// session loop holds it: a key, a click or a frame hands any command to a
/// worker. Debug builds and tests panic if this thread starts one.
pub struct NoCommands(());

pub fn forbid_commands() -> NoCommands {
    NO_COMMANDS.set(true);
    NoCommands(())
}

impl Drop for NoCommands {
    fn drop(&mut self) {
        NO_COMMANDS.set(false);
    }
}

/// Called by every function here that starts a process.
pub fn assert_commands_allowed(what: &str) {
    debug_assert!(
        !NO_COMMANDS.get(),
        "{what} would start a command on the board's input/render thread"
    );
}

#[derive(Debug)]
pub struct Finished {
    pub success: bool,
    /// Core results/errors use stdout. Invoke also drains bounded stderr.
    pub stdout: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    Spawn,
    Timeout,
    OutputLimit,
    Io,
    Cancelled,
}

/// One generation's stop flag, set by the refresh owner and never reset.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub fn run(
    program: &Path,
    args: &[OsString],
    input: &[u8],
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<Finished, RunError> {
    run_cancellable(program, args, input, timeout, max_output_bytes, None)
}

pub fn run_cancellable(
    program: &Path,
    args: &[OsString],
    input: &[u8],
    timeout: Duration,
    max_output_bytes: usize,
    cancellation: Option<&Cancellation>,
) -> Result<Finished, RunError> {
    run_cancellable_with_environment(
        program,
        args,
        input,
        timeout,
        max_output_bytes,
        cancellation,
        EnvironmentPolicy::Inherit,
    )
}

pub(crate) fn run_cancellable_with_environment(
    program: &Path,
    args: &[OsString],
    input: &[u8],
    timeout: Duration,
    max_output_bytes: usize,
    cancellation: Option<&Cancellation>,
    environment: EnvironmentPolicy<'_>,
) -> Result<Finished, RunError> {
    assert_commands_allowed("run");
    check_cancelled(cancellation)?;
    let output = tmt_invoke::invoke(
        Request {
            program,
            args,
            input,
            deadline: Instant::now() + timeout,
            max_stream_bytes: max_output_bytes,
            launch: LaunchOptions {
                environment,
                ..Default::default()
            },
        },
        cancellation.map(|token| token.0.as_ref()),
    )
    .map_err(|error| run_error(error.kind))?;
    check_cancelled(cancellation)?;
    Ok(Finished {
        success: output.status.success(),
        stdout: output.stdout,
    })
}

/// Context runs inside the hook's isolated host-owned group; a started failure
/// aborts that invocation, never an interactive caller's group.
pub fn run_inherited(
    program: &Path,
    args: &[OsString],
    input: &[u8],
    deadline: Instant,
    max_output_bytes: usize,
    environment: EnvironmentPolicy<'_>,
) -> Result<Finished, RunError> {
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::{getpgrp, getpid},
    };
    assert_commands_allowed("run_inherited");
    let owner = getpid();
    if getpgrp() != owner {
        return Err(RunError::Spawn);
    }
    let output = tmt_invoke::invoke(
        Request {
            program,
            args,
            input,
            deadline,
            max_stream_bytes: max_output_bytes,
            launch: LaunchOptions {
                environment,
                process_group: ProcessGroup::InheritCaller,
                ..Default::default()
            },
        },
        None,
    )
    .map_err(|error| {
        if matches!(error.cleanup, Cleanup::CallerOwned) {
            let _ = killpg(owner, Signal::SIGKILL);
        }
        run_error(error.kind)
    })?;
    Ok(Finished {
        success: output.status.success(),
        stdout: output.stdout,
    })
}

fn check_cancelled(cancellation: Option<&Cancellation>) -> Result<(), RunError> {
    if cancellation.is_some_and(Cancellation::cancelled) {
        Err(RunError::Cancelled)
    } else {
        Ok(())
    }
}

fn run_error(kind: FailureKind) -> RunError {
    match kind {
        FailureKind::Spawn => RunError::Spawn,
        FailureKind::Deadline => RunError::Timeout,
        FailureKind::Interrupted => RunError::Cancelled,
        FailureKind::OutputLimit(_) => RunError::OutputLimit,
        FailureKind::Io(_) => RunError::Io,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str, timeout: Duration, limit: usize) -> Result<Finished, RunError> {
        run(
            Path::new("/bin/sh"),
            &["-c".into(), script.into()],
            b"input",
            timeout,
            limit,
        )
    }

    #[test]
    fn success_failure_timeout_and_limit_are_distinct() {
        let echoed = sh("cat; echo err >&2", Duration::from_secs(5), 64).unwrap();
        assert!(echoed.success);
        assert_eq!(echoed.stdout, b"input");
        assert_eq!(
            sh("yes >&2 | head -c 4096 >&2", Duration::from_secs(5), 64).unwrap_err(),
            RunError::OutputLimit,
            "stderr is bounded too"
        );
        assert!(!sh("exit 3", Duration::from_secs(5), 64).unwrap().success);
        let started = Instant::now();
        assert_eq!(
            sh("sleep 30", Duration::from_millis(200), 64).unwrap_err(),
            RunError::Timeout
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timeout kills, never waits"
        );
        assert_eq!(
            sh("yes | head -c 4096", Duration::from_secs(5), 64).unwrap_err(),
            RunError::OutputLimit
        );
        assert_eq!(
            run(
                Path::new("/nonexistent/tmt"),
                &[],
                b"",
                Duration::from_secs(1),
                64
            )
            .unwrap_err(),
            RunError::Spawn
        );
    }

    #[test]
    fn cancellation_prevents_spawn_and_preserves_output_across_read_slices() {
        let obsolete = Cancellation::default();
        obsolete.cancel();
        assert_eq!(
            run_cancellable(
                Path::new("/nonexistent/tmt"),
                &[],
                b"",
                Duration::from_secs(2),
                64,
                Some(&obsolete)
            )
            .unwrap_err(),
            RunError::Cancelled
        );
        let current = Cancellation::default();
        let finished = run_cancellable(
            Path::new("/bin/sh"),
            &[
                "-c".into(),
                "printf first; sleep 0.08; printf second".into(),
            ],
            b"",
            Duration::from_secs(2),
            64,
            Some(&current),
        )
        .unwrap();
        assert!(finished.success);
        assert_eq!(finished.stdout, b"firstsecond");
    }

    #[test]
    fn cancellation_kills_the_group_with_open_or_closed_output_pipes() {
        for closed in [false, true] {
            let root =
                std::env::temp_dir().join(format!("squad-cancel-{}-{closed}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            let ready = root.join("ready");
            let survived = root.join("survived");
            let generation = Cancellation::default();
            let cancellation = generation.clone();
            let script = format!(
                "{} (sleep 1; touch '{}') & touch '{}'; sleep 30",
                if closed { "exec >/dev/null 2>&1;" } else { "" },
                survived.display(),
                ready.display()
            );
            let child = std::thread::spawn(move || {
                run_cancellable(
                    Path::new("/bin/sh"),
                    &["-c".into(), script.into()],
                    b"",
                    Duration::from_secs(3),
                    64,
                    Some(&cancellation),
                )
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            while !ready.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(ready.exists(), "child reached the cancellable wait");
            let cancelled = Instant::now();
            generation.cancel();
            assert_eq!(child.join().unwrap().unwrap_err(), RunError::Cancelled);
            assert!(cancelled.elapsed() < Duration::from_secs(1));
            std::thread::sleep(Duration::from_millis(1100));
            assert!(!survived.exists(), "the descendant was killed too");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[cfg(test)]
#[path = "runner_inherited_tests.rs"]
mod inherited_tests;
