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
use std::collections::{BTreeMap, HashMap};
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
    job.squad = "alpha".into();
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
            current: None,
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
            current: None,
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
    // Only frames that paint an unread count carry the regions that open the Unread overlay.
    if !app.unread_hits.borrow().is_empty() {
        captured["unreadHits"] = json!(format!("{:?}", app.unread_hits.borrow()));
    }
    if state.starts_with("bindings-") {
        captured["bindingGeometry"] = app.settings.as_ref().unwrap().captured_binding_geometry();
    }
    captured
}

/// The width x height pairs of a scene, widths outermost.
fn grid(widths: &[u16], heights: &[u16]) -> Vec<(u16, u16)> {
    widths
        .iter()
        .flat_map(|width| heights.iter().map(move |height| (*width, *height)))
        .collect()
}

/// Every look x state x size of one scene group, in that order. `sizes` gives the (width, height)
/// pairs of a state, `build` makes its board and `extra` adds a group-specific field to the frame.
///
/// `captures()` renders all groups on one thread, and a function's debug stack frame holds every
/// local of every loop inlined into it. Each group therefore lives in its own function (this one,
/// or a dedicated one when its boards need more than a builder), so adding a group cannot push
/// the test thread's stack past its limit.
#[inline(never)]
fn scene_group(
    styles: &mut Styles,
    states: &[&str],
    sizes: impl Fn(&str) -> Vec<(u16, u16)>,
    build: impl Fn(&str, crate::look::Look, u16, u16) -> App,
    extra: impl Fn(&mut Value, &App),
) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for &state in states {
            for (width, height) in sizes(state) {
                let app = build(state, look, width, height);
                let mut captured = capture_frame(&app, state, look_name, width, height, styles);
                extra(&mut captured, &app);
                frames.push(captured);
            }
        }
    }
    frames
}

/// The Terminal tab of a built-in-host member, with a published screen injected.
///
/// `captures()` is one very large function whose debug stack frame grows with every inlined scene
/// loop, so groups added late live in their own functions to keep the test thread's stack small.
fn terminal_tab_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::terminal_tab::scenes::SCENES,
        |_| grid(&[100, 60], &[36]),
        |scene, look, _, _| crate::board::terminal_tab::scenes::capture_case(scene, look),
        |_, _| {},
    )
}

/// The Settings page's keys group: scrolled to the checklist key, and its editor open.
fn settings_keys_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &["squad-settings-keys", "squad-settings-key-edit"],
        |_| grid(&[100, 60], &[36]),
        |state, look, _, _| capture_case(state, look),
        |_, _| {},
    )
}

#[inline(never)]
fn base_frames(styles: &mut Styles) -> Vec<Value> {
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
            "unread-core-final",
            "unread-core-capture",
            "unread-core-conflict",
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
                    let captured = capture_frame(&app, state, look_name, width, height, styles);
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
    frames
}

fn chart_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &crate::board::overview::chart::host::tests::cases::NAMES,
        |_| grid(&[60, 100, 103, 148, 200], &[51]),
        |state, look, width, _| {
            crate::board::overview::chart::host::tests::cases::board_case(state, look, width)
        },
        |_, _| {},
    )
}

/// All-key Settings and inert parameter previews use the actual board owner.
fn bindings_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &[
            "bindings-list",
            "bindings-punctuation",
            "bindings-preview",
            "bindings-unavailable",
            "bindings-masked",
            "bindings-j-preset",
            "bindings-inventory-scrolled",
        ],
        |_| grid(&[160, 100, 80, 60], &[36, 8]),
        crate::board::settings::tests::host_action::capture_case,
        |_, _| {},
    )
}

/// Idle alert and the squad settings reached from the Workspace menu: additive scenes.
fn settings_menu_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &[
            "idle-alert",
            "idle-alert-overview",
            "settings-menu",
            "settings-menu-pool",
            "squad-settings",
        ],
        |_| grid(&[160, 100, 80, 60], &[36]),
        |state, look, _, _| capture_case(state, look),
        |_, _| {},
    )
}

