//! A stored binding on an external host, end to end (#570 slice 3b-2a-2): a
//! shell-script driver approved in a disposable home, a binding made through
//! the core port on that driver, then the real `tmt` reading it. Routing by
//! name is current-server-only, so from a tmux caller the name is not active
//! and `--inbox` queues; without its driver, or with a changed one, the
//! binding stays unknown and is never retired.
#![cfg(unix)]

use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output, Stdio},
};
use tmt_adapters::{
    host::{Host, external::registry},
    process::{UnixCommandRunner, runtime::observe_start},
    storage::Storage,
};
use tmt_core::{
    binding::bind_identity,
    endpoint::ServerEvidence,
    host::{HostGrammar, HostKind, HostName, HostServerIds, HostServerIncarnation},
};

/// Answers from files beside it, records each operation, keeps the marker
/// `publish` sent and returns it from `snapshot`, as a real host would.
const DRIVER: &str = r#"#!/bin/sh
d=$(dirname "$0")
echo "$3" >> "$d/calls"
input=$(cat)
case "$3" in
  capabilities) cat "$d/capabilities" ;;
  server) cat "$d/server" ;;
  publish)
    printf '%s' "$input" | sed -n 's/.*"marker":\({[^}]*}\).*/\1/p' > "$d/marker"
    printf '{"ok":{}}' ;;
  clear) rm -f "$d/marker"; printf '{"ok":{"cleared":true}}' ;;
  snapshot)
    m=$(cat "$d/marker" 2>/dev/null); [ -n "$m" ] || m=null
    printf '{"ok":{"panes":[{"id":"fake-1","target":"f1","cwd":"/src","command":"sh","panePid":%s,"suggestedName":null,"marker":%s}]}}' "$(cat "$d/pid")" "$m" ;;
  *) printf '{"error":{"code":"unsupported","message":"no %s"}}' "$3" ;;
esac
"#;

