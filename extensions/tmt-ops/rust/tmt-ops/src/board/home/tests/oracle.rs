//! The single decoded Ops workspace oracle: actual frame cells, styles, clipped
//! hits, stable targets and input geometry across the new shell. Literal fixtures
//! share the same owner as private workspace captures; no alternate painter runs.
//! Normal tests never write frozen bytes. Review decoded differences before the
//! explicitly ignored record command, then regenerate in a separate commit.
use super::interaction::board;
use super::*;
use crate::board::{
    app::App,
    app::workspace::tests::{MEMBER, ROOM, capture_case},
    cronboard::{TEST_NOW, test_cron, test_view},
    home_leads::{Kind, LeadPreview},
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use std::collections::BTreeMap;
use tmt_ops::cron::ClockStatus;

const DAY: u64 = 86_400_000;

/// Ages are relative to the real clock the painter reads; whole days keep the
/// labels stable for the length of a test run.
fn ago(days: f64) -> u64 {
    crate::status::now_ms() - (days * DAY as f64) as u64
}

fn waiting_member(id: &str, name: &str, days: f64) -> Value {
    let mut member = row(id, name, "blocked");
    member["waitingOnYou"] = json!([{
        "requestId": format!("q-{id}"),
        "preparedAtMs": ago(days),
        "preview": "Should this decision proceed?"
    }]);
    member
}

fn blocked_member(id: &str, name: &str, days: f64) -> Value {
    let mut member = row(id, name, "blocked");
    member["staleness"] = json!({"unchangedSinceMs": ago(days)});
    member
}

/// Five squads: attention in the first, every lead exchange kind, two leads
/// without an exchange, an unknown member state, and a cron job.
pub(in crate::board) fn rich() -> App {
    let mut question = row("LA", "lead-a", "working");
    question["waitingOnYou"] = json!([{
        "requestId": "lead-question",
        "preparedAtMs": ago(2.5),
        "preview": "Approve this change?"
    }]);
    let mut app = board(&[
        (
            "alpha",
            document(
                "alpha",
                question,
                vec![
                    waiting_member("W1", "worker-one", 2.5),
                    waiting_member("W2", "worker-two", 3.5),
                    blocked_member("B1", "stuck-one", 4.5),
                ],
            ),
        ),
        (
            "beta",
            document(
                "beta",
                row("LB", "lead-b", "working"),
                vec![
                    row("R1", "reviewer", "review"),
                    row("I1", "idle-one", "idle"),
                ],
            ),
        ),
        (
            "gamma",
            document(
                "gamma",
                row("LG", "lead-g", "idle"),
                vec![
                    row("P1", "paused-one", "paused"),
                    row("K1", "working-one", "working"),
                ],
            ),
        ),
        (
            "delta-with-a-long-squad-name",
            document(
                "delta-with-a-long-squad-name",
                row("LD", "lead-delta-with-a-long-name", "working"),
                vec![],
            ),
        ),
        (
            "epsilon",
            document("epsilon", row("LE", "lead-e", "idle"), vec![]),
        ),
    ]);
    let view = app.view.as_mut().unwrap();
    view.me_id = Some("user".into());
    app.home_leads
        .reconcile(view.home.as_ref().unwrap(), view.me_id.as_deref());
    let exchanges = [
        ("lead-a", Kind::Question, "Approve this change?", 2.5),
        (
            "lead-b",
            Kind::Reply,
            "The public replies and results reads are merged.",
            1.5,
        ),
        ("lead-g", Kind::Asked, "Please review the draft", 0.5),
    ];
    for lead in &mut app.home_leads.leads {
        if let Some((_, kind, preview, days)) =
            exchanges.iter().find(|(name, ..)| *name == lead.name())
        {
            lead.exchange = Some(LeadPreview {
                kind: *kind,
                request: Some(format!("request-{}", lead.name())),
                since_ms: Some(ago(*days)),
                preview: (*preview).into(),
                status: "retained".into(),
            });
        }
    }
    // Injected observations must pass through the same ordering as a refresh.
    app.home_leads
        .reconcile(view.home.as_ref().unwrap(), view.me_id.as_deref());
    let mut job = test_view("lead-a", "merge queue sweep", Some(TEST_NOW + 3_600_000));
    job.job.squad = "alpha".into();
    app.cron
        .replace(Ok(test_cron(vec![job], ClockStatus::NoClock)));
    app.select(0);
    app
}

#[test]
fn oracle_leads_follow_production_exchange_order() {
    let app = rich();
    assert_eq!(
        app.home_leads
            .leads
            .iter()
            .map(|lead| lead.name())
            .collect::<Vec<_>>(),
        [
            "lead-a",
            "lead-g",
            "lead-b",
            "lead-delta-with-a-long-name",
            "lead-e"
        ]
    );
}

/// Style identities are interned: a frame lists style ids, not repeated debug text.
#[derive(Default)]
struct Styles(BTreeMap<String, String>);
impl Styles {
    fn id(&mut self, cell: &ratatui::buffer::Cell) -> String {
        let mut metadata = cell.clone();
        metadata.set_symbol("");
        let text = format!("{metadata:?}");
        let next = format!("s{}", self.0.len());
        self.0.entry(text).or_insert(next).clone()
    }
}

fn buffer_json(buffer: &Buffer, styles: &mut Styles) -> Value {
    let width = usize::from(buffer.area.width);
    let mut lines = Vec::new();
    let mut runs = Vec::new();
    for cells in buffer.content.chunks(width) {
        // Blank cells carry no more than their style, which the runs record.
        lines.push(
            cells
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_owned(),
        );
        let mut row: Vec<String> = Vec::new();
        let mut from = 0;
        let ids = cells.iter().map(|cell| styles.id(cell)).collect::<Vec<_>>();
        for index in 1..=ids.len() {
            if index == ids.len() || ids[index] != ids[from] {
                row.push(format!("{from}:{}={}", index - from, ids[from]));
                from = index;
            }
        }
        runs.push(row.join(" "));
    }
    json!({"lines": lines, "styles": runs})
}

pub(in crate::board) fn looks() -> [(&'static str, crate::look::Look); 4] {
    use tmt_cli_style::{Depth, Theme, theme::Base};
    [
        ("tmt", Base::Tmt, Depth::TrueColor),
        ("tmt-light", Base::TmtLight, Depth::TrueColor),
        ("terminal", Base::Terminal, Depth::Ansi16),
        ("NO_COLOR", Base::Tmt, Depth::None),
    ]
    .map(|(name, base, depth)| {
        (
            name,
            crate::look::Look {
                theme: Theme::new(base),
                depth,
            },
        )
    })
}

/// Existing runtime history and provider-limit fixture observations. The reader
/// is not called, and no savings, forecast or missing reading is synthesized.
pub(in crate::board) fn metered(look: crate::look::Look) -> App {
    use crate::board::{
        app::RateView,
        limits::{Provider, Reading, Standing, Trend, Window},
        rate::{
            Input,
            tests::{fixture_seeds, history_fixture},
        },
    };
    use crate::config::TokenRate;
    const NOW: u64 = 1_791_600_000_000;
    const HOUR: u64 = 3_600_000;
    let mut app = capture_case("overview", look);
    let mut seed = fixture_seeds(&history_fixture())
        .remove("a")
        .unwrap()
        .unwrap();
    let offset = NOW / 5_000 * 5_000 - 25_000;
    seed.from += offset;
    seed.through += offset;
    seed.available = seed.available.map(|at| at + offset);
    seed.sampled = seed.sampled.map(|at| at + offset);
    for bucket in &mut seed.buckets {
        bucket.from_ms += offset;
        bucket.to_ms += offset;
    }
    let view = app.view.as_mut().unwrap();
    let home = view.home.as_mut().unwrap();
    let row = home.workspace.rows[&crate::board::workspace::OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: MEMBER.into(),
    }]
        .clone();
    home.workspace.roster.squads[0].lead = Some(MEMBER.into());
    let mut public_row = row.clone();
    public_row["squad"] = json!("product");
    view.document["sections"] = json!([{"rows":[public_row]}]);
    home.squads = vec![SquadLine {
        squad: "product".into(),
        lead: Some(row),
        counts: Counts {
            members: 1,
            working: 1,
            ..Default::default()
        },
    }];
    view.home_rate.insert(
        "product".into(),
        RateView {
            input: Input {
                room: ROOM.into(),
                resumes: BTreeMap::from([(MEMBER.into(), seed.latest.clone())]),
                names: BTreeMap::from([(MEMBER.into(), "worker".into())]),
            },
            settings: TokenRate {
                enabled: true,
                ..Default::default()
            },
            history: Some(BTreeMap::from([(MEMBER.into(), Some(seed))])),
        },
    );
    let mut snapshot = crate::board::app::tests::snapshot(ALL, json!([]));
    snapshot.tabs = app.tabs.clone();
    snapshot.view = Ok(app.view.take().unwrap());
    app.apply(snapshot);
    app.set_limits(vec![
        Standing {
            rates: Default::default(),
            provider: Provider::named("claude"),
            latest: Some(Reading {
                observed_at_ms: NOW - 60_000,
                weekly: Window {
                    left: 44.0,
                    resets_at_ms: NOW + 87 * HOUR,
                },
                short: Some(Window {
                    left: 94.0,
                    resets_at_ms: NOW + 3 * HOUR,
                }),
            }),
            burn: Some(1.1),
            trend: Some(Trend::Up),
        },
        Standing {
            rates: Default::default(),
            provider: Provider::named("codex"),
            latest: Some(Reading {
                observed_at_ms: NOW - 60_000,
                weekly: Window {
                    left: 5.0,
                    resets_at_ms: NOW + 83 * HOUR,
                },
                short: None,
            }),
            burn: Some(1.5),
            trend: Some(Trend::Same),
        },
    ]);
    app
}

fn capture_frame(
    app: &App,
    state: &str,
    look_name: &str,
    width: u16,
    height: u16,
    styles: &mut Styles,
) -> Value {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    let mut captured = buffer_json(terminal.backend().buffer(), styles);
    captured["state"] = json!(state);
    captured["look"] = json!(look_name);
    captured["width"] = json!(width);
    captured["height"] = json!(height);
    captured["hits"] = json!(format!("{:?}", app.hits.borrow()));
    captured["actions"] = json!(format!("{:?}", app.workspace_action_hits.borrow()));
    captured["tabs"] = json!(format!("{:?}", app.tab_hits.borrow()));
    captured["starts"] = json!(*app.row_starts.borrow());
    captured["selected"] = json!(format!("{:?}", app.workspace.selected));
    captured["selectionState"] = json!(format!("{:?}", app.workspace_selection_state()));
    captured["focus"] = json!(format!("{:?}", app.focused_pane()));
    captured["input"] = json!(format!("{:?}", app.input_band.get()));
    if state.starts_with("bindings-") {
        captured["bindingGeometry"] = app.settings.as_ref().unwrap().captured_binding_geometry();
    }
    captured
}

fn captures() -> Value {
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        let mut styles = Styles::default();
        let mut frames = Vec::new();
        for (look_name, look) in looks() {
            // `overview` and the `*-overview` names below are frozen fixture names;
            // they now render the merged Agents tab.
            for state in [
                "overview",
                "agents",
                "agents-sorted",
                "agents-state-sorted",
                "agents-header-hover",
                "digest",
                "digest-first-open",
                "digest-busy-late",
                "digest-retained-pick",
                "digest-empty-flow",
                "digest-unavailable-flow",
                "digest-no-agents",
                "digest-allowance",
                "digest-allowance-tokens",
                "empty",
                "pool",
                "dense",
                "squads-eight",
                "squads-thirty-six",
                "partial",
                "stale",
                "help",
                "review",
                "result",
                "conversation",
                "composer-empty",
                "composer-2",
                "composer-6",
                "composer-8",
                "header-usage",
                "unread-working",
                "unread-idle",
                "unread-waiting",
                "unread-squads",
                "unread-cleared",
                "unread-turn",
            ]
            .into_iter()
            .chain(
                crate::board::conversation::outgoing::tests::captures::SCENES
                    .iter()
                    .copied(),
            ) {
                for width in [160, 100, 80, 60] {
                    for height in [36, 8] {
                        let app = if matches!(
                            state,
                            "digest-first-open"
                                | "digest-busy-late"
                                | "digest-retained-pick"
                                | "digest-empty-flow"
                                | "digest-unavailable-flow"
                                | "digest-no-agents"
                        ) {
                            crate::board::digest_dashboard::tests::capture_case(state, look)
                        } else if state == "header-usage" {
                            metered(look)
                        } else if crate::board::conversation::outgoing::tests::captures::SCENES
                            .contains(&state)
                        {
                            crate::board::conversation::outgoing::tests::captures::fixture(
                                state, look, NOW,
                            )
                        } else {
                            capture_case(state, look)
                        };
                        let captured =
                            capture_frame(&app, state, look_name, width, height, &mut styles);
                        if state.starts_with("chat-") && width == 160 && height == 36 {
                            let text = captured["lines"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|line| line.as_str().unwrap())
                                .collect::<Vec<_>>()
                                .join("\n");
                            assert!(
                                text.contains(
                                    crate::board::conversation::outgoing::tests::captures::expected(
                                        state
                                    )
                                ),
                                "missing capture state {state}: {text}"
                            );
                        }
                        frames.push(captured);
                    }
                }
            }
        }
        for (look_name, look) in looks() {
            for state in crate::board::overview::chart::host::tests::cases::NAMES {
                for width in [60, 100, 103, 148, 200] {
                    let app = crate::board::overview::chart::host::tests::cases::board_case(
                        state, look, width,
                    );
                    frames.push(capture_frame(
                        &app,
                        state,
                        look_name,
                        width,
                        51,
                        &mut styles,
                    ));
                }
            }
        }
        // All-key Settings and inert parameter previews use the actual board owner.
        for (look_name, look) in looks() {
            for state in [
                "bindings-list",
                "bindings-punctuation",
                "bindings-preview",
                "bindings-unavailable",
                "bindings-masked",
                "bindings-j-preset",
                "bindings-inventory-scrolled",
            ] {
                for width in [160, 100, 80, 60] {
                    for height in [36, 8] {
                        let app = crate::board::settings::tests::host_action::capture_case(
                            state, look, width, height,
                        );
                        frames.push(capture_frame(
                            &app,
                            state,
                            look_name,
                            width,
                            height,
                            &mut styles,
                        ));
                    }
                }
            }
        }
        // Idle alert and the squad settings reached from the Workspace menu: additive scenes.
        for (look_name, look) in looks() {
            for state in [
                "idle-alert",
                "idle-alert-overview",
                "settings-menu",
                "settings-menu-pool",
                "squad-settings",
            ] {
                for width in [160, 100, 80, 60] {
                    let app = capture_case(state, look);
                    frames.push(capture_frame(
                        &app,
                        state,
                        look_name,
                        width,
                        36,
                        &mut styles,
                    ));
                }
            }
        }
        // Measure, the right column's summary view: an agent, a squad and the Agents chip.
        for (look_name, look) in looks() {
            for scene in ["measure-agent", "measure-squad", "measure-agents-chip"] {
                for width in [160, 100] {
                    let mut app = crate::board::app::workspace::tests::populated_board(1, 24);
                    app.view.as_mut().unwrap().look = look;
                    let workspace =
                        &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                    for squad in &workspace.roster.squads {
                        workspace.squad_looks.insert(squad.room_id.clone(), look);
                    }
                    let member = workspace
                        .rows
                        .keys()
                        .find(|target| {
                            matches!(target, crate::board::workspace::OpsTarget::Member { .. })
                        })
                        .cloned()
                        .unwrap();
                    let room = workspace.roster.squads[0].room_id.clone();
                    app.shell.tab = crate::board::shell::OpsTab::Agents;
                    app.right_tab = crate::board::shell::RightTab::Measure;
                    app.workspace.filter = if scene == "measure-agents-chip" {
                        crate::board::workspace::AgentsFilter::Agents
                    } else {
                        crate::board::workspace::AgentsFilter::All
                    };
                    crate::board::overview::capture::populate_lower_band(&mut app, "digest-rows");
                    crate::board::overview::capture::populate_final(&mut app, "digest-rows");
                    app.workspace.expanded.insert(room.clone());
                    app.workspace.selected = Some(if scene == "measure-squad" {
                        crate::board::workspace::OpsTarget::Squad(room)
                    } else {
                        member
                    });
                    app.workspace_reconcile();
                    frames.push(capture_frame(
                        &app,
                        scene,
                        look_name,
                        width,
                        51,
                        &mut styles,
                    ));
                }
            }
        }
        // Search has its own committed indicator and clear hit. These additive
        // scenes keep the existing unfiltered baseline independently frozen.
        for (look_name, look) in looks() {
            for mode in [
                "overview-agents",
                "overview-squads",
                "agents-all",
                "agents-pinned",
            ] {
                for state in ["matches", "empty"] {
                    for width in [160, 100, 80] {
                        let mut app = crate::board::app::workspace::tests::populated_board(8, 28);
                        app.view.as_mut().unwrap().look = look;
                        let workspace =
                            &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                        for squad in &workspace.roster.squads {
                            workspace.squad_looks.insert(squad.room_id.clone(), look);
                        }
                        if mode == "agents-pinned" {
                            workspace
                                .preferences
                                .pinned
                                .insert(workspace.roster.squads[7].room_id.clone());
                            app.workspace.filter = crate::board::workspace::AgentsFilter::Pinned;
                        }
                        app.shell.tab = crate::board::shell::OpsTab::Agents;
                        if mode != "agents-pinned" {
                            app.workspace.filter = if mode == "overview-squads" {
                                crate::board::workspace::AgentsFilter::All
                            } else {
                                crate::board::workspace::AgentsFilter::Agents
                            };
                        }
                        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
                        app.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
                        app.paste(if state == "matches" {
                            "agent-00007"
                        } else {
                            "no-such-agent"
                        });
                        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        let mut captured = capture_frame(
                            &app,
                            &format!("search-{mode}-{state}"),
                            look_name,
                            width,
                            36,
                            &mut styles,
                        );
                        captured["stripHits"] =
                            json!(format!("{:?}", app.workspace_strip_hits.borrow()));
                        frames.push(captured);
                    }
                }
            }
        }
        // Measure's `Exchanges · <range>` section with one, a few and many agents: additive scenes.
        for (look_name, look) in looks() {
            for (scene, extra, flows) in [
                ("measure-exchanges-one", 0, &[][..]),
                (
                    "measure-exchanges-three",
                    2,
                    &[(1, 2, 7), (2, 0, 3), (0, 1, 12)][..],
                ),
                (
                    "measure-exchanges-many",
                    8,
                    &[(8, 7, 90), (7, 8, 80), (1, 2, 1), (0, 3, 4000), (3, 0, 2)][..],
                ),
            ] {
                for width in [200, 160, 140] {
                    let mut app = capture_case("agents", look);
                    crate::board::app::workspace::tests::measure_exchanges(&mut app, extra, flows);
                    frames.push(capture_frame(
                        &app,
                        scene,
                        look_name,
                        width,
                        51,
                        &mut styles,
                    ));
                }
            }
        }
        // Paste attachments and forms share their scene owner with decoded review.
        for (look_name, look) in looks() {
            for state in crate::board::composer::paste::tests::SCENES {
                let height = crate::board::composer::paste::tests::capture_height(state);
                for width in [160, 100, 80] {
                    let app = crate::board::composer::paste::tests::capture_case(
                        state, look, width, height,
                    );
                    let mut captured =
                        capture_frame(&app, state, look_name, width, height, &mut styles);
                    captured["paste"] =
                        crate::board::composer::paste::tests::capture_metadata(&app);
                    frames.push(captured);
                }
            }
        }
        // Dropped files become chips in the same composer; the scenes use fixed, synthetic paths.
        for (look_name, look) in looks() {
            for state in crate::board::composer::files::tests::SCENES {
                let height = if *state == "file-tiny" { 8 } else { 36 };
                for width in [160, 100, 80] {
                    let app = crate::board::composer::files::tests::capture_case(
                        state, look, width, height,
                    );
                    let mut captured =
                        capture_frame(&app, state, look_name, width, height, &mut styles);
                    captured["files"] =
                        crate::board::composer::files::tests::capture_metadata(&app);
                    frames.push(captured);
                }
            }
        }
        // Inline quote and answer placement uses the existing paste/file fold owner.
        for (look_name, look) in looks() {
            for state in crate::board::composer::tests::QUOTE_SCENES {
                let height = if *state == "quote-tiny" { 8 } else { 36 };
                for width in [160, 100, 80, 60] {
                    let app = crate::board::composer::tests::quote_capture_case(
                        state, look, width, height,
                    );
                    let mut captured =
                        capture_frame(&app, state, look_name, width, height, &mut styles);
                    captured["quotes"] =
                        crate::board::composer::tests::quote_capture_metadata(&app);
                    frames.push(captured);
                }
            }
        }
        // A link in the chat opens a small popover; the address is fixed and synthetic.
        for (look_name, look) in looks() {
            for state in crate::board::member_links::popover::SCENES {
                for width in [160, 100, 60] {
                    let app =
                        crate::board::member_links::popover::capture_case(state, look, width, 36);
                    frames.push(capture_frame(
                        &app,
                        state,
                        look_name,
                        width,
                        36,
                        &mut styles,
                    ));
                }
            }
        }
        for (look_name, look) in looks() {
            for state in crate::board::member_links::tests::FILE_SCENES {
                for width in [160, 100, 80, 60] {
                    let app = crate::board::member_links::tests::capture_file_case(
                        state, look, width, 36,
                    );
                    let mut captured =
                        capture_frame(&app, state, look_name, width, 36, &mut styles);
                    captured["references"] =
                        json!(crate::board::member_links::tests::capture_file_hits(&app));
                    frames.push(captured);
                }
            }
        }
        // The Talk pane's live screen of the selected agent, in each state it can be in.
        for (look_name, look) in looks() {
            for state in crate::board::conversation::screen::tests::SCENES {
                for width in [160, 100, 60] {
                    let app = crate::board::conversation::screen::tests::capture_case(
                        state, look, width, 40,
                    );
                    frames.push(capture_frame(
                        &app,
                        state,
                        look_name,
                        width,
                        40,
                        &mut styles,
                    ));
                }
            }
        }
        // One loading mark or a retained conversation plus its refresh status,
        // then a single complete publication through the real selected owner.
        for (look_name, look) in looks() {
            for scene in crate::board::conversation::tests::refresh::SCENES {
                for width in [160, 100, 80, 60] {
                    for height in [36, 8] {
                        let app = crate::board::conversation::tests::refresh::capture_case(
                            scene, look, width, height,
                        );
                        frames.push(capture_frame(
                            &app,
                            scene,
                            look_name,
                            width,
                            height,
                            &mut styles,
                        ));
                    }
                }
            }
        }
        // Exact reported Markdown sources through the live conversation projection.
        for (look_name, look) in looks() {
            for scene in crate::board::conversation::tests::markdown_packet::REPORTED_SCENES {
                for width in [160, 100, 80, 60] {
                    let app = crate::board::conversation::tests::markdown_packet::reported_case(
                        scene, look,
                    );
                    frames.push(capture_frame(
                        &app,
                        scene,
                        look_name,
                        width,
                        36,
                        &mut styles,
                    ));
                }
            }
        }
        // A drag selection and the Selection menu opened below its end.
        for (look_name, look) in looks() {
            for scene in crate::board::conversation::source::scenes::SCENES {
                let app = crate::board::conversation::source::scenes::capture_case(scene, look);
                let (width, height) = crate::board::conversation::source::scenes::size(scene);
                frames.push(capture_frame(
                    &app,
                    scene,
                    look_name,
                    width,
                    height,
                    &mut styles,
                ));
            }
        }
        for (look_name, look) in looks() {
            for scene in crate::board::member_session::tests::SCENES {
                for width in [160, 100, 80, 60] {
                    for height in [36, 8] {
                        let app = crate::board::member_session::tests::capture_case(scene, look);
                        let mut captured =
                            capture_frame(&app, scene, look_name, width, height, &mut styles);
                        if app.member_session.is_some() {
                            captured["session"] =
                                crate::board::member_session::tests::capture_metadata(&app);
                        }
                        frames.push(captured);
                    }
                }
            }
        }
        // The Notes chip once its read has answered: additive scenes.
        for (look_name, look) in looks() {
            for scene in crate::board::conversation_notes::tests::SCENES {
                for width in [100, 80] {
                    let app = crate::board::conversation_notes::tests::capture_case(scene, look);
                    frames.push(capture_frame(
                        &app,
                        scene,
                        look_name,
                        width,
                        36,
                        &mut styles,
                    ));
                }
            }
        }
        // Add to Notes shares the existing draft and keeps its held source visible.
        for (look_name, look) in looks() {
            for scene in crate::board::conversation_notes::tests::overlay::ADD_SCENES {
                for width in [160, 100, 80, 60] {
                    let app =
                        crate::board::conversation_notes::tests::overlay::add_case(scene, look);
                    frames.push(capture_frame(
                        &app,
                        scene,
                        look_name,
                        width,
                        36,
                        &mut styles,
                    ));
                }
            }
        }
        // The Notes list overlay: additive scenes.
        for (look_name, look) in looks() {
            for scene in crate::board::conversation_notes::tests::overlay::LIST_SCENES {
                // The list also paints where its title, host and author give way.
                let widths: &[u16] = if *scene == "notes-list" {
                    &[100, 160, 60]
                } else {
                    &[100]
                };
                for width in widths {
                    let app =
                        crate::board::conversation_notes::tests::overlay::list_case(scene, look);
                    frames.push(capture_frame(
                        &app,
                        scene,
                        look_name,
                        *width,
                        36,
                        &mut styles,
                    ));
                }
            }
        }
        // One turn that answered three sends, painted under the last of them: additive scenes.
        for (look_name, look) in looks() {
            for scene in crate::board::conversation::outgoing::tests::captures::ONE_TURN_SCENES {
                for width in [100, 80] {
                    let app = crate::board::conversation::outgoing::tests::captures::one_turn_scene(
                        scene, look, NOW,
                    );
                    frames.push(capture_frame(
                        &app,
                        scene,
                        look_name,
                        width,
                        51,
                        &mut styles,
                    ));
                }
            }
        }
        // The running versions in the header and Help: additive scenes.
        for (look_name, look) in looks() {
            for scene in crate::board::versions::tests::SCENES {
                for width in [150, 80] {
                    let app = crate::board::versions::tests::capture_case(scene, look, width);
                    frames.push(capture_frame(
                        &app,
                        scene,
                        look_name,
                        width,
                        36,
                        &mut styles,
                    ));
                }
            }
        }
        for (look_name, look) in looks() {
            for scene in crate::board::new_member::tests::captures::SCENES {
                for width in [80, 150] {
                    for height in [43, 8] {
                        let app = crate::board::new_member::tests::captures::case(scene, look);
                        let mut captured =
                            capture_frame(&app, scene, look_name, width, height, &mut styles);
                        captured["newMember"] =
                            crate::board::new_member::tests::captures::metadata(&app);
                        frames.push(captured);
                    }
                }
            }
        }
        frames.extend(idle_alert_frames(&mut styles));
        frames.extend(crud_frames(
            crate::board::app::workspace::tests::review_modal::CRUD_SCENES,
            &mut styles,
        ));
        // The unread count under the pointer: one additive scene, appended so the style table keeps its order.
        for (look_name, look) in looks() {
            for width in [160, 100, 80, 60] {
                let app = capture_case("unread-chip-hover", look);
                frames.push(capture_frame(
                    &app,
                    "unread-chip-hover",
                    look_name,
                    width,
                    36,
                    &mut styles,
                ));
            }
        }
        let styles = styles
            .0
            .into_iter()
            .map(|(text, id)| (id, text))
            .collect::<BTreeMap<_, _>>();
        json!({"styles": styles, "frames": frames})
    })
}

