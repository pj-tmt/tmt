use super::*;
use serde_json::json;

fn failed(code: &str, message: &str) -> Result<Value, SquadError> {
    Err(SquadError::new(code, message))
}

#[test]
fn each_remote_open_result_reads_as_one_status_line() {
    let link = "http://127.0.0.1:50711/remote/";
    let document = |opened: bool, state: &str| {
        Ok(json!({"version": 1, "link": link, "opened": opened, "state": state}))
    };
    assert_eq!(
        outcome(document(true, "opened")),
        Ok(format!("Opened {link} in the browser."))
    );
    assert_eq!(
        outcome(document(false, "failed")),
        Ok(format!("Could not open the browser; open {link} yourself."))
    );
    for state in ["skipped", "no-opener"] {
        assert_eq!(
            outcome(document(false, state)),
            Ok(format!("Open {link} in your browser."))
        );
    }
    assert!(outcome(Ok(json!({"version": 1, "opened": true}))).is_err());
}

#[test]
fn a_missing_or_stopped_remote_names_the_command_and_ops_runs_none() {
    assert_eq!(
        outcome(failed(
            "EXTENSION_NOT_INSTALLED",
            "tmt remote needs the official extension \"remote\", which is not installed."
        )),
        Err("The web UI needs Remote: run tmt extension install remote.".into())
    );
    let stopped = "Remote is not running; start it with tmt remote serve.";
    assert_eq!(
        outcome(failed("REMOTE_NOT_RUNNING", stopped)),
        Err(stopped.into())
    );
    assert_eq!(
        outcome(failed("REMOTE_NOT_RUNNING", "Remote is not running.")),
        Err("Remote is not running: run tmt remote serve --background.".into())
    );
    assert_eq!(
        outcome(failed("REMOTE_OPEN_UNAVAILABLE", "Try again.")),
        Err("Try again.".into())
    );
}

#[test]
fn g_runs_only_tmt_remote_open_through_core_on_the_lane() {
    let dir = std::env::temp_dir().join(format!("tmt-ops-web-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let calls = dir.join("calls");
    let fake = dir.join("tmt");
    crate::test_support::write_ready_executable(
        &fake,
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\necho '{{\"version\":1,\"link\":\"http://127.0.0.1:9/remote/\",\"opened\":true,\"state\":\"opened\"}}'\n",
            calls.display()
        ),
    );
    let event = crate::board::lane::run(
        &Core::at(fake),
        None,
        crate::board::lane::Job::Act(crate::board::app::Request::OpenWeb),
    );
    let crate::board::BoardEvent::Acted { outcome, .. } = event else {
        panic!("the request answers as an action");
    };
    assert!(matches!(
        outcome,
        Ok(crate::board::ActionOutcome::Message(message))
            if message == "Opened http://127.0.0.1:9/remote/ in the browser."
    ));
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap(),
        "remote open --json\n",
        "one read-only call; no install, serve or pairing command"
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
