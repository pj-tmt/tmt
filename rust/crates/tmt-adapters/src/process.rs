//! Bounded Unix child ownership shared by tmux and caller ancestry discovery.
//! The communication primitive multiplexes pipes; this boundary owns deadlines,
//! per-stream limits, failure classification, and explicit termination/reaping.

pub mod interactive;
pub mod ps;
pub mod runtime;

use nix::{
    errno::Errno,
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use std::{
    ffi::{OsStr, OsString},
    fmt,
    io::{self, Write},
    time::{Duration, Instant},
};
use subprocess::{Exec, ExecExt, Job, JobExt, Redirection};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);

pub struct CommandRequest<'a> {
    pub program: &'a OsStr,
    pub args: &'a [OsString],
    pub input: &'a [u8],
    pub deadline: Instant,
    pub max_output_bytes: usize,
}

#[derive(Debug)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandFailure {
    Spawn,
    Timeout,
    OutputLimit,
    Io,
    Exit {
        code: Option<u32>,
        signal: Option<i32>,
    },
}

#[derive(Debug)]
pub struct CommandError {
    pub kind: CommandFailure,
    pub cleanup_error: Option<io::Error>,
    /// Bounded output only for a fully observed nonzero exit. Never formatted
    /// implicitly; callers must validate a protocol before exposing it.
    pub output: Option<CommandOutput>,
    cause: Option<io::Error>,
}

impl CommandError {
    pub(crate) fn new(kind: CommandFailure) -> Self {
        Self {
            kind,
            cleanup_error: None,
            output: None,
            cause: None,
        }
    }

    fn io(kind: CommandFailure, cause: io::Error) -> Self {
        Self {
            cause: Some(cause),
            ..Self::new(kind)
        }
    }

    pub fn raw_os_error(&self) -> Option<i32> {
        self.cause.as_ref().and_then(io::Error::raw_os_error)
    }

    pub fn cleanup_failed(&self) -> bool {
        self.cleanup_error.is_some()
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Neither argv nor subprocess output belongs in public adapter errors.
        write!(output, "External command failed: {:?}", self.kind)?;
        if self.cleanup_failed() {
            write!(output, " (cleanup failed)")?;
        }
        Ok(())
    }
}

impl std::error::Error for CommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|cause| cause as _)
    }
}

pub trait CommandRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError>;
}

pub struct UnixCommandRunner;

/// Probes inside an already supervised worker must remain in its process group.
/// The worker must propagate failure to its supervisor, which owns group cleanup.
pub struct SupervisedProbeRunner;

impl SupervisedProbeRunner {
    /// Abort the worker and all probes before its leader can be reaped. A
    /// post-exit group signal is deliberately not used: its numeric ID may
    /// already be reusable. Never signal an inherited shell/process group.
    pub fn abort_worker_group() -> io::Result<()> {
        let own_pid = nix::unistd::getpid();
        if nix::unistd::getpgrp() != own_pid {
            return Err(io::Error::other("Worker does not own its process group"));
        }
        killpg(own_pid, Signal::SIGKILL).map_err(io::Error::from)
    }
}

impl CommandRunner for SupervisedProbeRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        start_command(request, false)?.wait()
    }
}

impl CommandRunner for UnixCommandRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        self.start(request)?.wait()
    }
}

impl UnixCommandRunner {
    /// Spawn under a caller's short-lived ownership guard, then wait outside it.
    pub(crate) fn start(
        &self,
        request: CommandRequest<'_>,
    ) -> Result<RunningCommand, CommandError> {
        start_command(request, true)
    }
}

fn start_command(
    request: CommandRequest<'_>,
    owns_group: bool,
) -> Result<RunningCommand, CommandError> {
    remaining(request.deadline)?;
    let command = Exec::cmd(request.program)
        .args(request.args.iter().cloned())
        .stdin(request.input.to_vec())
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe);
    let command = if owns_group {
        command.setpgid()
    } else {
        command
    };
    let job = command
        .start()
        .map_err(|cause| CommandError::io(CommandFailure::Spawn, cause))?;
    Ok(RunningCommand {
        job,
        finished: false,
        deadline: request.deadline,
        max_output_bytes: request.max_output_bytes,
        owns_group,
    })
}

/// Owns the same bounded child from successful spawn through wait or abandonment.
pub(crate) struct RunningCommand {
    job: Job,
    finished: bool,
    deadline: Instant,
    max_output_bytes: usize,
    owns_group: bool,
}

impl RunningCommand {
    pub(crate) fn wait(mut self) -> Result<CommandOutput, CommandError> {
        match communicate(&mut self.job, self.deadline, self.max_output_bytes) {
            Ok(output) => {
                self.finished = true;
                Ok(output)
            }
            Err(mut error) => {
                error.cleanup_error = self.cleanup().err();
                Err(error)
            }
        }
    }
}

fn remaining(deadline: Instant) -> Result<Duration, CommandError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| CommandError::new(CommandFailure::Timeout))
}