/// Measure, the right column's summary view: an agent, a squad and the Agents chip.
#[inline(never)]
fn measure_frames(styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for scene in ["measure-agent", "measure-squad", "measure-agents-chip"] {
            for width in [160, 100] {
                let mut app = crate::board::app::workspace::tests::populated_board(1, 24);
                app.view.as_mut().unwrap().look = look;
                let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
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
                frames.push(capture_frame(&app, scene, look_name, width, 51, styles));
            }
        }
    }
    frames
}

/// Search has its own committed indicator and clear hit. These additive
/// scenes keep the existing unfiltered baseline independently frozen.
#[inline(never)]
fn search_frames(styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
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
                        styles,
                    );
                    captured["stripHits"] =
                        json!(format!("{:?}", app.workspace_strip_hits.borrow()));
                    frames.push(captured);
                }
            }
        }
    }
    frames
}

/// Measure's `Exchanges · <range>` section with one, a few and many agents: additive scenes.
#[inline(never)]
fn measure_exchanges_frames(styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
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
                frames.push(capture_frame(&app, scene, look_name, width, 51, styles));
            }
        }
    }
    frames
}

/// Paste attachments and forms share their scene owner with decoded review.
fn paste_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::composer::paste::tests::SCENES,
        |state| {
            grid(
                &[160, 100, 80],
                &[crate::board::composer::paste::tests::capture_height(state)],
            )
        },
        crate::board::composer::paste::tests::capture_case,
        |captured, app| {
            captured["paste"] = crate::board::composer::paste::tests::capture_metadata(app);
        },
    )
}

/// Dropped files become chips in the same composer; the scenes use fixed, synthetic paths.
fn file_chip_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::composer::files::tests::SCENES,
        |state| {
            grid(
                &[160, 100, 80],
                &[if state == "file-tiny" { 8 } else { 36 }],
            )
        },
        crate::board::composer::files::tests::capture_case,
        |captured, app| {
            captured["files"] = crate::board::composer::files::tests::capture_metadata(app);
        },
    )
}

/// Inline quote and answer placement uses the existing paste/file fold owner.
fn quote_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::composer::tests::QUOTE_SCENES,
        |state| {
            grid(
                &[160, 100, 80, 60],
                &[if state == "quote-tiny" { 8 } else { 36 }],
            )
        },
        crate::board::composer::tests::quote_capture_case,
        |captured, app| {
            captured["quotes"] = crate::board::composer::tests::quote_capture_metadata(app);
        },
    )
}

/// A link in the chat opens a small popover; the address is fixed and synthetic.
fn link_popover_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::member_links::popover::SCENES,
        |_| grid(&[160, 100, 60], &[36]),
        crate::board::member_links::popover::capture_case,
        |_, _| {},
    )
}

fn link_file_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::member_links::tests::FILE_SCENES,
        |_| grid(&[160, 100, 80, 60], &[36]),
        crate::board::member_links::tests::capture_file_case,
        |captured, app| {
            captured["references"] =
                json!(crate::board::member_links::tests::capture_file_hits(app));
        },
    )
}

/// The Talk pane's live screen of the selected agent, in each state it can be in.
fn talk_screen_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::conversation::screen::tests::SCENES,
        |_| grid(&[160, 100, 60], &[40]),
        crate::board::conversation::screen::tests::capture_case,
        |_, _| {},
    )
}

/// One loading mark or a retained conversation plus its refresh status,
/// then a single complete publication through the real selected owner.
fn conversation_refresh_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &crate::board::conversation::tests::refresh::SCENES,
        |_| grid(&[160, 100, 80, 60], &[36, 8]),
        crate::board::conversation::tests::refresh::capture_case,
        |_, _| {},
    )
}

