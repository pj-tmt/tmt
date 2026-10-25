//! The Claude allowance mod's intake keeps pushed windows only for a consenting setup, in a
//! private file, and never disturbs its caller (#5187).
#![cfg(unix)]
#[allow(dead_code)]
mod support;

use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, symlink},
    path::PathBuf,
    process::{Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

const PUSH: &str = r#"{"rateLimits":[
    {"kind":"five_hour","percentUsed":21,"resetsAt":"2999-01-01T05:00:00.000Z"},
    {"kind":"seven_day","percentUsed":64,"resetsAt":"2999-01-04T00:00:00.000Z"},
    {"kind":"spend_limit","percentUsed":3}]}"#;

struct World {
    root: PathBuf,
}
impl World {
    fn new(setup: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-allowance-intake-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let world = Self { root };
        fs::create_dir(world.root.join("bin")).unwrap();
        symlink(env!("CARGO_BIN_EXE_tmt"), world.root.join("bin/tmt")).unwrap();
        let output = world.run(setup, "");
        assert!(output.status.success(), "setup failed: {output:?}");
        world
    }
    fn account(&self, uuid: &str) {
        fs::write(
            self.root.join("home/.claude.json"),
            format!(r#"{{"oauthAccount":{{"accountUuid":"{uuid}"}}}}"#),
        )
        .unwrap();
    }
    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = support::command(&self.root, args)
            .env("PATH", format!("{}/bin:/usr/bin:/bin", self.root.display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The intake may exit before it reads an oversized document: a closed pipe is fine.
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }
    fn push(&self, stdin: &str) -> Output {
        let output = self.run(&["__allowance", "claude"], stdin);
        assert!(
            output.status.success(),
            "the intake always exits 0: {output:?}"
        );
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "{output:?}"
        );
        output
    }
    fn store(&self) -> PathBuf {
        support::state_dir(&self.root).join("claude/allowance.json")
    }
}
impl Drop for World {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn a_consenting_setup_keeps_the_named_windows_privately_per_account() {
    let world = World::new(&["setup", "claude", "--yes"]);
    world.account("acct-1");
    world.push(PUSH);
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(world.store()).unwrap()).unwrap();
    let limits = &stored["accounts"]["acct-1"];
    assert_eq!(limits["provider"], "claude");
    assert!(limits["observedAtMs"].as_u64().unwrap() > 0);
    let windows: Vec<(u64, f64)> = limits["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|window| {
            (
                window["windowMinutes"].as_u64().unwrap(),
                window["usedPercent"].as_f64().unwrap(),
            )
        })
        .collect();
    // spend_limit is not an allowance window tmt reads.
    assert_eq!(windows, [(300, 21.0), (10_080, 64.0)]);
    assert_eq!(fs::metadata(world.store()).unwrap().mode() & 0o777, 0o600);
    let directory = world.store().parent().unwrap().to_owned();
    assert_eq!(fs::metadata(directory).unwrap().mode() & 0o777, 0o700);

    // A second account is kept beside the first; the same account is replaced, not appended.
    world.account("acct-2");
    world.push(PUSH);
    world.push(PUSH);
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(world.store()).unwrap()).unwrap();
    assert_eq!(stored["accounts"].as_object().unwrap().len(), 2);
}

#[test]
fn a_usage_opt_out_keeps_nothing() {
    let world = World::new(&["setup", "claude", "--no-usage", "--yes"]);
    world.account("acct-1");
    world.push(PUSH);
    assert!(!world.store().exists());
}

#[test]
fn a_malformed_oversized_or_accountless_push_keeps_nothing_and_still_exits_zero() {
    let world = World::new(&["setup", "claude", "--yes"]);
    world.account("acct-1");
    for stdin in [
        "not json",
        r#"{"rateLimits":[{"kind":"five_hour","percentUsed":1}]}"#,
        r#"{"rateLimits":[{"kind":"five_hour","percentUsed":1,"resetsAt":"2999-01-01T05:00:00.000Z"},{"kind":"five_hour","percentUsed":2,"resetsAt":"2999-01-01T05:00:00.000Z"}]}"#,
        &"x".repeat(64 * 1024),
        "",
    ] {
        world.push(stdin);
    }
    assert!(!world.store().exists());

    fs::remove_file(world.root.join("home/.claude.json")).unwrap();
    world.push(PUSH);
    assert!(!world.store().exists());
}

#[test]
fn an_unknown_provider_is_a_usage_error_not_an_intake() {
    let world = World::new(&["setup", "claude", "--yes"]);
    let output = world.run(&["__allowance", "nobody"], "");
    assert!(!output.status.success());
    assert!(!world.store().exists());
}
