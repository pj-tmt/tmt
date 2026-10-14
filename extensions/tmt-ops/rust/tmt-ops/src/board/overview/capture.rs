//! Explicit actual-renderer evidence for WP3's new read-only modal surfaces.
use crate::board::{
    app::{Effect, Request},
    cronboard::{self, CronRequest},
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

#[test]
#[ignore = "explicit WP3 detail and timer-review evidence export"]
fn capture_monitor_modal_packet() {
    let output = std::env::var("TMT_MONITOR_MODAL_CAPTURE").expect("task-owned evidence path");
    let mut frames = Vec::new();
    for scene in [
        "details",
        "details-link-end",
        "timer-pending",
        "timer-ready",
        "timer-failed",
    ] {
        for width in [160, 140, 100, 80, 60] {
            for height in [36, 8] {
                for (base, depth) in [
                    ("tmt", tmt_cli_style::Depth::TrueColor),
                    ("tmt-light", tmt_cli_style::Depth::TrueColor),
                    ("terminal", tmt_cli_style::Depth::Ansi16),
                    ("tmt", tmt_cli_style::Depth::None),
                ] {
                    let look = crate::look::Look {
                        theme: tmt_cli_style::Theme::new(
                            tmt_cli_style::theme::Base::parse(base).unwrap(),
                        ),
                        depth,
                    };
                    let mut app =
                        crate::board::app::workspace::tests::capture_case("overview", look);
                    if scene.starts_with("details") {
                        let target = app.workspace.selected.clone().unwrap();
                        app.view
                            .as_mut()
                            .unwrap()
                            .home
                            .as_mut()
                            .unwrap()
                            .workspace
                            .rows
                            .get_mut(&target)
                            .unwrap()["fields"]["pr_link"] =
                            json!("https://例.test/a/very/long/path?literal=$(id)&value=unchanged");
                        assert_eq!(app.overview_details(), Effect::None);
                        assert!(app.monitor_details.is_some());
                    } else {
                        app.cron.replace(Ok(cronboard::test_cron(
                            vec![],
                            tmt_ops::cron::ClockStatus::NoClock,
                        )));
                        let request = CronRequest::Add {
                            actor: app.cron.cron.as_ref().unwrap().actor.clone().unwrap(),
                            squad: "product".into(),
                            room_id: "room".into(),
                            owner: "owner".into(),
                            message: "Review tests; no tokens or dollar estimate".into(),
                            schedule: "every 30m".into(),
                            zone: "UTC".into(),
                        };
                        let Effect::Act(Request::CronPreview(task)) =
                            app.cron_review_request(request)
                        else {
                            panic!("the existing worker lane owns preview acquisition")
                        };
                        if scene == "timer-ready" {
                            let (task, result) = cronboard::preview::run(*task, 0);
                            app.cron_review_ready(task, result);
                        } else if scene == "timer-failed" {
                            app.cron_review_ready(
                                *task,
                                Err("Schedule observation unavailable".into()),
                            );
                        }
                    }
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| crate::board::view::render(frame, &app))
                        .unwrap();
                    if scene == "details-link-end" {
                        app.monitor_details_event(&ratatui::crossterm::event::Event::Key(
                            ratatui::crossterm::event::KeyEvent::new(
                                ratatui::crossterm::event::KeyCode::End,
                                ratatui::crossterm::event::KeyModifiers::NONE,
                            ),
                        ));
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                    }
                    let buffer = terminal.backend().buffer();
                    let cells = buffer.content.iter().map(|cell| json!({
                        "symbol":cell.symbol(), "fg":format!("{:?}",cell.fg),
                        "bg":format!("{:?}",cell.bg), "modifier":format!("{:?}",cell.modifier),
                    })).collect::<Vec<_>>();
                    let lines = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect::<Vec<_>>();
                    assert!(lines.iter().all(|line| !line.chars().any(char::is_control)));
                    frames.push(
                        json!({"scene":scene,"width":width,"height":height,"base":base,
                        "depth":format!("{depth:?}"),"cells":cells,"lines":lines}),
                    );
                }
            }
        }
    }
    assert_eq!(frames.len(), 200);
    std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
}

