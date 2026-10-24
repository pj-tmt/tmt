//! Real frame admission and private Config files; no host tmux, browser or store.
use super::super::test_support::Fixture;
use super::*;
use crate::board::{
    ActionOutcome, BoardEvent,
    app::workspace::tests::{MEMBER, ROOM, board},
    lane,
};
use crate::config::Pane;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use std::fs;

const ACTOR: &str = "33333333-3333-4333-8333-333333333333";

fn unread_app() -> App {
    let mut app = board();
    app.home_leads.test_results(ACTOR, json!({"items":[{
        "requestId":"answer", "kind":"request", "sender":{"identityId":ACTOR}, "recipientId":MEMBER,
        "final":{"status":"retained","submittedAtMs":100}, "responsePreview":"Exact answer", "recipientAcknowledged":true
    }],"nextBefore":null}));
    app.overview_rows.borrow_mut().clear();
    app
}
fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
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
fn unread_opening_requires_a_drawn_acquired_reply_and_a_successful_core_ack() {
    let fixture = Fixture::new();
    let mut app = unread_app();
    assert_eq!(app.unread_count(MEMBER).total, 1);
    press(&mut app, KeyCode::Tab);
    draw(&app);
    app.unread_drawn();
    assert!(app.unread.writer.offer().is_none(), "loading is not read");
    let key = app.detail_read().unwrap();
    app.apply_message(&key, Ok("Exact answer".into()));
    assert!(draw(&app).contains("Exact answer"));
    app.help = true;
    app.unread_drawn();
    assert!(
        app.unread.writer.offer().is_none(),
        "covered content is not read"
    );
    app.help = false;
    draw(&app);
    {
        let _no_commands = crate::runner::forbid_commands();
        app.unread_drawn();
    }
    assert_eq!(
        app.unread_count(MEMBER).total,
        1,
        "admission is not acknowledgment"
    );
    fs::write(fixture.0.join("fail"), "").unwrap();
    let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
    assert!(result.is_err());
    app.finished_unread(result);
    assert_eq!(app.unread_count(MEMBER).total, 1);
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("Answers not acknowledged")
    );
    assert!(app.unread.writer.offer().is_none(), "no automatic retry");
    fs::remove_file(fixture.0.join("fail")).unwrap();
    press(&mut app, KeyCode::Esc);
    draw(&app);
    app.unread_drawn();
    press(&mut app, KeyCode::Tab);
    draw(&app);
    app.unread_drawn();
    let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
    assert!(result.is_ok());
    app.finished_unread(result);
    assert_eq!(app.unread_count(MEMBER).total, 0);
    assert_eq!(
        fs::read_to_string(fixture.0.join("ops.toml")).unwrap(),
        "# authored preferences\n"
    );
    let calls = fs::read_to_string(fixture.0.join("calls")).unwrap();
    let envelopes = calls
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(
        envelopes.iter().all(
            |envelope| envelope["identity"] == ACTOR && envelope["operation"] == "requests.ack"
        )
    );
    // Repeated unchanged paints cannot re-submit an already admitted revision.
    draw(&app);
    app.unread_drawn();
    assert!(app.unread.writer.offer().is_none());
}

