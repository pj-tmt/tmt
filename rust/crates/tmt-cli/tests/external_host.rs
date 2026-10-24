//! A stored binding on an external host, end to end (#570 slice 3b-2a-2): a
//! shell-script driver approved in a disposable home, a binding made through
//! the core port on that driver, then the real `tmt` reading it. Routing by
//! name selects its recorded server independently of the caller. Without its
//! driver, or with a changed one, the binding stays unknown and is never retired.
#![cfg(unix)]
// This fixture uses only the shared private command environment.
#[path = "external_host/caller_session.rs"]
mod caller_session;
#[path = "external_host/discovery.rs"]
mod discovery;
#[path = "external_host/driver_switch.rs"]
mod driver_switch;
#[path = "external_host/hand_resume.rs"]
mod hand_resume;
#[path = "external_host/identity_pin.rs"]
mod identity_pin;
#[path = "external_host/move_session.rs"]
mod move_session;
#[path = "external_host/nested_host.rs"]
mod nested_host;
#[path = "external_host/nested_provider.rs"]
mod nested_provider;
#[path = "external_host/passthrough.rs"]
mod passthrough;
#[path = "external_host/resume_new.rs"]
mod resume_new;
#[path = "external_host/run_exit.rs"]
mod run_exit;
#[path = "external_host/sampling.rs"]
mod sampling;
#[path = "external_host/send_now.rs"]
mod send_now;
#[path = "external_host/stop.rs"]
mod stop;
#[allow(dead_code)]
mod support;
#[path = "external_host/unrecoverable_session.rs"]
mod unrecoverable_session;
#[path = "external_host/uuid_selectors.rs"]
mod uuid_selectors;

