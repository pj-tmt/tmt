//! A stored binding on an external host, end to end (#570 slice 3b-2a-2): a
//! shell-script driver approved in a disposable home, a binding made through
//! the core port on that driver, then the real `tmt` reading it. Routing by
//! name is current-server-only, so from a tmux caller the name is not active
//! and `--inbox` queues; without its driver, or with a changed one, the
//! binding stays unknown and is never retired.
#![cfg(unix)]
// This fixture uses only the shared private command environment.
#[allow(dead_code)]
mod support;

use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Output, Stdio},
};
use tmt_adapters::{
    driver_protocol::registry,
    host::Host,
    process::{
        CommandError, CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner,
        runtime::{ProcessObservation, observe_runtime_process, observe_start},
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
  name-agent)
    printf '%s\n' "$input" >> "$d/named"
    # A held call blocks, never honoring its deadline, until the test
    # removes `hold`; `held` names this driver process.
    if [ -f "$d/hold" ]; then
      echo $$ > "$d/held"
      while [ -f "$d/hold" ]; do sleep 0.05; done
    fi
    # One answer per call from the queue; an empty queue sets the name.
    a=$(head -n 1 "$d/naming" 2>/dev/null)
    if [ -f "$d/naming" ]; then tail -n +2 "$d/naming" > "$d/naming.next"; mv "$d/naming.next" "$d/naming"; fi
    # Like a host driver, a name replaces any agent name, and `name: null`
    # takes the agent name away only when it equals `expected`.
    n=$(printf '%s' "$input" | sed -n 's/.*"name":"\([^"]*\)".*/\1/p')
    e=$(printf '%s' "$input" | sed -n 's/.*"expected":"\([^"]*\)".*/\1/p')
    h=$(cat "$d/agentname" 2>/dev/null)
    case "$a" in
      '') if [ -n "$n" ]; then printf '%s' "$n" > "$d/agentname"; elif [ -n "$e" ] && [ "$h" = "$e" ]; then rm -f "$d/agentname"; fi
          printf '{"ok":{}}' ;;
      ok) printf '{"ok":{}}' ;;
      *) printf '{"error":{"code":"%s","message":"host says %s"}}' "$a" "$a" ;;
    esac ;;
  resolve-target)
    printf '%s\n' "$input" >> "$d/resolved"
    if [ -f "$d/slow" ]; then sleep 3; fi
    if [ -f "$d/fail" ]; then printf '{"error":{"code":"failed","message":"host is wedged"}}'; exit 0; fi
    case "$input" in
      *'"target":"f1"'*) printf '{"ok":{"paneId":"fake-1"}}' ;;
      *) printf '{"ok":{"paneId":null}}' ;;
    esac ;;
  snapshot)
    m=$(cat "$d/marker" 2>/dev/null); [ -n "$m" ] || m=null
    a=$(cat "$d/agentname" 2>/dev/null); if [ -n "$a" ]; then a=",\"agentName\":\"$a\""; fi
    printf '{"ok":{"panes":[{"id":"fake-1","target":"f1","cwd":"/src","command":"sh","panePid":%s,"suggestedName":null,"marker":%s%s}]}}' "$(cat "$d/pid")" "$m" "$a" ;;
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
        self.command_in(pane, args).output().unwrap()
    }

    fn command_in(&self, pane: Option<&str>, args: &[&str]) -> std::process::Command {
        let mut command = support::command(&self.root, args);
        command
            .env("TMT_HOME", self.state())
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .env("TMUX_TMPDIR", self.root.join("tmux"))
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LANG", "en_US.UTF-8")
            .current_dir(&self.root)
            .stdin(Stdio::null());
        if let Some(pane) = pane {
            command.env("FAKE_PANE_ID", pane);
        }
        command
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
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
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

    // A number may hold uppercase letters, and the target reaches its driver
    // as typed. Text with lowercase letters there is not selected as this
    // host's target: `talk` reads it as a name and asks no driver.
    let asked = |fixture: &Fixture| -> Vec<Value> {
        fixture
            .requests("resolved")
            .iter()
            .map(|request| request["target"].clone())
            .collect()
    };
    let before = asked(&fixture).len();
    let lettered = fixture.tmt(&["add", "fZ9", "other", "--json"]);
    assert!(!lettered.status.success());
    let failure: Value = serde_json::from_slice(&lettered.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "PANE_NOT_FOUND", "{failure:#}");
    assert_eq!(asked(&fixture)[before..], ["fZ9"]);
    for (text, code, resolved) in [("fZ9", "PANE_NOT_FOUND", 2), ("fz9", "NAME_NOT_FOUND", 2)] {
        let talk = fixture.tmt(&["talk", text, "hello", "--detach", "--json"]);
        assert!(!talk.status.success(), "{talk:?}");
        let failure: Value = serde_json::from_slice(&talk.stdout).unwrap();
        assert_eq!(failure["error"]["code"], code, "{text}: {failure:#}");
        let asked = asked(&fixture);
        assert_eq!(asked.len(), before + resolved, "{text}");
        assert_eq!(asked.last(), Some(&Value::from("fZ9")), "{text}");
    }
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

#[test]
fn focus_into_is_unsupported_on_external_hosts_without_a_focus_effect() {
    register();
    let fixture = Fixture::delivering("focus-into-unsupported");
    fixture.approve();
    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(named.status.success(), "{named:?}");
    let before = fixture.calls().len();
    let refused = fixture.tmt_in(Some("f1"), &["focus", "worker", "--into", "left", "--json"]);
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let document: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(document["error"]["code"], "HOST_UNSUPPORTED");
    assert!(
        fixture.calls()[before..]
            .iter()
            .all(|call| call != "focus" && call != "input" && call != "prompt")
    );
}

#[test]
fn focus_uuid_survives_rename_and_name_reuse_without_changing_check_routing() {
    register();
    let fixture = Fixture::delivering("focus-uuid-rename");
    fixture.approve();
    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(named.status.success(), "{named:?}");
    let original = fixture.json(&["identity", "show", "worker", "--json"]);
    let id = original["identity"]["id"].as_str().unwrap();
    fixture.json(&["mv", "worker", "reviewer", "--json"]);
    fixture.json(&["identity", "create", "worker", "--json"]);
    fixture.json(&["identity", "create", id, "--json"]);

    let before = fixture.calls().len();
    let focused = fixture.tmt_in(Some("f1"), &["focus", id, "--json"]);
    assert!(focused.status.success(), "{focused:?}");
    let result: Value = serde_json::from_slice(&focused.stdout).unwrap();
    assert_eq!(result["focused"]["pane"], "fake-1");
    let calls = fixture.calls();
    assert!(calls[before..].iter().any(|call| call == "focus"));
    assert!(fixture.requests("typed").is_empty());
    assert!(fixture.requests("prompted").is_empty());

    // Check retains name routing: this UUID-shaped display name is offline.
    let checked = fixture.tmt_in(Some("f1"), &["check", id, "--capture-only", "--json"]);
    assert_eq!(checked.status.code(), Some(3), "{checked:?}");
    let failure: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "NAME_NOT_FOUND");
    let named = fixture.tmt_in(Some("f1"), &["focus", "reviewer", "--json"]);
    assert!(named.status.success(), "{named:?}");
    let explicit = fixture.tmt_in(Some("f1"), &["focus", "f1", "--json"]);
    assert!(explicit.status.success(), "{explicit:?}");
    assert!(fixture.requests("typed").is_empty());
}

