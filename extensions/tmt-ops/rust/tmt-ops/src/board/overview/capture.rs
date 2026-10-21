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
    if std::env::var_os("TMT_OVERVIEW_S2_CAPTURE").is_some()
        || std::env::var_os("TMT_OVERVIEW_S3_CAPTURE").is_some()
        || std::env::var_os("TMT_OVERVIEW_S4_CAPTURE").is_some()
    {
        capture_squads_packet(&output);
        return;
    }
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
                                .workspace_placement(app.conversation.borrow().area)
                                .unwrap()
                                .list
                                .unwrap();
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
                activity: Default::default(),
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
                idle_over: false,
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

/// S2 evidence uses acquired fixture documents and the actual App painter.
fn capture_squads_packet(output: &str) {
    let now = if std::env::var_os("TMT_OVERVIEW_S3_CAPTURE").is_some()
        || std::env::var_os("TMT_OVERVIEW_S4_CAPTURE").is_some()
    {
        1_792_108_800_000
    } else {
        1_791_600_000_000
    };
    crate::status::with_now_ms(now, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut frames = Vec::new();
        let s4 = std::env::var_os("TMT_OVERVIEW_S4_CAPTURE").is_some();
        let mut scenes = vec![
            "squads",
            "agents",
            "squads-empty",
            "agents-empty",
            "squads-partial",
            "agents-partial",
        ];
        if s4 {
            scenes.extend([
                "squads-valued",
                "squads-focused",
                "squads-reply",
                "squads-readonly",
            ]);
        }
        for scene in scenes {
            for width in [60, 103, 148] {
                if width < 140
                    && matches!(scene, "squads-focused" | "squads-reply" | "squads-readonly")
                {
                    continue;
                }
                for (base, depth) in [
                    ("tmt", tmt_cli_style::Depth::TrueColor),
                    ("tmt-light", tmt_cli_style::Depth::TrueColor),
                    ("tmt", tmt_cli_style::Depth::None),
                ] {
                    let mut app = crate::board::app::workspace::tests::populated_board(8, 28);
                    app.shell.tab = crate::board::shell::OpsTab::Agents;
                    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
                    app.workspace.filter = if scene.starts_with("agents") {
                        crate::board::workspace::AgentsFilter::Agents
                    } else {
                        crate::board::workspace::AgentsFilter::All
                    };
                    let view = app.view.as_mut().unwrap();
                    view.look = crate::look::Look {
                        theme: tmt_cli_style::Theme::new(
                            tmt_cli_style::theme::Base::parse(base).unwrap(),
                        ),
                        depth,
                    };
                    let snapshot = &mut view.home.as_mut().unwrap().workspace;
                    // Literal current observations exercise nine questions without
                    // invoking the retired HOME projection or inventing token data.
                    let waiting: std::collections::BTreeSet<_> =
                        snapshot.roster.members.keys().take(9).cloned().collect();
                    for (id, member) in &mut snapshot.roster.members {
                        member.waiting = waiting.contains(id);
                    }
                    for squad in &mut snapshot.roster.squads {
                        for (id, member) in &mut squad.member_states {
                            member.waiting = waiting.contains(id);
                            let target = crate::board::workspace::OpsTarget::Member {
                                room_id: Some(squad.room_id.clone()),
                                identity_id: id.clone(),
                            };
                            let row = snapshot.rows.get_mut(&target).unwrap();
                            row["fields"]["task"] = json!("Bounded observed task");
                            if member.waiting {
                                row["waitingOnYou"] = json!([{
                                    "requestId":format!("question-{id}"),
                                    "preparedAtMs":1_791_599_000_000u64,
                                    "preview":format!("Approve the change for {}?", row["name"].as_str().unwrap()),
                                }]);
                            }
                        }
                    }
                    if scene.ends_with("-empty") {
                        snapshot.roster.squads.clear();
                        snapshot.roster.members.clear();
                        snapshot.rows.clear();
                    } else if scene.ends_with("-partial") {
                        snapshot.roster.membership_complete = false;
                    }
                    snapshot.shell_counts = snapshot.roster.shell_counts();
                    if std::env::var_os("TMT_OVERVIEW_S3_CAPTURE").is_some()
                        || std::env::var_os("TMT_OVERVIEW_S4_CAPTURE").is_some()
                    {
                        populate_lower_band(&mut app, scene);
                    }
                    app.overview_rows.borrow_mut().clear();
                    app.workspace_reconcile();
                    app.workspace.selected =
                        app.workspace_rows().first().map(|row| row.target.clone());
                    if s4 {
                        populate_final(&mut app, scene);
                        app.workspace_reconcile();
                    }
                    let height = 51;
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| {
                            crate::board::view::prepare(&mut app, frame.area());
                            crate::board::view::render(frame, &app);
                        })
                        .unwrap();
                    if matches!(scene, "squads-focused" | "squads-reply") {
                        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
                        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        if scene == "squads-reply" {
                            app.paste("Approved. Keep the startup budget unchanged.");
                            let crate::board::app::Effect::Act(request) =
                                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                            else {
                                panic!("fixture answer");
                            };
                            let ticket = app.outgoing_ticket(&request).unwrap();
                            let to = app.composer_projection().unwrap().recipient.to_owned();
                            app.finished_answer(crate::board::conversation::outgoing::Answered {
                                ticket,
                                outcome: Ok(()),
                                to,
                            });
                            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                        }
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                    }
                    let buffer = terminal.backend().buffer();
                    let cells = buffer.content.iter().map(|cell| json!({
                        "symbol":cell.symbol(), "fg":format!("{:?}",cell.fg),
                        "bg":format!("{:?}",cell.bg), "modifier":format!("{:?}",cell.modifier),
                    })).collect::<Vec<_>>();
                    let lines: Vec<_> = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect();
                    let cursor = if app.input.is_some()
                        && app.input_viewport.get().is_some()
                        && !app.modal_open()
                    {
                        ratatui::backend::Backend::get_cursor_position(terminal.backend_mut())
                            .ok()
                            .map(|position| [position.x, position.y])
                    } else {
                        None
                    };
                    frames.push(json!({"scene":scene,"width":width,"height":height,"base":base,
                        "depth":format!("{depth:?}"),"lines":lines,"cells":cells,
                        "selected":format!("{:?}",app.workspace.selected),"hits":format!("{:?}",app.hits.borrow()),"controls":format!("{:?}",app.workspace_strip_hits.borrow()),
                        "inputBand":format!("{:?}",app.input_band.get()),"inputViewport":format!("{:?}",app.input_viewport.get()),"cursor":cursor,
                        "sendHit":app.composer.send_hit.borrow().as_ref().map(|hit|json!({"area":format!("{:?}",hit.area),"sender":hit.key.0,"recipient":hit.key.1})),
                        "quoteHit":app.composer.quote_hit.borrow().as_ref().map(|hit|format!("{:?}",hit.area))}));
                }
            }
        }
        assert_eq!(frames.len(), if s4 { 72 } else { 54 });
        std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
    });
}