#[test]
fn jumping_never_acknowledges_answers_even_after_success() {
    let fixture = Fixture::new();
    let app = unread_app();
    fs::write(fixture.0.join("fail"), "").unwrap();
    let failed = lane::run(
        &fixture.core(),
        None,
        lane::Job::Jump {
            member: MEMBER.into(),
        },
    );
    assert!(matches!(
        failed,
        BoardEvent::Acted {
            jump: true,
            outcome: Err(_),
            ..
        }
    ));
    fs::remove_file(fixture.0.join("fail")).unwrap();
    let success = lane::run(
        &fixture.core(),
        None,
        lane::Job::Jump {
            member: MEMBER.into(),
        },
    );
    assert!(matches!(
        success,
        BoardEvent::Acted {
            jump: true,
            outcome: Ok(ActionOutcome::Message(_)),
            ..
        }
    ));
    assert_eq!(app.unread_count(MEMBER).total, 1);
    assert!(app.unread.writer.offer().is_none());
    assert_eq!(
        fs::read_to_string(fixture.0.join("calls")).unwrap(),
        "focus\nfocus\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("ops.toml")).unwrap(),
        "# authored preferences\n"
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
    let shown = draw(&app);
    assert!(
        shown.contains("● 0 unread"),
        "the stable info line reports known zero"
    );
    assert!(
        !shown.contains("1 unread"),
        "empty results clear the painted count"
    );
}

#[test]
fn unread_sorts_before_waiting_and_preserves_selected_uuid_after_core_ack() {
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
            idle_over: false,
        },
    );
    let other_target = OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: other.into(),
    };
    snapshot.rows.insert(other_target.clone(), json!({"id":other,"name":"aaa-waiting","state":"working","waitingOnYou":[{"requestId":"waiting","preparedAtMs":50}]}));
    snapshot.preferences.agent_pins.insert(other.into());
    for tab in [
        crate::board::shell::OpsTab::Agents,
        crate::board::shell::OpsTab::Agents,
    ] {
        app.shell.tab = tab;
        app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
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
    let fixture = Fixture::new();
    assert!(app.mark_read(MEMBER));
    let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
    app.finished_unread(result);
    assert_eq!(app.workspace.selected, Some(target.clone()));
    assert_eq!(app.workspace_rows()[app.selected].target, target);
    assert_eq!(app.unread_count(MEMBER).total, 0);
    assert!(app.unread_identities().is_empty());
    let snapshot = app.workspace_snapshot().unwrap();
    assert!(snapshot.roster.squads[0].member_states[other].waiting);
}

#[test]
fn resetting_named_layers_does_not_change_core_acknowledgment() {
    let fixture = Fixture::new();
    let mut app = unread_app();
    app.mark_read(MEMBER);
    let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
    app.finished_unread(result);
    for layer in ["default", "popup"] {
        let directory = fixture.0.join(layer);
        fs::create_dir(&directory).unwrap();
        let path = directory.join("view.json");
        fs::write(&path, "{}").unwrap();
        let _layer =
            crate::board::view_state::Session::open(path.clone(), false, false, true).unwrap();
        assert!(!path.exists());
        assert_eq!(app.unread_count(MEMBER).total, 0);
    }
}

#[test]
fn unread_squad_maximum_label_is_whole_and_tokens_align_with_the_header() {
    for (width, identity_observed) in [103, 148]
        .into_iter()
        .flat_map(|width| [true, false].map(|observed| (width, observed)))
    {
        let mut app = crate::board::app::workspace::tests::capture_case(
            "unread-squads",
            crate::look::Look::default(),
        );
        if !identity_observed {
            // Admitted room membership can outlive an unavailable identity or
            // roster observation; its UUID still has submitted reply evidence.
            let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
            snapshot.roster.members.clear();
            snapshot.roster.observation = crate::board::workspace::OpsObservation::Unavailable;
            snapshot.roster.membership_complete = false;
            for squad in &mut snapshot.roster.squads {
                squad.member_states.clear();
                squad.lead = None;
                squad.observation = crate::board::workspace::OpsObservation::Unavailable;
            }
            snapshot
                .rows
                .retain(|target, _| !matches!(target, OpsTarget::Member { .. }));
        }
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
            .find(|line| line.contains("SQUAD / AGENT") && line.contains("STATE"))
            .unwrap();
        let token_end =
            header[..header.find("TOK 1h").unwrap()].chars().count() + "TOK 1h".len() - 1;
        let row = lines
            .iter()
            .find(|line| {
                line.contains(" product ") && line.contains("50+ unread") && !line.contains(" · ")
            })
            .unwrap();
        assert!(row.contains("● 50+ unread"), "{row}");
        assert_eq!(row.chars().nth(token_end), Some('—'), "{header}\n{row}");
    }
}