#[test]
fn focus_missing_and_retired_uuids_refuse_even_when_their_names_are_live() {
    register();
    for retired in [false, true] {
        let fixture = Fixture::delivering(if retired {
            "focus-retired-uuid"
        } else {
            "focus-missing-uuid"
        });
        fixture.approve();
        let id = if retired {
            let record = fixture.json(&["identity", "create", "worker", "--json"]);
            let id = record["identity"]["id"].as_str().unwrap().to_owned();
            fixture.json(&["rm", "worker", "--force", "--json"]);
            id
        } else {
            "6c57fcc9-96b1-4022-b071-870b81288814".into()
        };
        let named = fixture.tmt_in(Some("f1"), &["name", &id, "--save", "--json"]);
        assert!(named.status.success(), "{named:?}");
        let before = fixture.calls().len();
        let focused = fixture.tmt_in(Some("f1"), &["focus", &id, "--json"]);
        assert_eq!(focused.status.code(), Some(3), "{focused:?}");
        let failure: Value = serde_json::from_slice(&focused.stdout).unwrap();
        assert_eq!(failure["error"]["code"], "NAME_NOT_FOUND");
        let calls = fixture.calls();
        assert!(calls[before..].iter().all(|call| call != "focus"));
        assert!(fixture.requests("typed").is_empty());
        // Noncanonical spelling is still a name; it reaches the live collision.
        let named = fixture.tmt_in(Some("f1"), &["focus", &id.to_uppercase(), "--json"]);
        assert!(named.status.success(), "{named:?}");
    }
}

