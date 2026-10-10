//! Explicit resolution failures must not claim that a pane is absent.
#![cfg(unix)]

mod support;

use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
    process::{Child, Stdio},
    time::Duration,
};

struct Fixture {
    root: PathBuf,
    child: Option<Child>,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("tmt-target-resolution-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("bin")).unwrap();
        // Permission evidence belongs to this disposable socket, not stderr.
        let socket = root.join("private.sock");
        drop(UnixListener::bind(&socket).unwrap());
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o000)).unwrap();
        let fixture = Self { root, child: None };
        tmt_test_support::write_executable(
            &fixture.root.join("bin/tmux"),
            br##"#!/bin/sh
[ "$*" = '-u display-message -p -t 10.3 #{pane_id}' ] || exit 99
if [ "$TMT_TEST_RESOLUTION_MODE" = timeout ]; then
  echo $$ > "$TMT_TEST_TMUX_PID"
  exec /bin/sleep 10
fi
printf '%s\n' "$*" >> "$TMT_TEST_TMUX_LOG"
case "$TMT_TEST_RESOLUTION_MODE" in
  missing) echo "can't find window: 10" >&2; exit 1 ;;
  denied) printf 'error connecting to %s (Permission denied)\n' "$TMT_TEST_DENIED_SOCKET" >&2; exit 1 ;;
  unconfirmed) printf 'error connecting to %s (Permission denied)\n' "$TMT_TEST_MISSING_SOCKET" >&2; exit 1 ;;
esac
exit 98
"##,
            0o755,
        )
        .unwrap();
        fixture
    }

    fn run(&mut self, args: &[&str], mode: &str) -> std::process::Output {
        let log = self.root.join("tmux.log");
        fs::write(&log, "").unwrap();
        let pid = self.root.join("tmux.pid");
        match fs::remove_file(&pid) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("clear previous mock PID: {error}"),
        }
        let mut command = support::command(&self.root, args);
        command
            .env("PATH", self.root.join("bin"))
            .env("TMT_TEST_RESOLUTION_MODE", mode)
            .env("TMT_TEST_DENIED_SOCKET", self.root.join("private.sock"))
            .env("TMT_TEST_MISSING_SOCKET", self.root.join("missing.sock"))
            .env("TMT_TEST_TMUX_LOG", &log)
            .env("TMT_TEST_TMUX_PID", pid)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.child = Some(command.spawn().unwrap());
        support::wait(&mut self.child, Duration::from_secs(5))
            .wait_with_output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn explicit_resolution_preserves_completed_and_expired_lookups() {
    let mut fixture = Fixture::new();
    for args in [
        &["check", "10.3", "--json"][..],
        &["add", "10.3", "target-test", "--json"][..],
    ] {
        for (mode, status, code) in [
            ("timeout", 1, "RECONCILIATION_FAILED"),
            ("missing", 3, "PANE_NOT_FOUND"),
            ("denied", 1, "TMUX_PERMISSION_DENIED"),
            ("unconfirmed", 3, "PANE_NOT_FOUND"),
        ] {
            // Root bypasses socket mode bits; the Docker real-server fixture
            // explicitly runs its denial client as nobody instead.
            if mode == "denied" && nix::unistd::geteuid().is_root() {
                continue;
            }
            let output = fixture.run(args, mode);
            assert!(output.stderr.is_empty(), "{output:?}");
            let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let log = fs::read_to_string(fixture.root.join("tmux.log")).unwrap();
            let timed_out = document["error"]["code"] == "RECONCILIATION_FAILED";
            if timed_out {
                // The production deadline includes startup: an unscheduled mock
                // need not have logged yet. Only the documented timeout is valid.
                assert_eq!(output.status.code(), Some(1), "{mode}: {output:?}");
                assert_eq!(
                    document,
                    serde_json::json!({"error": {
                        "code": "RECONCILIATION_FAILED",
                        "message": "Could not execute tmux operation (ETIMEDOUT)."
                    }}),
                );
                assert!(
                    log.is_empty() || log == "-u display-message -p -t 10.3 #{pane_id}\n",
                    "unexpected timeout invocation: {log:?}",
                );
            } else {
                assert_eq!(output.status.code(), Some(status), "{mode}: {output:?}");
                assert_eq!(document["error"]["code"], code);
                assert_eq!(log, "-u display-message -p -t 10.3 #{pane_id}\n");
            }
            if mode == "timeout" {
                assert!(timed_out, "held mock did not time out: {output:?}");
                assert!(log.is_empty(), "held mock wrote its completion log");
            }
            let database = rusqlite::Connection::open_with_flags(
                support::state_dir(&fixture.root).join("tmux-team.db"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            for table in ["identities", "bindings"] {
                let rows: i64 = database
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(rows, 0, "{mode} resolution changed {table}");
            }
            if mode == "timeout" {
                match fs::read_to_string(fixture.root.join("tmux.pid")) {
                    // Opening the file can precede the shell's PID write.
                    Ok(pid) if pid.is_empty() => {}
                    Ok(pid) => {
                        let pid = pid.trim().parse::<i32>().unwrap();
                        assert_eq!(
                            kill(Pid::from_raw(pid), None),
                            Err(Errno::ESRCH),
                            "timed-out tmux was not reaped"
                        );
                    }
                    // The mock may expire before executing its first write.
                    // Ready-child reaping is proved at the process boundary.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("read owned mock PID: {error}"),
                }
            }
        }
    }
}
