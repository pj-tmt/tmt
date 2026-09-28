use crate::process::{
    CommandError, CommandFailure, CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner,
};
use crate::test_support::TestDirectory;
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const SHELL: &str = "/bin/sh";
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLEANUP_WAIT: Duration = Duration::from_secs(2);

#[test]
fn runtime_observer_detects_child_exit_without_a_provider_end_hook() {
    use crate::process::runtime::{ProcessObservation, observe_runtime_process};
    let directory = TestDirectory::new();
    let pid_file = directory.path.join("runtime-pid");
    let mut cleanup = PidCleanup {
        path: pid_file.clone(),
        armed: true,
    };
    let args = shell_args("echo $$ > \"$1\"; exec sleep 10", &[path_arg(&pid_file)]);
    let running = UnixCommandRunner
        .start(CommandRequest {
            program: OsStr::new(SHELL),
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(3),
            max_output_bytes: 64,
        })
        .unwrap();
    let pid = wait_for_pid(&pid_file);
    let observe = || {
        observe_runtime_process(
            &UnixCommandRunner,
            u64::try_from(pid.as_raw()).unwrap(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap()
    };
    assert!(matches!(observe(), ProcessObservation::Live(_)));
    // Dropping the existing owner kills and reaps the process without any hook.
    drop(running);
    assert_pid_gone(pid, &mut cleanup);
    assert_eq!(observe(), ProcessObservation::Gone);
}

#[test]
fn started_command_retains_the_original_deadline() {
    let directory = TestDirectory::new();
    let pid_file = directory.path.join("pid");
    let mut cleanup = PidCleanup {
        path: pid_file.clone(),
        armed: true,
    };
    let deadline = Instant::now() + Duration::from_millis(500);
    let running = {
        let args = shell_args(
            "echo $$ > \"$1\"; read -r input; printf '%s' \"$input\"",
            &[path_arg(&pid_file)],
        );
        UnixCommandRunner
            .start(CommandRequest {
                program: OsStr::new(SHELL),
                args: &args,
                input: b"private input\n",
                deadline,
                max_output_bytes: 64,
            })
            .unwrap()
    };
    let pid = wait_for_pid(&pid_file);
    // Deliberately consume the original budget before wait. Starting a second
    // timeout at wait would keep this already-expired child alive.
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        thread::sleep(remaining);
    }
    let error = running.wait().unwrap_err();
    assert_eq!(error.kind, CommandFailure::Timeout);
    assert!(!error.cleanup_failed());
    assert_pid_gone(pid, &mut cleanup);
}

#[test]
fn started_command_owns_arguments_and_input_until_wait() {
    let running = {
        let args = shell_args(
            "read -r input; printf '%s:%s' \"$1\" \"$input\"",
            &[OsString::from("literal !")],
        );
        let input = b"private input\n".to_vec();
        UnixCommandRunner
            .start(CommandRequest {
                program: OsStr::new(SHELL),
                args: &args,
                input: &input,
                deadline: Instant::now() + Duration::from_secs(2),
                max_output_bytes: 64,
            })
            .unwrap()
    };
    let output = running.wait().unwrap();
    assert_eq!(output.stdout, b"literal !:private input");
    assert!(output.stderr.is_empty());
}

#[test]
fn dropping_or_unwinding_before_wait_reaps_the_started_child() {
    for unwind in [false, true] {
        let directory = TestDirectory::new();
        let pid_file = directory.path.join("pid");
        let mut cleanup = PidCleanup {
            path: pid_file.clone(),
            armed: true,
        };
        let args = shell_args("echo $$ > \"$1\"; exec sleep 5", &[path_arg(&pid_file)]);
        let running = UnixCommandRunner
            .start(CommandRequest {
                program: OsStr::new(SHELL),
                args: &args,
                input: &[],
                deadline: Instant::now() + Duration::from_secs(3),
                max_output_bytes: 64,
            })
            .unwrap();
        let pid = wait_for_pid(&pid_file);
        if unwind {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _owned = running;
                    panic!("controlled caller unwind");
                }))
                .is_err()
            );
        } else {
            drop(running);
        }
        assert_pid_gone(pid, &mut cleanup);
    }
}