#[test]
fn shutdown_reports_unconsumed_ack_failure_without_an_automatic_retry() {
    let fixture = Fixture::new();
    let mut app = unread_app();
    app.mark_read(MEMBER);
    let writer = app.unread.writer.clone();
    fs::write(fixture.0.join("fail"), "").unwrap();
    let (events, _unconsumed) = std::sync::mpsc::channel();
    let lane = lane::Lane::spawn(fixture.core(), None, events);
    lane.submit(lane::Job::AcknowledgeAnswers(writer.offer().unwrap()));
    lane.finish();
    let before = fs::read_to_string(fixture.0.join("calls")).unwrap();
    assert_eq!(
        writer.finish(&fixture.core()).unwrap_err().code,
        "SQUAD_ACK_FAILED"
    );
    assert_eq!(fs::read_to_string(fixture.0.join("calls")).unwrap(), before);
    fs::remove_file(fixture.0.join("fail")).unwrap();
    assert!(
        writer.finish(&fixture.core()).is_err(),
        "repair alone is not a retry"
    );
    app.mark_read(MEMBER);
    writer.finish(&fixture.core()).unwrap();
    assert!(writer.failure().is_none());
}

#[test]
fn moving_onto_an_unread_row_never_marks_it_read() {
    let mut app = unread_app();
    draw(&app);
    let key = app.detail_read().unwrap();
    app.apply_message(&key, Ok("Exact answer".into()));
    assert_eq!(app.unread_count(MEMBER).total, 1);
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    // Move off the row and back onto it; the conversation shows the reply on
    // every frame, but the list still owns the keyboard.
    for code in [KeyCode::Up, KeyCode::Down, KeyCode::Up, KeyCode::Down] {
        press(&mut app, code);
        assert!(!draw(&app).is_empty());
        app.unread_drawn();
    }
    assert!(
        matches!(app.workspace.selected, Some(OpsTarget::Member { .. })),
        "the cursor is back on the unread row"
    );
    assert!(draw(&app).contains("Exact answer"));
    app.unread_drawn();
    assert_eq!(app.unread_count(MEMBER).total, 1, "the count is stable");
    assert!(app.unread.displayed.is_empty(), "nothing is admitted");
    assert!(app.unread.writer.offer().is_none(), "nothing is queued");
    // Only opening the reader reads it.
    press(&mut app, KeyCode::Tab);
    draw(&app);
    app.unread_drawn();
    assert!(
        app.unread.writer.offer().is_some(),
        "painted={:?}; reply={:?}",
        app.conversation.borrow().painted_answers,
        app.conversation_reply().map(|reply| (
            reply.at_ms,
            reply.acquired,
            reply.source,
            reply.key
        ))
    );
}

/// A direct send answered by a captured turn alone: the member's list holds no final reply.
fn turn_app(settled: u64) -> App {
    let mut app = board();
    app.home_leads.test_pages(
        ACTOR,
        json!({"items":[],"nextBefore":null}),
        json!({"items":[{
            "requestId":"direct", "kind":"request", "sender":{"identityId":ACTOR},
            "recipientId":MEMBER, "answeredAtMs":settled, "answer":"turn", "revision":1,"acknowledged":false
        }],"nextBefore":null}),
    );
    app.overview_rows.borrow_mut().clear();
    app
}

/// The conversation lists the captured turn of that send, settled at `settled`.
fn show_turn(app: &mut App, request: &str, settled: u64) {
    let owner = app.conversation_finals().expect("a conversation owner");
    app.apply_sent(
        crate::board::home_leads::SentKey {
            sender: owner.sender,
            agent: owner.agent,
        },
        Ok(vec![json!({
            "requestId":request, "preparedAtMs":settled - 5_000, "text":"please check", "truncated":false,
            "mode":"passthrough",
            "messageState":{"state":"delivered","reason":{"text":"delivered"}},
            "passthrough":{"state":"answered","boundAtMs":settled - 4_000,"settledAtMs":settled,
                "answer":{"text":"the turn output","truncated":false,"provider":"claude","endedMs":settled-1_000}}
        })]),
    );
}

