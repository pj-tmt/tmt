//! Real frame admission and private Config files; no host tmux, browser or store.
use super::*;
use crate::{
    board::{
        ActionOutcome, BoardEvent,
        app::workspace::tests::{MEMBER, ROOM, board},
        lane,
    },
    config::Config,
    core::Core,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use std::{fs, path::PathBuf};

const ACTOR: &str = "33333333-3333-4333-8333-333333333333";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ops-unread-opening-{}",
            crate::id::new_v4().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("ops.toml"), "# authored preferences\n").unwrap();
        fs::write(path.join(".ops-paths-v1"), "{}").unwrap();
        let config = json!({"paths":{"global":path.join("config.json")},"resolved":{}}).to_string();
        crate::test_support::write_ready_executable(
            &path.join("tmt"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\ncase \"$1\" in\nconfig) printf '%s\\n' '{config}' ;;\nfocus) if [ -e '{fail}' ]; then echo '{{\"error\":{{\"code\":\"STALE\",\"message\":\"Target changed\"}}}}'; exit 5; fi; echo '{{\"focused\":{{\"pane\":\"%7\"}}}}' ;;\n*) exit 99 ;;\nesac\n",
                log = path.join("calls").display(),
                fail = path.join("fail").display(),
            ),
        );
        Self(path)
    }
    fn core(&self) -> Core {
        Core::at(self.0.join("tmt"))
    }
    fn config(&self) -> Config {
        Config::read(self.0.join("ops.toml")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn unread_app() -> App {
    let mut app = board();
    app.home_leads.test_results(ACTOR, json!({"items":[{
        "requestId":"answer", "kind":"request", "sender":{"identityId":ACTOR}, "recipientId":MEMBER,
        "final":{"status":"retained","submittedAtMs":100}, "responsePreview":"Exact answer", "recipientAcknowledged":true
    }],"nextBefore":null}));
    app.overview_rows.borrow_mut().clear();
    app
}
fn draw(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(148, 51)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn unread_opening_requires_a_drawn_acquired_reply_and_a_successful_durable_save() {
    let fixture = Fixture::new();
    let mut app = unread_app();
    assert_eq!(app.unread_count(MEMBER).total, 1);
    draw(&app);
    app.unread_drawn();
    assert!(
        app.unread.writer.offer().is_none(),
        "loading source is not seen"
    );
    let key = app.detail_read().unwrap();
    app.apply_message(&key, Ok("Exact answer".into()));
    draw(&app);
    app.help = true;
    app.unread_drawn();
    assert!(
        app.unread.writer.offer().is_none(),
        "a modal cannot mark the covered conversation"
    );
    app.help = false;
    assert!(draw(&app).contains("Exact answer"));
    {
        let _no_commands = crate::runner::forbid_commands();
        app.unread_drawn();
    }
    assert_eq!(app.unread_count(MEMBER).total, 1, "queued is not saved");
    let writer = app.unread.writer.offer().unwrap();
    fs::write(fixture.0.join("ops.toml"), "[broken").unwrap();
    let result = writer.publish(&fixture.core());
    assert!(result.is_err());
    app.finished_unread(result);
    assert_eq!(
        app.unread_count(MEMBER).total,
        1,
        "failed publication keeps unread evidence"
    );
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|notice| notice.contains("Reopen the conversation or jump to retry"))
    );
    assert!(app.unread.writer.offer().is_none(), "no automatic retry");
    fs::write(fixture.0.join("ops.toml"), "# authored preferences\n").unwrap();
    app.unread.displayed = None; // Explicit reopen; the ordinary unchanged frame cannot retry.
    draw(&app);
    app.unread_drawn();
    let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
    assert!(result.is_ok());
    app.finished_unread(result);
    assert_eq!(app.unread_count(MEMBER).total, 0);
    assert_eq!(
        fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
        100
    );
    let mut restarted = unread_app();
    restarted
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .read_markers = fixture.config().read_markers().unwrap();
    assert_eq!(
        restarted.unread_count(MEMBER).total,
        0,
        "a new App restores the shared marker"
    );
    assert!(
        !fs::read_to_string(fixture.0.join("calls"))
            .unwrap()
            .contains("ack")
    );
}

#[test]
fn unread_jump_writes_only_after_success_and_keeps_the_admitted_actor() {
    let fixture = Fixture::new();
    let mut app = unread_app();
    let seen = app.unread_jump(MEMBER).unwrap();
    let before = fs::read(fixture.0.join("ops.toml")).unwrap();
    fs::write(fixture.0.join("fail"), "").unwrap();
    let event = lane::run(
        &fixture.core(),
        None,
        lane::Job::Jump {
            member: MEMBER.into(),
            seen: Some(seen.clone()),
            publisher: app.unread.writer.clone(),
        },
    );
    assert!(matches!(
        event,
        BoardEvent::Acted {
            jump: true,
            outcome: Err(_),
            ..
        }
    ));
    assert_eq!(fs::read(fixture.0.join("ops.toml")).unwrap(), before);
    assert_eq!(
        fs::read_to_string(fixture.0.join("calls"))
            .unwrap()
            .lines()
            .count(),
        1,
        "failed jump never loads Config"
    );
    fs::remove_file(fixture.0.join("fail")).unwrap();
    let mismatch = lane::run(
        &fixture.core(),
        None,
        lane::Job::Jump {
            member: "44444444-4444-4444-8444-444444444444".into(),
            seen: Some(seen.clone()),
            publisher: app.unread.writer.clone(),
        },
    );
    assert!(matches!(
        mismatch,
        BoardEvent::Acted {
            jump: true,
            outcome: Ok(ActionOutcome::Message(_)),
            ..
        }
    ));
    assert_eq!(
        fs::read(fixture.0.join("ops.toml")).unwrap(),
        before,
        "a different focus target cannot inherit the read marker"
    );
    app.view.as_mut().unwrap().me_id = Some("44444444-4444-4444-8444-444444444444".into());
    let event = lane::run(
        &fixture.core(),
        None,
        lane::Job::Jump {
            member: MEMBER.into(),
            seen: Some(seen),
            publisher: app.unread.writer.clone(),
        },
    );
    let BoardEvent::Acted {
        jump: true,
        outcome: Ok(ActionOutcome::JumpRead { markers, .. }),
        ..
    } = event
    else {
        panic!("expected successful jump");
    };
    app.finished_unread(markers);
    let saved = fixture.config().read_markers().unwrap();
    assert_eq!(saved.get(ACTOR, MEMBER), 100);
    assert_eq!(
        saved.get("44444444-4444-4444-8444-444444444444", MEMBER),
        0,
        "late completion retains its admitted actor"
    );
}

#[test]
fn unread_global_and_squad_counts_deduplicate_uuid_memberships_and_respect_pool_evidence() {
    let mut app = unread_app();
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut duplicate = snapshot.roster.squads[0].clone();
    duplicate.room_id = "44444444-4444-4444-8444-444444444444".into();
    duplicate.members.push(MEMBER.into());
    snapshot.roster.squads.push(duplicate);
    assert_eq!(app.unread_global().total, 1);
    assert_eq!(app.unread_target(&OpsTarget::Squad(ROOM.into())).total, 1);
    assert_eq!(app.unread_target(&OpsTarget::Pool).total, 0);
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.roster.squads.clear();
    snapshot.roster.membership_complete = false;
    assert_eq!(
        app.unread_target(&OpsTarget::Pool).total,
        0,
        "unknown membership is not pool evidence"
    );
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .membership_complete = true;
    assert_eq!(app.unread_target(&OpsTarget::Pool).total, 1);
}

#[test]
fn unread_projections_require_a_nonempty_page_for_the_current_actor() {
    let mut app = unread_app();
    let targets = [
        OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        },
        OpsTarget::Squad(ROOM.into()),
        OpsTarget::Pool,
    ];
    app.view.as_mut().unwrap().me_id = Some("44444444-4444-4444-8444-444444444444".into());
    assert_eq!(app.unread_global(), Count::default());
    assert!(app.unread_identities().is_empty());
    for target in &targets {
        assert_eq!(app.unread_target(target), Count::default());
    }
    assert!(app.unread_jump(MEMBER).is_none());
    app.view.as_mut().unwrap().me_id = Some(ACTOR.into());
    assert_eq!(
        app.unread_global().total,
        1,
        "the held page still belongs to its actor"
    );
    app.home_leads
        .test_results(ACTOR, json!({"items": [], "nextBefore": null}));
    assert_eq!(app.unread_global(), Count::default());
    assert!(app.unread_identities().is_empty());
    for target in &targets {
        assert_eq!(app.unread_target(target), Count::default());
    }
    assert!(app.unread_jump(MEMBER).is_none());
    assert!(
        !draw(&app).contains(" unread"),
        "empty results restore the source states"
    );
}

