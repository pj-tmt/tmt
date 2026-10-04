//! Bounded Unix child ownership shared by tmux and caller ancestry discovery.
//! The communication primitive multiplexes pipes; this boundary owns deadlines,
//! per-stream limits, failure classification, and explicit termination/reaping.

pub mod ancestry;
pub mod detached;
pub mod interactive;
mod process_info;
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

pub(crate) const CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);

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
    /// Native process evidence, when this runner owns real local effects.
    /// Scripted runners retain the ps protocol unless they supply this evidence.
    fn process_observation(
        &self,
        _pid: u64,
        _deadline: Instant,
    ) -> Option<runtime::ProcessObservation> {
        None
    }

    /// A native parent PID, or None to retain the bounded ps fallback.
    fn process_parent(&self, _pid: u64, _deadline: Instant) -> Option<u64> {
        None
    }

    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError>;
}

/// A borrowed runner, so a short-lived caller (one external host driver per
/// binding session) runs through its handle's runner.
impl<R: CommandRunner + ?Sized> CommandRunner for &R {
    fn process_parent(&self, pid: u64, deadline: Instant) -> Option<u64> {
        (**self).process_parent(pid, deadline)
    }

    fn process_observation(
        &self,
        pid: u64,
        deadline: Instant,
    ) -> Option<runtime::ProcessObservation> {
        (**self).process_observation(pid, deadline)
    }

    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        (**self).execute(request)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UnixCommandRunner;

/// Probes inside an already supervised worker must remain in its process group.
/// The worker must propagate failure to its supervisor, which owns group cleanup.
#[derive(Debug, Clone, Copy, Default)]
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
    fn process_parent(&self, pid: u64, deadline: Instant) -> Option<u64> {
        process_info::parent(pid, deadline)
    }

    fn process_observation(
        &self,
        pid: u64,
        deadline: Instant,
    ) -> Option<runtime::ProcessObservation> {
        process_info::observe(pid, deadline)
    }

    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        let result = start_command(request, false)?.wait();
        if result.as_ref().is_err_and(|error| {
            error.cleanup_failed()
                || matches!(
                    error.kind,
                    CommandFailure::Timeout | CommandFailure::OutputLimit | CommandFailure::Io
                )
        }) {
            // Observation adapters may map unavailable evidence to None. A
            // failed bounded probe is not absence: abort while the worker owns
            // its group so that such mapping cannot leave descendants alive.
            let _ = Self::abort_worker_group();
        }
        result
    }
}

impl CommandRunner for UnixCommandRunner {
    fn process_parent(&self, pid: u64, deadline: Instant) -> Option<u64> {
        process_info::parent(pid, deadline)
    }

    fn process_observation(
        &self,
        pid: u64,
        deadline: Instant,
    ) -> Option<runtime::ProcessObservation> {
        process_info::observe(pid, deadline)
    }

    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        self.start(request)?.wait()
    }
}