struct Fixture {
    root: PathBuf,
    driver: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("tmt-external-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in ["home", "xdg", "state", "tmux", "driver"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let driver = root.join("driver/tmt-driver-fake");
        fs::write(&driver, DRIVER).unwrap();
        fs::set_permissions(&driver, fs::Permissions::from_mode(0o755)).unwrap();
        let pid = std::process::id().to_string();
        let beside =
            |file: &str, text: &str| fs::write(root.join("driver").join(file), text).unwrap();
        beside(
            "capabilities",
            r#"{"ok":{"protocols":[1],"kind":"host","name":"fake","version":"0.0.0-test","ops":["server","snapshot","publish","clear"],"paneId":{"prefix":"fake-"},"target":"f{n}","callerEnv":[]}}"#,
        );
        // This test process stands for both the host server and the pane
        // shell: a live process whose start core can observe.
        beside(
            "server",
            &format!(
                r#"{{"ok":{{"server":{{"socket":"/tmp/fake.sock","pid":{pid},"startTime":"driver-says"}}}}}}"#
            ),
        );
        beside("pid", &pid);
        Self { root, driver }
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    fn approve(&self) -> registry::DriverRecord {
        registry::approve(&self.state(), &self.driver, &UnixCommandRunner).unwrap()
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("driver/calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn tmt(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tmt"))
            .args(args)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("TMUX_TEAM_HOME", self.state())
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .env("TMUX_TMPDIR", self.root.join("tmux"))
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LANG", "en_US.UTF-8")
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.tmt(args);
        assert!(output.status.success(), "tmt {args:?}: {output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Binds `worker` to the driver's pane through the core port, as `tmt add`
/// will once 3b-2b selects external hosts.
fn bind(fixture: &Fixture, record: registry::DriverRecord) {
    let fake = HostName::new("fake").unwrap();
    let pid = u64::from(std::process::id());
    let start = observe_start(&UnixCommandRunner, pid, deadline())
        .unwrap()
        .unwrap();
    let mut storage = Storage::open(fixture.state().join("tmux-team.db")).unwrap();
    let server = ServerEvidence {
        host: HostKind::External(fake),
        server_id: storage
            .server_id(&HostServerIncarnation {
                host: HostKind::External(fake),
                socket_path: "/tmp/fake.sock",
                server_pid: pid,
                server_start_time: &start,
            })
            .unwrap(),
        socket_path: "/tmp/fake.sock".into(),
        server_pid: pid,
        server_start_time: start,
    };
    let host = Host::for_server_with(&server, UnixCommandRunner).with_drivers(vec![record]);
    let mut session = host.session();
    bind_identity(&mut storage, &mut session, "fake-1", "worker", true).unwrap();
    storage.close().unwrap();
}

fn deadline() -> std::time::Instant {
    std::time::Instant::now() + std::time::Duration::from_secs(5)
}

fn register() {
    // Once per test process, as the CLI does at start.
    let grammar = HostGrammar::new("fake", "fake-", Some("f{n}")).unwrap();
    let _ = tmt_core::host::register_external_hosts(vec![grammar]);
}

fn worker(listed: &Value) -> &Value {
    listed["identities"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["name"] == "worker"))
        .unwrap_or_else(|| panic!("worker is listed: {listed:#}"))
}

/// Talk by name routes only on the caller's own server (ARCHITECTURE,
/// routing): from a tmux caller, a name bound on another host is not
/// active. Its own host is only read, for the recipient's availability.
fn assert_talk_fails_closed(fixture: &Fixture) {
    let before = fixture.calls().len();
    let talk = fixture.tmt(&["talk", "worker", "hello", "--detach", "--json"]);
    assert!(!talk.status.success(), "{talk:?}");
    let failure: Value = serde_json::from_slice(&talk.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "NAME_NOT_FOUND", "{failure:#}");
    let calls = fixture.calls();
    assert!(
        calls[before..].iter().all(|call| call == "snapshot"),
        "{calls:?}"
    );
}

#[test]
fn an_external_binding_reads_through_its_driver_and_routes_only_on_its_server() {
    register();
    let fixture = Fixture::new("bound");
    let record = fixture.approve();
    bind(&fixture, record);
    fs::remove_file(fixture.root.join("driver/calls")).unwrap();

    let listed = fixture.json(&["ls", "--json"]);
    let row = worker(&listed);
    assert_eq!(row["presence"], "active", "{row:#}");
    assert_eq!(row["pane"], "fake-1");
    assert_eq!(row["address"], "fake:f1");
    assert_eq!(row["driver"], "fake");
    // The marker publish wrote came back through the driver's snapshot.
    assert_eq!(fixture.calls(), ["snapshot"]);

    assert_talk_fails_closed(&fixture);
    assert_eq!(
        worker(&fixture.json(&["ls", "--json"]))["presence"],
        "active"
    );

    let calls = fixture.calls().len();
    let queued = fixture.json(&["talk", "worker", "hello", "--inbox", "--detach", "--json"]);
    assert_eq!(fixture.calls().len(), calls, "{queued:#}");
    let inbox = fixture.json(&["inbox", "--identity", "worker", "--json"]);
    assert!(inbox.to_string().contains("hello"), "{inbox:#}");
}

#[test]
fn without_its_approved_driver_a_binding_stays_unknown_and_is_never_retired() {
    register();
    let fixture = Fixture::new("missing");
    let record = fixture.approve();
    bind(&fixture, record);

    // Removed from the registry: never run.
    assert!(registry::remove(&fixture.state(), "fake").unwrap());
    fs::remove_file(fixture.root.join("driver/calls")).unwrap();
    let row = worker(&fixture.json(&["ls", "--json"])).clone();
    assert_eq!(row["presence"], "unknown", "{row:#}");
    assert!(fixture.calls().is_empty());
    assert_talk_fails_closed(&fixture);

    // Approved again, then changed on disk: its digest no longer matches, so
    // it is never run, and the binding still reads as unknown.
    let record = fixture.approve();
    assert_eq!(record.name, "fake");
    fs::write(&fixture.driver, format!("{DRIVER}\n# changed\n")).unwrap();
    fs::remove_file(fixture.root.join("driver/calls")).unwrap();
    let row = worker(&fixture.json(&["ls", "--json"])).clone();
    assert_eq!(row["presence"], "unknown", "{row:#}");
    assert!(fixture.calls().is_empty());

    // Restored and approved again: the same binding reads active again.
    fs::write(&fixture.driver, DRIVER).unwrap();
    fixture.approve();
    assert_eq!(
        worker(&fixture.json(&["ls", "--json"]))["presence"],
        "active"
    );
}