#[test]
fn unread_sorts_before_waiting_and_preserves_selected_uuid_when_the_marker_clears() {
    let mut app = unread_app();
    assert_eq!(
        app.unread_identities(),
        std::collections::BTreeSet::from([MEMBER])
    );
    let target = app.workspace.selected.clone().unwrap();
    let other = "44444444-4444-4444-8444-444444444444";
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut member = snapshot.roster.members[MEMBER].clone();
    member.id = other.into();
    member.name = "aaa-waiting".into();
    member.waiting = true;
    snapshot.roster.members.insert(other.into(), member);
    snapshot.roster.squads[0].members.push(other.into());
    snapshot.roster.squads[0].member_states.insert(
        other.into(),
        crate::board::workspace::OpsMembershipState {
            state: Some("working".into()),
            waiting: true,
            rank: None,
        },
    );
    let other_target = OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: other.into(),
    };
    snapshot.rows.insert(other_target.clone(), json!({"id":other,"name":"aaa-waiting","state":"working","waitingOnYou":[{"requestId":"waiting","preparedAtMs":50}]}));
    snapshot.preferences.agent_pins.insert(other.into());
    for tab in [
        crate::board::shell::OpsTab::Overview,
        crate::board::shell::OpsTab::Agents,
    ] {
        app.shell.tab = tab;
        app.overview_view = crate::board::overview::monitor::View::Agents;
        app.overview_rows.borrow_mut().clear();
        app.workspace_reconcile();
        let rows = app.workspace_rows();
        let unread_index = rows.iter().position(|row| row.target == target).unwrap();
        let waiting_index = rows
            .iter()
            .position(|row| row.target == other_target)
            .unwrap();
        assert!(unread_index < waiting_index);
        assert_eq!(
            rows[unread_index].state.as_deref(),
            Some("working"),
            "raw state is unchanged"
        );
        assert_eq!(app.workspace.selected, Some(target.clone()));
    }
    let mut markers = crate::config::ReadMarkers::default();
    markers.advance(ACTOR, MEMBER, 100).unwrap();
    app.finished_unread(Ok(markers));
    assert_eq!(app.workspace.selected, Some(target.clone()));
    assert_eq!(app.workspace_rows()[app.selected].target, target);
    assert_eq!(app.unread_count(MEMBER).total, 0);
    assert!(app.unread_identities().is_empty());
    let snapshot = app.workspace_snapshot().unwrap();
    assert!(snapshot.roster.squads[0].member_states[other].waiting);
}

