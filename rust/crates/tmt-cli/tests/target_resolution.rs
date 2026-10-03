//! Explicit resolution failures must not claim that a pane is absent.
#![cfg(unix)]

mod support;

use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use std::{
    fs,
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
        let fixture = Self { root, child: None };
        tmt_test_support::write_executable(
            &fixture.root.join("bin/tmux"),
            br##"#!/bin/sh
printf '%s\n' "$*" >> "$TMT_TEST_TMUX_LOG"
[ "$*" = 'display-message -p -t 10.3 #{pane_id}' ] || exit 99
case "$TMT_TEST_RESOLUTION_MODE" in
  timeout) echo $$ > "$TMT_TEST_TMUX_PID"; exec /bin/sleep 10 ;;
  missing) echo "can't find window: 10" >&2; exit 1 ;;
  denied) echo 'error connecting to /tmp/private.sock (Permission denied)' >&2; exit 1 ;;
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
        let mut command = support::command(&self.root, args);
        command
            .env("PATH", self.root.join("bin"))
            .env("TMT_TEST_RESOLUTION_MODE", mode)
            .env("TMT_TEST_TMUX_LOG", &log)
            .env("TMT_TEST_TMUX_PID", self.root.join("tmux.pid"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.child = Some(command.spawn().unwrap());
        let output = support::wait(&mut self.child, Duration::from_secs(5))
            .wait_with_output()
            .unwrap();
        assert_eq!(
            fs::read_to_string(log).unwrap(),
            "display-message -p -t 10.3 #{pane_id}\n"
        );
        output
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn timed_out_resolution_is_unavailable_for_check_and_add() {
    let mut fixture = Fixture::new();
    for args in [
        &["check", "10.3", "--json"][..],
        &["add", "10.3", "target-test", "--json"][..],
    ] {
        for (mode, status, code) in [
            ("timeout", 1, "RECONCILIATION_FAILED"),
            ("missing", 3, "PANE_NOT_FOUND"),
            ("denied", 1, "TMUX_PERMISSION_DENIED"),
        ] {
            let output = fixture.run(args, mode);
            assert_eq!(output.status.code(), Some(status), "{mode}: {output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(document["error"]["code"], code);
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
                assert!(
                    String::from_utf8(output.stdout)
                        .unwrap()
                        .contains("ETIMEDOUT")
                );
                let pid = fs::read_to_string(fixture.root.join("tmux.pid"))
                    .unwrap()
                    .trim()
                    .parse::<i32>()
                    .unwrap();
                assert_eq!(
                    kill(Pid::from_raw(pid), None),
                    Err(Errno::ESRCH),
                    "timed-out tmux was not reaped"
                );
            }
        }
    }
}