/// Admitted fixture observations pass through the normal label/cron projections.
pub(in crate::board) fn populate_lower_band(app: &mut crate::board::app::App, scene: &str) {
    use crate::board::cronboard::{test_cron, test_views};
    use tmt_ops::cron::{ClockStatus, Holder};
    let now = crate::status::now_ms() as i64;
    let mut document = crate::board::digest_dashboard::tests::valued_document();
    let template = document["members"][0].clone();
    document["observedAtMs"] = json!(now);
    document["members"] = json!(
        app.workspace_snapshot()
            .unwrap()
            .roster
            .members
            .iter()
            .map(|(id, member)| {
                let mut row = template.clone();
                row["identityId"] = json!(id);
                row["name"] = json!(member.name);
                row["savings"] = json!(null);
                row["statsObservedAtMs"] = json!(now - 1_000);
                row
            })
            .collect::<Vec<_>>()
    );
    app.labels =
        crate::labels::Supplied::from_document("digest", &document, scene.ends_with("-partial"));
    let core = test_views(4, "tmt-core", "core-room");
    let infra = test_views(2, "tmt-infra", "infra-room");
    let mut ux = test_views(1, "tmt-ux", "ux-room").remove(0);
    ux.owner_missing = true;
    let mut jobs = vec![
        core[3].clone(),
        infra[0].clone(),
        core[0].clone(),
        infra[1].clone(),
        ux,
    ];
    for (job, minutes) in jobs.iter_mut().zip([19, 21, 49, 3 * 24 * 60 + 11 * 60, 0]) {
        job.next_ms = vec![now + minutes * 60_000];
    }
    if scene.ends_with("-empty") {
        jobs.clear();
    }
    let mut cron = test_cron(
        jobs,
        ClockStatus::Running(Holder {
            pid: 7,
            pane: Some("%41".into()),
            since_ms: now - 60_000,
            expires_ms: now + 5_000,
            build: None,
        }),
    );
    cron.read_ms = now;
    app.cron.replace(Ok(cron));
    if scene.ends_with("-partial") {
        app.cron
            .replace(Err("Timer observation unavailable".into()));
    }
}