/// Exact reported Markdown sources through the live conversation projection.
fn markdown_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &crate::board::conversation::tests::markdown_packet::REPORTED_SCENES,
        |_| grid(&[160, 100, 80, 60], &[36]),
        |scene, look, _, _| {
            crate::board::conversation::tests::markdown_packet::reported_case(scene, look)
        },
        |_, _| {},
    )
}

/// A drag selection and the Selection menu opened below its end.
#[inline(never)]
fn selection_frames(styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for scene in crate::board::conversation::source::scenes::SCENES {
            let app = crate::board::conversation::source::scenes::capture_case(scene, look);
            let (width, height) = crate::board::conversation::source::scenes::size(scene);
            frames.push(capture_frame(&app, scene, look_name, width, height, styles));
        }
    }
    frames
}

fn member_session_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::member_session::tests::SCENES,
        |_| grid(&[160, 100, 80, 60], &[36, 8]),
        |scene, look, _, _| crate::board::member_session::tests::capture_case(scene, look),
        |captured, app| {
            if app.member_session.is_some() {
                captured["session"] = crate::board::member_session::tests::capture_metadata(app);
            }
        },
    )
}

/// The Checklist chip once its read has answered: additive scenes.
fn checklist_chip_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::conversation_checklist::tests::SCENES,
        |_| grid(&[100, 80], &[36]),
        |scene, look, _, _| crate::board::conversation_checklist::tests::capture_case(scene, look),
        |_, _| {},
    )
}

/// The conversation checklist overlay: additive scenes.
#[inline(never)]
fn checklist_overlay_frames(styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for scene in crate::board::conversation_checklist::tests::overlay::LIST_SCENES {
            // The list also paints where its title, host and author give way.
            let widths: &[u16] = if *scene == "notes-list" {
                &[100, 160, 60]
            } else {
                &[100]
            };
            for width in widths {
                let app =
                    crate::board::conversation_checklist::tests::overlay::list_case(scene, look);
                frames.push(capture_frame(&app, scene, look_name, *width, 36, styles));
            }
        }
    }
    frames
}

/// One turn that answered three sends, painted under the last of them: additive scenes.
fn one_turn_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &crate::board::conversation::outgoing::tests::captures::ONE_TURN_SCENES,
        |_| grid(&[100, 80], &[51]),
        |scene, look, _, _| {
            crate::board::conversation::outgoing::tests::captures::one_turn_scene(scene, look, NOW)
        },
        |_, _| {},
    )
}

/// The running versions in the header and Help: additive scenes.
fn versions_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::versions::tests::SCENES,
        |_| grid(&[150, 80], &[36]),
        |scene, look, width, _| crate::board::versions::tests::capture_case(scene, look, width),
        |_, _| {},
    )
}

fn new_member_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::new_member::tests::captures::SCENES,
        |scene| {
            if scene.starts_with("edit-") {
                grid(&[60, 100, 160], &[36, 8])
            } else {
                grid(&[80, 150], &[43, 8])
            }
        },
        |scene, look, _, _| crate::board::new_member::tests::captures::case(scene, look),
        |captured, app| {
            captured["newMember"] = crate::board::new_member::tests::captures::metadata(app);
        },
    )
}

/// The unread count under the pointer: one additive scene, appended so the style table keeps its order.
fn unread_hover_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &["unread-chip-hover"],
        |_| grid(&[160, 100, 80, 60], &[36]),
        |state, look, _, _| capture_case(state, look),
        |_, _| {},
    )
}

/// The `tmt/` mention picker with three matching agents, one chosen.
fn mention_picker_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::mentions::tests::SCENES,
        |_| grid(&[100, 60], &[36]),
        |state, look, _, _| crate::board::mentions::tests::capture_case(state, look),
        |_, _| {},
    )
}

