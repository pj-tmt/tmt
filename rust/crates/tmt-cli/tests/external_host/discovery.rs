//! Discovery and guarded add on an external host (#4063), end to end: the
//! fake driver script answers `server` and `snapshot`, approvals are read
//! once and never adopted or rewritten, and a guarded add records only the
//! server incarnation it fenced. Pane shells the tests swap in are `sleep`
//! children they own and reap; no other process is signalled.

use super::*;
use serde_json::json;
use std::process::Child;

const SOCKET: &str = "/tmp/fake.sock";

/// A pane shell this test owns: killed and reaped when dropped.
struct Shell(Child);

impl Shell {
    fn new() -> Self {
        Self(
            std::process::Command::new("/bin/sleep")
                .arg("60")
                .stdin(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    fn pid(&self) -> String {
        self.0.id().to_string()
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn beside(fixture: &Fixture, file: &str, text: &str) {
    fs::write(fixture.root.join("driver").join(file), text).unwrap();
}

fn registry_file(fixture: &Fixture) -> PathBuf {
    registry::registry_path(&fixture.state())
}

/// Replaces the registry with exactly `records`, as approvals left it, and
/// removes the lock that approving left, so a test sees any new one.
fn approvals(fixture: &Fixture, records: &[registry::DriverRecord]) {
    fs::write(
        registry_file(fixture),
        serde_json::to_vec(&json!({"version": 1, "drivers": records})).unwrap(),
    )
    .unwrap();
    let _ = fs::remove_file(fixture.state().join("drivers.lock"));
}

/// `record` as a first-party approval: in this test binary, which is not a
/// managed installation, its release ships no such driver.
fn first_party(mut record: registry::DriverRecord) -> registry::DriverRecord {
    record.source = registry::DriverSource::FirstParty;
    record
}

/// An unrelated first-party approval, for another host.
fn unrelated(record: &registry::DriverRecord) -> registry::DriverRecord {
    let mut other = first_party(record.clone());
    other.name = "other".into();
    other.capabilities = serde_json::from_value(json!({"protocols": [1], "kind": "host",
        "name": "other", "version": "0.0.0-test", "ops": ["caller", "server", "snapshot"],
        "paneId": {"prefix": "other-"}, "target": null, "callerEnv": ["OTHER_PANE_ID"]}))
    .unwrap();
    other
}

/// What must stay unchanged across a scan or guarded add: the registry's
/// bytes, and no registry lock.
fn approvals_untouched(fixture: &Fixture, before: &[u8]) {
    assert_eq!(fs::read(registry_file(fixture)).unwrap(), before);
    assert!(!fixture.state().join("drivers.lock").exists());
}

fn discover(fixture: &Fixture, pane: Option<&str>, args: &[&str]) -> Value {
    let mut all = vec!["discover"];
    all.extend_from_slice(args);
    all.push("--json");
    let output = fixture.tmt_in(pane, &all);
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn scan(fixture: &Fixture) -> Value {
    discover(fixture, None, &["--host", "fake", "--socket", SOCKET])
}

fn expect(document: &Value) -> String {
    document["candidates"][0]["expect"]
        .as_str()
        .unwrap_or_else(|| panic!("an adoptable row: {document:#}"))
        .to_owned()
}

/// `tmt add` with `args`, its exit code and JSON.
fn add(fixture: &Fixture, args: &[&str]) -> (i32, Value) {
    let mut all = vec!["add"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--socket", SOCKET, "--json"]);
    let output = fixture.tmt(&all);
    let document = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("JSON from tmt {all:?}: {output:?}"));
    (output.status.code().unwrap(), document)
}

fn publishes(fixture: &Fixture) -> usize {
    fixture
        .calls()
        .iter()
        .filter(|call| *call == "publish")
        .count()
}

fn rows(fixture: &Fixture, table: &str) -> i64 {
    let database = fixture.state().join("tmt.db");
    if !database.exists() {
        return 0;
    }
    rusqlite::Connection::open(database)
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

/// Nothing an adoption makes: no identity, binding, marker or host server.
fn nothing_adopted(fixture: &Fixture) {
    assert_eq!(rows(fixture, "identities"), 0);
    assert_eq!(rows(fixture, "bindings"), 0);
    assert_eq!(rows(fixture, "host_servers"), 0);
    assert_eq!(publishes(fixture), 0);
    assert!(!fixture.root.join("driver/marker").exists());
}

#[test]
fn discovery_lists_an_approved_hosts_panes_with_their_fence() {
    let fixture = Fixture::new("discover-rows");
    fixture.approve();
    beside(&fixture, "agentname", "Ada");
    let document = scan(&fixture);
    assert_eq!(
        document["sources"],
        json!([{"host": "fake", "socket": SOCKET, "status": "ok", "reason": null}])
    );
    assert_eq!(document["truncated"], false);
    let row = &document["candidates"][0];
    assert_eq!(row["host"], "fake");
    assert_eq!(row["pane"], "fake-1");
    assert_eq!(row["target"], "f1");
    assert_eq!(row["session"], Value::Null);
    assert_eq!(row["state"], "unknown");
    assert_eq!(row["adoptable"], true);
    assert_eq!(row["server"]["identity"], Value::Null);
    assert_eq!(row["display"]["agentName"], "Ada");
    let digest = expect(&document);
    // Implicitly, from the driver's own pane: the same host and fence.
    let implicit = discover(&fixture, Some("f1"), &[]);
    assert_eq!(implicit["sources"][0]["host"], "fake");
    assert_eq!(implicit["sources"][0]["status"], "ok");
    assert_eq!(expect(&implicit), digest);
    // A scan creates no store, and its rows stay unknown.
    assert!(!fixture.state().join("tmt.db").exists());
}

#[test]
fn an_unapproved_or_outdated_driver_is_unsupported_and_never_called() {
    let fixture = Fixture::new("discover-unapproved");
    let document = discover(&fixture, None, &["--host", "fake"]);
    assert_eq!(
        document["sources"],
        json!([{"host": "fake", "socket": null, "status": "unsupported",
            "reason": "driver_not_approved"}])
    );
    assert_eq!(document["candidates"], json!([]));
    assert!(fixture.calls().is_empty());
    // A first-party approval its release no longer ships is not adopted.
    let record = fixture.approve();
    approvals(&fixture, &[first_party(record)]);
    let before = fs::read(registry_file(&fixture)).unwrap();
    let document = discover(&fixture, None, &["--host", "fake"]);
    assert_eq!(document["sources"][0]["status"], "unsupported");
    assert_eq!(document["sources"][0]["reason"], "driver_not_shipped");
    approvals_untouched(&fixture, &before);
    assert_eq!(fixture.calls(), ["capabilities"]);
}

/// A `tmux` on PATH that only logs its arguments and fails.
fn tmux_shim(fixture: &Fixture) -> PathBuf {
    let bin = fixture.root.join("shim");
    fs::create_dir_all(&bin).unwrap();
    tmt_test_support::write_executable(
        &bin.join("tmux"),
        b"#!/bin/sh\nprintf 'call %s %s %s %s\\n' \"$1\" \"$2\" \"$3\" \"$4\" >> \"$(dirname \"$0\")/log\"\nexit 1\n",
        0o755,
    )
    .unwrap();
    bin
}

fn shimmed(fixture: &Fixture, nested: bool, args: &[&str]) -> Value {
    let bin = tmux_shim(fixture);
    let mut command = fixture.command_in(Some("f1"), args);
    command.env(
        "PATH",
        format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
    if nested {
        let socket = fixture.root.join("tmux/nest");
        command
            .env("TMUX", format!("{},1,0", socket.display()))
            .env("TMUX_PANE", "%1");
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn shim_log(fixture: &Fixture) -> Vec<String> {
    fs::read_to_string(fixture.root.join("shim/log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn implicit_discovery_in_a_gap_hosts_pane_runs_nothing() {
    let fixture = Fixture::new("discover-gap");
    let record = fixture.approve();
    approvals(&fixture, &[first_party(record)]);
    let before = fs::read(registry_file(&fixture)).unwrap();
    let gap = json!([{"host": "fake", "socket": null, "status": "unsupported",
        "reason": "driver_not_shipped"}]);
    // In the gap host's pane, alone or nested in tmux: no tmux fallback.
    for nested in [false, true] {
        let document = shimmed(&fixture, nested, &["discover", "--json"]);
        assert_eq!(document["sources"], gap, "nested: {nested}");
        assert_eq!(document["candidates"], json!([]));
    }
    approvals_untouched(&fixture, &before);
    assert_eq!(fixture.calls(), ["capabilities"]);
    assert!(shim_log(&fixture).is_empty());
    // An unreadable registry: only tmux is known, and it is not scanned.
    fs::write(registry_file(&fixture), b"{").unwrap();
    let document = shimmed(&fixture, true, &["discover", "--json"]);
    assert_eq!(
        document["sources"],
        json!([{"host": "tmux", "socket": null, "status": "unknown",
            "reason": "driver_registry_unreadable"}])
    );
    assert_eq!(document["truncated"], true);
    assert!(shim_log(&fixture).is_empty());
    assert_eq!(fixture.calls(), ["capabilities"]);
    // An explicit tmux socket is never affected by driver approvals.
    let socket = fixture.root.join("tmux/nest");
    let socket = socket.to_str().unwrap();
    let document = shimmed(&fixture, true, &["discover", "--socket", socket, "--json"]);
    assert_eq!(document["sources"][0]["host"], "tmux");
    assert_eq!(
        shim_log(&fixture),
        [format!("call -u -S {socket} list-panes")]
    );
}

#[test]
fn guarded_add_adopts_once_and_repeats_only_with_a_fresh_expectation() {
    let fixture = Fixture::new("guarded-whole");
    let record = fixture.approve();
    // The binding's host stays admitted beside an unrelated gap.
    approvals(&fixture, &[record.clone(), unrelated(&record)]);
    let before = fs::read(registry_file(&fixture)).unwrap();
    let digest = expect(&scan(&fixture));
    let (code, bound) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
    assert_eq!(code, 0, "{bound:#}");
    assert_eq!(bound["status"], "bound");
    assert_eq!(bound["name"], "worker");
    assert_eq!(bound["pane"], "fake-1");
    assert_eq!(publishes(&fixture), 1);
    assert_eq!(rows(&fixture, "host_servers"), 1);
    // The same pre-bind expectation refuses: the marker changed the fence.
    let (code, refused) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
    assert_eq!(code, 3, "{refused:#}");
    assert_eq!(refused["error"]["code"], "PANE_CHANGED");
    assert_eq!(refused["status"], "refused");
    assert_eq!(publishes(&fixture), 1);
    // A fresh scan knows the binding; its UUID and expectation repeat it.
    let rescan = scan(&fixture);
    let row = &rescan["candidates"][0];
    assert_eq!(row["state"], "known", "{rescan:#}");
    assert_eq!(row["identity"]["name"], "worker");
    assert_eq!(row["server"]["identity"].as_str().map(str::len), Some(36));
    let fresh = expect(&rescan);
    assert_ne!(fresh, digest);
    let id = row["identity"]["id"].as_str().unwrap().to_owned();
    let (code, repeated) = add(&fixture, &["fake-1", "--identity", &id, "--expect", &fresh]);
    assert_eq!(code, 0, "{repeated:#}");
    assert_eq!(repeated["status"], "already_bound");
    assert_eq!(repeated["id"], id.as_str());
    assert_eq!(publishes(&fixture), 1);
    assert_eq!(rows(&fixture, "bindings"), 1);
    assert_eq!(rows(&fixture, "host_servers"), 1);
    approvals_untouched(&fixture, &before);
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| *call == "capabilities")
            .count(),
        1
    );
}

#[test]
fn a_pane_reused_before_the_server_is_recorded_records_nothing() {
    let fixture = Fixture::new("guarded-reuse-before");
    fixture.approve();
    let digest = expect(&scan(&fixture));
    let shell = Shell::new();
    beside(&fixture, "pid", &shell.pid());
    let (code, refused) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
    assert_eq!(code, 3, "{refused:#}");
    assert_eq!(refused["error"]["code"], "PANE_CHANGED");
    nothing_adopted(&fixture);
}

#[test]
fn a_pane_reused_after_the_server_is_recorded_keeps_only_that_server() {
    for admission in ["1", "2"] {
        let fixture = Fixture::new(&format!("guarded-reuse-after-{admission}"));
        fixture.approve();
        let digest = expect(&scan(&fixture));
        // Snapshots: the scan, admission 1, then admissions 2 and 3.
        let shell = Shell::new();
        beside(&fixture, "pid-swapped", &shell.pid());
        fs::remove_file(fixture.root.join("driver/snapshot-count")).unwrap();
        beside(&fixture, "swap-after", admission);
        let (code, refused) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
        assert_eq!(code, 3, "admission {admission}: {refused:#}");
        assert_eq!(refused["error"]["code"], "PANE_CHANGED");
        assert_eq!(rows(&fixture, "identities"), 0);
        assert_eq!(rows(&fixture, "bindings"), 0);
        assert_eq!(publishes(&fixture), 0);
        // Exactly the fenced server's incarnation was recorded.
        let pid = std::process::id();
        let start = observe_start(&UnixCommandRunner, u64::from(pid), deadline())
            .unwrap()
            .unwrap();
        let recorded: (String, i64, String) =
            rusqlite::Connection::open(fixture.state().join("tmt.db"))
                .unwrap()
                .query_row(
                    "SELECT socket_path, server_pid, server_start_time FROM host_servers",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
        assert_eq!(recorded, (SOCKET.to_owned(), i64::from(pid), start));
    }
}

#[test]
fn a_restarted_server_or_changed_ownership_refuses_with_nothing_recorded() {
    let fixture = Fixture::new("guarded-restart");
    fixture.approve();
    let digest = expect(&scan(&fixture));
    let shell = Shell::new();
    beside(
        &fixture,
        "server",
        &format!(
            r#"{{"ok":{{"server":{{"socket":"{SOCKET}","pid":{},"startTime":"driver-says"}}}}}}"#,
            shell.pid()
        ),
    );
    let (code, refused) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
    assert_eq!(code, 3, "{refused:#}");
    assert_eq!(refused["error"]["code"], "PANE_CHANGED");
    nothing_adopted(&fixture);

    let fixture = Fixture::new("guarded-marker");
    fixture.approve();
    let digest = expect(&scan(&fixture));
    let pid = std::process::id();
    beside(
        &fixture,
        "marker",
        &format!(
            r#"{{"name":"x","canonicalName":"x","identityId":"22222222-2222-4222-8222-222222222222","bindingId":"33333333-3333-4333-8333-333333333333","serverId":"11111111-1111-4111-8111-111111111111","panePid":{pid}}}"#
        ),
    );
    let (code, refused) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
    assert_eq!(code, 3, "{refused:#}");
    assert_eq!(refused["error"]["code"], "PANE_CHANGED");
    assert_eq!(rows(&fixture, "identities"), 0);
    assert_eq!(rows(&fixture, "host_servers"), 0);
}

#[test]
fn a_moved_pane_still_binds_and_its_agent_name_is_never_an_identity() {
    let fixture = Fixture::new("guarded-moved");
    fixture.approve();
    // The host calls the agent by an existing identity's name.
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    tmt_core::identity::create_or_resolve(
        &mut storage,
        "taken",
        tmt_core::identity::Lifetime::Saved,
    )
    .unwrap();
    storage.close().unwrap();
    beside(&fixture, "agentname", "taken");
    let document = scan(&fixture);
    assert_eq!(document["candidates"][0]["identity"], Value::Null);
    assert_eq!(document["candidates"][0]["state"], "unknown");
    let digest = expect(&document);
    beside(&fixture, "target", "f9");
    let (code, bound) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
    assert_eq!(code, 0, "{bound:#}");
    assert_eq!(bound["name"], "worker");
    let rescan = scan(&fixture);
    assert_eq!(rescan["candidates"][0]["target"], "f9");
    assert_eq!(rescan["candidates"][0]["display"]["agentName"], "taken");
    assert_eq!(rescan["candidates"][0]["identity"]["name"], "worker");
}

#[test]
fn guarded_add_refuses_conflicts_and_a_changed_or_removed_driver() {
    let fixture = Fixture::new("guarded-conflicts");
    fixture.approve();
    let digest = expect(&scan(&fixture));
    // An existing canonical name refuses before any effect.
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    tmt_core::identity::create_or_resolve(
        &mut storage,
        "worker",
        tmt_core::identity::Lifetime::Saved,
    )
    .unwrap();
    storage.close().unwrap();
    let (code, refused) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
    assert_eq!(code, 5, "{refused:#}");
    assert_eq!(refused["error"]["code"], "NAME_ALREADY_ACTIVE");
    assert_eq!(publishes(&fixture), 0);
    // The pane bound to another identity refuses a second one.
    let (code, _) = add(&fixture, &["fake-1", "first", "--expect", &digest]);
    assert_eq!(code, 0);
    let fresh = expect(&scan(&fixture));
    let (code, refused) = add(&fixture, &["fake-1", "second", "--expect", &fresh]);
    assert_eq!(code, 5, "{refused:#}");
    assert_eq!(refused["error"]["code"], "PANE_ALREADY_BOUND");
    assert_eq!(publishes(&fixture), 1);

    // A driver changed or removed after the scan never runs; nothing is written.
    for removed in [false, true] {
        let fixture = Fixture::new(&format!("guarded-driver-{removed}"));
        fixture.approve();
        let digest = expect(&scan(&fixture));
        if removed {
            fs::remove_file(&fixture.driver).unwrap();
        } else {
            let mut changed = DRIVER.to_owned();
            changed.push_str("# changed\n");
            fs::write(&fixture.driver, changed).unwrap();
        }
        let (code, refused) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
        assert_ne!(code, 0, "{refused:#}");
        assert_eq!(refused["status"], "refused", "{refused:#}");
        assert_eq!(rows(&fixture, "identities"), 0);
        assert_eq!(rows(&fixture, "host_servers"), 0);
    }
}

#[test]
fn a_late_or_oversized_snapshot_is_unknown_and_adopts_nothing() {
    for switch in ["slow-snapshot", "big-snapshot"] {
        let fixture = Fixture::new(switch);
        fixture.approve();
        beside(&fixture, switch, "");
        let document = scan(&fixture);
        assert_eq!(document["sources"][0]["status"], "unknown", "{document:#}");
        assert_eq!(document["truncated"], true);
        assert_eq!(document["candidates"], json!([]));
    }
}

#[test]
fn recording_the_server_for_one_pane_keeps_the_other_expectations_of_its_scan() {
    let fixture = Fixture::new("guarded-two-panes");
    fixture.approve();
    let shell = Shell::new();
    beside(&fixture, "two-panes", "");
    beside(&fixture, "pid-2", &shell.pid());
    let document = scan(&fixture);
    let candidates = document["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2, "{document:#}");
    let first = candidates[0]["expect"].as_str().unwrap().to_owned();
    let second = candidates[1]["expect"].as_str().unwrap().to_owned();
    let (code, bound) = add(&fixture, &["fake-1", "first", "--expect", &first]);
    assert_eq!(code, 0, "{bound:#}");
    assert_eq!(rows(&fixture, "host_servers"), 1);
    let (code, bound) = add(&fixture, &["fake-2", "second", "--expect", &second]);
    assert_eq!(code, 0, "{bound:#}");
    assert_eq!(bound["pane"], "fake-2");
    assert_eq!(rows(&fixture, "bindings"), 2);
    assert_eq!(rows(&fixture, "host_servers"), 1);
}

/// The driver's default server is another one, `/tmp/fake-b.sock`; the
/// caller's pane and every fenced scan are on `SOCKET`.
fn other_default(fixture: &Fixture) {
    let pid = std::process::id();
    beside(
        fixture,
        "server-default",
        &format!(
            r#"{{"ok":{{"server":{{"socket":"/tmp/fake-b.sock","pid":{pid},"startTime":"driver-says"}}}}}}"#
        ),
    );
}

/// The `server` and `snapshot` requests the driver received, each as its
/// operation and the socket it asked about (`null` for the default).
fn inventory(fixture: &Fixture) -> Vec<(String, Value)> {
    fs::read_to_string(fixture.root.join("driver/inventory"))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let (op, request) = line.split_once(' ').unwrap();
            let request: Value = serde_json::from_str(request).unwrap();
            let socket = request
                .pointer("/request/socket")
                .or_else(|| request.get("socket"))
                .cloned()
                .unwrap_or_else(|| panic!("a socket in {request}"));
            (op.to_owned(), socket)
        })
        .collect()
}

#[test]
fn an_explicit_host_scans_the_callers_own_server_before_the_default() {
    let fixture = Fixture::new("discover-caller-server");
    fixture.approve();
    other_default(&fixture);
    // Inside its pane: the caller's server, never the default.
    let inside = discover(&fixture, Some("f1"), &["--host", "fake"]);
    assert_eq!(inside["sources"][0]["socket"], SOCKET, "{inside:#}");
    assert_eq!(inside["candidates"][0]["socket"], SOCKET);
    assert_eq!(
        fixture.calls(),
        ["capabilities", "caller", "server", "snapshot"]
    );
    assert!(
        inventory(&fixture)
            .iter()
            .all(|(_, socket)| socket == SOCKET)
    );
    // Outside any pane: the default server, named in the result and fence.
    let _ = fs::remove_file(fixture.root.join("driver/inventory"));
    let outside = discover(&fixture, None, &["--host", "fake"]);
    assert_eq!(
        outside["sources"][0]["socket"], "/tmp/fake-b.sock",
        "{outside:#}"
    );
    assert_eq!(outside["candidates"][0]["socket"], "/tmp/fake-b.sock");
    assert_eq!(
        inventory(&fixture),
        [
            ("server".to_owned(), Value::Null),
            ("snapshot".to_owned(), json!("/tmp/fake-b.sock"))
        ]
    );
}

#[test]
fn a_guarded_add_on_a_server_other_than_the_default_asks_only_that_server() {
    let fixture = Fixture::new("guarded-other-server");
    fixture.approve();
    other_default(&fixture);
    let digest = expect(&scan(&fixture));
    let _ = fs::remove_file(fixture.root.join("driver/inventory"));
    let (code, bound) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
    assert_eq!(code, 0, "{bound:#}");
    assert_eq!(bound["status"], "bound");
    let asked = inventory(&fixture);
    assert!(asked.iter().any(|(op, _)| op == "snapshot"), "{asked:?}");
    assert!(
        asked.iter().all(|(_, socket)| socket == SOCKET),
        "every request of the add names the fenced server: {asked:?}"
    );
}

#[test]
fn a_guarded_add_with_a_driver_that_cannot_list_panes_is_an_endpoint_failure() {
    let fixture = Fixture::new("guarded-no-inventory");
    fixture.approve();
    let digest = expect(&scan(&fixture));
    // Approved again, now declaring neither `server` nor `snapshot`.
    beside(
        &fixture,
        "capabilities",
        r#"{"ok":{"protocols":[1],"kind":"host","name":"fake","version":"0.0.0-test","ops":["caller","resolve-target","publish","clear"],"paneId":{"prefix":"fake-"},"target":"f{n}","callerEnv":["FAKE_PANE_ID"]}}"#,
    );
    fixture.approve();
    let calls = fixture.calls().len();
    let _ = fs::remove_file(fixture.root.join("driver/inventory"));
    let (code, refused) = add(&fixture, &["fake-1", "worker", "--expect", &digest]);
    assert_eq!(code, 1, "{refused:#}");
    assert_eq!(refused["error"]["code"], "RECONCILIATION_FAILED");
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(message.contains("driver_lacks_inventory"), "{message}");
    assert!(inventory(&fixture).is_empty());
    assert_eq!(fixture.calls().len(), calls, "{:?}", fixture.calls());
    nothing_adopted(&fixture);
}

#[test]
fn a_source_without_inventory_keeps_its_outcome_over_a_broken_store() {
    let fixture = Fixture::new("discover-offline-store");
    fixture.approve();
    // A store that cannot be opened.
    fs::create_dir(fixture.state().join("tmt.db")).unwrap();
    let scan = || discover(&fixture, None, &["--host", "fake", "--socket", SOCKET]);
    let listed = scan();
    assert_eq!(listed["sources"][0]["status"], "unknown", "{listed:#}");
    assert_eq!(listed["candidates"][0]["adoptable"], false);
    beside(&fixture, "server", r#"{"ok":{"server":null}}"#);
    let offline = scan();
    assert_eq!(
        offline["sources"],
        json!([{"host": "fake", "socket": SOCKET, "status": "offline", "reason": null}])
    );
}