const SETTLED: u64 = 1_000_000;

#[test]
fn a_captured_turn_alone_is_unread_and_reading_it_marks_it_read() {
    let fixture = Fixture::new();
    let mut app = turn_app(SETTLED);
    assert_eq!(
        app.unread_count(MEMBER).total,
        1,
        "the answer is a turn; there is no final reply"
    );
    assert!(
        draw(&app).contains("● 1 unread"),
        "the indicator shows it\n{}",
        draw(&app)
    );
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    draw(&app);
    app.unread_drawn();
    assert!(
        app.unread.writer.offer().is_none(),
        "no capture is shown yet, so nothing is read"
    );
    show_turn(&mut app, "direct", SETTLED);
    assert!(draw(&app).contains("the turn output"));
    app.unread_drawn();
    let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
    assert!(result.is_ok());
    app.finished_unread(result);
    assert_eq!(app.unread_count(MEMBER).total, 0);
    assert_eq!(
        fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
        0
    );
}

#[test]
fn a_hidden_or_unfocused_turn_marks_nothing_and_a_turn_not_on_the_page_marks_nothing() {
    let mut app = turn_app(SETTLED);
    press(&mut app, KeyCode::Tab);
    draw(&app);
    show_turn(&mut app, "direct", SETTLED);
    draw(&app);
    app.help = true;
    app.unread_drawn();
    assert!(app.unread.writer.offer().is_none(), "a modal covers it");
    app.help = false;
    press(&mut app, KeyCode::Esc);
    draw(&app);
    app.unread_drawn();
    assert!(
        app.unread.writer.offer().is_none(),
        "the reader does not hold focus"
    );
    // A capture of a request the answers page does not list is not evidence.
    let mut other = turn_app(SETTLED);
    press(&mut other, KeyCode::Tab);
    draw(&other);
    show_turn(&mut other, "elsewhere", SETTLED + 500_000);
    draw(&other);
    other.unread_drawn();
    let fixture = Fixture::new();
    let result = other
        .unread
        .writer
        .offer()
        .unwrap()
        .publish(&fixture.core())
        .unwrap();
    assert!(result.items.is_empty());
    other.finished_unread(Ok(result));
    assert_eq!(other.unread_count(MEMBER).total, 1);
    assert!(
        !fs::read_to_string(fixture.0.join("calls"))
            .unwrap()
            .contains("requests.ack")
    );
}

mod mark_read {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