#[test]
fn focus_inactive_and_unverified_uuids_do_not_call_the_host_focus_effect() {
    register();
    let fixture = Fixture::delivering("focus-unverified-uuid");
    fixture.approve();
    let created = fixture.json(&["identity", "create", "worker", "--json"]);
    let id = created["identity"]["id"].as_str().unwrap();
    for bind_then_clear_marker in [false, true] {
        if bind_then_clear_marker {
            let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
            assert!(named.status.success(), "{named:?}");
            fs::remove_file(fixture.root.join("driver/marker")).unwrap();
        }
        let before = fixture.calls().len();
        let focused = fixture.tmt_in(Some("f1"), &["focus", id, "--json"]);
        assert_eq!(focused.status.code(), Some(3), "{focused:?}");
        let failure: Value = serde_json::from_slice(&focused.stdout).unwrap();
        assert_eq!(failure["error"]["code"], "NAME_NOT_FOUND");
        assert_eq!(
            failure["error"]["message"],
            format!("Identity '{id}' is not active.")
        );
        let calls = fixture.calls();
        assert!(calls[before..].iter().all(|call| call != "focus"));
        assert!(fixture.requests("typed").is_empty());
        assert!(fixture.requests("prompted").is_empty());
    }
}

/// A driver that also names agents, with `answers` queued for `name-agent`.
fn naming(name: &str, answers: &[&str]) -> Fixture {
    let fixture = Fixture::new(name);
    fs::write(
        fixture.root.join("driver/capabilities"),
        r#"{"ok":{"protocols":[1],"kind":"host","name":"fake","version":"0.0.0-test","ops":["caller","server","resolve-target","snapshot","publish","clear","name-agent"],"paneId":{"prefix":"fake-"},"target":"f{n}","callerEnv":["FAKE_PANE_ID"]}}"#,
    )
    .unwrap();
    let mut queue = answers.join("\n");
    queue.push('\n');
    fs::write(fixture.root.join("driver/naming"), queue).unwrap();
    fixture
}

/// The `name-agent` requests the driver received.
fn named(fixture: &Fixture) -> Vec<Value> {
    fixture.requests("named")
}