/// S4 fixture data enters the same held RateView/Meter and exact-question ports.
pub(in crate::board) fn populate_final(app: &mut crate::board::app::App, scene: &str) {
    use crate::board::{
        app::RateView,
        rate::{
            Input,
            tests::{fixture_seeds, history_fixture},
        },
    };
    use std::collections::BTreeMap;
    let now = crate::status::now_ms();
    if scene.ends_with("-empty") {
        return;
    }
    for row in app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .values_mut()
    {
        for (index, question) in row["waitingOnYou"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .enumerate()
        {
            question["preparedAtMs"] = json!(now - (index as u64 + 1) * 60_000);
        }
    }
    if matches!(scene, "squads-valued" | "digest-rows") {
        let mut document = crate::board::digest_dashboard::tests::valued_document();
        let template = document["members"][0].clone();
        document["observedAtMs"] = json!(now);
        document["members"] = json!(
            app.workspace_snapshot()
                .unwrap()
                .roster
                .members
                .iter()
                .map(|(id, member)| {
                    let mut value = template.clone();
                    value["identityId"] = json!(id);
                    value["name"] = json!(member.name);
                    value["statsObservedAtMs"] = json!(now - 1_000);
                    value
                })
                .collect::<Vec<_>>()
        );
        app.labels = crate::labels::Supplied::from_document("digest", &document, false);
    }
    if scene == "squads-readonly" {
        app.view.as_mut().unwrap().me_id = None;
    }
    let squad = app.workspace_snapshot().unwrap().roster.squads[0].clone();
    let roster = &app.workspace_snapshot().unwrap().roster;
    let mut seeds = BTreeMap::new();
    let observed = if scene == "digest-rows" {
        squad.members.len()
    } else {
        3
    };
    for (index, id) in squad.members.iter().take(observed).enumerate() {
        let mut seed = fixture_seeds(&history_fixture())
            .remove("a")
            .unwrap()
            .unwrap();
        let offset = now / 5_000 * 5_000 - 25_000;
        seed.from += offset;
        seed.through += offset;
        seed.available = seed.available.map(|at| at + offset);
        seed.sampled = seed.sampled.map(|at| at + offset);
        let factor = (observed - index) as u64;
        for bucket in &mut seed.buckets {
            bucket.from_ms += offset;
            bucket.to_ms += offset;
            bucket.input_tokens *= factor;
            bucket.output_tokens *= factor;
            bucket.cached_input_tokens *= factor;
        }
        seed.latest["session"] = json!(id);
        seed.latest["consumption"]["observedAtMs"] =
            json!(seed.latest["consumption"]["observedAtMs"].as_u64().unwrap() + offset);
        for field in ["inputTokens", "outputTokens", "cachedInputTokens"] {
            seed.latest["consumption"][field] =
                json!(seed.latest["consumption"][field].as_u64().unwrap() * factor);
        }
        seeds.insert(id.clone(), Some(seed));
    }
    let input = Input {
        room: squad.room_id.clone(),
        resumes: seeds
            .iter()
            .map(|(id, seed)| (id.clone(), seed.as_ref().unwrap().latest.clone()))
            .collect(),
        names: seeds
            .keys()
            .map(|id| (id.clone(), roster.members[id].name.clone()))
            .collect(),
    };
    let settings = crate::config::TokenRate {
        enabled: true,
        ..Default::default()
    };
    crate::board::app::tests::install_rate_observation(
        app,
        squad.name,
        RateView {
            input,
            settings,
            history: Some(seeds),
        },
    );
    if let Some(target @ crate::board::workspace::OpsTarget::Member { .. }) =
        app.workspace_message_target()
        && let Some(row) = app
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .rows
            .get_mut(&target)
    {
        row["waitingOnYou"] = json!([{"requestId":"req_99999999-9999-4999-8999-999999999999","preparedAtMs":now-6*3_600_000,"preview":"The implementation and checks are ready. Should I proceed with the reviewed change, or hold it for the next window?"}]);
        row["fields"]["pr_link"] = json!("https://github.com/pj-tmt/tmt/pull/3176");
    }
    app.overview_rows.borrow_mut().clear();
}