impl UnixCommandRunner {
    /// Spawn under a caller's short-lived ownership guard, then wait outside it.
    pub fn start(&self, request: CommandRequest<'_>) -> Result<RunningCommand, CommandError> {
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
pub struct RunningCommand {
    job: Job,
    finished: bool,
    deadline: Instant,
    max_output_bytes: usize,
    owns_group: bool,
}

impl RunningCommand {
    pub fn wait(mut self) -> Result<CommandOutput, CommandError> {
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
    fn ready_child_exit_and_expired_wait_both_reap_the_owned_process() {
        use crate::test_support::TestDirectory;
        use nix::{
            sys::{signal::kill, stat::Mode, wait::waitpid},
            unistd::mkfifo,
        };
        use std::{fs, mem::ManuallyDrop};

        // Retain evidence on any failed lifecycle assertion; delete only after
        // both owned processes and their groups have been confirmed absent.
        let directory = ManuallyDrop::new(TestDirectory::new());
        eprintln!("owned ready-child fixture: {}", directory.path.display());
        let release = directory.path.join("release");
        mkfifo(&release, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
        for mode in ["finish", "blocked"] {
            let ready = directory.path.join(format!("{mode}.ready"));
            let marker = directory.path.join(format!("{mode}.finished"));
            let args = [
                "-i".into(),
                {
                    let mut home = OsString::from("HOME=");
                    home.push(&directory.path);
                    home
                },
                "/bin/sh".into(),
                "-c".into(),
                "printf '%s\\n' \"$$\" > \"$1\"; if [ \"$2\" = blocked ]; then read -r release < \"$3\"; fi; printf finished > \"$4\"".into(),
                "owned-probe".into(),
                ready.clone().into_os_string(),
                mode.into(),
                release.clone().into_os_string(),
                marker.clone().into_os_string(),
            ];
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut running = UnixCommandRunner
                .start(CommandRequest {
                    program: OsStr::new("/usr/bin/env"),
                    args: &args,
                    input: &[],
                    deadline,
                    max_output_bytes: 128,
                })
                .unwrap();
            let pid = Pid::from_raw(i32::try_from(running.job.pid()).unwrap());
            loop {
                match fs::read_to_string(&ready) {
                    Ok(value) if !value.is_empty() => {
                        assert_eq!(value, format!("{pid}\n"));
                        break;
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => panic!("read owned probe readiness: {error}"),
                }
                assert!(Instant::now() < deadline, "owned probe was not ready");
                std::thread::sleep(Duration::from_millis(10));
            }
            if mode == "blocked" {
                // Readiness is independent of scheduling. Expiring the private
                // handle now exercises real timeout cleanup of a known live child.
                assert_eq!(kill(pid, None), Ok(()));
                running.deadline = Instant::now();
                let error = running.wait().unwrap_err();
                assert_eq!(error.kind, CommandFailure::Timeout);
                assert!(!error.cleanup_failed());
                assert!(!marker.exists());
            } else {
                let output = running.wait().unwrap();
                assert!(output.stdout.is_empty());
                assert!(output.stderr.is_empty());
                assert_eq!(fs::read_to_string(marker).unwrap(), "finished");
            }
            assert_eq!(kill(pid, None), Err(Errno::ESRCH));
            assert_eq!(killpg(pid, None), Err(Errno::ESRCH));
            assert_eq!(waitpid(pid, None), Err(Errno::ECHILD));
        }
        drop(ManuallyDrop::into_inner(directory));
    }

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
    }

    #[test]
    fn supervised_timeout_cleans_up_with_and_without_group_ownership() {
        use crate::test_support::TestDirectory;
        use nix::sys::signal::kill;

        for mode in ["leader", "supervisor"] {
            let directory = TestDirectory::new();
            let args = timeout_fixture_args(mode, &directory.path);
            // The existing bounded runner owns this fixture group on success,
            // assertion failure and timeout; never use the test runner's group.
            let result = UnixCommandRunner.execute(CommandRequest {
                program: OsStr::new("/usr/bin/env"),
                args: &args,
                input: &[],
                deadline: Instant::now() + Duration::from_secs(15),
                max_output_bytes: 16_384,
            });
            let ready = std::fs::read_to_string(directory.path.join("ready"))
                .expect("probe published deterministic readiness before timeout");
            let ids: Vec<i32> = ready
                .split_whitespace()
                .map(|id| id.parse().unwrap())
                .collect();
            assert_eq!(ids.len(), 2);
            let (probe, group) = (Pid::from_raw(ids[0]), Pid::from_raw(ids[1]));
            let deadline = Instant::now() + Duration::from_secs(2);
            while killpg(group, None) != Err(Errno::ESRCH) {
                assert!(
                    Instant::now() < deadline,
                    "fixture group {group} survived: {result:?}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(kill(probe, None), Err(Errno::ESRCH), "probe was not reaped");
            if mode == "leader" {
                assert_eq!(
                    result.unwrap_err().kind,
                    CommandFailure::Exit {
                        code: None,
                        signal: Some(Signal::SIGKILL as i32),
                    }
                );
            } else {
                result.expect("non-leader helper returned after reaping its probe");
                assert_eq!(
                    std::fs::read_to_string(directory.path.join("result")).unwrap(),
                    "timeout-clean"
                );
            }
        }
    }

    const TIMEOUT_FIXTURE: &str =
        "process::cleanup_policy_tests::supervised_timeout_reaps_probe_before_worker_exit";
    const TIMEOUT_MODE: &str = "TMT_SUPERVISED_TIMEOUT_MODE";
    const TIMEOUT_DIRECTORY: &str = "TMT_SUPERVISED_TIMEOUT_DIRECTORY";

    fn timeout_fixture_args(mode: &str, directory: &std::path::Path) -> Vec<OsString> {
        let mut directory_env = OsString::from(format!("{TIMEOUT_DIRECTORY}="));
        directory_env.push(directory);
        vec![
            format!("{TIMEOUT_MODE}={mode}").into(),
            directory_env,
            std::env::current_exe().unwrap().into_os_string(),
            "--exact".into(),
            TIMEOUT_FIXTURE.into(),
            "--ignored".into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ]
    }

    #[test]
    #[ignore = "re-exec helper owned by the supervised timeout test"]
    fn supervised_timeout_reaps_probe_before_worker_exit() {
        use crate::test_support::TestChild;
        use nix::{
            poll::{PollFd, PollFlags, poll},
            unistd::{getpgrp, getpid, setpgid},
        };
        use std::{
            io::{BufRead, BufReader, Read},
            os::{
                fd::AsFd,
                unix::net::{UnixListener, UnixStream},
            },
            path::PathBuf,
            process::{Command, Stdio},
        };

        let Ok(mode) = std::env::var(TIMEOUT_MODE) else {
            return;
        };
        let directory = PathBuf::from(std::env::var_os(TIMEOUT_DIRECTORY).unwrap());
        let socket = directory.join("probe.sock");
        // Leave room for the terminator in macOS's 104-byte sockaddr_un.sun_path.
        assert!(socket.as_os_str().as_encoded_bytes().len() < 104);
        if mode == "probe" {
            let mut stream = UnixStream::connect(socket).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            writeln!(stream, "{} {}", getpid(), getpgrp()).unwrap();
            let mut release = [0];
            // Readiness is a socket message, not a scheduling delay. The runner
            // must kill and reap this blocked probe before aborting its worker.
            let _ = stream.read_exact(&mut release);
            return;
        }
        if mode == "supervisor" {
            setpgid(Pid::from_raw(0), Pid::from_raw(0)).unwrap();
            let child = Command::new("/usr/bin/env")
                .args(timeout_fixture_args("nonleader", &directory))
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            let mut child = TestChild::new(child);
            assert!(child.wait_for_exit(Duration::from_secs(10)).success());
            return;
        }
        if mode == "leader" {
            setpgid(Pid::from_raw(0), Pid::from_raw(0)).unwrap();
            assert_eq!(getpid(), getpgrp());
        } else {
            assert_eq!(mode, "nonleader");
            assert_ne!(getpid(), getpgrp());
        }
        let listener = UnixListener::bind(socket).unwrap();
        let args = timeout_fixture_args("probe", &directory);
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                SupervisedProbeRunner.execute(CommandRequest {
                    program: OsStr::new("/usr/bin/env"),
                    args: &args,
                    input: &[],
                    deadline: Instant::now() + Duration::from_secs(5),
                    max_output_bytes: 16_384,
                })
            });
            let mut events = [PollFd::new(listener.as_fd(), PollFlags::POLLIN)];
            assert_eq!(
                poll(&mut events, 2_000u16).unwrap(),
                1,
                "probe readiness deadline"
            );
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut stream = BufReader::new(stream);
            let mut ready = String::new();
            stream.read_line(&mut ready).unwrap();
            let ids: Vec<i32> = ready
                .split_whitespace()
                .map(|id| id.parse().unwrap())
                .collect();
            assert_eq!(ids.len(), 2);
            assert_eq!(
                ids[1],
                getpgrp().as_raw(),
                "probe must inherit worker group"
            );
            std::fs::write(directory.join("ready"), ready).unwrap();
            // Keep the readiness socket open until cleanup has reaped the probe.
            // A leader never returns here: abort_worker_group kills this process.
            let error = worker.join().unwrap().unwrap_err();
            assert_eq!(mode, "nonleader", "group leader must be killed");
            assert_eq!(error.kind, CommandFailure::Timeout);
            assert!(!error.cleanup_failed());
            assert_eq!(
                nix::sys::signal::kill(Pid::from_raw(ids[0]), None),
                Err(Errno::ESRCH)
            );
            std::fs::write(directory.join("result"), "timeout-clean").unwrap();
            drop(stream);
        });
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