/// Waits, with a bound, until `ready` holds.
fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !ready() {
        assert!(std::time::Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// A fixture process a test must see end, held by its exact incarnation:
/// a recycled pid is never signalled, and only confirmed evidence counts as
/// an end. A driver leads its own process group, so its end also needs the
/// group to be empty: a held call's own children count too.
struct Owned {
    incarnation: tmt_core::endpoint::ProcessIncarnation,
    group: bool,
}

/// What one observation says about an owned process.
#[derive(Debug, PartialEq)]
enum Seen {
    Live,
    Stopped,
    /// Gone, a zombie, or its pid now names another process.
    Ended,
    /// No evidence either way: never counted as an end.
    Unknown,
}

impl Owned {
    /// The live process whose pid `file` names, once the file is written.
    fn adopt(file: &std::path::Path, group: bool) -> Self {
        wait_until("the process writes its pid", || {
            fs::read_to_string(file).is_ok_and(|pid| pid.ends_with('\n'))
        });
        let pid: u64 = fs::read_to_string(file).unwrap().trim().parse().unwrap();
        let observed = observe_runtime_process(&UnixCommandRunner, pid, deadline()).unwrap();
        let ProcessObservation::Live(incarnation) = observed else {
            panic!("process {pid} is not live: {observed:?}");
        };
        Self { incarnation, group }
    }

    fn pid(&self) -> nix::unistd::Pid {
        nix::unistd::Pid::from_raw(i32::try_from(self.incarnation.pid()).unwrap())
    }

    fn seen(&self) -> Seen {
        match observe_runtime_process(&UnixCommandRunner, self.incarnation.pid(), deadline()) {
            Ok(ProcessObservation::Live(found)) if found == self.incarnation => Seen::Live,
            Ok(ProcessObservation::Stopped(found)) if found == self.incarnation => Seen::Stopped,
            Ok(
                ProcessObservation::Live(_)
                | ProcessObservation::Stopped(_)
                | ProcessObservation::UnreapedZombie(_)
                | ProcessObservation::Gone,
            ) => Seen::Ended,
            Ok(ProcessObservation::Unknown) | Err(_) => Seen::Unknown,
        }
    }

    /// Whether any process group with the driver's id has a member. Read
    /// only: once the driver is gone, the id may name a stranger's group.
    fn group_occupied(&self) -> bool {
        !matches!(
            nix::sys::signal::killpg(self.pid(), None),
            Err(nix::errno::Errno::ESRCH)
        )
    }

    /// Confirmed ended: the process, and for a driver its whole group.
    fn ended(&self) -> bool {
        self.seen() == Seen::Ended && !(self.group && self.group_occupied())
    }

    /// SIGKILL, only while this exact incarnation is still seen live or
    /// stopped: it anchors the pid, and for a driver the group it leads.
    /// Once the anchor is gone nothing is signalled, never a numeric group;
    /// the driver's children then end by themselves, and `ended` waits for
    /// that read-only.
    fn kill(&self) {
        use nix::sys::signal::{Signal::SIGKILL, kill, killpg};
        if matches!(self.seen(), Seen::Live | Seen::Stopped) {
            let _ = if self.group {
                killpg(self.pid(), SIGKILL)
            } else {
                kill(self.pid(), SIGKILL)
            };
        }
    }
}

/// An agent for `tmt run` that writes its pid and runs until the test
/// creates `release`; a TERM or HUP it receives is acknowledged in `acked`.
const AGENT: &str = "echo $$ > agent.pid; trap 'touch acked; exit 3' TERM HUP; \
    while [ ! -f release ]; do sleep 0.05; done; exit 7";

/// A `tmt run` or `tmt resume` the test owns, with the agent and naming
/// driver it adopted. Its output goes to files, so collecting it never
/// waits on a pipe a descendant still holds. Dropping it, also after a
/// failed assertion, releases the held call and the agent, reaps the
/// command within a bound and kills what is still live.
struct Launched {
    child: Option<std::process::Child>,
    root: PathBuf,
    owned: Vec<Owned>,
}

impl Launched {
    fn start(fixture: &Fixture, mut command: std::process::Command) -> Self {
        let output = |name: &str| fs::File::create(fixture.root.join(name)).unwrap();
        let child = command
            .stdout(output("launched.out"))
            .stderr(output("launched.err"))
            .spawn()
            .unwrap();
        Self {
            child: Some(child),
            root: fixture.root.clone(),
            owned: Vec::new(),
        }
    }

    /// Takes ownership of the process whose pid `file` names.
    fn adopt(&mut self, file: &str, group: bool) -> &Owned {
        self.owned.push(Owned::adopt(&self.root.join(file), group));
        self.owned.last().unwrap()
    }

    /// Lets the agent exit, reaps the command within a bound, and asserts
    /// that every adopted process has ended. Returns the exit code and
    /// stderr.
    fn finish(mut self) -> (Option<i32>, String) {
        fs::write(self.root.join("release"), "").unwrap();
        let status = support::wait(&mut self.child, std::time::Duration::from_secs(20))
            .wait()
            .unwrap();
        self.assert_owned_gone();
        let stderr = fs::read_to_string(self.root.join("launched.err")).unwrap();
        (status.code(), stderr)
    }

    /// Kills `tmt run` itself with SIGKILL and reaps it; its agent and a
    /// held naming driver stay.
    fn kill(&mut self) {
        let mut child = self.child.take().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }

    /// Releases a held call and the agent, then asserts that every adopted
    /// process, and a driver's whole group, is confirmed ended within a
    /// bound. Unknown evidence keeps waiting and fails at the bound.
    fn assert_owned_gone(&self) {
        let _ = fs::remove_file(self.root.join("driver/hold"));
        let _ = fs::write(self.root.join("release"), "");
        wait_until("every adopted process ends", || {
            self.owned.iter().all(Owned::ended)
        });
    }
}

impl Drop for Launched {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.root.join("driver/hold"));
        let _ = fs::write(self.root.join("release"), "");
        if let Some(mut child) = self.child.take() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while matches!(child.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        // Released processes get a moment to end on their own; what is left is
        // killed, and ownership lasts until the end is confirmed.
        let ended = |owned: &[Owned], within: u64| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(within);
            while !owned.iter().all(Owned::ended) {
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            true
        };
        if !ended(&self.owned, 5) {
            for owned in &self.owned {
                owned.kill();
            }
            if !ended(&self.owned, 5) {
                eprintln!(
                    "fixture processes under {} did not end",
                    self.root.display()
                );
            }
        }
    }
}

/// `tmt run` starts `AGENT` in the fake pane `f1` as `worker`, and the test
/// owns the agent.
fn run_agent(fixture: &Fixture) -> Launched {
    let mut launched = Launched::start(
        fixture,
        fixture.command_in(Some("f1"), &["run", "worker", "/bin/sh", "-c", AGENT]),
    );
    launched.adopt("agent.pid", false);
    launched
}

/// After `tmt run` starts an agent, the host names it with the identity's
/// name without a user step: an attempt the host answers "no agent yet" is
/// tried again, and the attempt it accepts is the last (#3238). Nothing is
/// printed about it.
#[test]
fn run_names_the_agent_it_started_after_the_host_sees_it() {
    register();
    let fixture = naming("run-names", &["no_agent"]);
    fixture.approve();
    let run = run_agent(&fixture);
    let agent_name = fixture.root.join("driver/agentname");
    wait_until("the agent is named", || agent_name.exists());
    assert_eq!(fs::read_to_string(&agent_name).unwrap(), "worker");
    let (code, stderr) = run.finish();
    assert_eq!(code, Some(7), "{stderr}");
    let named = named(&fixture);
    assert_eq!(named.len(), 2, "{named:?}");
    assert!(named.iter().all(|request| request["name"] == "worker"));
    assert!(!stderr.contains("nam"), "{stderr}");
}

/// `tmt resume` names the agent it starts the same way: it launches
/// through the same `run_bound`. The remembered Claude session resumes
/// through a stand-in `claude` on `PATH`.
#[test]
fn resume_names_the_agent_it_started() {
    use tmt_core::{
        binding::{
            BindingRepository,
            session::{HarnessId, ProviderSessionId, RememberedSession, RuntimeMode},
        },
        identity::IdentityReader,
    };
    register();
    let fixture = naming("resume-names", &[]);
    fixture.approve();
    let bound = fixture.tmt_in(Some("f1"), &["name", "worker", "--save"]);
    assert!(bound.status.success(), "{bound:?}");
    // A new agent in the pane has no name yet.
    fs::remove_file(fixture.root.join("driver/agentname")).unwrap();
    fs::remove_file(fixture.root.join("driver/named")).unwrap();
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let identity = storage.find_identity("worker").unwrap().unwrap();
    storage
        .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|records| {
            let mut preferences = records.session_preferences(&identity.id)?;
            preferences.remembered = Some(RememberedSession {
                harness: HarnessId::new("claude").unwrap(),
                mode: RuntimeMode::new("default").unwrap(),
                provider_session: ProviderSessionId::new("4f8c2a7e-0d3b-4c61-9a52-7b1e6d0c9f13")
                    .unwrap(),
                state: None,
                stale_at_ms: None,
                resume_pending_at_ms: None,
            });
            records.set_session_preferences(&identity.id, &preferences)
        })
        .unwrap();
    storage.close().unwrap();
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    fs::write(&claude, format!("#!/bin/sh\n{AGENT}\n")).unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = fixture.command_in(Some("f1"), &["resume", "worker"]);
    command.env(
        "PATH",
        format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
    let mut resume = Launched::start(&fixture, command);
    resume.adopt("agent.pid", false);
    let agent_name = fixture.root.join("driver/agentname");
    wait_until("the resumed agent is named", || agent_name.exists());
    assert_eq!(fs::read_to_string(&agent_name).unwrap(), "worker");
    let (code, stderr) = resume.finish();
    assert_eq!(code, Some(7), "{stderr}");
    assert_eq!(named(&fixture).len(), 1, "{:?}", named(&fixture));
}

/// A command that exits at once gets no naming call, whether or not its
/// exit was seen before admission: either no attempt starts, or `tmt run`
/// stops it before the first one is due.
#[test]
fn a_command_that_exits_at_once_is_not_named() {
    register();
    let fixture = naming("run-quick", &[]);
    fixture.approve();
    let output = fixture.tmt_in(Some("f1"), &["run", "worker", "/bin/sh", "-c", "exit 7"]);
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert!(named(&fixture).is_empty(), "{:?}", named(&fixture));
}

/// A driver without `name-agent` is never asked, and the run keeps its
/// lifecycle: the exit status comes back and the binding's marker names the
/// identity. The command runs past the first attempt's due time.
#[test]
fn run_never_names_through_a_driver_without_name_agent() {
    register();
    let fixture = Fixture::new("run-unnamed");
    fixture.approve();
    let output = fixture.tmt_in(
        Some("f1"),
        &["run", "worker", "/bin/sh", "-c", "sleep 2; exit 7"],
    );
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert!(
        !fixture.calls().iter().any(|call| call == "name-agent"),
        "{:?}",
        fixture.calls()
    );
    let marker: Value =
        serde_json::from_str(&fs::read_to_string(fixture.root.join("driver/marker")).unwrap())
            .unwrap();
    assert_eq!(marker["name"], "worker");
}

/// The accepted limit L1: after SIGKILL of `tmt run`, nothing in TMT ends a
/// naming call that ignores its deadline, as on the explicit naming path.
/// The held driver stays live without a break until well past its 2 s
/// deadline; then the test releases it and the orphaned agent and sees both
/// end.
#[test]
fn after_sigkill_of_tmt_run_a_stubborn_naming_call_outlives_it() {
    register();
    let fixture = naming("run-sigkill", &[]);
    fixture.approve();
    fs::write(fixture.root.join("driver/hold"), "").unwrap();
    let mut run = run_agent(&fixture);
    run.adopt("driver/held", true);
    run.kill();
    // The call started before `held` was read, so its deadline has passed
    // by `until`. Observe the driver the whole time, not once at the end.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while std::time::Instant::now() < until {
        assert_eq!(
            run.owned[1].seen(),
            Seen::Live,
            "the naming call outlived tmt run"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    run.assert_owned_gone();
}

/// Reads and a provider hook never name the agent: after the bind's one
/// call, `ls`, `ls --json`, `ls worker`, `check` and a hook add none.
#[test]
fn reads_and_hooks_never_call_name_agent() {
    register();
    let fixture = naming("reads-unnamed", &["ok"]);
    fixture.approve();
    let bound = fixture.tmt_in(Some("f1"), &["name", "worker", "--save"]);
    assert!(bound.status.success(), "{bound:?}");
    assert_eq!(named(&fixture).len(), 1);
    for args in [
        &["ls"][..],
        &["ls", "--json"],
        &["ls", "worker"],
        &["check", "worker"],
    ] {
        let output = fixture.tmt_in(Some("f1"), args);
        assert!(output.status.success(), "{args:?}: {output:?}");
    }
    for event in ["SessionStart", "Stop"] {
        let mut hook = fixture.command_in(Some("f1"), &["__hook", "claude"]);
        let mut child = hook
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let input = serde_json::json!({"hook_event_name": event, "session_id": "s-1",
            "source": "startup", "reason": "done", "cwd": "/src"});
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), input.to_string().as_bytes())
            .unwrap();
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{event}: {output:?}");
    }
    assert_eq!(named(&fixture).len(), 1, "{:?}", named(&fixture));
}