    fn paint(app: &App) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(148, 51)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
        terminal.backend().buffer().clone()
    }
    fn mouse(kind: MouseEventKind, area: ratatui::layout::Rect) -> MouseEvent {
        MouseEvent {
            kind,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        }
    }
    fn chip(app: &App) -> Option<ratatui::layout::Rect> {
        app.composer
            .unread_hit
            .borrow()
            .as_ref()
            .map(|hit| hit.area)
    }
    fn text(buffer: &Buffer, area: ratatui::layout::Rect) -> String {
        (area.x..area.right())
            .map(|x| buffer[(x, area.y)].symbol())
            .collect()
    }

    #[test]
    fn clicking_the_unread_count_marks_the_conversation_read_after_the_durable_save() {
        let fixture = Fixture::new();
        let mut app = unread_app();
        let buffer = paint(&app);
        let area = chip(&app).expect("the unread count is a target");
        assert_eq!(text(&buffer, area), "● 1 unread");
        assert!(app.input.is_none(), "the composer is not focused");
        app.mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), area),
            std::time::Instant::now(),
        );
        assert!(
            app.input.is_none(),
            "marking read does not focus the composer"
        );
        assert_eq!(app.unread_count(MEMBER).total, 1, "queued is not saved");
        let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
        assert!(result.is_ok());
        app.finished_unread(result);
        assert_eq!(app.unread_count(MEMBER).total, 0);
        assert_eq!(
            fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
            0
        );
        let buffer = paint(&app);
        assert!(chip(&app).is_none(), "● 0 unread is not a target");
        assert!(
            buffer
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .contains("0 unread")
        );
    }

    #[test]
    fn a_failed_save_keeps_the_count_and_says_so() {
        let fixture = Fixture::new();
        let mut app = unread_app();
        paint(&app);
        let area = chip(&app).unwrap();
        app.mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), area),
            std::time::Instant::now(),
        );
        fs::write(fixture.0.join("fail"), "").unwrap();
        let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
        assert!(result.is_err());
        app.finished_unread(result);
        assert_eq!(app.unread_count(MEMBER).total, 1);
        assert!(
            app.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("Answers not acknowledged"))
        );
    }

    #[test]
    fn a_newer_answer_after_marking_read_is_unread_again() {
        let fixture = Fixture::new();
        let mut app = unread_app();
        assert!(app.mark_read(MEMBER));
        let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
        app.finished_unread(result);
        assert_eq!(app.unread_count(MEMBER).total, 0);
        assert!(!app.mark_read(MEMBER), "nothing left to mark");
        app.home_leads.test_results(ACTOR, json!({"items":[{
            "requestId":"answer", "kind":"request", "sender":{"identityId":ACTOR}, "recipientId":MEMBER,
            "final":{"status":"retained","submittedAtMs":100}, "responsePreview":"Exact answer", "recipientAcknowledged":true
        },{
            "requestId":"later", "kind":"request", "sender":{"identityId":ACTOR}, "recipientId":MEMBER,
            "final":{"status":"retained","submittedAtMs":200}, "responsePreview":"Later answer", "recipientAcknowledged":true
        }],"nextBefore":null}));
        assert_eq!(app.unread_count(MEMBER).total, 1);
    }

    #[test]
    fn hovering_the_count_restyles_it_and_a_zero_count_is_not_a_target() {
        let fixture = Fixture::new();
        let mut app = unread_app();
        let idle = paint(&app);
        let area = chip(&app).unwrap();
        assert!(app.move_pointer(mouse(MouseEventKind::Moved, area)));
        let hot = paint(&app);
        assert_ne!(
            idle[(area.x, area.y)].style(),
            hot[(area.x, area.y)].style()
        );
        assert_eq!(
            text(&idle, area),
            text(&hot, area),
            "only the style changes"
        );
        assert!(
            !app.move_pointer(mouse(MouseEventKind::Moved, area)),
            "unchanged hover repaints nothing"
        );
        app.mark_read(MEMBER);
        let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
        app.finished_unread(result);
        app.move_pointer(mouse(
            MouseEventKind::Moved,
            ratatui::layout::Rect::new(0, 0, 1, 1),
        ));
        paint(&app);
        let zero = chip(&app);
        assert!(zero.is_none(), "● 0 unread has no hit");
        if let Some(area) = zero {
            assert!(!app.move_pointer(mouse(MouseEventKind::Moved, area)));
        }
    }

    #[test]
    fn the_action_and_the_menu_entry_do_the_same_and_nothing_unread_says_so() {
        let fixture = Fixture::new();
        let mut app = unread_app();
        assert!(
            !app.bindings()
                .iter()
                .any(|(_, action)| action.text == "mark-read"),
            "no default key"
        );
        app.context_menu();
        let menu = app.menu.take().unwrap();
        let entry = menu
            .entries
            .iter()
            .find(|entry| entry.label == "Mark read")
            .expect("the menu offers it while something is unread");
        assert!(
            matches!(&entry.choice, crate::board::app::Choice::Action(action)
            if action.verb == crate::action::Verb::Ui(crate::action::OpsUiAction::MarkRead))
        );
        app.mark_read_selected();
        let result = app.unread.writer.offer().unwrap().publish(&fixture.core());
        app.finished_unread(result);
        assert_eq!(app.unread_count(MEMBER).total, 0);
        app.mark_read_selected();
        assert_eq!(app.notice.as_deref(), Some("Nothing unread here."));
        app.context_menu();
        assert!(
            app.menu
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .all(|entry| entry.label != "Mark read")
        );
    }
}

