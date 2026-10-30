use super::*;

fn failed(message: &str) -> Result<String, SquadError> {
    Err(SquadError::new("COMMAND_FAILED", message))
}

/// A fake `tmt` that answers like Remote's `open`: `--json` always skips opening, `--open`
/// opens, and otherwise the captured (non-terminal) output only prints the link.
fn fake_remote(dir: &std::path::Path, running: bool) -> (Core, std::path::PathBuf) {
    let calls = dir.join("calls");
    let fake = dir.join("tmt");
    let link = "http://127.0.0.1:9/remote/";
    let answer = if running {
        format!(
            "case \" $* \" in\n  *' --json '*) echo '{{\"version\":1,\"link\":\"{link}\",\"opened\":false,\"state\":\"skipped\"}}' ;;\n  *' --open '*) echo '✓ Opened {link}' ;;\n  *) echo '✓ Open this link in your browser: {link}' ;;\nesac\n"
        )
    } else {
        "echo 'error: Remote is not running; start it with tmt remote serve' >&2\necho 'hint: tmt remote serve --background' >&2\nexit 1\n".to_owned()
    };
    crate::test_support::write_ready_executable(
        &fake,
        &format!("#!/bin/sh\necho \"$*\" >> '{}'\n{answer}", calls.display()),
    );
    (Core::at(fake), calls)
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tmt-ops-web-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn remotes_own_result_line_is_the_status_line() {
    assert_eq!(
        outcome(Ok("✓ Opened http://127.0.0.1:50711/remote/\n".into())),
        Ok("✓ Opened http://127.0.0.1:50711/remote/".into())
    );
    assert_eq!(
        outcome(Ok(
            "\nwarning: Could not open the browser. Open http://x/remote/ yourself\n".into()
        )),
        Ok("warning: Could not open the browser. Open http://x/remote/ yourself".into())
    );
    assert!(outcome(Ok(" \n".into())).is_err());
}

#[test]
fn a_missing_or_stopped_remote_names_the_command_and_ops_runs_none() {
    assert_eq!(
        outcome(failed(
            "tmt remote needs the official extension \"remote\", which is not installed"
        )),
        Err("The web UI needs Remote: run tmt extension install remote.".into())
    );
    let stopped = "Remote is not running; start it with tmt remote serve";
    assert_eq!(outcome(failed(stopped)), Err(stopped.into()));
    assert_eq!(
        outcome(failed("Remote is not running")),
        Err("Remote is not running: run tmt remote serve --background.".into())
    );
    assert_eq!(outcome(failed("Try again")), Err("Try again".into()));
}

#[test]
fn g_runs_only_tmt_remote_open_with_open_through_core_on_the_lane() {
    let dir = scratch("open");
    let (core, calls) = fake_remote(&dir, true);
    let event = crate::board::lane::run(
        &core,
        None,
        crate::board::lane::Job::Act(crate::board::app::Request::OpenWeb),
    );
    let crate::board::BoardEvent::Acted { outcome, .. } = event else {
        panic!("the request answers as an action");
    };
    assert!(
        matches!(
            &outcome,
            Ok(crate::board::ActionOutcome::Message(message))
                if message == "✓ Opened http://127.0.0.1:9/remote/"
        ),
        "the fake opens only for --open without --json"
    );
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap(),
        "remote open --open\n",
        "one call; no install, serve or pairing command"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_stopped_remote_reports_its_own_error_line() {
    let dir = scratch("stopped");
    let (core, _) = fake_remote(&dir, false);
    assert_eq!(
        open(&core),
        Err("Remote is not running; start it with tmt remote serve".into())
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_web_button_sits_after_snapshots_and_runs_the_same_action_as_g() {
    use crate::action::{OpsUiAction, Verb};
    use crate::board::app::Choice;
    let mut app = crate::board::app::workspace::tests::board();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 36)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let top: String = (0..160)
        .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
        .collect();
    let (snapshots, web) = (top.find("Snapshots").unwrap(), top.find("Web").unwrap());
    assert!(snapshots < web && top[..web].ends_with("g  "), "{top}");
    let footer: String = (0..160)
        .map(|x| terminal.backend().buffer()[(x, 35)].symbol())
        .collect();
    assert!(
        !footer.contains("g web"),
        "the button carries its key; the footer keeps main's hints: {footer}"
    );
    let hit = |action: OpsUiAction| {
        app.workspace_strip_hits
            .borrow()
            .iter()
            .find(|(_, choice)| {
                matches!(choice, Choice::Action(found) if found.verb == Verb::Ui(action))
            })
            .map(|(area, _)| *area)
    };
    let (snapshots, web) = (
        hit(OpsUiAction::WorkspaceSnapshots).unwrap(),
        hit(OpsUiAction::OpenWeb).unwrap(),
    );
    assert_eq!(snapshots.y, web.y);
    assert!(snapshots.right() < web.x);
    assert_eq!(
        app.key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('g'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        )),
        crate::board::app::Effect::Act(crate::board::app::Request::OpenWeb)
    );
}