#[test]
fn unread_markers_survive_resetting_each_named_layer() {
    let fixture = Fixture::new();
    let seen = unread_app().unread_jump(MEMBER).unwrap();
    super::super::publish::Writer::default()
        .publish_seen(&seen, &fixture.core())
        .unwrap();
    for layer in ["default", "popup"] {
        let directory = fixture.0.join(layer);
        fs::create_dir(&directory).unwrap();
        let path = directory.join("view.json");
        fs::write(&path, "{}").unwrap();
        let _layer =
            crate::board::view_state::Session::open(path.clone(), false, false, true).unwrap();
        assert!(!path.exists(), "reset actually removes the per-layer hints");
        assert_eq!(
            fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
            100
        );
    }
}

#[test]
fn unread_squad_maximum_label_is_whole_and_tokens_align_with_the_header() {
    for width in [103, 148] {
        let app = crate::board::app::workspace::tests::capture_case(
            "unread-squads",
            crate::look::Look::default(),
        );
        let mut terminal = Terminal::new(TestBackend::new(width, 51)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let lines: Vec<String> = terminal
            .backend()
            .buffer()
            .content
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect();
        let header = lines
            .iter()
            .find(|line| line.contains("SQUAD") && line.contains("TEAM"))
            .unwrap();
        let token_end = header.find("TOK 1h").unwrap() + "TOK 1h".len() - 1;
        let row = lines
            .iter()
            .find(|line| line.contains("product") && line.contains("50+ unread"))
            .unwrap();
        assert!(row.contains("● 50+ unread"), "{row}");
        assert_eq!(row.chars().nth(token_end), Some('—'), "{header}\n{row}");
    }
}

#[test]
fn unread_shutdown_reports_unconsumed_lane_failure_and_drains_only_an_explicit_retry() {
    let fixture = Fixture::new();
    let app = unread_app();
    let writer = app.unread.writer.clone();
    let seen = app.unread_jump(MEMBER).unwrap();
    writer.seen(&seen);
    fs::write(fixture.0.join("ops.toml"), "[broken").unwrap();
    let (events, _unconsumed) = std::sync::mpsc::channel();
    let lane = lane::Lane::spawn(fixture.core(), None, events);
    lane.submit(lane::Job::ReadMarkers(writer.offer().unwrap()));
    lane.finish();
    // The UI quit before receiving ReadMarkersPublished. Shutdown still sees it.
    let before = fs::read_to_string(fixture.0.join("calls")).unwrap();
    let error = writer.finish(&fixture.core()).unwrap_err();
    assert_eq!(error.code, "SQUAD_CONFIG_WRITE_FAILED");
    assert!(
        error
            .message
            .contains("Reopen the conversation or jump to retry")
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("calls")).unwrap(),
        before,
        "failure alone cannot cause an automatic shutdown retry"
    );
    assert_eq!(app.unread_count(MEMBER).total, 1);
    fs::write(fixture.0.join("ops.toml"), "# authored preferences\n").unwrap();
    assert!(
        writer.finish(&fixture.core()).is_err(),
        "repairing the file alone is not a retry"
    );
    writer.seen(&seen);
    writer.finish(&fixture.core()).unwrap();
    assert!(writer.failure().is_none());
    assert_eq!(
        fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
        100
    );
}

