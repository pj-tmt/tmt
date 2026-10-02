//! Bounded process-start observations. These do not establish interface ownership.

use super::{CommandError, CommandRunner, ps::query_ps};
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use std::{
    collections::{BTreeSet, HashMap},
    ffi::OsString,
    time::Instant,
};
use tmt_core::binding::session::RuntimeLiveness;
use tmt_core::endpoint::ProcessIncarnation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessObservation {
    Live(ProcessIncarnation),
    /// Present but stopped: refuse a second foreground launch, without treating
    /// the suspended process as a deliverable runtime.
    Stopped(ProcessIncarnation),
    /// Only an owned, unreaped child's probe retains this identity. Ordinary
    /// runtime observation maps zombies to Gone, and this never permits input.
    UnreapedZombie(ProcessIncarnation),
    Gone,
    Unknown,
}

impl ProcessObservation {
    pub fn matches(&self, expected: &ProcessIncarnation) -> RuntimeLiveness {
        match self {
            Self::Live(actual) if actual == expected => RuntimeLiveness::Alive,
            Self::Stopped(actual) if actual == expected => RuntimeLiveness::Unknown,
            Self::Live(_) | Self::Stopped(_) | Self::UnreapedZombie(_) | Self::Gone => {
                RuntimeLiveness::Gone
            }
            Self::Unknown => RuntimeLiveness::Unknown,
        }
    }
}

/// Uses the same child deadline/output/cleanup owner as other native probes.
/// ps start identities have second resolution; they are not cryptographic tokens.
/// Callers must also establish that this process belongs to the target interface.
pub fn observe_runtime_process<R: CommandRunner>(
    runner: &R,
    pid: u64,
    deadline: Instant,
) -> Result<ProcessObservation, CommandError> {
    observe_process(runner, pid, deadline).map(|observation| match observation {
        ProcessObservation::UnreapedZombie(_) => ProcessObservation::Gone,
        other => other,
    })
}

/// Runtime liveness of a binding whose endpoint the caller has verified. This
/// neither establishes presence nor grants routing authority; every host uses
/// it after its own endpoint check.
pub fn binding_runtime<R: CommandRunner>(
    runner: &R,
    binding: &tmt_core::binding::Binding,
    deadline: Instant,
) -> Result<tmt_core::binding::session::RuntimeState, CommandError> {
    use tmt_core::binding::session::RuntimeState;
    let mut runtime = binding.session.state;
    if runtime == RuntimeState::Ended {
        return Ok(runtime);
    }
    if let Some(key) = &binding.session.key {
        let observation = observe_runtime_process(runner, key.incarnation.pid(), deadline)?;
        runtime = match observation.matches(&key.incarnation) {
            RuntimeLiveness::Alive => runtime,
            RuntimeLiveness::Gone => RuntimeState::Ended,
            RuntimeLiveness::Unknown => RuntimeState::Unknown,
        };
        if runtime == RuntimeState::Running
            && let Some(owner) = &binding.session.launch_owner
        {
            let observation = observe_runtime_process(runner, owner.pid(), deadline)?;
            if observation.matches(owner) != RuntimeLiveness::Alive {
                runtime = RuntimeState::Unknown;
            }
        }
    } else if runtime == RuntimeState::Running {
        runtime = RuntimeState::Unknown;
    }
    Ok(runtime)
}

/// Whether a recorded process is conclusively gone (`ESRCH`); anything else,
/// including a reused PID we cannot tell apart, is not proof of loss.
pub fn recorded_process_gone(pid: u64) -> bool {
    i32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 0)
        .is_some_and(|pid| kill(Pid::from_raw(pid), None) == Err(Errno::ESRCH))
}

/// Called only while InteractiveChild still owns an unreaped PID. This is not
/// a general routing probe: retained zombie evidence is lifecycle-only.
pub(super) fn observe_owned_child<R: CommandRunner>(
    runner: &R,
    child: &super::interactive::InteractiveChild,
    deadline: Instant,
) -> Result<ProcessObservation, CommandError> {
    observe_process(runner, u64::from(child.pid()), deadline)
}