/// The `agentName` member of a JSON bind or rename.
fn agent_name_of(document: &Value) -> &Value {
    &document["agentName"]
}

/// An explicit bind sets the agent name with the binding's exact request,
/// and says so in JSON; repeating the bind asks again and is set again.
#[test]
fn an_explicit_bind_sets_the_agent_name_and_a_repeat_sets_it_again() {
    register();
    let fixture = naming("bind-named", &["ok", "ok"]);
    fixture.approve();
    for _ in 0..2 {
        let output = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
        assert!(output.status.success(), "{output:?}");
        let bound: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            agent_name_of(&bound),
            &serde_json::json!({"status": "set", "name": "worker"})
        );
    }
    let named = fixture.requests("named");
    assert_eq!(named.len(), 2);
    let pid = u64::from(std::process::id());
    for request in &named {
        assert_eq!(request["name"], "worker");
        assert_eq!(request["socket"], "/tmp/fake.sock");
        assert_eq!(request["paneId"], "fake-1");
        assert_eq!(request["panePid"], pid);
        assert!(
            request["bindingId"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
    }
    assert_eq!(named[0]["bindingId"], named[1]["bindingId"]);
}

/// Each answer keeps the binding. A definite refusal is the `refused`
/// status and says the name was not set. No agent yet, or an unknown
/// outcome, gives no status but one structured JSON warning, never `set` or
/// `refused`. Human output gives one warning with its fix on stderr.
#[test]
fn a_refused_or_unconfirmed_agent_name_keeps_the_binding_and_says_which() {
    register();
    for (answer, warning, said) in [
        ("refused", None, "The agent name worker was not set"),
        (
            "no_agent",
            Some("AGENT_NAME_NO_AGENT"),
            "No agent to name yet",
        ),
        (
            "not_ready",
            Some("AGENT_NAME_NOT_READY"),
            "No agent to name yet",
        ),
        (
            "failed",
            Some("AGENT_NAME_UNCONFIRMED"),
            "TMT could not confirm the agent name worker",
        ),
    ] {
        let fixture = naming(&format!("bind-{answer}"), &[answer, answer]);
        fixture.approve();
        let output = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
        assert!(output.status.success(), "{answer}: {output:?}");
        // One JSON document on stdout, and nothing on stderr.
        let bound: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.stderr.is_empty(), "{answer}: {output:?}");
        assert_eq!(bound["name"], "worker", "{answer}: {bound:#}");
        match warning {
            None => {
                assert_eq!(
                    agent_name_of(&bound),
                    &serde_json::json!({"status": "refused", "name": "worker",
                        "message": "Host driver fake: host says refused"}),
                    "{answer}"
                );
                assert!(bound.get("warnings").is_none(), "{bound:#}");
            }
            Some(code) => {
                assert!(bound.get("agentName").is_none(), "{answer}: {bound:#}");
                let warnings = bound["warnings"].as_array().unwrap();
                assert_eq!(warnings.len(), 1, "{answer}: {bound:#}");
                assert_eq!(warnings[0]["code"], code, "{answer}");
                let message = warnings[0]["message"].as_str().unwrap();
                assert!(message.contains(said), "{answer}: {message}");
                assert!(message.contains("The binding stays"), "{answer}");
            }
        }
        // The binding stands, and the host has no agent name.
        let row = worker(&fixture.json(&["ls", "--json"])).clone();
        assert_eq!(row["presence"], "active", "{answer}: {row:#}");
        assert!(!fixture.root.join("driver/agentname").exists(), "{answer}");

        let human = fixture.tmt_in(Some("f1"), &["name", "worker", "--save"]);
        assert!(human.status.success(), "{answer}: {human:?}");
        let stderr = String::from_utf8_lossy(&human.stderr);
        assert!(stderr.contains(said), "{answer}: {stderr}");
        assert!(stderr.contains("The binding stays"), "{answer}: {stderr}");
        assert_eq!(stderr.matches("warning:").count(), 1, "{answer}: {stderr}");
        assert_eq!(
            stderr.contains("was not set"),
            warning.is_none(),
            "{answer}"
        );
    }
}