#[test]
fn unread_popup_paints_a_failed_marker_save_after_successful_focus_instead_of_closing() {
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::{
        cell::RefCell,
        sync::{atomic::AtomicUsize, mpsc::channel},
    };
    let fixture = Fixture::new();
    let mut app = unread_app();
    app.popup = true;
    fs::write(fixture.0.join("ops.toml"), "[broken").unwrap();
    let event = lane::run(
        &fixture.core(),
        None,
        lane::Job::Jump {
            member: MEMBER.into(),
            seen: app.unread_jump(MEMBER),
            publisher: app.unread.writer.clone(),
        },
    );
    assert!(matches!(
        &event,
        BoardEvent::Acted {
            outcome: Ok(ActionOutcome::JumpRead {
                markers: Err(_),
                ..
            }),
            ..
        }
    ));
    let pending = RefCell::new(Some(event));
    let (events, input) = channel();
    events
        .send(BoardEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        ))))
        .unwrap();
    let mut notices = Vec::new();
    let ended = crate::board::tests::session(
        &mut app,
        &AtomicUsize::new(0),
        &input,
        |_, _, _, _| {},
        |_, _| {},
        |_| {},
        |job| {
            assert!(matches!(job, lane::Job::Jump { .. }));
            events.send(pending.borrow_mut().take().unwrap()).unwrap();
            events.send(BoardEvent::InputClosed).unwrap();
        },
        |app| {
            notices.push(app.notice.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        ended,
        Some(signal_hook::consts::SIGHUP),
        "only input closure ends this popup"
    );
    assert!(
        notices
            .iter()
            .flatten()
            .any(|text| text.contains("Read marker not saved"))
    );
    assert_eq!(app.unread_count(MEMBER).total, 1);
    assert!(app.unread.writer.finish(&fixture.core()).is_err());
}
