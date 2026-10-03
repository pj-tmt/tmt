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
    driver_protocol::registry,
    host::Host,
    process::{
        CommandError, CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner,
        runtime::observe_start,
    },
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
  caller)
    case "$input" in
      *'"FAKE_PANE_ID":"f1"'*)
        printf '{"ok":{"pane":{"id":"fake-1","socket":"/tmp/fake.sock","shellPid":%s}}}' "$(cat "$d/shell")" ;;
      *) printf '{"ok":{"pane":null}}' ;;
    esac ;;
  input) printf '%s\n' "$input" >> "$d/typed"; printf '{"ok":{}}' ;;
  prompt)
    a=$(cat "$d/agent" 2>/dev/null)
    case "$a" in
      ok) printf '%s\n' "$input" >> "$d/prompted"; printf '{"ok":{}}' ;;
      blocked|not_ready) printf '{"error":{"code":"%s","message":"agent busy"}}' "$a" ;;
      *) printf '{"error":{"code":"no_agent","message":"no agent"}}' ;;
    esac ;;
  capture) printf '{"ok":{"text":"captured line"}}' ;;
  focus) printf '{"ok":{}}' ;;
  resolve-target)
    if [ -f "$d/slow" ]; then sleep 3; fi
    if [ -f "$d/fail" ]; then printf '{"error":{"code":"failed","message":"host is wedged"}}'; exit 0; fi
    case "$input" in
      *'"target":"f1"'*) printf '{"ok":{"paneId":"fake-1"}}' ;;
      *) printf '{"ok":{"paneId":null}}' ;;
    esac ;;
  snapshot)
    m=$(cat "$d/marker" 2>/dev/null); [ -n "$m" ] || m=null
    printf '{"ok":{"panes":[{"id":"fake-1","target":"f1","cwd":"/src","command":"sh","panePid":%s,"suggestedName":null,"marker":%s}]}}' "$(cat "$d/pid")" "$m" ;;
  *) printf '{"error":{"code":"unsupported","message":"no %s"}}' "$3" ;;
esac
"#;

/// The real process owner with a long deadline, for test setup only.
struct Patient;