#[inline(never)]
fn checklist_capture_frames(styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for scene in crate::board::conversation_checklist::tests::captures::SCENES {
            for (width, height) in [(160, 36), (120, 40), (100, 32)] {
                let app = crate::board::conversation_checklist::tests::captures::case(scene, look);
                frames.push(capture_frame(&app, scene, look_name, width, height, styles));
            }
        }
        for scene in crate::board::conversation_checklist::tests::captures::ROOM_SCENES {
            for (width, height) in [(160, 36), (100, 36), (80, 36), (60, 36)] {
                let app =
                    crate::board::conversation_checklist::tests::captures::room_case(scene, look);
                let mut captured = capture_frame(&app, scene, look_name, width, height, styles);
                captured["stripHits"] = json!(format!("{:?}", app.workspace_strip_hits.borrow()));
                frames.push(captured);
            }
        }
    }
    frames
}

fn action_needed_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &["action-needed"],
        |_| vec![(160, 36), (120, 40), (100, 32)],
        |_, look, _, _| crate::board::recovery::tests::action_needed_case(look),
        |_, _| {},
    )
}

#[inline(never)]
fn digest_impact_frames(styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for scene in crate::board::digest_dashboard::tests::impact::SCENES {
            for (width, height) in [(160, 50), (120, 40), (100, 32)] {
                let mut app =
                    crate::board::digest_dashboard::tests::impact::capture_case(scene, look);
                if *scene == "impact-basis" {
                    let _ = capture_frame(&app, scene, look_name, width, height, styles);
                    app.choose(crate::board::app::Choice::DigestBasis);
                }
                let mut captured = capture_frame(&app, scene, look_name, width, height, styles);
                captured["stripHits"] = json!(format!("{:?}", app.workspace_strip_hits.borrow()));
                frames.push(captured);
            }
        }
    }
    frames
}

fn answer_paging_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &crate::board::conversation::tests::answer_paging::SCENES,
        |_| grid(&[160, 100, 80, 60], &[36]),
        |scene, look, _, _| {
            crate::board::conversation::tests::answer_paging::capture_case(scene, look)
        },
        |_, _| {},
    )
}

fn checklist_activity_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::conversation_checklist::tests::activity_scenes::SCENES,
        |_| grid(&[160, 100, 80, 60], &[36]),
        |scene, look, _, _| {
            crate::board::conversation_checklist::tests::activity_scenes::capture_case(scene, look)
        },
        |_, _| {},
    )
}

fn timeline_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::conversation::timeline::tests::SCENES,
        |_| grid(&[160, 80], &[50, 8]),
        |scene, look, _, _| crate::board::conversation::timeline::tests::capture_case(scene, look),
        |_, _| {},
    )
}

fn snapshot_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        &[
            "snapshots-generations",
            "snapshots-generation-preview",
            "snapshots-save-latest",
            "snapshots-save-reopened",
        ],
        |_| grid(&[150, 80], &[43]),
        |scene, look, _, _| crate::board::recovery::tests::generation_case(scene, look),
        |_, _| {},
    )
}

/// The quota Details modal with its times in a pinned local zone.
fn quota_details_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        super::limits::QUOTA_SCENES,
        |_| grid(&[100, 40], &[24]),
        |scene, look, _, _| super::limits::quota_case(scene, look),
        |_, _| {},
    )
}

/// The Unread overlay: both scopes, loading and a failed agent read.
fn unread_overlay_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::unread_overlay::tests::SCENES,
        |_| {
            vec![
                (100, 36),
                (60, 36),
                (160, 50),
                (120, 40),
                (100, 32),
                (80, 30),
                (99, 32),
                (100, 8),
            ]
        },
        |scene, look, _, _| crate::board::unread_overlay::tests::capture_case(scene, look),
        |_, _| {},
    )
}