/// A driver approved before `name-agent` existed is never asked: the bind
/// reports `not_linked` and prints no warning.
#[test]
fn a_driver_without_name_agent_is_not_linked() {
    register();
    let fixture = Fixture::new("bind-old-driver");
    fixture.approve();
    let output = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let bound: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        agent_name_of(&bound),
        &serde_json::json!({"status": "not_linked"})
    );
    assert!(!fixture.calls().iter().any(|call| call == "name-agent"));
    let human = fixture.tmt_in(Some("f1"), &["name", "worker", "--save"]);
    assert!(human.status.success(), "{human:?}");
    assert!(
        !String::from_utf8_lossy(&human.stderr).contains("agent name"),
        "{human:?}"
    );
}

/// `tmt mv` names the agent after the rename; a rename to the same name
/// asks nothing.
#[test]
fn a_rename_sets_the_new_agent_name_and_the_same_name_asks_nothing() {
    register();
    let fixture = naming("mv-named", &["ok", "ok"]);
    fixture.approve();
    assert!(
        fixture
            .tmt_in(Some("f1"), &["name", "worker", "--save"])
            .status
            .success()
    );
    let moved = fixture.json(&["mv", "worker", "boss", "--json"]);
    assert_eq!(
        agent_name_of(&moved),
        &serde_json::json!({"status": "set", "name": "boss"})
    );
    let same = fixture.json(&["mv", "boss", "boss", "--json"]);
    assert!(same.get("agentName").is_none(), "{same:#}");
    let names: Vec<_> = fixture
        .requests("named")
        .iter()
        .map(|request| request["name"].clone())
        .collect();
    assert_eq!(names, ["worker", "boss"]);
}

