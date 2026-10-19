//! A provider hook in a nested host (#3583): tmux variables beside the fake
//! external host's, as in Herdr inside tmux. One pinned ancestry orders both
//! panes, and the nearest located pane owns the caller; any doubt refuses.
//!
//! No tmux server runs: a fake `tmux`, first on `PATH`, logs each call and
//! answers the one pane read with a scripted line, or fails as a stale
//! socket does. The tmux owner's record needs the real tmux reads after the
//! choice, so the adapters tests and the E2E suite prove it.
use super::*;

const SOCKET: &str = "/tmp/tmt-outer.sock";
const SERVER: &str = "4242";
const SEPARATOR: &str = "__TMT_CALLER_PANE_4f1c__";

const TMUX: &str = r#"#!/bin/sh
d=$(dirname "$0")
printf '%s\n' "$*" >> "$d/calls"
if [ -f "$d/answer" ]; then cat "$d/answer"; exit 0; fi
echo "no server running on $3" >&2
exit 1
"#;

/// The fake tmux's directory, for the front of `PATH`. With `answer`, the
/// pane read prints it; without, every call fails as a stale socket does.
fn fake_tmux(fixture: &Fixture, answer: Option<String>) -> PathBuf {
    let dir = fixture.root.join("fake-tmux");
    fs::create_dir_all(&dir).unwrap();
    let tmux = dir.join("tmux");
    fs::write(&tmux, TMUX).unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
    if let Some(answer) = answer {
        fs::write(dir.join("answer"), answer).unwrap();
    }
    dir
}

/// Pane `%1` on the outer server, whose shell is `pane_pid`.
fn pane(pane_pid: u64, server: &str) -> String {
    ["%1", SOCKET, server, &pane_pid.to_string(), "0"].join(SEPARATOR) + "\n"
}

/// The tmux variables for pane `%1` of the outer server, and `PATH` with the
/// fake tmux first.
fn tmux_env(dir: &std::path::Path) -> [(&'static str, String); 3] {
    [
        ("TMUX", format!("{SOCKET},{SERVER},0")),
        ("TMUX_PANE", "%1".into()),
        (
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", dir.display()),
        ),
    ]
}

fn nested_hook(fixture: &Fixture, dir: &std::path::Path) -> Output {
    let env = tmux_env(dir);
    let env: Vec<_> = env
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    hook_with(fixture, Some("f1"), &env, true, "SessionStart", || {})
}

/// The fake tmux's calls: only the one pane read on the outer socket.
fn assert_one_pane_read(dir: &std::path::Path) {
    let calls = fs::read_to_string(dir.join("calls")).unwrap_or_default();
    let format = [
        "#{pane_id}",
        "#{socket_path}",
        "#{pid}",
        "#{pane_pid}",
        "#{pane_dead}",
    ]
    .join(SEPARATOR);
    assert_eq!(
        calls,
        format!("-u -S {SOCKET} display-message -p -t %1 {format}\n")
    );
}

/// Herdr in tmux: the external pane shell (this test process) is nearer
/// than the tmux pane, which is either farther up the same ancestry (the
/// shape #3583 measured) or off it (after the client detached). Both record
/// the session on the external pane and its recorded server.
#[test]
fn a_hook_in_an_external_pane_inside_tmux_records_its_session() {
    register();
    let outer = u64::from(std::os::unix::process::parent_id());
    for (case, tmux_pane) in [("tmux pane above", outer), ("tmux pane elsewhere", 999_999)] {
        let fixture = Fixture::new(&format!("nested-admits-{tmux_pane}"));
        let record = fixture.approve();
        bind(&fixture, record);
        let servers = host_servers(&fixture);
        let dir = fake_tmux(&fixture, Some(pane(tmux_pane, SERVER)));
        let output = nested_hook(&fixture, &dir);
        assert!(output.status.success(), "{case}: {output:?}");
        assert!(output.stderr.is_empty(), "{case}: {output:?}");
        let context: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            context["hookSpecificOutput"]["hookEventName"], "SessionStart",
            "{case}"
        );
        let listed = fixture.json(&["ls", "--json"]);
        let row = worker(&listed);
        assert_eq!(row["resume"]["session"], "s-1", "{case}: {row:#}");
        assert_eq!(host_servers(&fixture), servers, "{case}");
        assert_one_pane_read(&dir);
    }
}