/// Hidden squads as the Agents list's last group, and the scope chip (#5643).
fn visibility_frames(styles: &mut Styles) -> Vec<Value> {
    scene_group(
        styles,
        crate::board::app::workspace::tests::VISIBILITY_SCENES,
        |_| grid(&[160, 100], &[36]),
        |state, look, _, _| {
            crate::board::app::workspace::tests::visibility_capture_case(state, look)
        },
        |_, _| {},
    )
}

fn account_limits_frames(styles: &mut Styles) -> Vec<Value> {
    use crate::board::limits::{
        Current, Provider, Standing,
        observation::{Account, Report},
    };
    scene_group(
        styles,
        &[
            "account-known",
            "account-waiting",
            "account-expired",
            "account-short",
            "account-multi",
            "account-details",
        ],
        |_| grid(&[160, 120, 100, 80], &[40, 8]),
        |state, look, _, _| {
            let mut app = metered(look);
            let account = Account {
                key: "0123456789ab".into(),
                label: "work".into(),
            };
            let mut windows =
                vec![json!({"windowMinutes":300,"usedPercent":8.0,"resetsAtMs":NOW+3_600_000})];
            if state != "account-short" {
                windows.push(
                    json!({"windowMinutes":10080,"usedPercent":60.0,"resetsAtMs":NOW+86_400_000}),
                );
            }
            let report=Report::read(&json!({"observedAtMs":NOW-60_000,"account":{"key":account.key,"label":account.label},"windows":windows})).unwrap();
            let current = match state {
                "account-waiting" | "account-details" => Current::Waiting,
                "account-expired" => Current::Expired,
                _ => Current::Reported(report),
            };
            let mut rows = vec![Standing {
                provider: Provider::named("claude").with_account(&account),
                current: Some(current),
                latest: None,
                burn: None,
                trend: None,
                rates: Default::default(),
            }];
            if state == "account-multi" || state == "account-details" {
                rows[0].provider = rows[0].provider.clone().showing_account(true);
                rows.push(Standing {
                    provider: Provider::named("claude")
                        .with_account(&Account {
                            key: "abcdef012345".into(),
                            label: "home".into(),
                        })
                        .showing_account(true),
                    current: Some(Current::Expired),
                    latest: None,
                    burn: None,
                    trend: None,
                    rates: Default::default(),
                });
            }
            app.set_limits(rows);
            if state == "account-short" {
                app.usage_chart
                    .borrow_mut()
                    .restore_quota_window(crate::board::limits::QuotaWindow::Short);
            }
            if state == "account-details" {
                app.open_quota_details();
            }
            app
        },
        |_, _| {},
    )
}

const NOW: u64 = 1_791_600_000_000;

fn captures() -> Value {
    crate::status::with_now_ms(NOW, || {
        let mut styles = Styles::default();
        let mut frames = Vec::new();
        frames.extend(base_frames(&mut styles));
        frames.extend(chart_frames(&mut styles));
        frames.extend(bindings_frames(&mut styles));
        frames.extend(settings_menu_frames(&mut styles));
        frames.extend(measure_frames(&mut styles));
        frames.extend(search_frames(&mut styles));
        frames.extend(measure_exchanges_frames(&mut styles));
        frames.extend(paste_frames(&mut styles));
        frames.extend(file_chip_frames(&mut styles));
        frames.extend(quote_frames(&mut styles));
        frames.extend(link_popover_frames(&mut styles));
        frames.extend(link_file_frames(&mut styles));
        frames.extend(talk_screen_frames(&mut styles));
        frames.extend(conversation_refresh_frames(&mut styles));
        frames.extend(markdown_frames(&mut styles));
        frames.extend(selection_frames(&mut styles));
        frames.extend(member_session_frames(&mut styles));
        frames.extend(checklist_chip_frames(&mut styles));
        frames.extend(add_note_frames(&mut styles));
        frames.extend(checklist_overlay_frames(&mut styles));
        frames.extend(one_turn_frames(&mut styles));
        frames.extend(versions_frames(&mut styles));
        frames.extend(new_member_frames(&mut styles));
        frames.extend(switch_frames(
            crate::board::switch_tool::tests::captures::SCENES,
            &mut styles,
        ));
        frames.extend(idle_alert_frames(&mut styles));
        frames.extend(crud_frames(
            crate::board::app::workspace::tests::review_modal::CRUD_SCENES,
            &mut styles,
        ));
        frames.extend(unread_hover_frames(&mut styles));
        frames.extend(checklist_capture_frames(&mut styles));
        frames.extend(action_needed_frames(&mut styles));
        frames.extend(digest_impact_frames(&mut styles));
        frames.extend(answer_paging_frames(&mut styles));
        frames.extend(checklist_activity_frames(&mut styles));
        frames.extend(timeline_frames(&mut styles));
        frames.extend(snapshot_frames(&mut styles));
        frames.extend(terminal_tab_frames(&mut styles));
        frames.extend(settings_keys_frames(&mut styles));
        frames.extend(quota_details_frames(&mut styles));
        frames.extend(unread_overlay_frames(&mut styles));
        frames.extend(account_limits_frames(&mut styles));
        frames.extend(mention_picker_frames(&mut styles));
        frames.extend(visibility_frames(&mut styles));
        let styles = styles
            .0
            .into_iter()
            .map(|(text, id)| (id, text))
            .collect::<BTreeMap<_, _>>();
        json!({"styles": styles, "frames": frames})
    })
}

