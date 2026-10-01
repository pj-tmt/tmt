//! Minimal bounded child runner. Squad links no TMT crate, so it cannot reuse
//! the core process owner; this keeps only what one-shot core calls need.

use std::{
    ffi::OsString,
    io::{self, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use subprocess::{Exec, ExecExt, Job, JobExt, Redirection};

const KILL: i32 = 9;
const CLEANUP: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct Finished {
    pub success: bool,
    /// Core reports results and structured errors on stdout; stderr is only
    /// drained within the same bound so the child can never block on it.
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

/// One board load's generation. Superseding a switch also cancels a child
/// currently being read, without changing deadlines for ordinary commands.
#[derive(Clone)]
pub struct Cancellation {
    generation: Arc<AtomicU64>,
    expected: u64,
}

impl Cancellation {
    pub fn new(generation: Arc<AtomicU64>, expected: u64) -> Self {
        Self {
            generation,
            expected,
        }
    }
    pub fn cancelled(&self) -> bool {
        self.generation.load(Ordering::Acquire) != self.expected
    }
}

const CANCEL_WAIT: Duration = Duration::from_millis(20);

/// Runs `program args` with `input` on stdin in its own process group. A
/// deadline or output-limit failure kills that group and reaps the child.
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
    check_cancelled(cancellation)?;
    let deadline = Instant::now() + timeout;
    let mut job = Exec::cmd(program)
        .args(args.iter().cloned())
        .stdin(input.to_vec())
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe)
        .setpgid()
        .start()
        .map_err(|_| RunError::Spawn)?;
    let result = communicate(&mut job, deadline, max_output_bytes, cancellation);
    if result.is_err() {
        // Best effort by design: the failure is already reported, and a
        // stuck child must not turn into an unbounded destructor wait.
        let _ = job.send_signal_group(KILL);
        if !matches!(job.wait_timeout(CLEANUP), Ok(Some(_))) {
            job.detach();
        }
    }
    result
}

/// Read-only context calls inherit the hook's isolated host-owned group.
/// A failed communication aborts that invocation instead of handing off
/// partial context or leaving a descendant beyond the deadline.
pub fn run_inherited(
    program: &Path,
    args: &[OsString],
    input: &[u8],
    deadline: Instant,
    max_output_bytes: usize,
) -> Result<Finished, RunError> {
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::{getpgrp, getpid},
    };
    let owner = getpid();
    // A live process cannot have its PID recycled. Equality proves it owns
    // this group; an interactive caller's group is never a signal target.
    if getpgrp() != owner {
        return Err(RunError::Spawn);
    }
    remaining(deadline)?;
    let mut job = Exec::cmd(program)
        .args(args.iter().cloned())
        .stdin(input.to_vec())
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe)
        .start()
        .map_err(|_| RunError::Spawn)?;
    let result = communicate(&mut job, deadline, max_output_bytes, None);
    if result.is_err() {
        let _ = killpg(owner, Signal::SIGKILL);
        job.detach();
    }
    result
}

fn remaining(deadline: Instant) -> Result<Duration, RunError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or(RunError::Timeout)
}

fn check_cancelled(cancellation: Option<&Cancellation>) -> Result<(), RunError> {
    if cancellation.is_some_and(Cancellation::cancelled) {
        Err(RunError::Cancelled)
    } else {
        Ok(())
    }
}

fn wait_budget(
    deadline: Instant,
    cancellation: Option<&Cancellation>,
) -> Result<Duration, RunError> {
    check_cancelled(cancellation)?;
    let left = remaining(deadline)?;
    Ok(if cancellation.is_some() {
        left.min(CANCEL_WAIT)
    } else {
        left
    })
}

fn communicate(
    job: &mut Job,
    deadline: Instant,
    limit: usize,
    cancellation: Option<&Cancellation>,
) -> Result<Finished, RunError> {
    let mut stdout = Capped::new(limit);
    let mut stderr = Capped::new(limit);
    {
        let mut communication = job.communicate().map_err(|_| RunError::Io)?;
        loop {
            communication = communication.limit_time(wait_budget(deadline, cancellation)?);
            match communication.read_to(&mut stdout, &mut stderr) {
                Ok(()) => break,
                Err(_) if stdout.exceeded || stderr.exceeded => return Err(RunError::OutputLimit),
                Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                    remaining(deadline)?;
                }
                Err(_) => return Err(RunError::Io),
            }
        }
    }
    // Pipes can close before the child exits. Cancellation and the original
    // deadline cover this phase too, with the same cleanup owner.
    let status = loop {
        if let Some(status) = job
            .wait_timeout(wait_budget(deadline, cancellation)?)
            .map_err(|_| RunError::Io)?
        {
            break status;
        }
    };
    check_cancelled(cancellation)?;
    Ok(Finished {
        success: status.success(),
        stdout: stdout.bytes,
    })
}

struct Capped {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl Capped {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        }
    }
}

impl Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("output limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
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
    fn a_timed_out_group_is_killed_including_descendants() {
        let marker = std::env::temp_dir().join(format!("tmt-squad-runner-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!("(sleep 1; touch '{}') & sleep 30", marker.display());
        assert_eq!(
            sh(&script, Duration::from_millis(200), 64).unwrap_err(),
            RunError::Timeout
        );
        std::thread::sleep(Duration::from_millis(1500));
        assert!(
            !marker.exists(),
            "the background descendant was killed with its group"
        );
    }
    #[test]
    fn cancellation_prevents_spawn_and_preserves_output_across_read_slices() {
        let generation = Arc::new(AtomicU64::new(1));
        let obsolete = Cancellation::new(Arc::clone(&generation), 0);
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
        let current = Cancellation::new(generation, 1);
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
            let generation = Arc::new(AtomicU64::new(0));
            let cancellation = Cancellation::new(Arc::clone(&generation), 0);
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
            generation.store(1, Ordering::Release);
            assert_eq!(child.join().unwrap().unwrap_err(), RunError::Cancelled);
            assert!(cancelled.elapsed() < Duration::from_secs(1));
            std::thread::sleep(Duration::from_millis(1100));
            assert!(!survived.exists(), "the descendant was killed too");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
