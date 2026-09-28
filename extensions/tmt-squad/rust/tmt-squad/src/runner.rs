//! Minimal bounded child runner. Squad links no TMT crate, so it cannot reuse
//! the core process owner; this keeps only what one-shot core calls need.

use std::{
    ffi::OsString,
    io::{self, Write},
    path::Path,
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
}

/// Runs `program args` with `input` on stdin in its own process group. A
/// deadline or output-limit failure kills that group and reaps the child.
pub fn run(
    program: &Path,
    args: &[OsString],
    input: &[u8],
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<Finished, RunError> {
    let deadline = Instant::now() + timeout;
    let mut job = Exec::cmd(program)
        .args(args.iter().cloned())
        .stdin(input.to_vec())
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe)
        .setpgid()
        .start()
        .map_err(|_| RunError::Spawn)?;
    let result = communicate(&mut job, deadline, max_output_bytes);
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

fn remaining(deadline: Instant) -> Result<Duration, RunError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or(RunError::Timeout)
}

fn communicate(job: &mut Job, deadline: Instant, limit: usize) -> Result<Finished, RunError> {
    let mut stdout = Capped::new(limit);
    let mut stderr = Capped::new(limit);
    {
        let mut communication = job
            .communicate()
            .map_err(|_| RunError::Io)?
            .limit_time(remaining(deadline)?);
        if let Err(error) = communication.read_to(&mut stdout, &mut stderr) {
            return Err(if stdout.exceeded || stderr.exceeded {
                RunError::OutputLimit
            } else if error.kind() == io::ErrorKind::TimedOut {
                RunError::Timeout
            } else {
                RunError::Io
            });
        }
    }
    // Closed pipes are not completion; the child must also exit in time.
    let status = job
        .wait_timeout(remaining(deadline)?)
        .map_err(|_| RunError::Io)?
        .ok_or(RunError::Timeout)?;
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
}