fn switch_frames(scenes: &[&str], styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for scene in scenes {
            for width in [160, 150, 100, 80, 60] {
                for height in [43, 8] {
                    let app = crate::board::switch_tool::tests::captures::case(scene, look);
                    let mut captured = capture_frame(&app, scene, look_name, width, height, styles);
                    captured["switchTool"] =
                        crate::board::switch_tool::tests::captures::metadata(&app);
                    frames.push(captured);
                }
            }
        }
    }
    frames
}

#[test]
#[ignore = "read-only switch revision inspection; never records frozen bytes"]
fn dump_switch_revision_oracle() {
    let value = crate::status::with_now_ms(1_791_600_000_000, || {
        let mut styles = Styles::default();
        let scenes = crate::board::switch_tool::tests::captures::SCENES
            .iter()
            .copied()
            .filter(|s| *s != "switch-tool-review")
            .collect::<Vec<_>>();
        let frames = switch_frames(&scenes, &mut styles);
        let styles = styles
            .0
            .into_iter()
            .map(|(text, id)| (id, text))
            .collect::<BTreeMap<_, _>>();
        json!({"styles":styles,"frames":frames})
    });
    fs::write(
        std::env::var("HOME_ORACLE_OUT").expect("task-owned correction path"),
        serde_json::to_string_pretty(&value).unwrap() + "\n",
    )
    .unwrap();
}

