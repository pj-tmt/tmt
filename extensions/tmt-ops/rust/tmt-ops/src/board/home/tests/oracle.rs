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

fn looks() -> [(&'static str, crate::look::Look); 4] {
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

fn captures() -> Value {
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        let mut styles = Styles::default();
        let mut frames = Vec::new();
        for (look_name, look) in looks() {
            for state in [
                "overview",
                "agents",
                "digest",
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
                "header-usage",
            ] {
                for width in [160, 100, 80, 60] {
                    for height in [36, 8] {
                        let app = if state == "header-usage" {
                            metered(look)
                        } else {
                            capture_case(state, look)
                        };
                        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                        let mut captured = buffer_json(terminal.backend().buffer(), &mut styles);
                        captured["state"] = json!(state);
                        captured["look"] = json!(look_name);
                        captured["width"] = json!(width);
                        captured["height"] = json!(height);
                        captured["hits"] = json!(format!("{:?}", app.hits.borrow()));
                        captured["actions"] =
                            json!(format!("{:?}", app.workspace_action_hits.borrow()));
                        captured["tabs"] = json!(format!("{:?}", app.tab_hits.borrow()));
                        captured["starts"] = json!(*app.row_starts.borrow());
                        captured["selected"] = json!(format!("{:?}", app.workspace.selected));
                        captured["selectionState"] =
                            json!(format!("{:?}", app.workspace_selection_state()));
                        captured["focus"] = json!(format!("{:?}", app.focused_pane()));
                        captured["input"] = json!(format!("{:?}", app.input_band.get()));
                        frames.push(captured);
                    }
                }
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
            .map(|line| line.as_str().unwrap().to_owned())
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
