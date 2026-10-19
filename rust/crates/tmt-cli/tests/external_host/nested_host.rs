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

/// The hook's output, and whether it took at least the nested proof's 1 s
/// budget: only then can load have refused it.
fn nested_hook(fixture: &Fixture, dir: &std::path::Path) -> (Output, bool) {
    let env = tmux_env(dir);
    let env: Vec<_> = env
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let start = std::time::Instant::now();
    let output = hook_with(fixture, Some("f1"), &env, true, "SessionStart", || {});
    (output, start.elapsed() >= Duration::from_secs(1))
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

/// The nested proof shares one 1 s budget across the pin, a driver process
/// and a tmux process. With the whole suite running, process starts can use
/// it up, and a late answer is doubt, so the hook refuses. That is load, not
/// the behaviour under test. A scenario asserts what load cannot change and
/// returns true when its proof did not finish, which it accepts only when the
/// hook took at least that 1 s (a faster refusal fails at once); then it runs
/// again in a fresh fixture, at most five times, and each such attempt is
/// reported. A fault that refuses every time still fails.
fn within_budget(name: &str, scenario: impl Fn(&Fixture) -> bool) {
    for attempt in 1..=5 {
        let fixture = Fixture::new(&format!("{name}-{attempt}"));
        if !scenario(&fixture) {
            return;
        }
        eprintln!("{name}: attempt {attempt} had its nested proof refused for its budget");
    }
    panic!("{name}: every attempt had its nested proof refused for its budget");
}

fn refused(output: &Output) -> bool {
    output.stderr == HOOK_UNAVAILABLE.as_bytes()
}

/// Herdr in tmux: the external pane shell (this test process) is nearer
/// than the tmux pane, which is either farther up the same ancestry (the
/// shape #3583 measured) or off it (after the client detached). Both record
/// the session on the external pane and its recorded server. A refused
/// attempt (load) must have changed no row.
#[test]
fn a_hook_in_an_external_pane_inside_tmux_records_its_session() {
    register();
    let outer = u64::from(std::os::unix::process::parent_id());
    for (case, tmux_pane) in [("tmux pane above", outer), ("tmux pane elsewhere", 999_999)] {
        within_budget(&format!("nested-admits-{tmux_pane}"), |fixture| {
            let record = fixture.approve();
            bind(fixture, record);
            let servers = host_servers(fixture);
            let rows = stored_rows(fixture);
            let dir = fake_tmux(fixture, Some(pane(tmux_pane, SERVER)));
            let (output, late) = nested_hook(fixture, &dir);
            if refused(&output) {
                assert!(late, "{case}: refused within the budget: {output:?}");
                assert_eq!(
                    stored_rows(fixture),
                    rows,
                    "{case}: a refused hook changed a row"
                );
                return true;
            }
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
            assert_eq!(host_servers(fixture), servers, "{case}");
            assert_one_pane_read(&dir);
            false
        });
    }
}

/// A refused nested hook with `answer` from the fake tmux: no row changed,
/// and the one pane read was made. An attempt whose proof never reached tmux
/// (load) runs again.
fn assert_refused_at_tmux(name: &str, answer: Option<String>) {
    register();
    within_budget(name, |fixture| {
        let record = fixture.approve();
        bind(fixture, record);
        let dir = fake_tmux(fixture, answer.clone());
        let rows = stored_rows(fixture);
        let (output, late) = nested_hook(fixture, &dir);
        assert_eq!(stored_rows(fixture), rows, "a refused hook changed a row");
        assert_hook_refused(fixture, &output);
        if !dir.join("calls").exists() {
            assert!(late, "refused before tmux within the budget: {output:?}");
            return true;
        }
        assert_one_pane_read(&dir);
        false
    });
}

/// Herdr started in a tmux the user has since closed: `TMUX` still names
/// its socket, and the pane read fails. That is doubt, never absence, so
/// the hook refuses although the external pane verifies.
#[test]
fn a_hook_beside_a_stale_tmux_socket_refuses() {
    assert_refused_at_tmux("nested-stale", None);
}

/// The pane read answers from another tmux server than `TMUX` names (one
/// restarted at the same socket path): doubt, so the hook refuses.
#[test]
fn a_hook_whose_tmux_server_restarted_refuses() {
    assert_refused_at_tmux("nested-restarted", Some(pane(999_999, "4243")));
}

/// The external pane's snapshot (and the binding made from it) names
/// another pane process than the shell the pinned ancestry located, so the
/// hook refuses. Only a proof that chose the host reaches the driver's
/// `snapshot`; an attempt without it (load) runs again. The same driver
/// with no tmux variables (one host, no pinned ancestry) is admitted: the
/// provider is below that process too.
#[test]
fn a_nested_pane_that_is_not_the_pinned_shell_refuses() {
    register();
    let outer = std::os::unix::process::parent_id().to_string();
    within_budget("nested-pane-pid", |fixture| {
        let record = fixture.approve();
        fs::write(fixture.root.join("driver/pid"), &outer).unwrap();
        bind(fixture, record);
        let dir = fake_tmux(fixture, Some(pane(999_999, SERVER)));
        let rows = stored_rows(fixture);
        let calls = fixture.calls().len();
        let (output, late) = nested_hook(fixture, &dir);
        // Read before `assert_hook_refused`, whose `ls` takes snapshots too.
        let chosen = fixture.calls()[calls..]
            .iter()
            .any(|call| call == "snapshot");
        assert_eq!(stored_rows(fixture), rows, "a refused hook changed a row");
        assert_hook_refused(fixture, &output);
        assert!(
            chosen || late,
            "refused before the host choice within the budget"
        );
        !chosen
    });

    let control = Fixture::new("nested-pane-pid-control");
    let record = control.approve();
    fs::write(control.root.join("driver/pid"), &outer).unwrap();
    bind(&control, record);
    let output = hook(&control, Some("f1"), true, "SessionStart", || {});
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

/// Caller-session discovery after an ordinary command is scheduled by the
/// caller's pane locators. A nested caller's only pane ID is tmux's (Herdr's
/// caller variable, like this fake host's, holds a target), so with no binding on that
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