impl CommandRunner for Patient {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        UnixCommandRunner.execute(CommandRequest {
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
            ..request
        })
    }
}

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
            r#"{"ok":{"protocols":[1],"kind":"host","name":"fake","version":"0.0.0-test","ops":["caller","server","resolve-target","snapshot","publish","clear"],"paneId":{"prefix":"fake-"},"target":"f{n}","callerEnv":["FAKE_PANE_ID"]}}"#,
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
        // The caller's pane shell: this test process, which is an ancestor
        // of every `tmt` it runs.
        beside("shell", &pid);
        Self { root, driver }
    }

    /// A driver that also types into its panes, prompts their agents, reads
    /// them and focuses them.
    fn delivering(name: &str) -> Self {
        let fixture = Self::new(name);
        fs::write(
            fixture.root.join("driver/capabilities"),
            r#"{"ok":{"protocols":[1],"kind":"host","name":"fake","version":"0.0.0-test","ops":["caller","server","resolve-target","snapshot","publish","clear","input","prompt","capture","focus"],"paneId":{"prefix":"fake-"},"target":"f{n}","callerEnv":["FAKE_PANE_ID"]}}"#,
        )
        .unwrap();
        fixture
    }

    /// The requests the driver received for `op`, one JSON document each.
    fn requests(&self, file: &str) -> Vec<Value> {
        fs::read_to_string(self.root.join("driver").join(file))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    /// Approval is setup, not the behavior under test: its `capabilities`
    /// call gets a patient deadline, so a loaded machine can't fail it.
    fn approve(&self) -> registry::DriverRecord {
        registry::approve(&self.state(), &self.driver, &Patient).unwrap()
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("driver/calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn tmt(&self, args: &[&str]) -> Output {
        self.tmt_in(None, args)
    }

    /// `tmt` run from inside the driver's pane `pane`, as its environment
    /// names it.
    fn tmt_in(&self, pane: Option<&str>, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tmt"));
        command
            .args(args)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("TMUX_TEAM_HOME", self.state())
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .env("TMUX_TMPDIR", self.root.join("tmux"))
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LANG", "en_US.UTF-8")
            .current_dir(&self.root)
            .stdin(Stdio::null());
        if let Some(pane) = pane {
            command.env("FAKE_PANE_ID", pane);
        }
        command.output().unwrap()
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

#[test]
fn a_caller_in_an_external_pane_binds_reads_and_routes_on_its_own_server() {
    register();
    let fixture = Fixture::new("caller");
    fixture.approve();
    let _ = fs::remove_file(fixture.root.join("driver/calls"));
    let in_pane = |args: &[&str]| {
        let output = fixture.tmt_in(Some("f1"), args);
        assert!(output.status.success(), "tmt {args:?}: {output:?}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };

    let named = in_pane(&["name", "worker", "--save", "--json"]);
    assert_eq!(named["name"], "worker");
    assert_eq!(named["pane"], "fake-1");
    assert_eq!(fixture.calls()[0], "caller");
    assert!(fixture.calls().contains(&"publish".to_owned()));
    assert_eq!(in_pane(&["whoami", "--json"])["name"], "worker");
    let row = worker(&in_pane(&["ls", "--json"])).clone();
    assert_eq!(row["presence"], "active", "{row:#}");
    assert_eq!(row["address"], "fake:f1");

    // Talk by name now routes on the caller's own server and reaches the
    // pane; the driver has no input until 3b-2b-2, so nothing is typed and
    // the request is kept.
    let before = fixture.calls().len();
    let talk = fixture.tmt_in(Some("f1"), &["talk", "worker", "hi", "--detach", "--json"]);
    assert!(!talk.status.success(), "{talk:?}");
    let failure: Value = serde_json::from_slice(&talk.stdout).unwrap();
    assert_eq!(
        failure["error"]["code"], "DELIVERY_PREPARATION_FAILED",
        "{failure:#}"
    );
    assert_eq!(failure["pane"], "fake-1");
    let calls = fixture.calls();
    assert!(
        calls[before..]
            .iter()
            .all(|call| ["caller", "server", "snapshot"].contains(&call.as_str())),
        "{calls:?}"
    );
}

#[test]
fn an_explicit_target_resolves_through_its_driver_without_asking_for_a_caller() {
    register();
    let fixture = Fixture::new("target");
    fixture.approve();
    let _ = fs::remove_file(fixture.root.join("driver/calls"));

    let added = fixture.json(&["add", "f1", "worker", "--json"]);
    assert_eq!(added["pane"], "fake-1", "{added:#}");
    let calls = fixture.calls();
    assert!(calls.contains(&"resolve-target".to_owned()), "{calls:?}");
    assert!(!calls.contains(&"caller".to_owned()), "{calls:?}");

    let missing = fixture.tmt(&["add", "f9", "other", "--json"]);
    assert!(!missing.status.success());
    let failure: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "PANE_NOT_FOUND", "{failure:#}");
}

#[test]
fn a_pane_whose_shell_is_not_the_callers_ancestor_is_not_the_caller() {
    register();
    let fixture = Fixture::new("stranger");
    fixture.approve();
    let _ = fs::remove_file(fixture.root.join("driver/calls"));
    // A pid that is no ancestor of `tmt` (none runs with it).
    fs::write(fixture.root.join("driver/shell"), "999999").unwrap();

    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--json"]);
    assert!(!named.status.success(), "{named:?}");
    let failure: Value = serde_json::from_slice(&named.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "PANE_NOT_FOUND", "{failure:#}");
    // Only asked, never published: the built-in choice (tmux) stands.
    assert_eq!(fixture.calls(), ["caller"]);
}

#[test]
fn a_failing_or_late_driver_is_a_failure_never_a_missing_pane() {
    register();
    let fixture = Fixture::new("wedged");
    fixture.approve();
    for mode in ["fail", "slow"] {
        fs::write(fixture.root.join("driver").join(mode), "").unwrap();
        let added = fixture.tmt(&["add", "f1", "worker", "--json"]);
        assert_eq!(added.status.code(), Some(1), "{mode}: {added:?}");
        let failure: Value = serde_json::from_slice(&added.stdout).unwrap();
        assert_eq!(
            failure["error"]["code"], "RECONCILIATION_FAILED",
            "{mode}: {failure:#}"
        );
        fs::remove_file(fixture.root.join("driver").join(mode)).unwrap();
    }
    // Answered again, the same target binds.
    assert_eq!(
        fixture.json(&["add", "f1", "worker", "--json"])["pane"],
        "fake-1"
    );
}

#[test]
fn a_message_to_an_external_pane_is_prompted_first_and_typed_only_without_an_agent() {
    register();
    let fixture = Fixture::delivering("delivery");
    fixture.approve();
    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(named.status.success(), "{named:?}");
    let talk = |agent: &str, text: &str| {
        fs::write(fixture.root.join("driver/agent"), agent).unwrap();
        let _ = fs::remove_file(fixture.root.join("driver/typed"));
        let _ = fs::remove_file(fixture.root.join("driver/prompted"));
        fixture.tmt_in(Some("f1"), &["talk", "worker", text, "--detach", "--json"])
    };

    // A recognized agent takes the prompt; nothing is typed.
    let sent = talk("ok", "go!");
    assert!(sent.status.success(), "{sent:?}");
    let prompted = fixture.requests("prompted");
    assert_eq!(prompted.len(), 1);
    // The message, then the reply frame core appends.
    assert!(
        prompted[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("go\u{ff01}\n")
    );
    assert_eq!(prompted[0]["paneId"], "fake-1");
    assert!(fixture.requests("typed").is_empty());

    // No agent: raw input, staged as paste then Enter alone.
    let typed = talk("none", "plain!");
    assert!(typed.status.success(), "{typed:?}");
    let input = fixture.requests("typed");
    assert_eq!(input.len(), 2, "{input:?}");
    assert_eq!(input[0]["enter"], false);
    assert!(
        input[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("plain\u{ff01}\n")
    );
    assert_eq!(input[1]["enter"], true);
    assert_eq!(input[1]["text"], "");

    // A blocked agent refuses; nothing reaches the pane and the request is kept.
    let blocked = talk("blocked", "wait");
    assert_eq!(blocked.status.code(), Some(1), "{blocked:?}");
    let failure: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(
        failure["error"]["code"], "DELIVERY_AWAITING_APPROVAL",
        "{failure:#}"
    );
    assert_eq!(
        failure["error"]["message"],
        "worker is waiting on its user; nothing was sent."
    );
    assert!(failure["requestId"].as_str().is_some());
    assert!(fixture.requests("typed").is_empty());
    assert!(fixture.requests("prompted").is_empty());

    // An agent that is not ready refuses too, and is never typed around.
    let not_ready = talk("not_ready", "later");
    assert_eq!(not_ready.status.code(), Some(1), "{not_ready:?}");
    assert!(fixture.requests("typed").is_empty());
}

#[test]
fn check_and_focus_reach_an_external_pane_through_its_driver() {
    register();
    let fixture = Fixture::delivering("inspect");
    fixture.approve();
    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(named.status.success(), "{named:?}");

    let checked = fixture.tmt_in(Some("f1"), &["check", "worker"]);
    assert!(checked.status.success(), "{checked:?}");
    assert!(String::from_utf8_lossy(&checked.stdout).contains("captured line"));

    let focused = fixture.tmt_in(Some("f1"), &["focus", "worker", "--json"]);
    assert!(focused.status.success(), "{focused:?}");
    assert!(fixture.calls().contains(&"focus".to_owned()));
}