/// Actual frame evidence over the existing frozen history/quota observations.
/// Never writes a checked-in fixture or acquires a new source.
#[test]
#[ignore = "explicit Overview mock-fidelity frame export"]
fn capture_overview_mock_packet() {
    let output = std::env::var("TMT_OVERVIEW_CAPTURE").expect("task-owned evidence path");
    crate::status::with_now_ms(1_791_600_000_000, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut frames = Vec::new();
        for scene in ["metered", "pinned-hover-large"] {
            for width in [160, 100, 80, 60] {
                for height in [44, 24, 18] {
                    for (base, depth) in [
                        ("tmt", tmt_cli_style::Depth::TrueColor),
                        ("tmt-light", tmt_cli_style::Depth::TrueColor),
                        ("terminal", tmt_cli_style::Depth::Ansi16),
                        ("tmt", tmt_cli_style::Depth::None),
                    ] {
                        let look = crate::look::Look {
                            theme: tmt_cli_style::Theme::new(
                                tmt_cli_style::theme::Base::parse(base).unwrap(),
                            ),
                            depth,
                        };
                        let mut app = if scene == "metered" {
                            crate::board::home::tests::oracle::metered(look)
                        } else {
                            pinned_large_case(look)
                        };
                        app.set_body_width(width);
                        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                        if scene == "pinned-hover-large" {
                            use ratatui::crossterm::event::{
                                KeyModifiers, MouseEvent, MouseEventKind,
                            };
                            let area = app
                                .overview_sort_hits
                                .borrow()
                                .iter()
                                .find(|(_, column)| *column == super::columns::Column::Agent)
                                .expect("the visible Agent header is sortable")
                                .0;
                            app.mouse(
                                MouseEvent {
                                    kind: MouseEventKind::Moved,
                                    column: area.x,
                                    row: area.y,
                                    modifiers: KeyModifiers::NONE,
                                },
                                std::time::Instant::now(),
                            );
                            terminal
                                .draw(|frame| crate::board::view::render(frame, &app))
                                .unwrap();
                        }
                        let buffer = terminal.backend().buffer();
                        let cells: Vec<_> = buffer.content.iter().map(|cell| json!({
                        "symbol":cell.symbol(), "fg":format!("{:?}",cell.fg), "bg":format!("{:?}",cell.bg), "modifier":format!("{:?}",cell.modifier),
                    })).collect();
                        let lines: Vec<_> = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                            .collect();
                        assert!(lines.iter().all(|line| !line.chars().any(char::is_control)));
                        frames.push(json!({"scene":scene,"width":width,"height":height,"base":base,"depth":format!("{depth:?}"),"lines":lines,"cells":cells,
                        "hits":format!("{:?}",app.hits.borrow()),"selected":format!("{:?}",app.workspace.selected),"conversation":format!("{:?}",app.conversation.borrow().pane)}));
                    }
                }
            }
        }
        assert_eq!(frames.len(), 96);
        std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
    });
}

/// Literal admitted observations for visual inspection: mixed roster states,
/// persisted UUID pins, and large partial counts through the existing Rate seed.
/// This does not alter the frozen fixture owner or synthesize product data.
fn pinned_large_case(look: crate::look::Look) -> crate::board::app::App {
    use crate::board::app::workspace::tests::{MEMBER, ROOM};
    use crate::board::workspace::{OpsMember, OpsMembershipState, OpsTarget};
    let mut app = crate::board::home::tests::oracle::metered(look);
    let view = app.view.as_mut().unwrap();
    let home = view.home.as_mut().unwrap();
    for (ordinal, name, state) in [
        (1, "Atlas", "working"),
        (2, "Pat", "review"),
        (3, "Rowan", "blocked"),
        (4, "Sky", "idle"),
        (5, "Long-agent-name-with-a-suffix", "working"),
    ] {
        let id = format!("44444444-4444-4444-8444-{ordinal:012}");
        home.workspace.roster.members.insert(
            id.clone(),
            OpsMember {
                id: id.clone(),
                name: name.into(),
                state: Some(state.into()),
                waiting: state == "blocked",
                presence: Some(true),
                usage: None,
            },
        );
        let squad = &mut home.workspace.roster.squads[0];
        squad.members.push(id.clone());
        squad.member_states.insert(
            id.clone(),
            OpsMembershipState {
                state: Some(state.into()),
                waiting: state == "blocked",
                rank: None,
            },
        );
        home.workspace.rows.insert(
            OpsTarget::Member {
                room_id: Some(ROOM.into()),
                identity_id: id.clone(),
            },
            json!({"id":id,"name":name,"state":state,"presence":"active"}),
        );
        if ordinal <= 2 {
            home.workspace.preferences.agent_pins.insert(id);
        }
    }
    home.summary.members = 6;
    home.summary.working = 3;
    home.summary.idle = 1;
    home.summary.blocked = 1;
    // Large counts retain the original coverage/partial flags. The product still
    // derives the chart through its normal admitted history projection.
    let rate = view.home_rate.get_mut("product").unwrap();
    let mut seed =
        crate::board::rate::tests::fixture_seeds(&crate::board::rate::tests::history_fixture())
            .remove("a")
            .unwrap()
            .unwrap();
    let offset = 1_791_600_000_000 / 5_000 * 5_000 - 25_000;
    seed.from += offset;
    seed.through += offset;
    seed.available = seed.available.map(|at| at + offset);
    seed.sampled = seed.sampled.map(|at| at + offset);
    for bucket in &mut seed.buckets {
        bucket.from_ms += offset;
        bucket.to_ms += offset;
        bucket.input_tokens *= 8_000_000;
        bucket.output_tokens *= 8_000_000;
        bucket.cached_input_tokens *= 8_000_000;
    }
    rate.history = Some([(MEMBER.into(), Some(seed))].into());
    let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    snapshot.tabs = app.tabs.clone();
    snapshot.view = Ok(app.view.take().unwrap());
    app.apply(snapshot);
    app
}
