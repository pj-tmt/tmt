//! Tick process locking and failure cleanup use a public Core fixture, never user state.
use nix::fcntl::{Flock, FlockArg};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-digest-tick-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let script=r#"#!/bin/sh
cd '__ROOT__' || exit 9
printf '%s\n' "$*" >> calls
case "$*" in
 'api') cat > input; printf '%s\n' '{"dataRoot":"__ROOT__/state"}' ;;
 'config show --json') printf '%s\n' '{"paths":{"global":"__ROOT__/config.json"}}' ;;
 *) printf '%s\n' '{"error":{"code":"FIXTURE_STOP","message":"no live Core in this fixture"}}'; exit 1 ;;
esac
"#.replace("__ROOT__", root.to_str().unwrap());
        fs::write(root.join("core"), script).unwrap();
        fs::set_permissions(root.join("core"), fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
    fn run(&self, words: &[&str]) -> Output {
        // These fast failure fixtures need an admitted minute, rather than an exit
        // caused by straddling the real minute boundary during process startup.
        if words == ["tick"] {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            let remaining = 60_000 - now % 60_000;
            if remaining < 5_000 {
                std::thread::sleep(std::time::Duration::from_millis(remaining as u64));
            }
        }
        Command::new(env!("CARGO_BIN_EXE_tmt-digest"))
            .args(words)
            .env("TMT_EXECUTABLE", self.0.join("core"))
            .env("TMT_HOME", self.0.join("state"))
            .env("NO_COLOR", "1")
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
    fn lock(&self) -> Flock<fs::File> {
        fs::create_dir_all(self.0.join("state/digest")).unwrap();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.0.join("state/digest/tick.lock"))
            .unwrap();
        Flock::lock(file, FlockArg::LockExclusiveNonblock).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn concurrent_tick_exits_without_reading_settings_and_failure_releases_lock_for_restart() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("digest.toml"), "flushCount = 0\n").unwrap();
    let guard = fixture.lock();
    let result = fixture.run(&["tick"]);
    assert!(result.status.success(), "{:?}", result.stderr);
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
    assert_eq!(
        fs::read_to_string(fixture.0.join("calls")).unwrap(),
        "api\n"
    );
    let envelope: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.0.join("input")).unwrap()).unwrap();
    assert_eq!(envelope["operation"], "storage.root");
    drop(guard);
    let result = fixture.run(&["tick"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("flushCount")
    );
    drop(fixture.lock()); // The failed child left no lock owner.
    fs::remove_file(fixture.0.join("digest.toml")).unwrap();
    let result = fixture.run(&["tick"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("FIXTURE_STOP")
    );
    drop(fixture.lock());
}

#[test]
fn tick_help_has_no_core_or_state_effects() {
    let fixture = Fixture::new();
    let result = fixture.run(&["tick", "--help"]);
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("during this minute")
    );
    assert!(!fixture.0.join("calls").exists());
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn failed_first_tick_creates_private_lock_state_and_releases_it() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("digest.toml"), "flushCount = 0\n").unwrap();
    let result = fixture.run(&["tick"]);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        fs::metadata(fixture.0.join("state/digest"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(fixture.0.join("state/digest/tick.lock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(fixture.lock());
}