fn add_note_frames(styles: &mut Styles) -> Vec<Value> {
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for scene in crate::board::conversation_checklist::tests::overlay::ADD_SCENES {
            for width in [160, 100, 80, 60] {
                let app =
                    crate::board::conversation_checklist::tests::overlay::add_case(scene, look);
                frames.push(capture_frame(&app, scene, look_name, width, 36, styles));
            }
        }
    }
    frames
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
#[ignore = "exports only the Add to checklist forms to HOME_ORACLE_OUT; never records fixtures"]
fn dump_add_to_notes_oracle() {
    let path = std::env::var("HOME_ORACLE_OUT").expect("task-owned evidence path");
    let captured = crate::status::with_now_ms(1_791_600_000_000, || {
        let mut styles = Styles::default();
        let frames = add_note_frames(&mut styles);
        let styles = styles
            .0
            .into_iter()
            .map(|(text, id)| (id, text))
            .collect::<BTreeMap<_, _>>();
        json!({"styles": styles, "frames": frames})
    });
    fs::write(
        path,
        serde_json::to_string_pretty(&captured).unwrap() + "\n",
    )
    .unwrap();
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
                "crud-moving",
                "crud-moving-pool",
                "close-member-confirm",
                "close-member-refused",
                "close-member-signalled",
                "close-member-unavailable",
                "close-member-running",
                "close-member-pending",
                "close-member-pane-kept",
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

// Intern repeated painted rows. Existing dictionary IDs remain stable across
// recordings; unused rows stay reserved, and new rows append in first-seen order.
fn encode_oracle(mut oracle: Value, previous: Option<&Value>) -> Value {
    let mut lines = prior_dictionary(previous, "line_dictionary");
    let mut styles = prior_dictionary(previous, "style_row_dictionary");
    let mut line_ids = dictionary_ids(&lines);
    let mut style_ids = dictionary_ids(&styles);
    for frame in oracle["frames"].as_array_mut().expect("oracle frames") {
        intern_rows(&mut frame["lines"], &mut line_ids, &mut lines);
        intern_rows(&mut frame["styles"], &mut style_ids, &mut styles);
    }
    oracle["line_dictionary"] = json!(lines);
    oracle["style_row_dictionary"] = json!(styles);
    oracle
}

fn prior_dictionary(previous: Option<&Value>, name: &str) -> Vec<String> {
    previous
        .and_then(|value| value.get(name))
        .map(|value| {
            value
                .as_array()
                .expect("oracle dictionary array")
                .iter()
                .map(|row| row.as_str().expect("oracle dictionary text").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn dictionary_ids(dictionary: &[String]) -> HashMap<String, usize> {
    let mut ids = HashMap::new();
    for (id, row) in dictionary.iter().enumerate() {
        assert!(
            ids.insert(row.clone(), id).is_none(),
            "duplicate oracle row"
        );
    }
    ids
}

fn intern_rows(rows: &mut Value, ids: &mut HashMap<String, usize>, dictionary: &mut Vec<String>) {
    for row in rows.as_array_mut().expect("oracle rows") {
        let text = row.as_str().expect("oracle row text");
        let id = match ids.get(text) {
            Some(id) => *id,
            None => {
                let id = dictionary.len();
                dictionary.push(text.to_owned());
                ids.insert(text.to_owned(), id);
                id
            }
        };
        *row = json!(id);
    }
}

fn decode_oracle(mut oracle: Value) -> Value {
    let object = oracle.as_object_mut().expect("oracle object");
    let lines = object.remove("line_dictionary").expect("line dictionary");
    let styles = object
        .remove("style_row_dictionary")
        .expect("style row dictionary");
    let lines = lines.as_array().expect("line dictionary array");
    let styles = styles.as_array().expect("style dictionary array");
    for frame in oracle["frames"].as_array_mut().expect("oracle frames") {
        expand_rows(&mut frame["lines"], lines);
        expand_rows(&mut frame["styles"], styles);
    }
    oracle
}

fn expand_rows(rows: &mut Value, dictionary: &[Value]) {
    for row in rows.as_array_mut().expect("oracle row indexes") {
        let id = row.as_u64().expect("oracle row index");
        let id = usize::try_from(id).expect("oracle row index fits usize");
        *row = dictionary
            .get(id)
            .expect("oracle row index in bounds")
            .clone();
        assert!(row.is_string(), "oracle dictionary entry must be a string");
    }
}

#[test]
fn oracle_format_b_interns_in_first_seen_order_and_round_trips() {
    let expanded = json!({
        "styles": {"s0": "plain"},
        "frames": [
            {"lines": ["b", "a", "b"], "styles": ["x", "y", "x"], "width": 2},
            {"lines": ["a", "c"], "styles": ["y", "z"], "width": 1}
        ]
    });
    let encoded = encode_oracle(expanded.clone(), None);
    assert_eq!(encoded["line_dictionary"], json!(["b", "a", "c"]));
    assert_eq!(encoded["style_row_dictionary"], json!(["x", "y", "z"]));
    assert_eq!(encoded["frames"][1]["lines"], json!([1, 2]));
    assert_eq!(encoded, encode_oracle(expanded.clone(), None));
    assert_eq!(decode_oracle(encoded), expanded);
}

#[test]
fn oracle_format_b_preserves_ids_and_keeps_unused_rows() {
    let old =
        json!({"frames": [{"lines": ["old", "keep"], "styles": ["old-style", "keep-style"]}]});
    let before = encode_oracle(old, None);
    let changed =
        json!({"frames": [{"lines": ["new", "keep"], "styles": ["new-style", "keep-style"]}]});
    let after = encode_oracle(changed.clone(), Some(&before));
    assert_eq!(after["line_dictionary"], json!(["old", "keep", "new"]));
    assert_eq!(
        after["style_row_dictionary"],
        json!(["old-style", "keep-style", "new-style"])
    );
    assert_eq!(after["frames"][0]["lines"], json!([2, 1]));
    assert_eq!(after["frames"][0]["styles"], json!([2, 1]));
    assert_eq!(decode_oracle(after.clone()), changed);
    let compact = encode_oracle(decode_oracle(after), None);
    assert_eq!(compact["line_dictionary"], json!(["new", "keep"]));
    assert_eq!(
        compact["style_row_dictionary"],
        json!(["new-style", "keep-style"])
    );
    assert_eq!(decode_oracle(compact), changed);
}

#[test]
#[should_panic(expected = "oracle row index in bounds")]
fn oracle_format_b_rejects_invalid_row_index() {
    decode_oracle(json!({
        "line_dictionary": ["one"], "style_row_dictionary": [],
        "frames": [{"lines": [1], "styles": []}]
    }));
}

#[test]
#[ignore = "compares the re-encoding with the approved expanded oracle at HOME_ORACLE_PREVIOUS"]
fn oracle_format_b_matches_approved_expanded_oracle() {
    let path = std::env::var("HOME_ORACLE_PREVIOUS").expect("approved expanded oracle path");
    let previous: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let encoded: Value = serde_json::from_str(include_str!("oracle.json")).unwrap();
    assert_eq!(decode_oracle(encoded.clone()), previous);
    assert_eq!(encoded, encode_oracle(previous, Some(&encoded)));
}

#[test]
fn home_oracle_keeps_every_cell_style_and_hit_without_stacked_rules() {
    // Nextest runs each test in its own process, so both checks must share this
    // local capture rather than a lazy value shared between separate tests.
    let captured = captures();
    assert_eq!(
        captured,
        decode_oracle(serde_json::from_str::<Value>(include_str!("oracle.json")).unwrap())
    );
    assert_no_home_frame_stacks_two_rules(&captured);
}

fn assert_no_home_frame_stacks_two_rules(captured: &Value) {
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
        serde_json::to_string_pretty(&encode_oracle(
            captures(),
            Some(&serde_json::from_str::<Value>(include_str!("oracle.json")).unwrap()),
        ))
        .unwrap()
            + "\n",
    )
    .unwrap();
}

#[test]
#[ignore = "compacts unused HOME oracle rows; separate commit after decoded-equality review"]
fn compact_home_oracle() {
    let encoded = serde_json::from_str::<Value>(include_str!("oracle.json")).unwrap();
    let compact = encode_oracle(decode_oracle(encoded), None);
    fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/board/home/tests/oracle.json"
        ),
        serde_json::to_string_pretty(&compact).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
#[ignore = "decodes the frozen HOME oracle to $HOME_ORACLE_OUT without recording"]
fn dump_frozen_home_oracle() {
    let path = std::env::var("HOME_ORACLE_OUT").expect("task-owned evidence path");
    let encoded = serde_json::from_str::<Value>(include_str!("oracle.json")).unwrap();
    fs::write(
        path,
        serde_json::to_string_pretty(&decode_oracle(encoded)).unwrap() + "\n",
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