#[test]
fn a_capture_below_the_viewport_is_not_acknowledged() {
    let mut app = turn_app(SETTLED);
    press(&mut app, KeyCode::Tab);
    draw(&app);
    let owner = app.conversation_finals().unwrap();
    app.apply_sent(
        crate::board::home_leads::SentKey { sender: owner.sender, agent: owner.agent },
        Ok(vec![json!({
            "requestId":"direct", "preparedAtMs":SETTLED-5000,
            "text":(0..100).map(|n|format!("outgoing line {n}\n")).collect::<String>(),
            "truncated":false,"mode":"passthrough",
            "messageState":{"state":"delivered","reason":{"text":"delivered"}},
            "passthrough":{"state":"answered","boundAtMs":SETTLED-4000,"settledAtMs":SETTLED,
                "answer":{"text":"the hidden captured answer","truncated":false,"provider":"claude","endedMs":SETTLED}}
        })]),
    );
    draw(&app);
    press(&mut app, KeyCode::Home);
    let frame = draw(&app);
    assert!(frame.contains("outgoing line"));
    assert!(!frame.contains("the hidden captured answer"));
    app.unread_drawn();
    assert!(
        app.unread.writer.offer().is_none(),
        "an offscreen capture was not read"
    );
}

#[test]
fn clipped_exchange_header_is_not_read_but_one_visible_wrapped_body_row_is() {
    use crate::board::app::RowTarget;
    use crate::board::conversation::{exchange, source};
    use ratatui::layout::Rect;
    for width in [80, 150] {
        let mut app = unread_app();
        press(&mut app, KeyCode::Tab);
        draw(&app);
        let key = app.detail_read().unwrap();
        app.apply_message(&key, Ok("one long answer ".repeat(100)));
        draw(&app);
        app.scroll_pane(Pane::Replies, crate::board::scroll::Step::Top);
        let target = RowTarget::Workspace(app.workspace.selected.clone().unwrap());
        let surface = exchange::Surface {
            target,
            interactive: true,
        };
        let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
        terminal
            .draw(|frame| {
                source::begin_frame(&app);
                exchange::render(frame, &app, Rect::new(0, 0, width, 1), &surface);
            })
            .unwrap();
        app.unread_drawn();
        assert!(
            app.unread.writer.offer().is_none(),
            "a header alone is not an answer, width={width}"
        );
        terminal
            .draw(|frame| {
                source::begin_frame(&app);
                exchange::render(frame, &app, Rect::new(0, 0, width, 3), &surface);
            })
            .unwrap();
        let painted = app.conversation.borrow().painted_answers.clone();
        assert_eq!(
            painted.len(),
            1,
            "one clipped body row suffices, width={width}"
        );
        assert_eq!(painted[0].request, "answer");
        assert_eq!(painted[0].revision, Some(1));
        app.unread_drawn();
        assert!(app.unread.writer.offer().is_some());
    }
}

#[test]
fn actor_change_and_invalidated_or_failed_draw_cannot_publish_old_frame_evidence() {
    let mut app = unread_app();
    press(&mut app, KeyCode::Tab);
    draw(&app);
    let key = app.detail_read().unwrap();
    app.apply_message(&key, Ok("Exact answer".into()));
    draw(&app);
    assert!(!app.conversation.borrow().painted_answers.is_empty());
    // A failed draw never calls unread_drawn; invalidating geometry also retires its proof.
    assert!(app.unread.writer.offer().is_none());
    app.conversation.borrow_mut().invalidate_source_geometry();
    app.unread_drawn();
    assert!(app.unread.writer.offer().is_none());
    draw(&app);
    app.view.as_mut().unwrap().me_id = Some("44444444-4444-4444-8444-444444444444".into());
    app.unread_drawn();
    assert!(
        app.unread.writer.offer().is_none(),
        "a frame is bound to its actor"
    );
}