fn communicate(
    job: &mut Job,
    deadline: Instant,
    max_output_bytes: usize,
) -> Result<CommandOutput, CommandError> {
    let mut stdout = CappedOutput::new(max_output_bytes);
    let mut stderr = CappedOutput::new(max_output_bytes);
    {
        // Keep the Job. Exec::communicate detaches; consuming convenience
        // timeout methods can instead block inside Process::drop on failure.
        let mut communication = job
            .communicate()
            .map_err(|cause| CommandError::io(CommandFailure::Io, cause))?
            .limit_time(remaining(deadline)?);
        if let Err(cause) = communication.read_to(&mut stdout, &mut stderr) {
            let kind = if stdout.exceeded || stderr.exceeded {
                CommandFailure::OutputLimit
            } else if cause.kind() == io::ErrorKind::TimedOut {
                CommandFailure::Timeout
            } else {
                CommandFailure::Io
            };
            return Err(CommandError::io(kind, cause));
        }
    }
    // EOF is not process completion: a child can close both pipes then hang.
    // Conversely, do not reap the leader while a descendant holds its pipes:
    // retaining the leader lets failure cleanup safely signal its group.
    let status = job
        .wait_timeout(remaining(deadline)?)
        .map_err(|cause| CommandError::io(CommandFailure::Io, cause))?
        .ok_or_else(|| CommandError::new(CommandFailure::Timeout))?;
    if !status.success() {
        let mut error = CommandError::new(CommandFailure::Exit {
            code: status.code(),
            signal: status.signal(),
        });
        error.output = Some(CommandOutput {
            stdout: stdout.bytes,
            stderr: stderr.bytes,
        });
        return Err(error);
    }
    Ok(CommandOutput {
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

struct CappedOutput {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl CappedOutput {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        }
    }
}

impl Write for CappedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("External command output limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl RunningCommand {
    fn cleanup(&mut self) -> io::Result<()> {
        let signal = if self.owns_group {
            self.job.send_signal_group(Signal::SIGKILL as i32)
        } else {
            self.job.send_signal(Signal::SIGKILL as i32)
        };
        let waited = self.job.wait_timeout(CLEANUP_TIMEOUT);
        // An exceptional OS cleanup failure must not turn into an unbounded
        // blocking destructor. No reader threads were spawned by this adapter.
        // Detachment here is reported as failed cleanup, never success.
        if !matches!(waited, Ok(Some(_))) {
            self.job.detach();
        }
        self.finished = true;
        if !self.owns_group && matches!(waited, Ok(Some(_))) {
            return match signal {
                Err(error) if error.raw_os_error() == Some(Errno::ESRCH as i32) => Ok(()),
                result => result,
            };
        }
        match waited {
            Ok(Some(_)) => confirm_group_termination(signal, || {
                let pid = i32::try_from(self.job.pid()).map_err(|_| Errno::EINVAL)?;
                // Read-only, after reaping. Never signal a numeric process group
                // again once its leader may have been recycled by the kernel.
                killpg(Pid::from_raw(pid), None)
            }),
            Ok(None) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "External command could not be reaped within the cleanup budget",
            )),
            Err(error) => Err(error),
        }
    }
}

fn confirm_group_termination(
    signal: io::Result<()>,
    inspect: impl FnOnce() -> Result<(), Errno>,
) -> io::Result<()> {
    match signal {
        Err(error) if error.raw_os_error() == Some(Errno::ESRCH as i32) => Ok(()),
        // Darwin may return EPERM when a group contains only a zombie leader.
        // Reaping that leader is not sufficient proof: another member could
        // actually deny signals. Only observed group absence clears the error.
        Err(error) if error.raw_os_error() == Some(Errno::EPERM as i32) => {
            if inspect() == Err(Errno::ESRCH) {
                Ok(())
            } else {
                Err(error)
            }
        }
        result => result,
    }
}

impl Drop for RunningCommand {
    fn drop(&mut self) {
        if !self.finished {
            // Abandonment/unwind fallback. Operational errors clean up explicitly
            // so they can preserve the primary error and expose cleanup failure.
            let _ = self.cleanup();
        }
    }
}

#[cfg(test)]
mod cleanup_policy_tests {
    use super::*;

    #[test]
    fn supervised_probes_stay_in_the_supervisors_group() {
        let args = ["-c".into(), "ps -o pgid= -p $$".into()];
        let request = || CommandRequest {
            program: OsStr::new("/bin/sh"),
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(2),
            max_output_bytes: 128,
        };
        let inherited = SupervisedProbeRunner.execute(request()).unwrap();
        let group: i32 = std::str::from_utf8(&inherited.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(group, nix::unistd::getpgrp().as_raw());
        let separate = UnixCommandRunner.execute(request()).unwrap();
        let group: i32 = std::str::from_utf8(&separate.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_ne!(group, nix::unistd::getpgrp().as_raw());

        let error = SupervisedProbeRunner
            .execute(CommandRequest {
                program: OsStr::new("/bin/sleep"),
                args: &["20".into()],
                input: &[],
                deadline: Instant::now() + Duration::from_millis(50),
                max_output_bytes: 128,
            })
            .unwrap_err();
        assert_eq!(error.kind, CommandFailure::Timeout);
        assert!(!error.cleanup_failed());
    }

    #[test]
    fn permission_failure_requires_observed_group_absence() {
        for observation in [Ok(()), Err(Errno::EPERM), Err(Errno::EIO)] {
            let error = confirm_group_termination(
                Err(io::Error::from_raw_os_error(Errno::EPERM as i32)),
                || observation,
            )
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(Errno::EPERM as i32));
        }
        assert!(
            confirm_group_termination(
                Err(io::Error::from_raw_os_error(Errno::EPERM as i32)),
                || Err(Errno::ESRCH),
            )
            .is_ok()
        );
    }

    #[test]
    fn successful_or_missing_groups_do_not_need_an_extra_probe() {
        for signal in [
            Ok(()),
            Err(io::Error::from_raw_os_error(Errno::ESRCH as i32)),
        ] {
            assert!(confirm_group_termination(signal, || panic!("unexpected probe")).is_ok());
        }
    }
}
