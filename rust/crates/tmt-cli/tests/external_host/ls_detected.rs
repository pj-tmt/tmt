//! `tmt ls --detected` on an external host (#4471), end to end: the fake
//! driver script stands for the host, a scripted `tmux` on PATH answers or
//! refuses every tmux call, and every store is the fixture's own. No test
//! reaches a real tmux server, Herdr or the user's stores.

use super::discovery::{
    SOCKET, Shell, approvals, approvals_untouched, beside, first_party, other_default,
    registry_file, rows, shim_log, shimmed, tmux_shim,
};
use super::*;
use serde_json::json;

/// `tmt ls` with `args` from outside any pane, through the scripted tmux:
/// its exit code, stdout and stderr.
fn ls(fixture: &Fixture, args: &[&str]) -> (i32, String, String) {
    ls_in(fixture, None, args)
}

fn ls_in(fixture: &Fixture, pane: Option<&str>, args: &[&str]) -> (i32, String, String) {
    let bin = tmux_shim(fixture);
    let mut all = vec!["ls"];
    all.extend_from_slice(args);
    let mut command = fixture.command_in(pane, &all);
    command.env(
        "PATH",
        format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
    );
    let output = command.output().unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// `tmt ls --json` with `args`, which must succeed with one document.
fn listed(fixture: &Fixture, args: &[&str]) -> Value {
    let mut all = args.to_vec();
    all.push("--json");
    let (code, stdout, stderr) = ls(fixture, &all);
    assert_eq!(code, 0, "tmt ls {all:?}: {stdout}{stderr}");
    assert_eq!(stdout.lines().count(), 1, "one document: {stdout}");
    serde_json::from_str(&stdout).unwrap()
}

fn scanned(fixture: &Fixture, args: &[&str]) -> Value {
    let mut all = vec!["--detected", "--host", "fake", "--socket", SOCKET];
    all.extend_from_slice(args);
    listed(fixture, &all)
}

fn panes(document: &Value) -> Vec<&str> {
    document["detected"]["candidates"]
        .as_array()
        .unwrap_or_else(|| panic!("detected candidates: {document:#}"))
        .iter()
        .map(|row| row["pane"].as_str().unwrap())
        .collect()
}

fn names(document: &Value) -> Vec<&str> {
    document["identities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect()
}

/// The driver requests that only a scan makes: the default-server lookup
/// and a snapshot of every pane.
fn scans(fixture: &Fixture) -> usize {
    fs::read_to_string(fixture.root.join("driver/inventory"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("snapshot") && !line.contains(r#""panes":["#))
        .count()
}

/// A dead pane shell: its PID has no start that core can read.
fn dead_pid() -> String {
    Shell::new().pid()
}

/// `worker` bound to fake-1; a second, unbound pane fake-2 whose host calls
/// its agent `agent`.
fn two_panes(name: &str, agent: &str) -> (Fixture, Shell) {
    let fixture = Fixture::new(name);
    let record = fixture.approve();
    let shell = Shell::new();
    beside(&fixture, "two-panes", "");
    beside(&fixture, "pid-2", &shell.pid());
    beside(&fixture, "agentname-2", agent);
    bind(&fixture, record);
    (fixture, shell)
}

/// Case 1: without the flag, `ls` asks the driver nothing more and its
/// document has no `detected` key; with it, the members are the same.
#[test]
fn without_the_flag_ls_scans_nothing_and_keeps_its_document() {
    let (fixture, _shell) = two_panes("ls-detected-baseline", "Ada");
    let calls = fixture.calls().len();
    let plain = listed(&fixture, &[]);
    assert_eq!(
        plain.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["identities"]
    );
    assert_eq!(scans(&fixture), 0);
    let probes = fixture.calls()[calls..].to_vec();
    assert!(!probes.iter().any(|call| call == "server"), "{probes:?}");
    let (code, text, _) = ls(&fixture, &[]);
    assert_eq!(code, 0);
    assert!(!text.contains("DETECTED"), "{text}");
    // The flag adds the scan and never changes the members.
    let flagged = scanned(&fixture, &[]);
    assert_eq!(flagged["identities"], plain["identities"]);
    assert_eq!(scans(&fixture), 1);
    assert!(shim_log(&fixture).is_empty(), "{:?}", shim_log(&fixture));
}

/// Cases 2 and 6: the bound pane is the member once; the other pane is
/// detected once, even when its agent name equals the member's name.
#[test]
fn an_unbound_agent_pane_is_detected_once_beside_its_bound_member() {
    let (fixture, _shell) = two_panes("ls-detected-rows", "worker");
    let document = scanned(&fixture, &[]);
    assert_eq!(names(&document), ["worker"], "{document:#}");
    assert_eq!(panes(&document), ["fake-2"], "{document:#}");
    let row = &document["detected"]["candidates"][0];
    assert_eq!(row["identity"], Value::Null);
    assert_eq!(row["display"]["agentName"], "worker");
    assert_eq!(row["adoptable"], true);
    for field in ["id", "name", "address"] {
        assert!(row.get(field).is_none(), "{field}: {row:#}");
    }
    assert_eq!(document["detected"]["version"], 1);
    assert_eq!(
        document["detected"]["sources"],
        json!([{"host": "fake", "socket": SOCKET, "status": "ok", "reason": null}])
    );
    let (code, text, _) = ls(
        &fixture,
        &["--detected", "--host", "fake", "--socket", SOCKET],
    );
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("\n\nDETECTED 1\n"), "{text}");
    let adds: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("tmt add"))
        .collect();
    assert_eq!(
        adds,
        [format!(
            "hint: tmt add fake-2 <name> --expect {} --socket '{SOCKET}'",
            row["expect"].as_str().unwrap()
        )],
        "{text}"
    );
    assert_eq!(rows(&fixture, "identities"), 1);
}

/// Case 3: a member whose probe fails is Unknown and shows no binding, and
/// the scan sees its pane without a fence. The stored binding still hides
/// the pane, and the binding stays stored.
#[test]
fn an_unknown_members_unfenced_pane_stays_out_of_detected() {
    let fixture = Fixture::new("ls-detected-unknown");
    let record = fixture.approve();
    beside(&fixture, "agentname", "Ada");
    bind(&fixture, record);
    beside(&fixture, "listing-fails", "");
    beside(&fixture, "scan-pid", &dead_pid());
    let document = scanned(&fixture, &[]);
    let member = worker(&document);
    assert_eq!(member["presence"], "unknown", "{document:#}");
    assert!(
        member.get("binding").is_none() || member["binding"].is_null(),
        "{member:#}"
    );
    assert_eq!(panes(&document), Vec::<&str>::new(), "{document:#}");
    assert_eq!(rows(&fixture, "bindings"), 1);
    // The control: on another server the same pane ID is not bound.
    other_default(&fixture);
    let elsewhere = listed(&fixture, &["--detected", "--host", "fake"]);
    assert_eq!(
        elsewhere["detected"]["sources"][0]["socket"], "/tmp/fake-b.sock",
        "{elsewhere:#}"
    );
    assert_eq!(panes(&elsewhere), ["fake-1"], "{elsewhere:#}");
    let row = &elsewhere["detected"]["candidates"][0];
    assert_eq!(row["reason"], "native_incarnation_unknown");
    assert_eq!(row["socket"], Value::Null);
    assert_eq!(rows(&fixture, "bindings"), 1);
}

/// Cases 5 and 7: a lifetime filter hides the member, never its pane; the
/// filter keeps an executable hint and drops a plain shell.
#[test]
fn a_filtered_member_still_hides_its_pane_and_shells_are_not_agents() {
    let fixture = Fixture::new("ls-detected-filtered");
    let record = fixture.approve();
    let shell = Shell::new();
    beside(&fixture, "two-panes", "");
    beside(&fixture, "pid-2", &shell.pid());
    beside(&fixture, "agentname", "Ada");
    bind(&fixture, record);
    // The scan sees fake-1 without a fence, so only its tuple can hide it.
    beside(&fixture, "scan-pid", &dead_pid());
    let document = scanned(&fixture, &["--temp"]);
    assert_eq!(names(&document), Vec::<&str>::new(), "{document:#}");
    // fake-2 runs a plain shell with no agent name.
    assert_eq!(panes(&document), Vec::<&str>::new(), "{document:#}");
    let full = listed(
        &fixture,
        &["--detected", "--host", "fake", "--socket", SOCKET, "--all"],
    );
    assert_eq!(names(&full), ["worker"]);
    assert_eq!(panes(&full), Vec::<&str>::new());
    // An executable-only row is an agent hint; `tmt discover` keeps every row.
    let fixture = Fixture::new("ls-detected-executable");
    fixture.approve();
    beside(&fixture, "command", "/usr/local/bin/codex --yolo");
    let document = scanned(&fixture, &[]);
    assert_eq!(panes(&document), ["fake-1"], "{document:#}");
    assert_eq!(
        document["detected"]["candidates"][0]["runtime"]["driver"],
        "codex"
    );
    beside(&fixture, "command", "zsh");
    assert_eq!(panes(&scanned(&fixture, &[])), Vec::<&str>::new());
    let all = shimmed(
        &fixture,
        false,
        &["discover", "--host", "fake", "--socket", SOCKET, "--json"],
    );
    assert_eq!(all["candidates"].as_array().unwrap().len(), 1);
}

/// Case 9: an unapproved, offline, slow, failing or malformed host never
/// fails `ls`; the members stay and the source says why.
#[test]
fn an_absent_slow_or_failing_host_is_scan_doubt_and_ls_succeeds() {
    let fixture = Fixture::new("ls-detected-unapproved");
    let document = listed(&fixture, &["--detected", "--host", "fake"]);
    assert_eq!(
        document["detected"]["sources"],
        json!([{"host": "fake", "socket": null, "status": "unsupported",
            "reason": "driver_not_approved"}])
    );
    assert!(fixture.calls().is_empty(), "{:?}", fixture.calls());
    for (switch, status) in [
        ("offline", "offline"),
        ("slow-snapshot", "unknown"),
        ("unreachable", "unknown"),
        ("bad-snapshot", "unknown"),
    ] {
        let fixture = Fixture::new(&format!("ls-detected-{switch}"));
        let record = fixture.approve();
        beside(&fixture, "agentname", "Ada");
        bind(&fixture, record);
        let before = listed(&fixture, &[]);
        if switch == "offline" {
            beside(&fixture, "server", r#"{"ok":{"server":null}}"#);
        } else {
            beside(&fixture, switch, "");
        }
        let started = std::time::Instant::now();
        let document = scanned(&fixture, &[]);
        let source = &document["detected"]["sources"][0];
        assert_eq!(source["status"], status, "{switch}: {document:#}");
        assert_eq!(document["detected"]["candidates"], json!([]), "{switch}");
        assert_eq!(names(&document), names(&before), "{switch}");
        if status == "unknown" {
            assert_eq!(document["detected"]["truncated"], true, "{switch}");
        }
        if switch == "slow-snapshot" {
            // The driver got the request (its readiness signal) and the scan
            // stopped at its allowance, not at the driver's 3 s answer.
            assert_eq!(scans(&fixture), 1);
            assert!(started.elapsed() < Duration::from_secs(30));
        }
    }
}

/// Case 12: a member listing that fails keeps its own error and exit code,
/// and no scan runs.
#[test]
fn a_failed_member_listing_keeps_its_error_and_scans_nothing() {
    let fixture = Fixture::new("ls-detected-broken-store");
    fixture.approve();
    fs::create_dir(fixture.state().join("tmt.db")).unwrap();
    let (code, plain, _) = ls(&fixture, &["--json"]);
    let (flagged_code, flagged, _) = ls(
        &fixture,
        &["--detected", "--host", "fake", "--socket", SOCKET, "--json"],
    );
    assert_ne!(code, 0, "{plain}");
    assert_eq!(flagged_code, code);
    let plain: Value = serde_json::from_str(&plain).unwrap();
    let flagged: Value = serde_json::from_str(&flagged).unwrap();
    assert_eq!(flagged, plain);
    assert!(flagged.get("detected").is_none());
    assert_eq!(scans(&fixture), 0);
    assert!(!fixture.calls().iter().any(|call| call == "server"));
}

/// Case 14: implicit selection refuses a gap host and runs nothing; an
/// explicit host and socket select only the scan.
#[test]
fn the_scan_selects_as_discover_does_and_never_selects_the_members() {
    let fixture = Fixture::new("ls-detected-gap");
    let record = fixture.approve();
    approvals(&fixture, &[first_party(record)]);
    let before = fs::read(registry_file(&fixture)).unwrap();
    let (code, stdout, stderr) = ls_in(&fixture, Some("f1"), &["--detected", "--json"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let document: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        document["detected"]["sources"],
        json!([{"host": "fake", "socket": null, "status": "unsupported",
            "reason": "driver_not_shipped"}]),
        "{document:#}"
    );
    approvals_untouched(&fixture, &before);
    assert_eq!(scans(&fixture), 0);
    assert!(shim_log(&fixture).is_empty(), "{:?}", shim_log(&fixture));

    // From inside the driver's pane: the caller's own server, as discover.
    let fixture = Fixture::new("ls-detected-caller");
    fixture.approve();
    beside(&fixture, "agentname", "Ada");
    other_default(&fixture);
    let (code, stdout, stderr) = ls_in(&fixture, Some("f1"), &["--detected", "--json"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let inside: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        inside["detected"]["sources"][0]["socket"], SOCKET,
        "{inside:#}"
    );
    let discovered = shimmed(&fixture, false, &["discover", "--json"]);
    assert_eq!(inside["detected"]["sources"], discovered["sources"]);
    // The scan's host and socket never select the members.
    let (_, plain, _) = ls_in(&fixture, Some("f1"), &["--json"]);
    let plain: Value = serde_json::from_str(&plain).unwrap();
    assert_eq!(inside["identities"], plain["identities"]);
}

/// Case 15: tmux rows come through the same selection and filter, from the
/// scripted runner only: an agent pane, and one pane linked into two
/// sessions, which is listed once and keeps discover's adoptable value.
#[test]
fn tmux_agent_panes_are_detected_through_the_scripted_runner() {
    let fixture = Fixture::new("ls-detected-tmux");
    let socket = fixture.root.join("tmux/nest");
    assert!(socket.starts_with(&fixture.root));
    let pid = std::process::id();
    fs::write(
        tmux_shim(&fixture).join("panes"),
        format!(
            "{s}\t{pid}\t\t%1\t{pid}\tmain\t0\t0\tshell\tcodex\t/src\t\tstart\n\
             {s}\t{pid}\t\t%2\t{pid}\tmain\t1\t0\tshell\tsh\t/src\t\tstart\n\
             {s}\t{pid}\t\t%3\t{pid}\tmain\t2\t0\tshell\tclaude\t/src\t\tstart\n\
             {s}\t{pid}\t\t%3\t{pid}\tother\t0\t0\tshell\tclaude\t/src\t\tstart\n",
            s = socket.display()
        ),
    )
    .unwrap();
    let socket = socket.to_str().unwrap();
    let (code, stdout, stderr) = ls(&fixture, &["--detected", "--socket", socket, "--json"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let document: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        document["detected"]["sources"][0]["host"], "tmux",
        "{document:#}"
    );
    assert_eq!(panes(&document), ["%1", "%3"], "{document:#}");
    let linked = &document["detected"]["candidates"][1];
    assert_eq!(linked["duplicate"], true, "{document:#}");
    let discovered = shimmed(&fixture, false, &["discover", "--socket", socket, "--json"]);
    let original = discovered["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["pane"] == "%3")
        .unwrap();
    assert_eq!(linked["adoptable"], original["adoptable"]);
    for call in shim_log(&fixture) {
        assert!(call.contains(&format!("-S {socket}")), "{call}");
    }
}

/// Case 16: the printed add, with a name in place of `<name>`, binds the
/// pane exactly; after the pane's process changes, the same add refuses and
/// writes nothing.
#[test]
fn the_printed_add_binds_the_pane_and_a_stale_one_refuses() {
    let (fixture, _shell) = two_panes("ls-detected-add", "Ada");
    let add = |fixture: &Fixture| -> Vec<String> {
        let (code, text, _) = ls(
            fixture,
            &["--detected", "--host", "fake", "--socket", SOCKET],
        );
        assert_eq!(code, 0, "{text}");
        let line = text
            .lines()
            .filter_map(|line| line.strip_prefix("hint: "))
            .find(|line| line.starts_with("tmt add "))
            .unwrap_or_else(|| panic!("an add hint: {text}"));
        // Read as a shell reads it; the user types a name for `<name>`.
        let argv = tmt_cli_style::help::ShownExample {
            note: String::new(),
            command: line.replace("<name>", "second"),
        }
        .argv()
        .unwrap();
        assert_eq!(argv[0], "tmt");
        argv[1..].to_vec()
    };
    let stale = add(&fixture);
    // The pane's shell changes: the printed expectation no longer holds.
    let replaced = Shell::new();
    beside(&fixture, "pid-2", &replaced.pid());
    let args: Vec<&str> = stale.iter().map(String::as_str).chain(["--json"]).collect();
    let output = fixture.tmt(&args);
    let refused: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(3), "{refused:#}");
    assert_eq!(refused["error"]["code"], "PANE_CHANGED");
    assert_eq!(rows(&fixture, "identities"), 1);
    assert_eq!(rows(&fixture, "bindings"), 1);
    // A fresh listing prints a fresh add, which binds exactly that pane.
    let fresh = add(&fixture);
    assert_ne!(fresh, stale);
    let args: Vec<&str> = fresh.iter().map(String::as_str).chain(["--json"]).collect();
    let output = fixture.tmt(&args);
    let bound: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{bound:#}");
    assert_eq!(bound["status"], "bound");
    let stored: (String, String, String) =
        rusqlite::Connection::open(fixture.state().join("tmt.db"))
            .unwrap()
            .query_row(
                "SELECT i.name, b.pane_id, s.socket_path FROM bindings b \
                 JOIN identities i ON i.id = b.identity_id \
                 JOIN host_servers s ON s.server_id = b.server_id WHERE b.pane_id = 'fake-2'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
    assert_eq!(stored, ("second".into(), "fake-2".into(), SOCKET.into()));
    // Now a member, the pane is detected no more.
    assert_eq!(panes(&scanned(&fixture, &[])), Vec::<&str>::new());
    // A pane ID no host reads is a usage error.
    let output = fixture.tmt(&[
        "add", "bad pane", "x", "--expect", "00", "--socket", SOCKET, "--json",
    ]);
    let refused: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(refused["error"]["code"], "USAGE_ERROR", "{refused:#}");
}

/// Case 17: framing and quote characters in a host's agent name stay inert
/// text and escaped JSON; a control character fails the source, never a row.
#[test]
fn hostile_agent_names_stay_inert_and_a_control_character_fails_the_source() {
    let fixture = Fixture::new("ls-detected-hostile");
    fixture.approve();
    // JSON string content as the driver sends it: `Ada "q' \ :,{`.
    beside(&fixture, "agentname", r#"Ada \"q' \\ :,{"#);
    let document = scanned(&fixture, &[]);
    assert_eq!(
        document["detected"]["candidates"][0]["display"]["agentName"], r#"Ada "q' \ :,{"#,
        "{document:#}"
    );
    let (code, text, _) = ls(
        &fixture,
        &["--detected", "--host", "fake", "--socket", SOCKET],
    );
    assert_eq!(code, 0);
    assert!(text.contains(r#"Ada "q' \ :,{"#), "{text}");
    beside(&fixture, "agentname", r"Ada\u0007");
    let document = scanned(&fixture, &[]);
    assert_eq!(
        document["detected"]["sources"][0]["status"], "unknown",
        "{document:#}"
    );
    assert_eq!(document["detected"]["candidates"], json!([]));
}

/// Case 8: the scan sees the bound pane with another live shell, so
/// discovery calls it `stale`; the member's tuple still hides it. The
/// listing's own probe sees the bound shell, so the member stays live.
#[test]
fn a_stale_row_on_a_bound_pane_is_shown_as_the_member_only() {
    let fixture = Fixture::new("ls-detected-stale");
    let record = fixture.approve();
    beside(&fixture, "agentname", "Ada");
    bind(&fixture, record);
    let other = Shell::new();
    beside(&fixture, "scan-pid", &other.pid());
    let discovered = shimmed(
        &fixture,
        false,
        &["discover", "--host", "fake", "--socket", SOCKET, "--json"],
    );
    assert_eq!(
        discovered["candidates"][0]["state"], "stale",
        "{discovered:#}"
    );
    assert_eq!(
        discovered["candidates"][0]["reason"],
        "stored_binding_stale"
    );
    let document = scanned(&fixture, &[]);
    assert_eq!(names(&document), ["worker"]);
    assert_eq!(worker(&document)["presence"], "active", "{document:#}");
    assert_eq!(panes(&document), Vec::<&str>::new(), "{document:#}");
}