fn observe_process<R: CommandRunner>(
    runner: &R,
    pid: u64,
    deadline: Instant,
) -> Result<ProcessObservation, CommandError> {
    let Ok(raw_pid) = i32::try_from(pid) else {
        return Ok(ProcessObservation::Unknown);
    };
    if raw_pid <= 0 {
        return Ok(ProcessObservation::Unknown);
    }
    let args: Vec<OsString> = ["-p", &pid.to_string(), "-o", "lstart=", "-o", "stat="]
        .into_iter()
        .map(Into::into)
        .collect();
    match query_ps(runner, &args, deadline, 512) {
        Ok(output) => return Ok(parse_process_observation(pid, &output.stdout)),
        Err(error) if error.cleanup_failed() => return Err(error),
        Err(_) => {}
    }
    Ok(if kill(Pid::from_raw(raw_pid), None) == Err(Errno::ESRCH) {
        ProcessObservation::Gone
    } else {
        ProcessObservation::Unknown
    })
}

/// The start token of a live or stopped process, recorded beside its pid so
/// a reused pid can later be told apart. `None` when the process can't be
/// observed or is gone; only a child that could not be cleaned up fails.
pub fn observe_start<R: CommandRunner>(
    runner: &R,
    pid: u64,
    deadline: Instant,
) -> Result<Option<String>, CommandError> {
    Ok(match observe_runtime_process(runner, pid, deadline)? {
        ProcessObservation::Live(process) | ProcessObservation::Stopped(process) => {
            Some(process.start_identity().to_owned())
        }
        _ => None,
    })
}

/// The incarnations of several processes from one batched `ps` call, for an
/// external host's verification (#570): its server and its scoped pane
/// shells. A pid that is gone, a zombie or not parsed is absent, and a failed
/// call leaves every pid absent; an absent pid is unknown, never evidence.
/// Only a child that could not be cleaned up fails. This is the ps path only:
/// a faster backend for it (#724) routes here later.
pub fn observe_starts<R: CommandRunner>(
    runner: &R,
    pids: &[u64],
    deadline: Instant,
) -> Result<HashMap<u64, ProcessIncarnation>, CommandError> {
    let pids: BTreeSet<u64> = pids
        .iter()
        .copied()
        .filter(|pid| i32::try_from(*pid).is_ok_and(|pid| pid > 0))
        .collect();
    if pids.is_empty() {
        return Ok(HashMap::new());
    }
    let list = pids
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let args: Vec<OsString> = ["-p", &list, "-o", "pid=", "-o", "lstart=", "-o", "stat="]
        .into_iter()
        .map(Into::into)
        .collect();
    let output = match query_ps(runner, &args, deadline, 128 * pids.len()) {
        Ok(output) => output,
        Err(error) if error.cleanup_failed() => return Err(error),
        Err(_) => return Ok(HashMap::new()),
    };
    let Ok(text) = std::str::from_utf8(&output.stdout) else {
        return Ok(HashMap::new());
    };
    Ok(text
        .lines()
        .filter_map(|line| {
            let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let pid = pid.parse::<u64>().ok().filter(|pid| pids.contains(pid))?;
            match parse_process_observation(pid, rest.as_bytes()) {
                ProcessObservation::Live(process) | ProcessObservation::Stopped(process) => {
                    Some((pid, process))
                }
                _ => None,
            }
        })
        .collect())
}