use serde_json::Value;
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Output, Stdio},
    time::Duration,
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
/// With `two-panes`, a second pane `fake-2` (target `f2`) has its own shell
/// (`pid-2`) and marker (`marker-2`).
const DRIVER: &str = r#"#!/bin/sh
d=$(dirname "$0")
echo "$3" >> "$d/calls"
echo $$ >> "$d/pids"
input=$(cat)
marker="$d/marker"
case "$input" in *'"paneId":"fake-2"'*) marker="$d/marker-2" ;; esac
case "$3" in
  capabilities) cat "$d/capabilities" ;;
  server)
    printf 'server %s\n' "$input" >> "$d/inventory"
    # With `server-default`, a request with no socket gets that other server.
    case "$input" in *'"socket":null'*) if [ -f "$d/server-default" ]; then cat "$d/server-default"; exit 0; fi ;; esac
    cat "$d/server" ;;
  publish)
    printf '%s' "$input" | sed -n 's/.*"marker":\({[^}]*}\).*/\1/p' > "$marker"
    printf '{"ok":{}}' ;;
  clear) rm -f "$marker"; printf '{"ok":{"cleared":true}}' ;;
  caller)
    # While `hold-caller` exists, a caller call records itself and a copy
    # of the file `watch` names, then waits (at most 30 s).
    if [ -f "$d/hold-caller" ]; then
      echo "$$ $PPID $(ps -o pgid= -p $$ | tr -d ' ')" > "$d/held.staged"
      cp "$(cat "$d/watch")" "$d/watched" 2>/dev/null
      mv "$d/held.staged" "$d/held"
      i=0; while [ -f "$d/hold-caller" ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i+1)); done
    fi
    case "$input" in
      *'"FAKE_PANE_ID":"f1"'*)
        printf '{"ok":{"pane":{"id":"fake-1","socket":"/tmp/fake.sock","shellPid":%s}}}' "$(cat "$d/shell")" ;;
      *'"FAKE_PANE_ID":"f2"'*)
        if [ -f "$d/two-panes" ]; then
          printf '{"ok":{"pane":{"id":"fake-2","socket":"/tmp/fake.sock","shellPid":%s}}}' "$(cat "$d/pid-2")"
        else printf '{"ok":{"pane":null}}'; fi ;;
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
  capture)
    printf '%s\n' "$input" >> "$d/captured"
    # `capture-answer`, when present, replaces the answer.
    if [ -f "$d/capture-answer" ]; then cat "$d/capture-answer"
    else printf '{"ok":{"text":"captured line"}}'; fi ;;
  focus) printf '{"ok":{}}' ;;
  close)
    printf '%s\n' "$input" >> "$d/closed"
    # `close-answer`, when present, replaces the answer.
    if [ -f "$d/close-answer" ]; then cat "$d/close-answer"; else printf '{"ok":{}}'; fi ;;
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
      '') if [ -n "$n" ]; then printf '%s' "$n" > "$d/agentname.next"; mv "$d/agentname.next" "$d/agentname"; elif [ -n "$e" ] && [ "$h" = "$e" ]; then rm -f "$d/agentname"; fi
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
    printf 'snapshot %s\n' "$input" >> "$d/inventory"
    if [ -f "$d/unreachable" ]; then printf '{"error":{"code":"unavailable","message":"server offline"}}'; exit 0; fi
    if [ -f "$d/absent" ]; then printf '{"ok":{"panes":[]}}'; exit 0; fi
    # A sampling fixture logs each snapshot with its caller and requested
    # panes. While `gate` exists, a caller `ps` cannot classify, and a
    # consumption sampler other than the one pid it names, are refused.
    if [ -f "$d/sampling" ]; then
      p=$(printf '%s' "$input" | sed -n 's/.*"panes":\(\[[^]]*\]\).*/\1/p')
      c=$(ps -o command= -p "$PPID" 2>/dev/null)
      case "$c" in
        *__consumption-sample*) r=sampler ;;
        '') r=unknown ;;
        *) r=other ;;
      esac
      v=answered
      if [ -f "$d/gate" ]; then
        if [ "$r" = unknown ] || { [ "$r" = sampler ] && [ "$PPID" != "$(cat "$d/gate")" ]; }; then v=denied; fi
      fi
      echo "pid=$$ ppid=$PPID caller=$r $v panes=${p:-null}" >> "$d/snapshots"
      if [ "$v" = denied ]; then printf '{"error":{"code":"failed","message":"gated"}}'; exit 0; fi
    fi
    # Discovery switches: `slow-snapshot` answers late, `big-snapshot` too
    # much; `target` moves the pane; after `swap-after` snapshots the pane's
    # shell is `pid-swapped`.
    if [ -f "$d/slow-snapshot" ]; then sleep 3; fi
    if [ -f "$d/big-snapshot" ]; then printf '{"ok":{"panes":[{"id":"fake-1","cwd":"'; head -c 1100000 /dev/zero | tr '\0' a; printf '"}]}}'; exit 0; fi
    n=$(( $(cat "$d/snapshot-count" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$d/snapshot-count"
    pid=$(cat "$d/pid")
    if [ -f "$d/swap-after" ] && [ "$n" -gt "$(cat "$d/swap-after")" ]; then pid=$(cat "$d/pid-swapped"); fi
    t=$(cat "$d/target" 2>/dev/null); [ -n "$t" ] || t=f1
    m=$(cat "$d/marker" 2>/dev/null); [ -n "$m" ] || m=null
    a=$(cat "$d/agentname" 2>/dev/null); if [ -n "$a" ]; then a=",\"agentName\":\"$a\""; fi
    # `opened` holds true or false when the host reports whether tmt opened the pane.
    o=$(cat "$d/opened" 2>/dev/null); if [ -n "$o" ]; then a="$a,\"openedByTmt\":$o"; fi
    # Only the panes asked about, as the protocol requires.
    p1=$(printf '{"id":"fake-1","target":"%s","cwd":"/src","command":"sh","panePid":%s,"suggestedName":null,"marker":%s%s}' "$t" "$pid" "$m" "$a")
    case "$input" in *'"panes":["fake-2"]'*) p1= ;; esac
    p2=
    if [ -f "$d/two-panes" ]; then
      m2=$(cat "$d/marker-2" 2>/dev/null); [ -n "$m2" ] || m2=null
      p2=$(printf '{"id":"fake-2","target":"f2","cwd":"/src","command":"sh","panePid":%s,"suggestedName":null,"marker":%s}' "$(cat "$d/pid-2")" "$m2")
      case "$input" in *'"panes":["fake-1"]'*) p2= ;; esac
    fi
    if [ -n "$p1" ] && [ -n "$p2" ]; then p1="$p1,"; fi
    printf '{"ok":{"panes":[%s%s]}}' "$p1" "$p2" ;;
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
        tmt_test_support::write_executable(&driver, DRIVER.as_bytes(), 0o755).unwrap();
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

    /// A driver that can also close a pane. What it says about who opened the
    /// pane is in `opened`: `true`, `false`, or no file at all.
    fn closing(name: &str) -> Self {
        let fixture = Self::new(name);
        fs::write(
            fixture.root.join("driver/capabilities"),
            r#"{"ok":{"protocols":[1],"kind":"host","name":"fake","version":"0.0.0-test","ops":["caller","server","resolve-target","snapshot","publish","clear","close"],"paneId":{"prefix":"fake-"},"target":"f{n}","callerEnv":["FAKE_PANE_ID"]}}"#,
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
    bind_pane(fixture, record, "fake-1", "worker");
}

/// `bind` for the identity `name` on the driver's pane `pane`.
fn bind_pane(fixture: &Fixture, record: registry::DriverRecord, pane: &str, name: &str) {
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
    bind_identity(&mut storage, &mut session, pane, name, true).unwrap();
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

/// An unapproved or changed driver is not an inactive identity or permission
/// to run the driver, touch a pane or discard the stored binding.
fn assert_talk_fails_closed(fixture: &Fixture) {
    let before = fixture.calls().len();
    let talk = fixture.tmt(&["talk", "worker", "hello", "--detach", "--json"]);
    assert!(!talk.status.success(), "{talk:?}");
    let failure: Value = serde_json::from_slice(&talk.stdout).unwrap();
    assert_eq!(
        failure["error"]["code"], "RECONCILIATION_FAILED",
        "{failure:#}"
    );
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not approved")
    );
    let human = fixture.tmt(&["check", "worker"]);
    assert_eq!(human.status.code(), Some(1), "{human:?}");
    assert!(String::from_utf8_lossy(&human.stderr).contains("tmt driver install <driver-path>"));
    let calls = fixture.calls();
    assert_eq!(
        calls.len(),
        before,
        "an unapproved driver must never run: {calls:?}"
    );
}

#[test]
fn an_external_binding_reads_through_its_driver() {
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

    assert_eq!(
        worker(&fixture.json(&["ls", "--json"]))["presence"],
        "active"
    );
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
    tmt_test_support::write_executable(
        &fixture.driver,
        format!("{DRIVER}\n# changed\n").as_bytes(),
        0o755,
    )
    .unwrap();
    fs::remove_file(fixture.root.join("driver/calls")).unwrap();
    let row = worker(&fixture.json(&["ls", "--json"])).clone();
    assert_eq!(row["presence"], "unknown", "{row:#}");
    assert!(fixture.calls().is_empty());
    assert_talk_fails_closed(&fixture);

    // Restored and approved again: the same binding reads active again.
    tmt_test_support::write_executable(&fixture.driver, DRIVER.as_bytes(), 0o755).unwrap();
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
fn a_durable_identity_selector_never_asks_the_driver_for_the_caller() {
    register();
    let fixture = Fixture::new("durable");
    fixture.approve();
    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(named.status.success(), "{named:?}");
    let _ = fs::remove_file(fixture.root.join("driver/calls"));

    // An explicit name and the MCP server's pinned saved UUID read storage
    // only, even from inside the driver's pane.
    let explicit = fixture.tmt_in(Some("f1"), &["notes", "path", "--identity", "worker"]);
    assert!(explicit.status.success(), "{explicit:?}");
    let mut server = fixture
        .command_in(Some("f1"), &["mcp", "--identity", "worker"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let frames = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tmt_list","arguments":{}}}"#,
    ];
    server
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}\n", frames.join("\n")).as_bytes())
        .unwrap();
    let output = support::wait(&mut Some(server), Duration::from_secs(10))
        .wait_with_output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let listed: Value = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|frame| frame["id"] == 2)
        .unwrap();
    assert_eq!(listed["result"]["isError"], false, "{listed:#}");
    assert_eq!(fixture.calls(), Vec::<String>::new());

    // The implicit pane selector still asks, and still refuses a pane whose
    // shell is not the caller's ancestor.
    let implicit = fixture.tmt_in(Some("f1"), &["notes", "path"]);
    assert!(implicit.status.success(), "{implicit:?}");
    assert_eq!(fixture.calls().first().map(String::as_str), Some("caller"));
    fs::write(fixture.root.join("driver/shell"), "999999").unwrap();
    let _ = fs::remove_file(fixture.root.join("driver/calls"));
    let refused = fixture.tmt_in(Some("f1"), &["notes", "path"]);
    assert!(!refused.status.success(), "{refused:?}");
    assert_eq!(fixture.calls(), ["caller"]);
    let explicit = fixture.tmt_in(Some("f1"), &["notes", "path", "--identity", "worker"]);
    assert!(explicit.status.success(), "{explicit:?}");
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
fn check_zero_asks_an_external_host_for_the_visible_screen() {
    register();
    let fixture = Fixture::delivering("check-zero");
    fixture.approve();
    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(named.status.success(), "{named:?}");

    let checked = fixture.tmt_in(
        Some("f1"),
        &["check", "worker", "0", "--capture-only", "--json"],
    );
    assert!(checked.status.success(), "{checked:?}");
    let checked: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(checked["lines"], 0);
    assert_eq!(checked["output"], "captured line");
    // Core passes 0 on unchanged: the driver contract defines it as the
    // visible screen.
    let captured = fixture.requests("captured");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["lines"], 0);
}

#[test]
fn a_failed_capture_on_an_external_host_names_its_driver_answer_not_tmux() {
    register();
    let fixture = Fixture::delivering("check-refused");
    fixture.approve();
    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(named.status.success(), "{named:?}");
    let answer = |text: &str| fs::write(fixture.root.join("driver/capture-answer"), text).unwrap();

    // An older driver that refuses the visible screen.
    answer(r#"{"error":{"code":"bad_request","message":"capture needs at least one line"}}"#);
    let refused = fixture.tmt_in(
        Some("f1"),
        &["check", "worker", "0", "--capture-only", "--json"],
    );
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let failure: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "ERROR");
    assert_eq!(
        failure["error"]["message"],
        "Failed to capture pane fake-1 on host fake: the fake driver answered bad_request: capture needs at least one line"
    );
    let human = fixture.tmt_in(Some("f1"), &["check", "worker", "0", "--capture-only"]);
    assert_eq!(human.status.code(), Some(1), "{human:?}");
    let stderr = String::from_utf8_lossy(&human.stderr);
    assert!(
        stderr.contains("Failed to capture pane fake-1 on host fake: the fake driver answered bad_request: capture needs at least one line"),
        "{stderr}"
    );
    assert!(!stderr.contains("tmux"), "{stderr}");

    // A driver that breaks the protocol: the host's own error, no tmux hint.
    answer("not json");
    let broken = fixture.tmt_in(Some("f1"), &["check", "worker", "--capture-only", "--json"]);
    assert_eq!(broken.status.code(), Some(1), "{broken:?}");
    let failure: Value = serde_json::from_slice(&broken.stdout).unwrap();
    let message = failure["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("Failed to capture pane fake-1 on host fake: "),
        "{message}"
    );
    assert!(!message.contains("tmux"), "{message}");
    assert!(fixture.requests("typed").is_empty());
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
    resume_names_the_agent("resume-names", false, false);
}

/// The same resume, selecting the member by its identity UUID.
#[test]
fn resume_by_identity_uuid_names_the_agent_it_started() {
    resume_names_the_agent("resume-names-uuid", true, false);
}

/// The member is selected by its UUID after a rename, while another identity holds its old name.
#[test]
fn resume_by_identity_uuid_follows_a_rename() {
    resume_names_the_agent("resume-names-renamed", true, true);
}

fn resume_names_the_agent(label: &str, by_uuid: bool, renamed: bool) {
    use tmt_core::{
        binding::{
            BindingRepository,
            session::{HarnessId, ProviderSessionId, RememberedSession, RuntimeMode},
        },
        identity::IdentityReader,
    };
    register();
    let fixture = naming(label, &[]);
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
    if renamed {
        let moved = fixture.tmt(&["mv", &identity.id, "renamed"]);
        assert!(moved.status.success(), "{moved:?}");
        let namesake = fixture.tmt(&["identity", "create", "worker", "--json"]);
        assert!(namesake.status.success(), "{namesake:?}");
    }
    let selector = if by_uuid {
        identity.id.as_str()
    } else {
        "worker"
    };
    let mut command = fixture.command_in(Some("f1"), &["resume", selector]);
    command.env(
        "PATH",
        format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
    let mut resume = Launched::start(&fixture, command);
    resume.adopt("agent.pid", false);
    let agent_name = fixture.root.join("driver/agentname");
    wait_until("the resumed agent is named", || agent_name.exists());
    let expected = if renamed { "renamed" } else { "worker" };
    assert_eq!(fs::read_to_string(&agent_name).unwrap(), expected);
    let (code, stderr) = resume.finish();
    assert_eq!(code, Some(7), "{stderr}");
    assert_eq!(named(&fixture).len(), 1, "{:?}", named(&fixture));
    if renamed {
        // The namesake was never bound: the resume stayed on the selected identity.
        let listed = fixture.json(&["ls", "--json"]).to_string();
        assert!(
            listed.contains("\"renamed\"") && listed.contains("\"worker\""),
            "{listed}"
        );
        let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
        let namesake = storage.find_identity("worker").unwrap().unwrap();
        assert_ne!(namesake.id, identity.id);
        let entry = storage
            .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|records| {
                records.entry_by_id(&namesake.id)
            })
            .unwrap()
            .unwrap();
        assert!(entry.binding.is_none(), "{entry:?}");
        storage.close().unwrap();
    }
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
        // Session completion reports independently of agent naming. A native
        // provider can be visible beneath this fixture's shared pane ancestor.
        let session_warning = usize::from(
            stderr.contains("The pane is bound, but the provider session was not recorded:"),
        );
        assert_eq!(
            stderr.matches("warning:").count(),
            1 + session_warning,
            "{answer}: {stderr}"
        );
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

/// The hook's one fixed line when it could not verify its caller (#3208).
const HOOK_UNAVAILABLE: &str = "tmt: lifecycle context unavailable; continuing without context.\n";

/// A Claude lifecycle event as the provider runs it: `tmt __hook claude` is
/// the child of a process named `claude`, which is the child of the pane
/// shell (this test process). A link to `sh` named `claude` stands in (`ps`
/// reports the name it was run by; a copied system binary would not run on
/// macOS); `provider: false` runs the hook with no provider ancestor. The
/// provider stays alive while `running` reads its state, then exits.
fn hook(
    fixture: &Fixture,
    pane: Option<&str>,
    provider: bool,
    event: &str,
    running: impl FnOnce(),
) -> Output {
    hook_with(fixture, pane, &[], provider, event, running)
}

/// `hook` with `extra` variables in the provider's environment.
fn hook_with(
    fixture: &Fixture,
    pane: Option<&str>,
    extra: &[(&str, &str)],
    provider: bool,
    event: &str,
    running: impl FnOnce(),
) -> Output {
    let tmt = fixture.command_in(pane, &[]);
    let program = if provider {
        let claude = fixture.root.join("bin/claude");
        if !claude.exists() {
            fs::create_dir_all(fixture.root.join("bin")).unwrap();
            std::os::unix::fs::symlink("/bin/sh", &claude).unwrap();
        }
        claude
    } else {
        PathBuf::from("/bin/sh")
    };
    let hooked = fixture.root.join("hook");
    for file in ["out", "err", "done", "release"] {
        let _ = fs::remove_file(hooked.with_extension(file));
    }
    let input = serde_json::json!({"hook_event_name": event, "session_id": "s-1",
        "source": "startup", "reason": "other", "cwd": "/src", "model": "claude-test"});
    fs::write(hooked.with_extension("json"), input.to_string()).unwrap();
    // The shell outlives the hook (no exec into it) until the test releases
    // it; a failing test reaps it through `Provider`, and the 30 s bound
    // covers a test process that is killed.
    let child = std::process::Command::new(program)
        .args([
            "-c",
            "\"$0\" __hook claude < \"$1.json\" > \"$1.out\" 2> \"$1.err\"; touch \"$1.done\"
             i=0; while [ ! -e \"$1.release\" ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i+1)); done",
        ])
        .arg(tmt.get_program())
        .arg(&hooked)
        .env_clear()
        .envs(
            tmt.get_envs()
                .filter_map(|(name, value)| value.map(|value| (name, value))),
        )
        .envs(extra.iter().copied())
        .current_dir(&fixture.root)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let mut provider = Provider {
        child,
        release: hooked.with_extension("release"),
    };
    wait_until("the hook", || hooked.with_extension("done").exists());
    running();
    fs::write(&provider.release, "").unwrap();
    let status = provider.child.wait().unwrap();
    Output {
        status,
        stdout: fs::read(hooked.with_extension("out")).unwrap(),
        stderr: fs::read(hooked.with_extension("err")).unwrap(),
    }
}

/// The stand-in provider, released and reaped even when a test panics.
struct Provider {
    child: std::process::Child,
    release: PathBuf,
}

impl Drop for Provider {
    fn drop(&mut self) {
        let _ = fs::write(&self.release, "");
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn host_servers(fixture: &Fixture) -> i64 {
    rusqlite::Connection::open(fixture.state().join("tmt.db"))
        .unwrap()
        .query_row("SELECT count(*) FROM host_servers", [], |row| row.get(0))
        .unwrap()
}

/// The hook refused: it still succeeds for the provider, says so in one
/// fixed line and records no session.
fn assert_hook_refused(fixture: &Fixture, output: &Output) {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stderr), HOOK_UNAVAILABLE);
    let listed = fixture.json(&["ls", "--json"]);
    if let Some(rows) = listed["identities"].as_array()
        && let Some(row) = rows.iter().find(|row| row["name"] == "worker")
    {
        assert!(row["resume"]["session"].is_null(), "{row:#}");
        assert!(row["runningDriver"].is_null(), "{row:#}");
    }
}

#[test]
fn a_provider_start_in_an_external_pane_records_its_session() {
    register();
    let fixture = Fixture::new("hook-admits");
    let record = fixture.approve();
    bind(&fixture, record);
    let servers = host_servers(&fixture);

    let mut listed = Value::Null;
    let output = hook(&fixture, Some("f1"), true, "SessionStart", || {
        listed = fixture.json(&["ls", "--json"]);
    });
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        context["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    let row = worker(&listed);
    assert_eq!(row["runningDriver"], "claude", "{row:#}");
    assert_eq!(row["resume"]["session"], "s-1", "{row:#}");
    // The server was the recorded one: nothing was added.
    assert_eq!(host_servers(&fixture), servers);
}

#[test]
fn a_restarted_external_server_refuses_the_hook_and_records_nothing() {
    register();
    let fixture = Fixture::new("hook-restarted");
    let record = fixture.approve();
    bind(&fixture, record);
    let servers = host_servers(&fixture);
    // The driver now names another live process for the same socket: a new
    // incarnation that no bind recorded.
    let parent = std::os::unix::process::parent_id();
    fs::write(
        fixture.root.join("driver/server"),
        format!(r#"{{"ok":{{"server":{{"socket":"/tmp/fake.sock","pid":{parent},"startTime":"driver-says"}}}}}}"#),
    )
    .unwrap();

    let output = hook(&fixture, Some("f1"), true, "SessionStart", || {});
    assert_hook_refused(&fixture, &output);
    assert_eq!(host_servers(&fixture), servers);
}

#[test]
fn an_unrecorded_external_server_refuses_the_hook_without_recording_it() {
    register();
    let fixture = Fixture::new("hook-unrecorded");
    fixture.approve();
    // Storage exists, but no bind ever recorded this server.
    fixture.json(&["ls", "--json"]);
    assert_eq!(host_servers(&fixture), 0);

    let output = hook(&fixture, Some("f1"), true, "SessionStart", || {});
    assert_hook_refused(&fixture, &output);
    assert_eq!(host_servers(&fixture), 0);
}

#[test]
fn a_hook_without_its_provider_ancestor_or_with_a_stale_marker_refuses() {
    register();
    let fixture = Fixture::new("hook-evidence");
    let record = fixture.approve();
    bind(&fixture, record);

    // No `claude` between the hook and the pane shell.
    let output = hook(&fixture, Some("f1"), false, "SessionStart", || {});
    assert_hook_refused(&fixture, &output);

    // The pane's marker names another binding.
    let marker = fixture.root.join("driver/marker");
    let text = fs::read_to_string(&marker).unwrap();
    let binding = serde_json::from_str::<Value>(&text).unwrap()["bindingId"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::write(
        &marker,
        text.replace(&binding, "00000000-0000-4000-8000-000000000000"),
    )
    .unwrap();
    let output = hook(&fixture, Some("f1"), true, "SessionStart", || {});
    assert_hook_refused(&fixture, &output);
}

/// Before #3208 an unverified external caller fell back to tmux, found no
/// pane and succeeded silently with nothing recorded.
#[test]
fn an_approved_driver_caller_that_cannot_be_verified_says_so() {
    register();
    let fixture = Fixture::new("hook-unverified");
    let record = fixture.approve();
    bind(&fixture, record);
    // The driver's pane shell is no ancestor of the hook.
    fs::write(fixture.root.join("driver/shell"), "999999").unwrap();

    let output = hook(&fixture, Some("f1"), true, "SessionStart", || {});
    assert_hook_refused(&fixture, &output);
}

#[test]
fn a_hook_outside_any_host_pane_stays_silent() {
    register();
    let fixture = Fixture::new("hook-no-host");
    let record = fixture.approve();
    bind(&fixture, record);
    let _ = fs::remove_file(fixture.root.join("driver/calls"));

    let output = hook(&fixture, None, true, "SessionStart", || {});
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(fixture.calls().is_empty(), "{:?}", fixture.calls());
}

/// One provider stand-in that runs each hook given to it from the same
/// process, so that its lifecycle events (start, prompt, interrupt, end)
/// belong to one provider incarnation. As in `hook`, it is a link to `sh`
/// named after `provider`, the child of this test process (the pane shell).
/// It is stopped and reaped on drop, also on panic.
struct SteppedProvider {
    child: std::process::Child,
    provider: &'static str,
    steps: PathBuf,
    next: usize,
    /// The child is a pane shell that leads its own process group, with the
    /// provider and its commands in it.
    shell: bool,
}

impl SteppedProvider {
    fn start(fixture: &Fixture, provider: &'static str, name: &str) -> Self {
        Self::start_in(fixture, provider, name, "f1", false)
    }

    /// `start` in the pane whose target is `pane`. With `shell`, the
    /// provider is the child of its own pane shell, a `sh` the test owns,
    /// instead of the child of this test process.
    fn start_in(
        fixture: &Fixture,
        provider: &'static str,
        name: &str,
        pane: &str,
        shell: bool,
    ) -> Self {
        Self::start_below(
            fixture,
            provider,
            name,
            pane,
            if shell { &["sh"] } else { &[] },
            &[],
        )
    }

    /// `start` in pane `f1`, below a chain of stand-ins named `ancestors`
    /// (outermost first; `sh` is the real shell), such as a terminal Claude,
    /// its daemon and a pty host above a daemon-hosted worker. The outermost
    /// leads a process group the test owns, as a pane shell does.
    fn start_nested(
        fixture: &Fixture,
        provider: &'static str,
        name: &str,
        ancestors: &[&'static str],
    ) -> Self {
        Self::start_nested_in(fixture, provider, name, "f1", ancestors)
    }

    /// `start_nested` in the pane whose target is `pane`.
    fn start_nested_in(
        fixture: &Fixture,
        provider: &'static str,
        name: &str,
        pane: &str,
        ancestors: &[&'static str],
    ) -> Self {
        assert!(!ancestors.is_empty());
        Self::start_below(fixture, provider, name, pane, ancestors, &[])
    }

    /// `start` with `words` after the stand-in's own arguments, as a provider
    /// launch's options appear in its command line.
    fn start_with_words(
        fixture: &Fixture,
        provider: &'static str,
        name: &str,
        words: &[&str],
    ) -> Self {
        Self::start_below(fixture, provider, name, "f1", &[], words)
    }

    fn start_below(
        fixture: &Fixture,
        provider: &'static str,
        name: &str,
        pane: &str,
        ancestors: &[&'static str],
        words: &[&str],
    ) -> Self {
        let stand_in = |name: &str| {
            if name == "sh" {
                return PathBuf::from("/bin/sh");
            }
            let program = fixture.root.join("bin").join(name);
            if !program.exists() {
                fs::create_dir_all(fixture.root.join("bin")).unwrap();
                std::os::unix::fs::symlink("/bin/sh", &program).unwrap();
            }
            program
        };
        let program = stand_in(provider);
        let steps = fixture.root.join(name);
        fs::create_dir_all(&steps).unwrap();
        let tmt = fixture.command_in(Some(pane), &[]);
        // Each step waits at most 30 s, so a killed test process cannot
        // leave the stand-in behind for long. A step is a hook (`<i>.json`)
        // or a command (`<i>.args`, one argument a line), with the
        // `NAME=value` lines of `<i>.env` exported. With `hang`, it ignores
        // its stop file and SIGHUP, for a cleanup test.
        let script = "echo $$ > \"$1/provider.pid\"
                 if [ -e \"$1/ignore-term\" ]; then trap 'printf \"TERM\\n\" >> \"$1/terms\"' TERM; fi
                 if [ -e \"$1/hang\" ]; then trap '' HUP
                   i=0; while [ $i -lt 600 ]; do sleep 0.05; i=$((i+1)); done; exit 0; fi
                 i=0; while :; do w=0
                   while [ ! -e \"$1/$i.json\" ] && [ ! -e \"$1/$i.args\" ] && [ ! -e \"$1/stop\" ] && [ $w -lt 600 ]; do sleep 0.05; w=$((w+1)); done
                   if [ -e \"$1/$i.json\" ]; then
                     (if [ -e \"$1/$i.env\" ]; then while IFS= read -r v; do export \"$v\"; done < \"$1/$i.env\"; fi
                      exec \"$0\" __hook \"$2\" < \"$1/$i.json\" > \"$1/$i.out\" 2> \"$1/$i.err\"); echo $? > \"$1/$i.status\"
                   elif [ -e \"$1/$i.args\" ]; then
                     (t=\"$0\"; s=\"$1/$i\"; set --
                      while IFS= read -r a; do set -- \"$@\" \"$a\"; done < \"$s.args\"
                      while IFS= read -r v; do export \"$v\"; done < \"$s.env\"
                      exec \"$t\" \"$@\" > \"$s.out\" 2> \"$s.err\"); echo $? > \"$1/$i.status\"
                   else exit 0; fi
                   touch \"$1/$i.done\"; i=$((i+1))
                 done";
        let mut command = if let Some((outermost, inner)) = ancestors.split_first() {
            // Each ancestor waits for the next (`; :` keeps it from replacing
            // itself with it). The outermost leads a group the test owns.
            use std::os::unix::process::CommandExt;
            let mut command = std::process::Command::new(stand_in(outermost));
            command.args(["-c", "\"$0\" \"$@\"; :"]);
            for ancestor in inner {
                command
                    .arg(stand_in(ancestor))
                    .args(["-c", "\"$0\" \"$@\"; :"]);
            }
            command.arg(program).process_group(0);
            command
        } else {
            std::process::Command::new(program)
        };
        if provider == "codex" {
            // Codex's direct caller proof conservatively rejects `-c` as a
            // config override. A file keeps this shell stand-in's argv honest.
            let script_path = steps.join("provider.sh");
            fs::write(
                &script_path,
                format!("t=$1; shift\n{}", script.replace("\"$0\"", "\"$t\"")),
            )
            .unwrap();
            command.arg(script_path);
        } else {
            command.args(["-c", script]);
        }
        let child = command
            .arg(tmt.get_program())
            .arg(&steps)
            .arg(provider)
            .args(words)
            .env_clear()
            .envs(
                tmt.get_envs()
                    .filter_map(|(name, value)| value.map(|value| (name, value))),
            )
            .current_dir(&fixture.root)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            child,
            provider,
            steps,
            next: 0,
            shell: !ancestors.is_empty(),
        }
    }

    /// The provider stand-in's pid, once it has started.
    fn provider_pid(&self) -> u64 {
        let file = self.steps.join("provider.pid");
        wait_until("the provider starts", || {
            fs::read_to_string(&file).is_ok_and(|pid| pid.ends_with('\n'))
        });
        fs::read_to_string(&file).unwrap().trim().parse().unwrap()
    }

    /// Runs `tmt` with `args` from the provider, with `env` added, and
    /// returns its exit code, stdout and stderr.
    fn command(&mut self, env: &[(&str, &str)], args: &[&str]) -> (i32, String, String) {
        let step = self.steps.join(self.next.to_string());
        let lines = |items: Vec<String>| {
            items
                .iter()
                .map(|item| format!("{item}\n"))
                .collect::<String>()
        };
        fs::write(
            step.with_extension("env"),
            lines(
                env.iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect(),
            ),
        )
        .unwrap();
        let staged = step.with_extension("staged");
        fs::write(
            &staged,
            lines(args.iter().map(|arg| (*arg).to_owned()).collect()),
        )
        .unwrap();
        fs::rename(&staged, step.with_extension("args")).unwrap();
        self.finish(step)
    }

    /// Runs one hook with `payload` and returns its exit code, stdout and
    /// stderr.
    fn hook(&mut self, payload: Value) -> (i32, String, String) {
        self.hook_with_env(payload, &[])
    }
    /// `hook` with these variables exported for that hook only.
    fn hook_with_env(&mut self, payload: Value, env: &[(&str, &[u8])]) -> (i32, String, String) {
        let step = self.steps.join(self.next.to_string());
        if !env.is_empty() {
            let mut lines = Vec::new();
            for (name, value) in env {
                lines.extend_from_slice(name.as_bytes());
                lines.push(b'=');
                lines.extend_from_slice(value);
                lines.push(b'\n');
            }
            fs::write(step.with_extension("env"), lines).unwrap();
        }
        let staged = step.with_extension("staged");
        fs::write(&staged, payload.to_string()).unwrap();
        fs::rename(&staged, step.with_extension("json")).unwrap();
        self.finish(step)
    }

    fn finish(&mut self, step: PathBuf) -> (i32, String, String) {
        wait_until(&format!("the {} step", self.provider), || {
            step.with_extension("done").exists()
        });
        self.next += 1;
        let read = |extension| fs::read_to_string(step.with_extension(extension)).unwrap();
        (
            read("status").trim().parse().unwrap(),
            read("out"),
            read("err"),
        )
    }
}

impl SteppedProvider {
    /// With a pane shell: asks the stand-in to stop, and gives the shell up
    /// to 5 s to exit, without reaping it. Then the owned group is ended
    /// while its leader is unreaped, so its id names no other group: a
    /// stopped or stuck provider and its commands end with it.
    fn stop_pane(&mut self) {
        let _ = fs::write(self.steps.join("stop"), "");
        let pid = u64::from(self.child.id());
        let graceful = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < graceful
            && !matches!(
                observe_runtime_process(&UnixCommandRunner, pid, deadline()),
                Ok(ProcessObservation::UnreapedZombie(_) | ProcessObservation::Gone)
            )
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        stop_owned_group(&mut self.child, "the pane shell");
    }
}

impl Drop for SteppedProvider {
    fn drop(&mut self) {
        if !self.shell {
            let _ = fs::write(self.steps.join("stop"), "");
            if !matches!(self.child.try_wait(), Ok(Some(_))) {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        } else if std::thread::panicking() {
            // The same cleanup; a second panic while unwinding would abort,
            // so a failure is only reported.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.stop_pane())).is_err()
            {
                eprintln!("the pane shell {} was not cleaned up", self.child.id());
            }
        } else {
            self.stop_pane();
        }
    }
}

/// Ends the group that `child` leads while `child` is still unreaped, so the
/// group's id names no other group; then reaps `child` within 3 s and waits,
/// read only, up to 3 s for the group to be empty. A failure panics with
/// `what`.
fn stop_owned_group(child: &mut std::process::Child, what: &str) -> std::process::ExitStatus {
    use nix::sys::signal::{Signal::SIGKILL, killpg};
    let group = nix::unistd::Pid::from_raw(i32::try_from(child.id()).unwrap());
    let _ = killpg(group, SIGKILL);
    let reaped = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(std::time::Instant::now() < reaped, "{what} was not reaped");
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let empty = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !matches!(killpg(group, None), Err(nix::errno::Errno::ESRCH)) {
        assert!(
            std::time::Instant::now() < empty,
            "{what}'s group is not empty"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    status
}

/// A Codex hook payload for `event` in session `s-1`; `turn` adds `turn_id`.
fn codex_event(event: &str, turn: Option<&str>) -> Value {
    let mut payload = serde_json::json!({"hook_event_name": event, "session_id": "s-1",
        "source": "startup", "cwd": "/src", "model": "gpt-test", "permission_mode": "default",
        "transcript_path": null, "prompt": "go", "last_assistant_message": "done",
        "stop_hook_active": false});
    if let Some(turn) = turn {
        payload["turn_id"] = turn.into();
    }
    payload
}

/// Every row of every table, so a rejected event can be shown to change no
/// stored state at all: activity, usage, consumption, sources, responses and
/// Digest claims included.
fn stored_rows(fixture: &Fixture) -> Vec<String> {
    let connection = rusqlite::Connection::open(fixture.state().join("tmt.db")).unwrap();
    let mut tables = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap();
    let names: Vec<String> = tables
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut rows = Vec::new();
    for name in names {
        let mut select = connection
            .prepare(&format!("SELECT * FROM \"{name}\""))
            .unwrap();
        let columns = select.column_count();
        let mut found = select.query([]).unwrap();
        while let Some(row) = found.next().unwrap() {
            let values: Vec<String> = (0..columns)
                .map(|index| match row.get_ref(index).unwrap() {
                    rusqlite::types::ValueRef::Text(text) => {
                        String::from_utf8_lossy(text).into_owned()
                    }
                    value => format!("{value:?}"),
                })
                .collect();
            rows.push(format!("{name}: {}", values.join(" | ")));
        }
    }
    // Some tables have no rowid; the sorted rows compare as a set.
    rows.sort();
    rows
}

/// The files under the provider and configuration homes, with their bytes:
/// an observer hook writes no settings or hook trust.
fn settings_files(fixture: &Fixture) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(path: &std::path::Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.map(Result::unwrap) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else {
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    for directory in ["home", "xdg", "xdg-config"] {
        walk(&fixture.root.join(directory), &mut files);
    }
    files.sort();
    files
}

fn activity(fixture: &Fixture) -> Value {
    worker(&fixture.json(&["ls", "--json"]))["session"]["activity"].clone()
}

/// An admitted Codex `Interrupt` ends its turn as idle (#3207), activity
/// only. Every interrupt the activity fences refuse leaves all stored state
/// and every settings file unchanged, and the provider always sees exit 0
/// with no output, so it can never read a decision.
#[test]
fn a_codex_interrupt_ends_only_its_own_turn_and_a_refused_one_changes_nothing() {
    register();
    let fixture = Fixture::new("codex-interrupt");
    let record = fixture.approve();
    bind(&fixture, record);
    // The consented usage hook makes an admitted Stop capture its response,
    // so its absence after an interrupt is meaningful.
    fs::create_dir_all(fixture.root.join("home/.codex")).unwrap();
    let observer = tmt_adapters::runtime::hook_protocol::command_entry("codex", "/stable/tmt");
    fs::write(
        fixture.root.join("home/.codex/hooks.json"),
        serde_json::json!({"hooks":{"Stop":[observer]}}).to_string(),
    )
    .unwrap();
    let mut codex = SteppedProvider::start(&fixture, "codex", "codex-steps");
    let captured = |fixture: &Fixture| {
        stored_rows(fixture)
            .into_iter()
            .filter(|row| row.starts_with("recent_responses:"))
            .count()
    };
    let silent = |(status, out, err): (i32, String, String)| {
        assert_eq!((status, out.as_str(), err.as_str()), (0, "", ""));
    };
    let unchanged = |codex: &mut SteppedProvider, payload: Value| {
        let rows = stored_rows(&fixture);
        let files = settings_files(&fixture);
        silent(codex.hook(payload.clone()));
        assert_eq!(stored_rows(&fixture), rows, "{payload}");
        assert_eq!(settings_files(&fixture), files, "{payload}");
    };

    let (status, _, err) = codex.hook(codex_event("SessionStart", None));
    assert_eq!((status, err.as_str()), (0, ""));
    assert_eq!(
        worker(&fixture.json(&["ls", "--json"]))["resume"]["session"],
        "s-1"
    );
    // No admitted start yet: an interrupt cannot end a turn.
    unchanged(&mut codex, codex_event("Interrupt", Some("a")));
    assert_eq!(activity(&fixture)["state"], "unknown");

    let (status, _, err) = codex.hook(codex_event("UserPromptSubmit", Some("a")));
    assert_eq!((status, err.as_str()), (0, ""));
    let working = activity(&fixture);
    assert_eq!(working["state"], "working", "{working:#}");

    // Not this turn's interrupt, or not a main-turn interrupt.
    unchanged(&mut codex, codex_event("Interrupt", None));
    unchanged(&mut codex, codex_event("Interrupt", Some("")));
    let mut subagent = codex_event("Interrupt", Some("a"));
    subagent["agent_id"] = "agent-1".into();
    unchanged(&mut codex, subagent);
    let mut typed = codex_event("Interrupt", Some("a"));
    typed["agent_type"] = Value::Null;
    unchanged(&mut codex, typed);
    let mut other = codex_event("Interrupt", Some("a"));
    other["session_id"] = "s-2".into();
    unchanged(&mut codex, other);
    {
        // A replaced provider process: the same session from another
        // incarnation is not the admitted one.
        let mut replaced = SteppedProvider::start(&fixture, "codex", "replaced-steps");
        unchanged(&mut replaced, codex_event("Interrupt", Some("a")));
    }
    assert_eq!(activity(&fixture), working);

    let responses = stored_rows(&fixture)
        .into_iter()
        .filter(|row| row.starts_with("recent_responses:") || row.starts_with("consumption_"))
        .collect::<Vec<_>>();
    let files = settings_files(&fixture);
    silent(codex.hook(codex_event("Interrupt", Some("a"))));
    let idle = activity(&fixture);
    assert_eq!(idle["state"], "idle", "{idle:#}");
    assert!(
        idle["sinceMs"].as_u64() >= working["sinceMs"].as_u64(),
        "{idle:#}"
    );
    // Activity only: no response, consumption source or sample was written.
    assert_eq!(
        stored_rows(&fixture)
            .into_iter()
            .filter(|row| row.starts_with("recent_responses:") || row.starts_with("consumption_"))
            .collect::<Vec<_>>(),
        responses
    );
    assert_eq!(settings_files(&fixture), files);

    // A repeated interrupt and the turn's own Stop are duplicates: the Stop
    // may read its usage, but changes no activity and captures no response.
    unchanged(&mut codex, codex_event("Interrupt", Some("a")));
    let (status, _, err) = codex.hook(codex_event("Stop", Some("a")));
    assert_eq!((status, err.as_str()), (0, ""));
    assert_eq!(activity(&fixture), idle);
    assert_eq!(captured(&fixture), 0);

    // A late interrupt of the earlier turn cannot end the next one.
    let (status, _, err) = codex.hook(codex_event("UserPromptSubmit", Some("b")));
    assert_eq!((status, err.as_str()), (0, ""));
    assert_eq!(activity(&fixture)["state"], "working");
    unchanged(&mut codex, codex_event("Interrupt", Some("a")));
    assert_eq!(activity(&fixture)["state"], "working");
    silent(codex.hook(codex_event("Interrupt", Some("b"))));
    assert_eq!(activity(&fixture)["state"], "idle");
    assert_eq!(captured(&fixture), 0);

    // Positive control: an admitted Stop of a running turn captures its answer.
    let (status, _, err) = codex.hook(codex_event("UserPromptSubmit", Some("c")));
    assert_eq!((status, err.as_str()), (0, ""));
    let (status, _, err) = codex.hook(codex_event("Stop", Some("c")));
    assert_eq!((status, err.as_str()), (0, ""));
    assert_eq!(activity(&fixture)["state"], "idle");
    assert_eq!(captured(&fixture), 1);
}

/// A person's passthrough text is matched to the Codex turn it started by the prompt the hook
/// reports, and that turn's final message becomes its answer (#4336). Another prompt's turn
/// neither binds nor lends its final.
#[test]
fn a_codex_turn_with_the_typed_prompt_answers_the_passthrough_and_another_turn_does_not() {
    register();
    let fixture = Fixture::new("codex-passthrough-answer");
    let record = fixture.approve();
    bind(&fixture, record);
    fs::create_dir_all(fixture.root.join("home/.codex")).unwrap();
    let observer = tmt_adapters::runtime::hook_protocol::command_entry("codex", "/stable/tmt");
    fs::write(
        fixture.root.join("home/.codex/hooks.json"),
        serde_json::json!({"hooks":{"Stop":[observer]}}).to_string(),
    )
    .unwrap();
    let mut codex = SteppedProvider::start(&fixture, "codex", "codex-steps");
    let (status, _, err) = codex.hook(codex_event("SessionStart", None));
    assert_eq!((status, err.as_str()), (0, ""));
    let agent = worker(&fixture.json(&["ls", "--json"]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let database = fixture.state().join("tmt.db");
    let now = tmt_adapters::request_runtime::wall_time_ms() as i64;
    let connection = rusqlite::Connection::open(&database).unwrap();
    // Two passthroughs typed into the agent's pane; the hook payloads carry the prompt `go`.
    for (request, text) in [("typed-go", "go"), ("typed-other", "something else")] {
        connection
            .execute(
                "INSERT INTO request_attempts (
                    attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
                    inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
                    message_text,message_bytes,message_expires_at_ms
                ) VALUES (?1,?2,'inbox',?3,0,'sent',0,0,?4,?5,?6,?7,?8,?6)",
                rusqlite::params![
                    format!("attempt-{request}"),
                    request,
                    agent,
                    now,
                    now + 3_600_000,
                    now + 604_800_000,
                    text,
                    text.len() as i64
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO request_passthrough(request_id,delivered_at_ms,state) VALUES(?1,?2,'pending')",
                rusqlite::params![request, now],
            )
            .unwrap();
    }
    let capture = |request: &str| -> (String, Option<String>, Option<String>) {
        rusqlite::Connection::open(&database)
            .unwrap()
            .query_row(
                "SELECT state,bound_turn,answer_text FROM request_passthrough WHERE request_id=?",
                [request],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    };

    let (status, _, err) = codex.hook(codex_event("UserPromptSubmit", Some("a")));
    assert_eq!((status, err.as_str()), (0, ""));
    assert_eq!(
        capture("typed-go"),
        ("bound".into(), Some("a".into()), None)
    );
    assert_eq!(
        capture("typed-other").0,
        "pending",
        "a different text is not this turn"
    );
    let (status, _, err) = codex.hook(codex_event("Stop", Some("a")));
    assert_eq!((status, err.as_str()), (0, ""));
    assert_eq!(
        capture("typed-go"),
        ("answered".into(), Some("a".into()), Some("done".into()))
    );

    // A later turn with the same prompt finds nothing pending to bind, and the other
    // passthrough still waits for its own text.
    let (status, _, err) = codex.hook(codex_event("UserPromptSubmit", Some("b")));
    assert_eq!((status, err.as_str()), (0, ""));
    let (status, _, err) = codex.hook(codex_event("Stop", Some("b")));
    assert_eq!((status, err.as_str()), (0, ""));
    assert_eq!(capture("typed-other"), ("pending".into(), None, None));
    assert_eq!(capture("typed-go").2.as_deref(), Some("done"));
}

/// A Digest checklist blocks a Stop and the provider continues the same turn
/// without a new prompt (#4159). The blocked Stop already ended the turn, so
/// the continuation's own Stop (`stop_hook_active`) is the only event that
/// ends its work: it is an admitted end and keeps its answer, while an
/// ordinary repeated Stop still changes nothing.
#[test]
fn a_stop_that_ends_a_digest_continuation_is_admitted_and_keeps_its_response() {
    register();
    let fixture = Fixture::new("codex-continued-stop");
    let record = fixture.approve();
    bind(&fixture, record);
    fs::create_dir_all(fixture.root.join("home/.codex")).unwrap();
    let observer = tmt_adapters::runtime::hook_protocol::command_entry("codex", "/stable/tmt");
    fs::write(
        fixture.root.join("home/.codex/hooks.json"),
        serde_json::json!({"hooks":{"Stop":[observer]}}).to_string(),
    )
    .unwrap();
    let mut codex = SteppedProvider::start(&fixture, "codex", "codex-steps");
    let answers = |fixture: &Fixture| {
        stored_rows(fixture)
            .into_iter()
            .filter(|row| row.starts_with("recent_responses:"))
            .collect::<Vec<_>>()
    };
    let stop = |turn: &str, text: &str, continued: bool| {
        let mut payload = codex_event("Stop", Some(turn));
        payload["last_assistant_message"] = text.into();
        payload["stop_hook_active"] = continued.into();
        payload
    };
    let ok = |(status, _, err): (i32, String, String)| assert_eq!((status, err.as_str()), (0, ""));

    ok(codex.hook(codex_event("SessionStart", None)));
    ok(codex.hook(codex_event("UserPromptSubmit", Some("a"))));
    assert_eq!(activity(&fixture)["state"], "working");

    // The Stop a checklist blocks ends the turn like any other.
    ok(codex.hook(stop("a", "answer before the checklist", false)));
    assert_eq!(activity(&fixture)["state"], "idle");
    assert_eq!(answers(&fixture).len(), 1, "{:#?}", answers(&fixture));

    // The provider continues the turn; its Stop is the continuation's end.
    ok(codex.hook(stop("a", "answer after the checklist", true)));
    let kept = answers(&fixture);
    assert_eq!(kept.len(), 2, "{kept:#?}");
    assert!(
        kept.iter()
            .any(|row| row.contains("answer after the checklist")),
        "{kept:#?}"
    );
    assert_eq!(activity(&fixture)["state"], "idle");

    // A repeated plain Stop and a continuation of another turn change no
    // activity and capture nothing (a refused Stop may still read usage, as in
    // the interrupt scenario).
    let (idle, kept) = (activity(&fixture), answers(&fixture));
    ok(codex.hook(stop("a", "answer before the checklist", false)));
    ok(codex.hook(stop("b", "another turn's stop", true)));
    assert_eq!(activity(&fixture), idle);
    assert_eq!(answers(&fixture), kept);
}

fn observed_model(fixture: &Fixture) -> Value {
    worker(&fixture.json(&["ls", "--json"]))["resume"]["observedModel"].clone()
}

/// `codex_event` reporting `model`.
fn codex_model_event(event: &str, turn: Option<&str>, model: &str) -> Value {
    let mut payload = codex_event(event, turn);
    payload["model"] = model.into();
    payload
}

/// The last observed model (#3207) changes only on an admitted Codex
/// transition, through the real provider hook. A rejected event keeps it
/// (a rejected Stop may still read its usage, so only the observation is
/// compared), a subagent never supplies it, a start sets or clears it, and
/// turn events never change `resume.model`.
#[test]
fn the_observed_model_follows_only_admitted_codex_events() {
    register();
    let fixture = Fixture::new("codex-observed-model");
    let record = fixture.approve();
    bind(&fixture, record);
    let mut codex = SteppedProvider::start(&fixture, "codex", "codex-steps");
    let mut run = |codex: &mut SteppedProvider, payload: Value| {
        // A start prints its identity context; every other event prints nothing.
        let (status, out, err) = codex.hook(payload.clone());
        assert_eq!((status, err.as_str()), (0, ""), "{payload}");
        if payload["hook_event_name"] != "SessionStart" {
            assert_eq!(out, "", "{payload}");
        }
    };
    let kept = |fixture: &Fixture,
                codex: &mut SteppedProvider,
                payload: Value,
                run: &mut dyn FnMut(&mut SteppedProvider, Value)| {
        let before = observed_model(fixture);
        run(codex, payload.clone());
        assert_eq!(observed_model(fixture), before, "{payload}");
    };
    let model = |fixture: &Fixture| observed_model(fixture)["model"].clone();
    let resume_model =
        |fixture: &Fixture| worker(&fixture.json(&["ls", "--json"]))["resume"]["model"].clone();

    run(&mut codex, codex_model_event("SessionStart", None, "m0"));
    assert_eq!(model(&fixture), "m0");
    let started = observed_model(&fixture)["observedAtMs"].as_u64().unwrap();
    assert!(started > 0);
    // No admitted start: the interrupt is rejected and its model is not kept.
    kept(
        &fixture,
        &mut codex,
        codex_model_event("Interrupt", Some("a"), "x"),
        &mut run,
    );

    run(
        &mut codex,
        codex_model_event("UserPromptSubmit", Some("a"), "m1"),
    );
    assert_eq!(model(&fixture), "m1");
    assert!(observed_model(&fixture)["observedAtMs"].as_u64().unwrap() >= started);
    kept(
        &fixture,
        &mut codex,
        codex_model_event("UserPromptSubmit", Some("a"), "dup"),
        &mut run,
    );
    kept(
        &fixture,
        &mut codex,
        codex_model_event("Stop", Some("old"), "late"),
        &mut run,
    );
    let mut other = codex_model_event("Stop", Some("a"), "other");
    other["session_id"] = "s-2".into();
    kept(&fixture, &mut codex, other, &mut run);
    {
        let mut replaced = SteppedProvider::start(&fixture, "codex", "replaced-steps");
        kept(
            &fixture,
            &mut replaced,
            codex_model_event("Stop", Some("a"), "replaced"),
            &mut run,
        );
    }

    // A subagent-marked prompt and Stop keep their activity decoding, but a
    // child model never becomes the main session's.
    let mut child = codex_model_event("UserPromptSubmit", Some("b"), "child");
    child["agent_id"] = "agent-1".into();
    kept(&fixture, &mut codex, child, &mut run);
    assert_eq!(activity(&fixture)["state"], "working");
    let mut child = codex_model_event("Stop", Some("b"), "child");
    child["agent_type"] = Value::Null;
    kept(&fixture, &mut codex, child, &mut run);
    assert_eq!(activity(&fixture)["state"], "idle");

    run(
        &mut codex,
        codex_model_event("UserPromptSubmit", Some("c"), "m2"),
    );
    assert_eq!(model(&fixture), "m2");
    let mut child = codex_model_event("Interrupt", Some("c"), "child");
    child["agent_id"] = Value::Null;
    kept(&fixture, &mut codex, child, &mut run);
    assert_eq!(activity(&fixture)["state"], "working");
    run(&mut codex, codex_model_event("Interrupt", Some("c"), "m3"));
    assert_eq!(model(&fixture), "m3");
    assert_eq!(activity(&fixture)["state"], "idle");
    kept(
        &fixture,
        &mut codex,
        codex_model_event("Stop", Some("c"), "dup"),
        &mut run,
    );
    kept(
        &fixture,
        &mut codex,
        codex_model_event("Interrupt", Some("c"), "dup"),
        &mut run,
    );

    // An admitted event without a valid model keeps the observation and its time.
    kept(
        &fixture,
        &mut codex,
        codex_model_event("UserPromptSubmit", Some("d"), "-x"),
        &mut run,
    );
    assert_eq!(activity(&fixture)["state"], "working");
    assert_eq!(
        resume_model(&fixture),
        "m0",
        "turn events never change resume.model"
    );

    // A start without a model clears the observation and keeps resume.model.
    let mut resumed = codex_event("SessionStart", None);
    resumed.as_object_mut().unwrap().remove("model");
    resumed["source"] = "resume".into();
    run(&mut codex, resumed);
    let listed = worker(&fixture.json(&["ls", "--json"])).clone();
    assert!(
        listed["resume"].get("observedModel").is_none(),
        "{listed:#}"
    );
    assert_eq!(listed["resume"]["model"], "m0");
    let mut cleared = codex_model_event("SessionStart", None, "m4");
    cleared["source"] = "clear".into();
    run(&mut codex, cleared);
    assert_eq!(model(&fixture), "m4");
    assert_eq!(resume_model(&fixture), "m4");
}

/// Claude reports a model only at SessionStart; `identity show` and
/// `ls --json` project the same `resume` document.
#[test]
fn a_claude_start_records_its_observed_model_in_both_projections() {
    register();
    let fixture = Fixture::new("claude-observed-model");
    let record = fixture.approve();
    bind(&fixture, record);
    let output = hook(&fixture, Some("f1"), true, "SessionStart", || {});
    assert!(output.status.success(), "{output:?}");
    let listed = worker(&fixture.json(&["ls", "--json"]))["resume"].clone();
    assert_eq!(
        listed["observedModel"]["model"], "claude-test",
        "{listed:#}"
    );
    assert_eq!(listed["model"], "claude-test");
    let shown = fixture.json(&["identity", "show", "worker", "--json"]);
    assert_eq!(shown["resume"], listed, "{shown:#}");
}

/// The driver operations an approved external Claude hook runs to verify its
/// caller, frozen from unfixed main: the caller's pane, its server, then the
/// pane's snapshot. Nothing after them belongs to the hook.
const VERIFIED_HOOK_CALLS: [&str; 3] = ["caller", "server", "snapshot"];

/// A Claude lifecycle payload for session `s-1`.
fn claude_event(event: &str) -> Value {
    serde_json::json!({"hook_event_name": event, "session_id": "s-1", "source": "startup",
        "reason": "other", "cwd": "/src", "model": "claude-test"})
}

/// The binding's committed session state, read only.
fn session_state(fixture: &Fixture) -> Vec<String> {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.state().join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection
        .query_row(
            "SELECT runtime_state, last_transition, runtime_pid, runtime_start_identity, \
             observed_provider_session_id FROM bindings",
            [],
            |row| {
                (0..5)
                    .map(|index| {
                        Ok(match row.get_ref(index)? {
                            rusqlite::types::ValueRef::Text(text) => {
                                String::from_utf8_lossy(text).into_owned()
                            }
                            rusqlite::types::ValueRef::Integer(value) => value.to_string(),
                            value => format!("{value:?}"),
                        })
                    })
                    .collect()
            },
        )
        .unwrap()
}

/// The stepped provider's live incarnation, so its end can be checked after
/// it is dropped.
fn live_provider(provider: &SteppedProvider) -> Owned {
    live_process(provider.provider_pid())
}

/// The live incarnation of `pid`, so its end can be checked later.
fn live_process(pid: u64) -> Owned {
    let observed = observe_runtime_process(&UnixCommandRunner, pid, deadline()).unwrap();
    let ProcessObservation::Live(incarnation) = observed else {
        panic!("process {pid} is not live: {observed:?}");
    };
    Owned {
        incarnation,
        group: false,
    }
}

/// Every fake driver process the test started has ended: `kill(pid, 0)`
/// finds no process, or `ps` shows the pid now names another program. A
/// process that cannot be inspected, a refused `kill` included, fails the
/// test.
fn assert_drivers_ended(fixture: &Fixture) {
    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
    let pids = fs::read_to_string(fixture.root.join("driver/pids")).unwrap();
    for pid in pids.lines() {
        let pid: i32 = pid.parse().unwrap();
        match kill(Pid::from_raw(pid), None) {
            Err(Errno::ESRCH) => {}
            Ok(()) => {
                let ps = std::process::Command::new("/bin/ps")
                    .args(["-o", "command=", "-p", &pid.to_string()])
                    .output()
                    .unwrap();
                let command = String::from_utf8_lossy(&ps.stdout);
                assert!(
                    ps.status.success() && !command.trim().is_empty(),
                    "driver {pid} could not be inspected: {ps:?}"
                );
                assert!(
                    !command.contains(fixture.driver.to_str().unwrap()),
                    "driver {pid} is still running: {command}"
                );
            }
            Err(error) => panic!("driver {pid} could not be inspected: {error}"),
        }
    }
}

/// An external pane has no badge (#3582): an admitted Claude start and end
/// run only the hook's own verification, and leave a stale marker name as it
/// is. With the driver removed, a hook starts no driver and changes nothing.
#[test]
fn an_external_hook_starts_no_driver_for_the_pane_badge() {
    register();
    let fixture = Fixture::new("hook-no-badge");
    let record = fixture.approve();
    bind(&fixture, record);
    let marker = fixture.root.join("driver/marker");
    let mut claude = SteppedProvider::start(&fixture, "claude", "claude-steps");
    let provider = live_provider(&claude);

    // Each step stales only the marker's name, keeping its ownership IDs, so
    // a refresh would rewrite it. The evidence is read at once, while the
    // provider lives and before any other command runs the driver.
    let mut steps = Vec::new();
    for event in ["SessionStart", "SessionEnd"] {
        let mut stale: Value = serde_json::from_str(&fs::read_to_string(&marker).unwrap()).unwrap();
        stale["name"] = "Stale".into();
        stale["canonicalName"] = "stale".into();
        let stale = stale.to_string();
        fs::write(&marker, &stale).unwrap();
        let before = fixture.calls().len();
        let output = claude.hook(claude_event(event));
        let calls = fixture.calls()[before..].to_vec();
        let after = fs::read_to_string(&marker).unwrap();
        let live = provider.seen();
        steps.push((
            event,
            output,
            calls,
            stale,
            after,
            session_state(&fixture),
            live,
        ));
    }

    // Both hooks were admitted, not refused or timed out.
    for (event, (status, stdout, stderr), ..) in &steps {
        assert_eq!(*status, 0, "{event}: {stdout} {stderr}");
        assert_eq!(stderr, "", "{event}");
        if *event == "SessionStart" {
            let context: Value = serde_json::from_str(stdout).unwrap();
            assert_eq!(
                context["hookSpecificOutput"]["hookEventName"],
                "SessionStart"
            );
        } else {
            assert_eq!(stdout, "", "{event}");
        }
    }
    // One comparison for both branches, so a refresh shows in each.
    let ran: Vec<_> = steps
        .iter()
        .map(|(event, _, calls, stale, after, ..)| (*event, calls.clone(), after == stale))
        .collect();
    assert_eq!(
        ran,
        [
            (
                "SessionStart",
                VERIFIED_HOOK_CALLS.map(String::from).to_vec(),
                true
            ),
            (
                "SessionEnd",
                VERIFIED_HOOK_CALLS.map(String::from).to_vec(),
                true
            ),
        ],
        "markers: {:?}",
        steps
            .iter()
            .map(|step| (&step.3, &step.4))
            .collect::<Vec<_>>()
    );
    let pid = provider.incarnation.pid().to_string();
    let start = provider.incarnation.start_identity().to_owned();
    let committed: Vec<_> = steps
        .iter()
        .map(|(_, _, _, _, _, session, live)| (session.clone(), live))
        .collect();
    assert_eq!(
        committed,
        [
            (
                vec![
                    "running".into(),
                    "started".into(),
                    pid.clone(),
                    start.clone(),
                    "s-1".into()
                ],
                &Seen::Live
            ),
            (
                vec!["unknown".into(), "ended".into(), pid, start, "s-1".into()],
                &Seen::Live
            ),
        ]
    );
    assert_drivers_ended(&fixture);
    drop(claude);
    assert_eq!(provider.seen(), Seen::Ended);

    // Removal leaves remembered sessions; the baseline is taken after it.
    let removed = fixture.tmt(&["driver", "rm", "fake"]);
    assert!(removed.status.success(), "{removed:?}");
    let rows = stored_rows(&fixture);
    let calls = fixture.calls().len();
    let mut unapproved = SteppedProvider::start(&fixture, "claude", "unapproved-steps");
    let unapproved_provider = live_provider(&unapproved);
    let (status, stdout, stderr) = unapproved.hook(claude_event("SessionStart"));
    assert_eq!((status, stdout.as_str(), stderr.as_str()), (0, "", ""));
    assert_eq!(fixture.calls().len(), calls);
    assert_eq!(stored_rows(&fixture), rows);
    drop(unapproved);
    assert_eq!(unapproved_provider.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

#[test]
fn a_default_caller_reaches_a_named_external_recipient() {
    register();
    let fixture = Fixture::delivering("cross-host-name");
    let record = fixture.approve();
    bind(&fixture, record);
    assert_eq!(
        worker(&fixture.json(&["ls", "--json"]))["presence"],
        "active"
    );
    let checked = fixture.tmt(&["check", "worker"]);
    assert!(checked.status.success(), "{checked:?}");
    assert!(String::from_utf8_lossy(&checked.stdout).contains("captured line"));
    fs::write(fixture.root.join("driver/agent"), "ok").unwrap();
    let sent = fixture.tmt(&["talk", "worker", "cross-host", "--detach", "--json"]);
    assert!(sent.status.success(), "{sent:?}");
    assert_eq!(fixture.requests("prompted").len(), 1);
    assert_eq!(fixture.requests("prompted")[0]["paneId"], "fake-1");
    assert!(fixture.requests("typed").is_empty());
    // Normal external focus is owned by that driver; caller tmux locators
    // supply a view without starting or accessing any real tmux server.
    let focused = fixture
        .command_in(None, &["focus", "worker", "--json"])
        .env(
            "TMUX",
            format!("{},123,0", fixture.root.join("tmux/caller.sock").display()),
        )
        .env("TMUX_PANE", "%1")
        .output()
        .unwrap();
    assert!(focused.status.success(), "{focused:?}");
    assert!(fixture.calls().contains(&"focus".to_owned()));
}

#[test]
fn an_unreachable_recipient_is_not_reported_as_an_inactive_name() {
    register();
    let fixture = Fixture::delivering("cross-host-unreachable");
    let record = fixture.approve();
    bind(&fixture, record);
    let before = worker(&fixture.json(&["ls", "--json"]))["id"].clone();
    fs::write(fixture.root.join("driver/unreachable"), "").unwrap();
    for args in [
        vec!["check", "worker", "--json"],
        vec!["talk", "worker", "hello", "--detach", "--json"],
    ] {
        let output = fixture.tmt(&args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let failure: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(failure["error"]["code"], "RECONCILIATION_FAILED");
        assert!(
            !failure["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not active")
        );
    }
    assert!(fixture.requests("prompted").is_empty());
    assert!(fixture.requests("typed").is_empty());
    let unknown = fixture.json(&["ls", "--json"]);
    assert_eq!(worker(&unknown)["id"], before);
    assert_eq!(worker(&unknown)["presence"], "unknown");
    fs::remove_file(fixture.root.join("driver/unreachable")).unwrap();
    assert_eq!(
        worker(&fixture.json(&["ls", "--json"]))["presence"],
        "active"
    );
}

#[test]
fn a_first_missing_recipient_refusal_uses_its_reconciled_lifetime() {
    register();
    for saved in [false, true] {
        for operation in ["check", "focus"] {
            let fixture = Fixture::delivering(&format!("missing-{saved}-{operation}"));
            fixture.approve();
            let mut name = vec!["name", "worker", "--json"];
            if saved {
                name.push("--save");
            }
            let named = fixture.tmt_in(Some("f1"), &name);
            assert!(named.status.success(), "{named:?}");
            let id = worker(&fixture.json(&["ls", "--json"]))["id"]
                .as_str()
                .unwrap()
                .to_owned();
            // No discovery command runs between disappearance and the first
            // refusal: this command itself must reconcile the temporary row.
            fs::write(fixture.root.join("driver/absent"), "").unwrap();
            let target = if operation == "focus" {
                id.as_str()
            } else {
                "worker"
            };
            let args = match operation {
                "focus" => vec!["focus", target, "--json"],
                _ => vec!["check", target, "--capture-only", "--json"],
            };
            let output = fixture.tmt_in(Some("f1"), &args);
            assert_eq!(output.status.code(), Some(3), "{output:?}");
            let failure: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(failure["error"]["code"], "NAME_NOT_FOUND");
            let reason = if saved {
                "is not active"
            } else {
                "was not found"
            };
            assert_eq!(
                failure["error"]["message"],
                format!("Identity '{target}' {reason}.")
            );
            let connection = rusqlite::Connection::open_with_flags(
                fixture.state().join("tmt.db"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let retired: Option<i64> = connection
                .query_row(
                    "SELECT retired_at_ms FROM identities WHERE id=?1",
                    [&id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(retired.is_some(), !saved);
            let bound: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM bindings WHERE identity_id=?1",
                    [&id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(bound, 0);
            assert!(fixture.requests("typed").is_empty());
            assert!(fixture.requests("prompted").is_empty());
            assert!(!fixture.calls().iter().any(|call| call == "focus"));
        }
    }
}