/// Herdr started in a tmux the user has since closed: `TMUX` still names
/// its socket, and the pane read fails. That is doubt, never absence, so
/// the hook refuses although the external pane verifies.
#[test]
fn a_hook_beside_a_stale_tmux_socket_refuses() {
    register();
    let fixture = Fixture::new("nested-stale");
    let record = fixture.approve();
    bind(&fixture, record);
    let dir = fake_tmux(&fixture, None);
    let rows = stored_rows(&fixture);
    let output = nested_hook(&fixture, &dir);
    assert_eq!(stored_rows(&fixture), rows, "a refused hook changed a row");
    assert_hook_refused(&fixture, &output);
    assert_one_pane_read(&dir);
}

/// The pane read answers from another tmux server than `TMUX` names (one
/// restarted at the same socket path): doubt, so the hook refuses.
#[test]
fn a_hook_whose_tmux_server_restarted_refuses() {
    register();
    let fixture = Fixture::new("nested-restarted");
    let record = fixture.approve();
    bind(&fixture, record);
    let dir = fake_tmux(&fixture, Some(pane(999_999, "4243")));
    let rows = stored_rows(&fixture);
    let output = nested_hook(&fixture, &dir);
    assert_eq!(stored_rows(&fixture), rows, "a refused hook changed a row");
    assert_hook_refused(&fixture, &output);
    assert_one_pane_read(&dir);
}

/// The external pane's snapshot (and the binding made from it) names
/// another pane process than the shell the pinned ancestry located, so the
/// hook refuses. The same driver with no tmux variables (one host, no
/// pinned ancestry) is admitted: the provider is below that process too.
#[test]
fn a_nested_pane_that_is_not_the_pinned_shell_refuses() {
    register();
    let outer = std::os::unix::process::parent_id().to_string();
    let fixture = Fixture::new("nested-pane-pid");
    let record = fixture.approve();
    fs::write(fixture.root.join("driver/pid"), &outer).unwrap();
    bind(&fixture, record);
    let dir = fake_tmux(&fixture, Some(pane(999_999, SERVER)));
    let rows = stored_rows(&fixture);
    let output = nested_hook(&fixture, &dir);
    assert_eq!(stored_rows(&fixture), rows, "a refused hook changed a row");
    assert_hook_refused(&fixture, &output);

    let control = Fixture::new("nested-pane-pid-control");
    let record = control.approve();
    fs::write(control.root.join("driver/pid"), &outer).unwrap();
    bind(&control, record);
    let output = hook(&control, Some("f1"), true, "SessionStart", || {});
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

/// Caller-session discovery after an ordinary command is scheduled by the
/// caller's pane locators. A nested caller's only pane ID is tmux's (an
/// external host's variable holds a target), so with no binding on that
/// tmux pane the worker does not run: nothing is asked or recorded. Without
/// the tmux variables the same command records the session
/// (`a_caller_in_an_external_pane_records_its_session_after_a_command`).
#[test]
fn a_nested_caller_without_a_tmux_binding_runs_no_caller_session_worker() {
    register();
    let fixture = Fixture::new("nested-caller-session");
    let record = fixture.approve();
    bind(&fixture, record);
    let dir = fake_tmux(&fixture, Some(pane(999_999, SERVER)));
    let mut claude = SteppedProvider::start(&fixture, "claude", "claude-steps");
    let pid = live_provider(&claude).incarnation.pid().to_string();
    let session = "6a0f3c1e-5b2d-4e8a-9c7f-1d2e3f4a5b6c";
    let rows = stored_rows(&fixture);
    let calls = fixture.calls().len();
    let tmux = tmux_env(&dir);
    let mut env: Vec<_> = tmux
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    env.extend([("CLAUDE_CODE_SESSION_ID", session), ("CLAUDE_PID", &pid)]);
    let (status, _, stderr) = claude.command(&env, &["config", "show", "--json"]);
    assert_eq!((status, stderr.as_str()), (0, ""));
    assert_eq!(fixture.calls()[calls..], [] as [String; 0]);
    assert_eq!(stored_rows(&fixture), rows);
    assert!(!dir.join("calls").exists(), "the precheck ran no tmux");
    drop(claude);
    assert_drivers_ended(&fixture);
}