fn parse_process_observation(pid: u64, bytes: &[u8]) -> ProcessObservation {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return ProcessObservation::Unknown;
    };
    let fields: Vec<_> = text.split_whitespace().collect();
    if bytes.len() > 512
        || fields.len() != 6
        || !["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].contains(&fields[0])
        || ![
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ]
        .contains(&fields[1])
        || !fields[2]
            .parse::<u8>()
            .is_ok_and(|day| (1..=31).contains(&day))
        || fields[3].len() != 8
        || fields[3].split(':').count() != 3
        || !fields[3]
            .split(':')
            .all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_digit()))
        || fields[4].len() != 4
        || !fields[4].bytes().all(|b| b.is_ascii_digit())
    {
        return ProcessObservation::Unknown;
    }
    if fields[5].starts_with('Z') {
        return ProcessIncarnation::new(pid, &format!("ps-v1:{}", fields[..5].join(" ")))
            .map(ProcessObservation::UnreapedZombie)
            .unwrap_or(ProcessObservation::Unknown);
    }
    if matches!(fields[5].as_bytes().first(), Some(b'T' | b't')) {
        return ProcessIncarnation::new(pid, &format!("ps-v1:{}", fields[..5].join(" ")))
            .map(ProcessObservation::Stopped)
            .unwrap_or(ProcessObservation::Unknown);
    }
    if !matches!(
        fields[5].as_bytes().first(),
        Some(b'R' | b'S' | b'I' | b'D' | b'U')
    ) {
        return ProcessObservation::Unknown;
    }
    ProcessIncarnation::new(pid, &format!("ps-v1:{}", fields[..5].join(" ")))
        .map(ProcessObservation::Live)
        .unwrap_or(ProcessObservation::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{CommandFailure, CommandOutput, CommandRequest, UnixCommandRunner};
    use std::time::Duration;

    #[test]
    fn strict_observations_distinguish_reused_pid_zombie_and_uncertainty() {
        let first = parse_process_observation(42, b"Sun Sep 27 10:00:00 2026 S+\n");
        let ProcessObservation::Live(key) = &first else {
            panic!("valid process");
        };
        assert_eq!(first.matches(key), RuntimeLiveness::Alive);
        assert_eq!(
            parse_process_observation(42, b"Sun Sep 27 10:00:00 2026 T+\n"),
            ProcessObservation::Stopped(key.clone())
        );
        assert_eq!(
            parse_process_observation(42, b"Sun Sep 27 10:00:01 2026 S+\n").matches(key),
            RuntimeLiveness::Gone
        );
        assert_eq!(
            parse_process_observation(42, b"Sun Sep 27 10:00:00 2026 Z\n").matches(key),
            RuntimeLiveness::Gone
        );
        for bytes in [
            b"".as_slice(),
            b"noise",
            b"Sun Sep 27 10:00:00 2026 S extra",
            b"Sun Sep 27 10:00:00 2026 ?",
            b"Sun Sep 27 10:00:00 2026 T",
            b"Sun Sep 27 10:00:00 2026 T+",
            b"Sun Sep 27 10:00:00 2026 t",
        ] {
            assert_eq!(
                parse_process_observation(42, bytes).matches(key),
                RuntimeLiveness::Unknown
            );
        }
    }

    #[test]
    fn a_recorded_start_is_the_runtime_observations_token_or_unknown() {
        let pid = u64::from(std::process::id());
        let deadline = Instant::now() + Duration::from_secs(5);
        let ProcessObservation::Live(single) =
            observe_runtime_process(&UnixCommandRunner, pid, deadline).unwrap()
        else {
            panic!("the test process is live");
        };
        assert_eq!(
            observe_start(&UnixCommandRunner, pid, deadline)
                .unwrap()
                .as_deref(),
            Some(single.start_identity())
        );
        struct Runner(&'static [u8]);
        impl CommandRunner for Runner {
            fn execute(&self, _: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
                Ok(CommandOutput {
                    stdout: self.0.to_vec(),
                    stderr: vec![],
                })
            }
        }
        for (stdout, expected) in [
            (
                b"Sun Sep 27 10:00:00 2026 T\n".as_slice(),
                Some("ps-v1:Sun Sep 27 10:00:00 2026"),
            ),
            (b"Sun Sep 27 10:00:00 2026 Z\n", None),
            (b"garbage\n", None),
        ] {
            assert_eq!(
                observe_start(&Runner(stdout), 42, deadline)
                    .unwrap()
                    .as_deref(),
                expected,
                "{stdout:?}"
            );
        }
    }

    #[test]
    fn one_ps_call_observes_many_starts_and_drops_what_it_cannot_prove() {
        struct Runner(std::cell::Cell<usize>, Result<&'static [u8], ()>);
        impl CommandRunner for Runner {
            fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
                self.0.set(self.0.get() + 1);
                assert_eq!(
                    request.args[3..]
                        .iter()
                        .map(|v| v.to_str().unwrap())
                        .collect::<Vec<_>>(),
                    [
                        "-p",
                        "7,42,43,44",
                        "-o",
                        "pid=",
                        "-o",
                        "lstart=",
                        "-o",
                        "stat="
                    ]
                );
                assert_eq!(request.max_output_bytes, 4 * 128);
                self.1
                    .map(|stdout| CommandOutput {
                        stdout: stdout.to_vec(),
                        stderr: vec![],
                    })
                    .map_err(|()| {
                        CommandError::new(CommandFailure::Exit {
                            code: Some(1),
                            signal: None,
                        })
                    })
            }
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        let runner = Runner(
            std::cell::Cell::new(0),
            Ok(b"   42 Sun Sep 27 10:00:00 2026 S+
   43 Sun Sep 27 10:00:01 2026 T
   44 Sun Sep 27 10:00:02 2026 Z
   99 Sun Sep 27 10:00:03 2026 S
    7 garbage
"),
        );
        // Duplicates and invalid pids are dropped before the one call.
        let observed =
            observe_starts(&runner, &[44, 42, 0, 43, 42, 7, u64::MAX], deadline).unwrap();
        assert_eq!(runner.0.get(), 1);
        assert_eq!(
            observed,
            HashMap::from([
                (
                    42,
                    ProcessIncarnation::new(42, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap()
                ),
                (
                    43,
                    ProcessIncarnation::new(43, "ps-v1:Sun Sep 27 10:00:01 2026").unwrap()
                ),
            ]),
            "a zombie, an unasked pid and an unparsed line are absent"
        );
        // A failed call proves nothing about any pid; no pids, no call.
        let failed = Runner(std::cell::Cell::new(0), Err(()));
        assert_eq!(
            observe_starts(&failed, &[7, 42, 43, 44], deadline).unwrap(),
            HashMap::new()
        );
        let idle = Runner(std::cell::Cell::new(0), Err(()));
        assert_eq!(
            observe_starts(&idle, &[0], deadline).unwrap(),
            HashMap::new()
        );
        assert_eq!(idle.0.get(), 0);
        // A real batched observation equals the single one.
        let pid = u64::from(std::process::id());
        let ProcessObservation::Live(single) =
            observe_runtime_process(&UnixCommandRunner, pid, deadline).unwrap()
        else {
            panic!("the test process is live");
        };
        assert_eq!(
            observe_starts(&UnixCommandRunner, &[pid], deadline)
                .unwrap()
                .get(&pid),
            Some(&single)
        );
    }

    #[test]
    fn probe_passes_fixed_locale_and_caller_budget_to_existing_owner() {
        struct Runner(Instant);
        impl CommandRunner for Runner {
            fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
                assert_eq!(request.program, "/usr/bin/env");
                assert_eq!(request.deadline, self.0);
                assert_eq!(request.max_output_bytes, 512);
                assert_eq!(
                    request
                        .args
                        .iter()
                        .map(|v| v.to_str().unwrap())
                        .collect::<Vec<_>>(),
                    [
                        "LC_ALL=C", "TZ=UTC", "/bin/ps", "-p", "42", "-o", "lstart=", "-o", "stat="
                    ]
                );
                Ok(CommandOutput {
                    stdout: b"Sun Sep 27 10:00:00 2026 S\n".to_vec(),
                    stderr: vec![],
                })
            }
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        assert!(matches!(
            observe_runtime_process(&Runner(deadline), 42, deadline).unwrap(),
            ProcessObservation::Live(_)
        ));
    }

    #[test]
    fn missing_ps_uses_only_the_second_fixed_candidate_with_the_same_budget() {
        struct Runner(std::cell::Cell<usize>, Instant);
        impl CommandRunner for Runner {
            fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
                let call = self.0.get();
                self.0.set(call + 1);
                assert_eq!(request.program, "/usr/bin/env");
                assert_eq!(request.deadline, self.1);
                assert_eq!(request.max_output_bytes, 512);
                assert_eq!(request.args[2], ["/bin/ps", "/usr/bin/ps"][call]);
                if call == 0 {
                    Err(CommandError::new(CommandFailure::Exit {
                        code: Some(127),
                        signal: None,
                    }))
                } else {
                    Ok(CommandOutput {
                        stdout: b"Sun Sep 27 10:00:00 2026 S\n".to_vec(),
                        stderr: vec![],
                    })
                }
            }
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        let runner = Runner(std::cell::Cell::new(0), deadline);
        assert!(matches!(
            observe_runtime_process(&runner, 42, deadline).unwrap(),
            ProcessObservation::Live(_)
        ));
        assert_eq!(runner.0.get(), 2);
    }

    #[test]
    fn current_process_has_repeatable_start_identity() {
        let read = || {
            observe_runtime_process(
                &UnixCommandRunner,
                u64::from(std::process::id()),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap()
        };
        let first = read();
        assert!(matches!(first, ProcessObservation::Live(_)));
        assert_eq!(first, read());
    }
}