fn crud_frames(scenes: &[&str], styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for scene in scenes {
            for width in [80, 150] {
                for height in [43, 8] {
                    let app = crate::board::app::workspace::tests::review_modal::crud_capture_case(
                        scene, look,
                    );
                    frames.push(capture_frame(&app, scene, look_name, width, height, styles));
                }
            }
        }
    }
    frames
}

#[test]
#[ignore = "exports only the revised CRUD dialog scenes; never records fixtures"]
fn dump_crud_dialog_oracle() {
    let path = std::env::var("HOME_ORACLE_OUT").expect("task-owned evidence path");
    let captured = crate::status::with_now_ms(1_791_600_000_000, || {
        let mut styles = Styles::default();
        let frames = crud_frames(
            &[
                "crud-rename-review",
                "crud-rename-pending",
                "crud-move-fields",
                "crud-move-review",
                "crud-move-success",
                "crud-move-refused",
                "crud-move-partial",
                "crud-remove",
                "crud-remove-copied",
                "crud-remove-long",
            ],
            &mut styles,
        );
        let styles = styles
            .0
            .into_iter()
            .map(|(text, id)| (id, text))
            .collect::<BTreeMap<_, _>>();
        json!({"styles":styles,"frames":frames})
    });
    fs::write(
        path,
        serde_json::to_string_pretty(&captured).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
fn home_oracle_keeps_every_cell_style_and_hit() {
    assert_eq!(
        captures(),
        serde_json::from_str::<Value>(include_str!("oracle.json")).unwrap()
    );
}

#[test]
fn no_home_frame_stacks_two_rules() {
    let captured = captures();
    for frame in captured["frames"].as_array().unwrap() {
        let lines = frame["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| {
                let mut line = line.as_str().unwrap().to_owned();
                // A floating review dialog is a separate layer. Its top edge
                // can sit below a background section rule; keep checking the
                // uncovered background columns, without counting that edge as
                // a second rule in the underlying pane.
                if matches!(frame["state"].as_str(), Some("review" | "result"))
                    && let Some(left) = line.find('┌')
                    && let Some(right) = line.rfind('┐')
                {
                    let end = right + '┐'.len_utf8();
                    let width = unicode_width::UnicodeWidthStr::width(&line[left..end]);
                    line.replace_range(left..end, &" ".repeat(width));
                }
                line
            })
            .collect::<Vec<_>>();
        assert_eq!(
            crate::test_support::stacked_rules(&lines),
            None,
            "{} / {} at {}x{}\n{}",
            frame["state"],
            frame["look"],
            frame["width"],
            frame["height"],
            lines.join("\n")
        );
    }
}

#[test]
fn overview_history_and_quota_keep_existing_observations_and_unknown_windows_distinct() {
    crate::status::with_now_ms(1_791_600_000_000, || {
        let app = metered(looks()[0].1);
        let usage = app
            .home_usage("product", std::time::Instant::now())
            .unwrap();
        assert_eq!(usage.lead[2].unwrap().tokens, 21);
        assert!(usage.lead[2].unwrap().partial);
        let limits = app.limits.as_ref().unwrap();
        assert_eq!(limits[0].latest.as_ref().unwrap().weekly.left, 44.0);
        assert_eq!(
            limits[0]
                .latest
                .as_ref()
                .unwrap()
                .short
                .as_ref()
                .unwrap()
                .left,
            94.0
        );
        assert!(limits[1].latest.as_ref().unwrap().short.is_none());
        assert_eq!(limits[0].burn, Some(1.1));
        assert_eq!(limits[1].burn, Some(1.5));
    });
}

#[test]
#[ignore = "regenerates the HOME oracle; separate commit after owning-lead decoded-diff approval"]
fn record_home_oracle() {
    fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/board/home/tests/oracle.json"
        ),
        serde_json::to_string_pretty(&captures()).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
#[ignore = "writes current captures to $HOME_ORACLE_OUT without touching frozen bytes"]
fn dump_home_oracle() {
    let path = std::env::var("HOME_ORACLE_OUT").expect("task-owned evidence path");
    fs::write(
        path,
        serde_json::to_string_pretty(&captures()).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
#[ignore = "exports only the 32 chat-fallback frames to HOME_ORACLE_OUT; never records fixtures"]
fn dump_chat_fallback_oracle() {
    let path = std::env::var("HOME_ORACLE_OUT").expect("task-owned evidence path");
    let captured = crate::status::with_now_ms(1_791_600_000_000, || {
        let mut styles = Styles::default();
        let mut frames = Vec::new();
        for (name, look) in looks() {
            for width in [160, 100, 80, 60] {
                for height in [36, 8] {
                    let app = crate::board::conversation::outgoing::tests::captures::fixture(
                        "chat-fallback",
                        look,
                        1_791_600_000_000,
                    );
                    frames.push(capture_frame(
                        &app,
                        "chat-fallback",
                        name,
                        width,
                        height,
                        &mut styles,
                    ));
                }
            }
        }
        let styles = styles
            .0
            .into_iter()
            .map(|(text, id)| (id, text))
            .collect::<BTreeMap<_, _>>();
        json!({"styles":styles, "frames":frames})
    });
    fs::write(
        path,
        serde_json::to_string_pretty(&captured).unwrap() + "\n",
    )
    .unwrap();
}

fn idle_alert_frames(styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for scene in crate::board::idle_alert_editor::tests::captures::SCENES {
            for width in [160, 100, 80, 60] {
                for height in [36, 8] {
                    let app = crate::board::idle_alert_editor::tests::captures::case(scene, look);
                    let mut frame = capture_frame(&app, scene, look_name, width, height, styles);
                    frame["idleAlertEditor"] =
                        crate::board::idle_alert_editor::tests::captures::metadata(&app);
                    frames.push(frame);
                }
            }
        }
    }
    frames
}
#[test]
#[ignore = "exports only the revised idle-alert scenes, never records fixtures"]
fn dump_idle_alert_editor_oracle() {
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut styles = Styles::default();
        let frames = idle_alert_frames(&mut styles);
        let styles = styles
            .0
            .into_iter()
            .map(|(text, id)| (id, text))
            .collect::<BTreeMap<_, _>>();
        fs::write(
            std::env::var("HOME_ORACLE_OUT").expect("task-owned evidence path"),
            serde_json::to_string_pretty(&json!({"styles":styles,"frames":frames})).unwrap() + "\n",
        )
        .unwrap();
    });
}
