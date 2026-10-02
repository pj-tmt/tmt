use crate::{
    Cleanup, EnvironmentPolicy, ExitStatus, FailureKind, InvokeError, Output, Phase, ProcessGroup,
    Request, Stream,
};
use nix::{errno::Errno, sys::signal::killpg, unistd::Pid};
use std::{
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use subprocess::{Exec, ExecExt, Job, JobExt, Redirection};

// Bound stop-flag latency without extending the absolute request deadline.
const PULSE: Duration = Duration::from_millis(20);
const CLEANUP: Duration = Duration::from_secs(1);

fn failure(kind: FailureKind, cause: Option<io::Error>) -> InvokeError {
    InvokeError {
        kind,
        cause,
        cleanup: Cleanup::NotStarted,
    }
}

fn remaining(deadline: Instant, stop: Option<&AtomicBool>) -> Result<Duration, InvokeError> {
    if stop.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(failure(FailureKind::Interrupted, None));
    }
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or_else(|| failure(FailureKind::Deadline, None))
}

pub(crate) fn invoke(
    request: Request<'_>,
    stop: Option<&AtomicBool>,
) -> Result<Output, InvokeError> {
    remaining(request.deadline, stop)?;
    let mut command = Exec::cmd(request.program);
    if let EnvironmentPolicy::ClearAllowlist(names) = request.launch.environment {
        command = command
            .env_clear()
            .env_extend(std::env::vars_os().filter(|(name, _)| names.contains(name)));
    }
    if request.launch.process_group == ProcessGroup::New {
        command = command.setpgid();
    }
    let job = command
        .args(request.args.iter().cloned())
        .stdin(request.input.to_vec())
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe)
        .start()
        .map_err(|cause| failure(FailureKind::Spawn, Some(cause)))?;
    let mut child = Child {
        job,
        finished: false,
        group: request.launch.process_group,
    };
    let observed = observe(
        &mut child.job,
        request.deadline,
        request.max_stream_bytes,
        stop,
    );
    let result = finish(observed, || child.cleanup());
    child.finished = true;
    result
}

// The private seam keeps exceptional cleanup testable without public runner modes.
fn finish(
    result: Result<Output, InvokeError>,
    cleanup: impl FnOnce() -> Cleanup,
) -> Result<Output, InvokeError> {
    result.map_err(|mut error| {
        error.cleanup = cleanup();
        error
    })
}

fn observe(
    job: &mut Job,
    deadline: Instant,
    limit: usize,
    stop: Option<&AtomicBool>,
) -> Result<Output, InvokeError> {
    let mut stdout = Capped::new(limit);
    let mut stderr = Capped::new(limit);
    {
        let mut communication = job
            .communicate()
            .map_err(|cause| failure(FailureKind::Io(Phase::OpenPipes), Some(cause)))?;
        loop {
            communication = communication.limit_time(remaining(deadline, stop)?.min(PULSE));
            match communication.read_to(&mut stdout, &mut stderr) {
                Ok(()) => break,
                Err(cause) if cause.kind() == io::ErrorKind::TimedOut => continue,
                Err(cause) => {
                    let kind = if stdout.exceeded {
                        FailureKind::OutputLimit(Stream::Stdout)
                    } else if stderr.exceeded {
                        FailureKind::OutputLimit(Stream::Stderr)
                    } else {
                        FailureKind::Io(Phase::Communicate)
                    };
                    return Err(failure(kind, Some(cause)));
                }
            }
        }
    }
    // EOF is not completion. Retain the leader until all pipe observation ends.
    loop {
        let left = remaining(deadline, stop)?;
        if let Some(status) = job
            .wait_timeout(left.min(PULSE))
            .map_err(|cause| failure(FailureKind::Io(Phase::Wait), Some(cause)))?
        {
            return Ok(Output {
                status: ExitStatus {
                    code: status.code(),
                    signal: status.signal(),
                },
                stdout: stdout.bytes,
                stderr: stderr.bytes,
            });
        }
    }
}

struct Capped {
    bytes: Vec<u8>,
    limit: usize,
    // Distinguish a rejected write from a pipe I/O error without parsing text.
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
            return Err(io::Error::other("output bound exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Child {
    job: Job,
    finished: bool,
    group: ProcessGroup,
}

impl Child {
    fn cleanup(&mut self) -> Cleanup {
        if self.group == ProcessGroup::InheritCaller {
            // No signal, wait or group inspection: this group belongs to the caller.
            self.job.detach();
            return Cleanup::CallerOwned;
        }
        let signal = self.job.send_signal_group(9);
        let waited = self.job.wait_timeout(CLEANUP);
        if !matches!(waited, Ok(Some(_))) {
            self.job.detach();
        }
        match confirm(signal, waited, || {
            let pid = i32::try_from(self.job.pid()).map_err(|_| Errno::EINVAL)?;
            // Read-only after reap: never signal a potentially recycled group.
            killpg(Pid::from_raw(pid), None)
        }) {
            Ok(()) => Cleanup::Confirmed,
            Err(cause) => Cleanup::Unconfirmed(cause),
        }
    }
}

fn confirm(
    signal: io::Result<()>,
    waited: io::Result<Option<subprocess::ExitStatus>>,
    inspect: impl FnOnce() -> Result<(), Errno>,
) -> io::Result<()> {
    match waited {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "child could not be reaped",
            ));
        }
        Err(cause) => return Err(cause),
    }
    match signal {
        Err(cause) if cause.raw_os_error() == Some(Errno::ESRCH as i32) => Ok(()),
        // Darwin can deny a zombie-only group; only observed absence clears denial.
        Err(cause) if cause.raw_os_error() == Some(Errno::EPERM as i32) => {
            if inspect() == Err(Errno::ESRCH) {
                Ok(())
            } else {
                Err(cause)
            }
        }
        result => result,
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use subprocess::unix::ExitStatusExt;

    #[test]
    fn cleanup_confirmation_and_original_failure_are_independent() {
        let reaped = || Ok(Some(subprocess::ExitStatus::from_raw(0)));
        assert!(confirm(Ok(()), reaped(), || panic!("no inspection needed")).is_ok());
        assert!(
            confirm(
                Err(io::Error::from_raw_os_error(Errno::ESRCH as i32)),
                reaped(),
                || panic!("absence established")
            )
            .is_ok()
        );
        for absence in [true, false] {
            let result = confirm(
                Err(io::Error::from_raw_os_error(Errno::EPERM as i32)),
                reaped(),
                || if absence { Err(Errno::ESRCH) } else { Ok(()) },
            );
            assert_eq!(result.is_ok(), absence);
        }
        assert!(confirm(Ok(()), Ok(None), || panic!("not reaped")).is_err());
        assert!(
            confirm(Ok(()), Err(io::Error::other("wait failed")), || panic!(
                "wait failed"
            ))
            .is_err()
        );
        let error = finish(Err(failure(FailureKind::Deadline, None)), || {
            Cleanup::Unconfirmed(io::Error::other("kill denied"))
        })
        .unwrap_err();
        assert_eq!(error.kind, FailureKind::Deadline);
        assert!(matches!(error.cleanup, Cleanup::Unconfirmed(_)));
        let output = Output {
            status: ExitStatus {
                code: Some(0),
                signal: None,
            },
            stdout: vec![],
            stderr: vec![],
        };
        assert!(finish(Ok(output), || panic!("completed child is not killed")).is_ok());
    }
}
