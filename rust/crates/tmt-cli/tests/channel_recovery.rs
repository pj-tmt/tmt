//! `tmt channel inspect|recover` through the real binary in an isolated home: the
//! durable record effects, refusals that leave every byte in place, no signal to a
//! recorded process, no tmux server, and independent homes.
#![cfg(unix)]

// Shared integration support; this binary uses only its environment helpers.
#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use std::{
    fs,
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
    process::{Child, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tmt_adapters::process::{
    UnixCommandRunner,
    runtime::{ProcessObservation, observe_runtime_process},
};

const BINDING: &str = "11111111-1111-4111-8111-111111111111";
const GENERATION: &str = "22222222-2222-4222-8222-222222222222";
const IDENTITY: &str = "33333333-3333-4333-8333-333333333333";
const SERVER_ID: &str = "44444444-4444-4444-8444-444444444444";
const CODEX_BINDING: &str = "55555555-5555-4555-8555-555555555555";
const CODEX_GENERATION: &str = "66666666-6666-4666-8666-666666666666";
const LIVE_BINDING: &str = "77777777-7777-4777-8777-777777777777";

static NEXT: AtomicU64 = AtomicU64::new(0);

/// An isolated home. Its root is short because the Claude socket path must fit a
/// Unix socket address.
struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let root = PathBuf::from(format!(
            "/tmp/tmt-chr-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let channels = support::state_dir(&root).join("channels");
        fs::create_dir_all(&channels).unwrap();
        fs::set_permissions(&channels, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }

    fn channels(&self) -> PathBuf {
        support::state_dir(&self.0).join("channels")
    }

    fn tmt(&self, args: &[&str]) -> Output {
        support::command(&self.0, args).output().unwrap()
    }

    fn json(&self, args: &[&str]) -> (Value, i32) {
        let output = self.tmt(args);
        let document = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("not one JSON document: {output:?}"));
        (document, output.status.code().unwrap())
    }

    /// Nothing here may start or reach a tmux server.
    fn assert_no_tmux(&self) {
        assert_eq!(
            fs::read_dir(self.0.join("tmux")).unwrap().count(),
            0,
            "a tmux socket was created"
        );
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A real process a record names; dropping it kills and reaps it.
struct Running(Child, Value);

impl Running {
    fn start() -> Self {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        match observe_runtime_process(&UnixCommandRunner, u64::from(child.id()), deadline) {
            Ok(ProcessObservation::Live(process)) => {
                let recorded = json!({"pid": process.pid(), "start": process.start_identity()});
                Self(child, recorded)
            }
            other => panic!("the child must be observable: {other:?}"),
        }
    }

    fn still_runs(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A process that has exited and been reaped.
fn gone() -> Value {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    json!({"pid": pid, "start": "Thu Oct  1 10:00:00 2026"})
}

fn private_file(path: &Path, bytes: &[u8]) {
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    fs::write(path, bytes).unwrap();
}

/// A Claude enrollment in the contract's record format, whose launcher died
/// before it published the foreground.
fn claude_record(home: &Home, binding_id: &str, owner: Value) -> PathBuf {
    let record = json!({
        "version": 1,
        "bindingId": binding_id,
        "identityId": IDENTITY,
        "generation": GENERATION,
        "launchOwner": owner,
        "pane": {
            "host": "tmux", "serverId": SERVER_ID, "socketPath": "/tmp/tmux-recovery",
            "serverPid": 10, "serverStartTime": "start", "paneId": "%7", "panePid": 11,
        },
        "claude": null,
    });
    let path = home.channels().join(format!("{binding_id}.json"));
    private_file(&path, record.to_string().as_bytes());
    path
}

fn bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

#[test]
fn an_abandoned_claude_enrollment_is_inspected_recovered_once_and_then_absent() {
    let home = Home::new();
    let record = claude_record(&home, BINDING, gone());
    let socket = home.channels().join(format!("{BINDING}.sock"));
    drop(UnixListener::bind(&socket).unwrap());
    let recover = format!("tmt channel recover --binding {BINDING} --generation {GENERATION}");

    let (inspected, status) = home.json(&["channel", "inspect", "--binding", BINDING, "--json"]);
    assert_eq!(status, 0);
    let enrollment = &inspected["enrollments"][0];
    assert_eq!(inspected["bindingId"], BINDING);
    assert_eq!(enrollment["driver"], "claude");
    assert_eq!(enrollment["state"], "unconfirmed");
    assert_eq!(enrollment["generation"], GENERATION);
    assert_eq!(enrollment["foregroundRecorded"], false);
    assert_eq!(enrollment["pane"]["paneId"], "%7");
    assert_eq!(enrollment["processes"][0]["role"], "launchOwner");
    assert_eq!(enrollment["processes"][0]["state"], "gone");
    assert_eq!(enrollment["recover"], recover.as_str());
    assert_eq!(
        enrollment["removes"],
        json!([record.display().to_string(), socket.display().to_string()])
    );
    let human = home.tmt(&["channel", "inspect", "--binding", BINDING]);
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains("claude channel enrollment"), "{text}");
    assert!(text.contains("not recorded"), "{text}");
    assert!(
        text.contains(&format!("After checking the pane: {recover}")),
        "{text}"
    );
    assert!(record.exists(), "inspect is read-only");

    let (recovered, status) = home.json(&[
        "channel",
        "recover",
        "--binding",
        BINDING,
        "--generation",
        GENERATION,
        "--json",
    ]);
    assert_eq!(status, 0);
    assert_eq!(
        recovered,
        json!({
            "bindingId": BINDING, "generation": GENERATION, "driver": "claude",
            "recovered": true,
            "removed": [record.display().to_string(), socket.display().to_string()],
            "kept": [],
        })
    );
    assert!(!record.exists());
    assert!(fs::symlink_metadata(&socket).is_err());

    // Repeating it is a no-op that says so.
    let (again, status) = home.json(&[
        "channel",
        "recover",
        "--binding",
        BINDING,
        "--generation",
        GENERATION,
        "--json",
    ]);
    assert_eq!(status, 0);
    assert_eq!(
        again,
        json!({"bindingId": BINDING, "generation": GENERATION, "recovered": false})
    );
    let (empty, _) = home.json(&["channel", "inspect", "--binding", BINDING, "--json"]);
    assert_eq!(empty["enrollments"], json!([]));
    home.assert_no_tmux();
}

#[test]
fn a_running_recorded_process_refuses_recovery_and_is_never_signaled() {
    let home = Home::new();
    let mut owner = Running::start();
    let record = claude_record(&home, LIVE_BINDING, owner.1.clone());
    let before = bytes(&record);

    let (inspected, _) = home.json(&["channel", "inspect", "--binding", LIVE_BINDING, "--json"]);
    assert_eq!(inspected["enrollments"][0]["state"], "running");
    assert_eq!(inspected["enrollments"][0]["recover"], Value::Null);

    let (refused, status) = home.json(&[
        "channel",
        "recover",
        "--binding",
        LIVE_BINDING,
        "--generation",
        GENERATION,
        "--json",
    ]);
    assert_eq!(status, 1);
    assert_eq!(refused["error"]["code"], "CHANNEL_ENROLLMENT_LIVE");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Nothing was removed")
    );
    assert_eq!(bytes(&record), before);
    assert!(owner.still_runs(), "recovery must never signal a process");
    home.assert_no_tmux();
}

#[test]
fn another_generation_an_unreadable_record_and_malformed_ids_change_nothing() {
    let home = Home::new();
    let record = claude_record(&home, BINDING, gone());
    let before = bytes(&record);
    let other = "88888888-8888-4888-8888-888888888888";

    let (changed, status) = home.json(&[
        "channel",
        "recover",
        "--binding",
        BINDING,
        "--generation",
        other,
        "--json",
    ]);
    assert_eq!(status, 1);
    assert_eq!(changed["error"]["code"], "CHANNEL_ENROLLMENT_CHANGED");
    assert_eq!(bytes(&record), before);

    let unreadable = home.channels().join(format!("{CODEX_BINDING}.json"));
    private_file(&unreadable, b"{ not json");
    let (invalid, status) = home.json(&[
        "channel",
        "recover",
        "--binding",
        CODEX_BINDING,
        "--generation",
        GENERATION,
        "--json",
    ]);
    assert_eq!(status, 1);
    assert_eq!(invalid["error"]["code"], "CHANNEL_ENROLLMENT_INVALID");
    // An unreadable record stays manual-only.
    assert!(
        invalid["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rm -- '")
    );
    assert_eq!(bytes(&unreadable), b"{ not json");

    for args in [
        &["channel", "inspect", "--binding", "../x", "--json"][..],
        &[
            "channel",
            "recover",
            "--binding",
            BINDING,
            "--generation",
            "not-a-uuid",
            "--json",
        ],
    ] {
        let (usage, status) = home.json(args);
        assert_eq!(status, 1, "{args:?}");
        assert_eq!(usage["error"]["code"], "USAGE_ERROR", "{args:?}");
    }
    // The grammar requires exactly one of a target and --binding, and a generation.
    for args in [
        &["channel", "inspect", "--json"][..],
        &[
            "channel",
            "inspect",
            "worker",
            "--binding",
            BINDING,
            "--json",
        ],
        &["channel", "recover", "--binding", BINDING, "--json"],
    ] {
        let output = home.tmt(args);
        assert!(!output.status.success(), "{args:?}");
    }
    assert_eq!(bytes(&record), before);
    home.assert_no_tmux();
}

#[test]
fn a_codex_enrollment_with_a_gone_app_server_is_recovered_with_its_generation_files() {
    let home = Home::new();
    let codex = home.channels().join("codex");
    fs::DirBuilder::new().mode(0o700).create(&codex).unwrap();
    let record = json!({
        "foreground": {"state": "unknown"},
        "attribution": {
            "identityId": IDENTITY, "host": "tmux", "serverId": SERVER_ID,
            "socketPath": "/tmp/tmux-recovery", "server": {"pid": 10, "start": "start"},
            "paneId": "%2", "panePid": 8,
        },
        "version": 1,
        "bindingId": CODEX_BINDING,
        "generation": CODEX_GENERATION,
        "launchOwner": gone(),
        "ready": {"server": gone(), "port": 49000, "thread": GENERATION},
    });
    let path = codex.join(format!("{CODEX_BINDING}.json"));
    private_file(&path, record.to_string().as_bytes());
    let generation = codex.join(CODEX_GENERATION);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&generation)
        .unwrap();
    private_file(&generation.join("capability"), b"secret");
    private_file(&generation.join("server.log"), b"log");

    let (inspected, _) = home.json(&["channel", "inspect", "--binding", CODEX_BINDING, "--json"]);
    assert_eq!(inspected["enrollments"][0]["driver"], "codex");
    assert_eq!(inspected["enrollments"][0]["state"], "unconfirmed");

    let (recovered, status) = home.json(&[
        "channel",
        "recover",
        "--binding",
        CODEX_BINDING,
        "--generation",
        CODEX_GENERATION,
        "--json",
    ]);
    assert_eq!(status, 0, "{recovered}");
    assert_eq!(recovered["driver"], "codex");
    assert_eq!(
        recovered["removed"],
        json!([
            path.display().to_string(),
            generation.join("capability").display().to_string(),
            generation.join("server.log").display().to_string(),
            generation.display().to_string(),
        ])
    );
    assert!(!path.exists());
    assert!(fs::symlink_metadata(&generation).is_err());
    assert!(codex.join(format!("{CODEX_BINDING}.lock")).exists());
    home.assert_no_tmux();
}

#[test]
fn recovery_in_one_home_leaves_an_independent_home_untouched() {
    let first = Home::new();
    let second = Home::new();
    let mine = claude_record(&first, BINDING, gone());
    let theirs = claude_record(&second, BINDING, gone());
    let before = bytes(&theirs);

    let (recovered, status) = first.json(&[
        "channel",
        "recover",
        "--binding",
        BINDING,
        "--generation",
        GENERATION,
        "--json",
    ]);
    assert_eq!(status, 0);
    assert_eq!(recovered["recovered"], true);
    assert!(!mine.exists());
    assert_eq!(bytes(&theirs), before);
}