#[test]
fn nonzero_exit_preserves_bounded_protocol_output_without_formatting_it() {
    let args = shell_args(
        "printf 'private stdout'; printf 'private stderr' >&2; exit 1",
        &[],
    );
    let error =
        execute_shell(&UnixCommandRunner, &args, &[], Duration::from_secs(2), 64).unwrap_err();
    assert_eq!(
        error.kind,
        CommandFailure::Exit {
            code: Some(1),
            signal: None
        }
    );
    assert!(!error.cleanup_failed());
    assert!(!error.to_string().contains("private"));
    let output = error.output.unwrap();
    assert_eq!(output.stdout, b"private stdout");
    assert_eq!(output.stderr, b"private stderr");

    let error =
        execute_shell(&UnixCommandRunner, &args, &[], Duration::from_secs(2), 3).unwrap_err();
    assert_eq!(error.kind, CommandFailure::OutputLimit);
    assert!(
        error.output.is_none(),
        "incomplete oversized output is not a protocol report"
    );
}

fn shell_args(script: &'static str, extra: &[OsString]) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-c"),
        OsString::from(script),
        OsString::from("fixture"),
    ];
    args.extend_from_slice(extra);
    args
}

fn execute_shell(
    runner: &UnixCommandRunner,
    args: &[OsString],
    input: &[u8],
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<CommandOutput, CommandError> {
    runner.execute(CommandRequest {
        program: OsStr::new(SHELL),
        args,
        input,
        deadline: Instant::now() + timeout,
        max_output_bytes,
    })
}

fn execute_expired(
    runner: &UnixCommandRunner,
    args: &[OsString],
) -> Result<CommandOutput, CommandError> {
    runner.execute(CommandRequest {
        program: OsStr::new(SHELL),
        args,
        input: &[],
        deadline: Instant::now(),
        max_output_bytes: 1024,
    })
}

fn path_arg(path: &Path) -> OsString {
    path.as_os_str().to_owned()
}

fn read_pid(path: &Path) -> Pid {
    let value = fs::read_to_string(path).expect("fixture wrote a child PID");
    let raw = value
        .trim()
        .parse::<i32>()
        .expect("fixture PID is an integer");
    assert!(raw > 0, "fixture PID must be positive");
    Pid::from_raw(raw)
}

fn wait_for_pid(path: &Path) -> Pid {
    let deadline = Instant::now() + CLEANUP_WAIT;
    loop {
        // Shell redirection creates the file before printf publishes its PID.
        // Readiness is the payload, not the existence of an empty file.
        if fs::read_to_string(path).is_ok_and(|value| !value.trim().is_empty()) {
            return read_pid(path);
        }
        assert!(Instant::now() < deadline, "fixture did not write its PID");
        thread::sleep(POLL_INTERVAL);
    }
}

fn assert_pid_gone(pid: Pid, cleanup: &mut PidCleanup) {
    let deadline = Instant::now() + CLEANUP_WAIT;
    loop {
        match kill(pid, Option::<Signal>::None) {
            Err(Errno::ESRCH) => {
                cleanup.armed = false;
                return;
            }
            Ok(()) => {}
            Err(error) => panic!("could not inspect fixture PID {pid}: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "fixture PID {pid} was not reaped"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

struct PidCleanup {
    path: PathBuf,
    armed: bool,
}

impl Drop for PidCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(value) = fs::read_to_string(&self.path) else {
            return;
        };
        let Ok(raw) = value.trim().parse::<i32>() else {
            return;
        };
        if raw <= 0 {
            return;
        }
        // These fixtures self-terminate after five seconds even if the runner
        // is broken. Never send a signal using an already-reaped numeric PID:
        // it could now belong to an unrelated process. Wait only, boundedly.
        let deadline = Instant::now() + Duration::from_secs(6);
        while kill(Pid::from_raw(raw), Option::<Signal>::None) != Err(Errno::ESRCH)
            && Instant::now() < deadline
        {
            thread::sleep(POLL_INTERVAL);
        }
    }
}

#[test]
fn executes_literal_argv_and_stdin_with_separate_output_streams() {
    let runner = UnixCommandRunner;
    let args = shell_args(
        r#"printf 'arg1=[%s]\narg2=[%s]\narg3=[%s]\n' "$1" "$2" "$3"; printf 'stderr=[%s]\n' "$4" >&2; read -r line; printf 'stdin=[%s]\n' "$line""#,
        &[
            OsString::from("$(printf hacked); alpha beta"),
            OsString::from("a'b\"c;*"),
            OsString::from("! literal --json"),
            OsString::from("--message=--json"),
        ],
    );

    let output = execute_shell(
        &runner,
        &args,
        b"input value\n",
        Duration::from_secs(2),
        1024,
    )
    .expect("literal command succeeds");

    assert_eq!(
        output.stdout,
        b"arg1=[$(printf hacked); alpha beta]\narg2=[a'b\"c;*]\narg3=[! literal --json]\nstdin=[input value]\n"
    );
    assert_eq!(output.stderr, b"stderr=[--message=--json]\n");
}

#[test]
fn classifies_nonzero_signal_and_spawn_failures() {
    let runner = UnixCommandRunner;
    let cases = [
        ("nonzero", "exit 7", Some(7), None),
        ("signal", "kill -TERM $$", None, Some(15)),
    ];

    for (name, script, expected_code, expected_signal) in cases {
        let args = shell_args(script, &[]);
        let error =
            execute_shell(&runner, &args, &[], Duration::from_secs(2), 1024).expect_err(name);
        assert!(
            !error.cleanup_failed(),
            "{name} failure should not require failed cleanup"
        );
        assert_eq!(
            error.kind,
            CommandFailure::Exit {
                code: expected_code,
                signal: expected_signal,
            },
            "{name} exit classification"
        );
    }

    let missing = OsStr::new("/definitely/missing/tmt-fixture-command");
    let error = runner
        .execute(CommandRequest {
            program: missing,
            args: &[],
            input: &[],
            deadline: Instant::now() + Duration::from_secs(2),
            max_output_bytes: 1024,
        })
        .expect_err("missing program must fail to spawn");
    assert_eq!(error.kind, CommandFailure::Spawn);
}

#[test]
fn enforces_each_output_stream_limit_and_accepts_exact_boundaries() {
    let runner = UnixCommandRunner;
    let exact_args = shell_args(
        r#"printf '%s' "$1"; printf '%s' "$2" >&2"#,
        &[OsString::from("12345678"), OsString::from("abcdefgh")],
    );
    let exact = execute_shell(&runner, &exact_args, &[], Duration::from_secs(2), 8)
        .expect("exact per-stream output boundary succeeds");
    assert_eq!(exact.stdout, b"12345678");
    assert_eq!(exact.stderr, b"abcdefgh");

    let cases = [
        (
            "stdout",
            r#"printf '%s' "$1""#,
            vec![OsString::from("123456789")],
        ),
        (
            "stderr",
            r#"printf '%s' "$1" >&2"#,
            vec![OsString::from("123456789")],
        ),
    ];
    for (name, script, extra) in cases {
        let args = shell_args(script, &extra);
        let error = execute_shell(&runner, &args, &[], Duration::from_secs(2), 8)
            .expect_err("one stream over the cap must fail");
        assert_eq!(error.kind, CommandFailure::OutputLimit, "{name} cap");
        assert!(!error.cleanup_failed(), "{name} cleanup: {error:?}");
    }
}

#[test]
fn drains_simultaneous_pipe_pressure_without_deadlock() {
    let runner = UnixCommandRunner;
    let count = 70_000usize;
    let args = shell_args(
        r#"i=0; while [ "$i" -lt "$1" ]; do printf O; printf E >&2; i=$((i + 1)); done"#,
        &[OsString::from(count.to_string())],
    );

    let output = execute_shell(&runner, &args, &[], Duration::from_secs(5), count)
        .expect("both full output pipes should drain");
    assert_eq!(output.stdout.len(), count);
    assert_eq!(output.stderr.len(), count);
    assert!(output.stdout.iter().all(|byte| *byte == b'O'));
    assert!(output.stderr.iter().all(|byte| *byte == b'E'));
}

#[test]
fn drains_substantial_stdin_while_stdout_and_stderr_are_under_pressure() {
    let runner = UnixCommandRunner;
    let count = 70_000usize;
    let mut input = Vec::with_capacity(128 * 1024);
    for _ in 0..128 {
        input.extend(std::iter::repeat_n(b'I', 1024));
        input.push(b'\n');
    }
    let args = shell_args(
        r#"i=0; while [ "$i" -lt "$1" ]; do printf O; printf E >&2; i=$((i + 1)); done; n=0; while IFS= read -r line; do [ "$line" = "$2" ] || exit 8; n=$((n + 1)); done; [ "$n" -eq 128 ] || exit 9; printf D; printf F >&2"#,
        &[
            OsString::from(count.to_string()),
            OsString::from("I".repeat(1024)),
        ],
    );

    let output = execute_shell(&runner, &args, &input, Duration::from_secs(5), count + 1)
        .expect("both output pipes and substantial stdin should drain");
    assert_eq!(output.stdout.len(), count + 1);
    assert_eq!(output.stderr.len(), count + 1);
    assert_eq!(output.stdout.last(), Some(&b'D'));
    assert_eq!(output.stderr.last(), Some(&b'F'));
    assert!(output.stdout[..count].iter().all(|byte| *byte == b'O'));
    assert!(output.stderr[..count].iter().all(|byte| *byte == b'E'));
}

#[test]
fn rejects_expired_deadline_before_spawning() {
    let directory = TestDirectory::new();
    let marker = directory.path.join("spawned");
    let args = shell_args(r#"printf spawned > "$1""#, &[path_arg(&marker)]);

    let error = execute_expired(&UnixCommandRunner, &args).expect_err("expired deadline");
    assert_eq!(error.kind, CommandFailure::Timeout);
    assert!(!marker.exists(), "expired deadline must prevent spawn");
}

#[test]
fn kills_and_reaps_term_ignoring_child_with_bounded_cleanup() {
    let directory = TestDirectory::new();
    let pid_path = directory.path.join("child.pid");
    let mut cleanup = PidCleanup {
        path: pid_path.clone(),
        armed: true,
    };
    let args = shell_args(
        r#"printf '%s' "$$" > "$1"; trap '' TERM; exec sleep 5"#,
        &[path_arg(&pid_path)],
    );

    let error = execute_shell(&UnixCommandRunner, &args, &[], Duration::from_secs(1), 1024)
        .expect_err("TERM-ignoring child must hit the deadline");
    assert_eq!(error.kind, CommandFailure::Timeout);
    assert!(
        !error.cleanup_failed(),
        "child cleanup should be observable: {error:?}"
    );
    let pid = wait_for_pid(&pid_path);
    assert_pid_gone(pid, &mut cleanup);
}

#[test]
fn treats_closed_output_as_eof_but_still_waits_for_running_child() {
    let directory = TestDirectory::new();
    let pid_path = directory.path.join("closed-output.pid");
    let mut cleanup = PidCleanup {
        path: pid_path.clone(),
        armed: true,
    };
    let args = shell_args(
        r#"printf '%s' "$$" > "$1"; exec 1>/dev/null 2>/dev/null; exec sleep 5"#,
        &[path_arg(&pid_path)],
    );

    let error = execute_shell(&UnixCommandRunner, &args, &[], Duration::from_secs(1), 1024)
        .expect_err("closed output must not imply process completion");
    assert_eq!(error.kind, CommandFailure::Timeout);
    assert!(!error.cleanup_failed(), "running child cleanup: {error:?}");
    let pid = wait_for_pid(&pid_path);
    assert_pid_gone(pid, &mut cleanup);
}

#[test]
fn kills_descendant_that_retains_pipes_after_leader_exit() {
    let directory = TestDirectory::new();
    let pid_path = directory.path.join("descendant.pid");
    let mut cleanup = PidCleanup {
        path: pid_path.clone(),
        armed: true,
    };
    let descendant = r#"trap '' TERM; exec sleep 5"#;
    let args = shell_args(
        r#"/bin/sh -c "$1" descendant & child=$!; printf '%s' "$child" > "$2"; exit 0"#,
        &[OsString::from(descendant), path_arg(&pid_path)],
    );

    let error = execute_shell(&UnixCommandRunner, &args, &[], Duration::from_secs(1), 1024)
        .expect_err("retained descendant pipe must hit the deadline");
    assert_eq!(error.kind, CommandFailure::Timeout);
    assert!(
        !error.cleanup_failed(),
        "process-group cleanup should succeed"
    );
    let pid = wait_for_pid(&pid_path);
    assert_pid_gone(pid, &mut cleanup);
}