/// Unbind takes the agent name away (`name: null`, `expected` the
/// identity's name) before it clears the marker, and clears whatever the
/// driver answers. A name that differs from the identity's stays.
#[test]
fn unbind_takes_the_agent_name_away_first_and_clears_whatever_it_answers() {
    register();
    for answer in ["", "refused", "failed"] {
        let fixture = naming(&format!("unbind-{answer}"), &["", answer]);
        fixture.approve();
        assert!(
            fixture
                .tmt_in(Some("f1"), &["name", "worker", "--save"])
                .status
                .success()
        );
        let before = fixture.calls().len();
        let output = fixture.tmt_in(Some("f1"), &["unbind", "--json"]);
        assert!(output.status.success(), "{answer}: {output:?}");
        let calls = fixture.calls()[before..].to_vec();
        let named = calls.iter().position(|call| call == "name-agent");
        let cleared = calls.iter().position(|call| call == "clear");
        assert!(
            named.is_some() && cleared.is_some() && named < cleared,
            "{answer}: {calls:?}"
        );
        let requests = fixture.requests("named");
        let last = requests.last().unwrap();
        assert!(last["name"].is_null(), "{answer}");
        assert_eq!(last["expected"], "worker", "{answer}");
        assert!(
            !fixture.root.join("driver/marker").exists(),
            "{answer}: marker cleared"
        );
        // Only the answer `{}` from a host that did it took the name away.
        assert_eq!(
            fixture.root.join("driver/agentname").exists(),
            !answer.is_empty(),
            "{answer}"
        );
    }
    // Another agent name than the identity's stays through the unbind.
    let fixture = naming("unbind-other", &["", ""]);
    fixture.approve();
    assert!(
        fixture
            .tmt_in(Some("f1"), &["name", "worker", "--save"])
            .status
            .success()
    );
    fs::write(fixture.root.join("driver/agentname"), "ann").unwrap();
    let output = fixture.tmt_in(Some("f1"), &["unbind", "--json"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fixture.requests("named").last().unwrap()["expected"],
        "worker"
    );
    assert!(!fixture.root.join("driver/marker").exists());
    assert_eq!(
        fs::read_to_string(fixture.root.join("driver/agentname")).unwrap(),
        "ann"
    );
}

/// `tmt ls` shows the host's agent name in JSON, and in human output says
/// when it differs from the TMT name.
#[test]
fn ls_shows_the_agent_name_and_advises_on_a_mismatch() {
    register();
    let fixture = naming("ls-agent-name", &["ok"]);
    fixture.approve();
    assert!(
        fixture
            .tmt_in(Some("f1"), &["name", "worker", "--save"])
            .status
            .success()
    );
    let agent_name = fixture.root.join("driver/agentname");

    // The same name: shown in JSON, no advice.
    fs::write(&agent_name, "worker").unwrap();
    let row = worker(&fixture.json(&["ls", "--json"])).clone();
    assert_eq!(row["agentName"], "worker", "{row:#}");
    let human = fixture.tmt(&["ls"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(!text.contains("agent name"), "{text}");

    // Someone renamed the agent in the host: the row shows the mismatch and
    // the repair.
    fs::write(&agent_name, "ann").unwrap();
    let human = fixture.tmt(&["ls"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("worker's agent has the fake agent name ann, not worker"),
        "{text}"
    );
    assert!(
        text.contains("In worker's pane run: tmt this worker"),
        "{text}"
    );
    assert!(!text.contains("label"), "{text}");

    // The advised command replaces the host's name with TMT's.
    let output = fixture.tmt_in(Some("f1"), &["this", "worker", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let set: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        agent_name_of(&set),
        &serde_json::json!({"status": "set", "name": "worker"})
    );
    assert_eq!(fs::read_to_string(&agent_name).unwrap(), "worker");
    let human = fixture.tmt(&["ls"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(!text.contains("agent name"), "{text}");

    // No agent name: the member is absent, never null, and the row gets
    // no agent-name warning and no `tmt this` advice.
    fs::remove_file(&agent_name).unwrap();
    let row = worker(&fixture.json(&["ls", "--json"])).clone();
    assert!(row.get("agentName").is_none(), "{row:#}");
    let human = fixture.tmt(&["ls"]);
    assert!(human.status.success(), "{human:?}");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(!text.contains("agent name"), "{text}");
    assert!(!text.contains("tmt this"), "{text}");
}

/// A `tmt mv` the host refused leaves the agent with the name TMT set
/// before. `tmt ls` shows the mismatch, and the advised `tmt this` sets the
/// new name.
#[test]
fn ls_advice_repairs_tmts_own_older_name_without_a_clear() {
    register();
    // The bind sets `worker`; the host refuses `boss` for the mv.
    let fixture = naming("ls-owned-name", &["", "refused"]);
    fixture.approve();
    assert!(
        fixture
            .tmt_in(Some("f1"), &["name", "worker", "--save"])
            .status
            .success()
    );
    let agent_name = fixture.root.join("driver/agentname");
    assert_eq!(fs::read_to_string(&agent_name).unwrap(), "worker");
    let moved = fixture.json(&["mv", "worker", "boss", "--json"]);
    assert_eq!(agent_name_of(&moved)["status"], "refused", "{moved:#}");
    assert_eq!(fs::read_to_string(&agent_name).unwrap(), "worker");

    let human = fixture.tmt(&["ls"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("boss's agent has the fake agent name worker, not boss"),
        "{text}"
    );
    assert!(text.contains("In boss's pane run: tmt this boss"), "{text}");

    let output = fixture.tmt_in(Some("f1"), &["this", "boss", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let set: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        agent_name_of(&set),
        &serde_json::json!({"status": "set", "name": "boss"})
    );
    assert_eq!(fs::read_to_string(&agent_name).unwrap(), "boss");
    let human = fixture.tmt(&["ls"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(!text.contains("agent name"), "{text}");
}
