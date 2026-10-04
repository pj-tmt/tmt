//! The decoded HOME oracle: every cell's symbol and style, every hit, the row
//! starts and the input band of whole frames, plus the squads table and the
//! header usage line, at the breakpoint boundaries in three looks. It was
//! generated from the painter that built HOME from ratatui spans, and the
//! markup migration (#1723) must keep it unchanged apart from the changes the
//! PR lists. The seams that feed it (`render_frame`, `tiles::paint`,
//! `paint::usage`) are glue only; the fixture does not depend on them.
use super::interaction::{board, keys, press};
use super::*;
use crate::board::{
    app::{App, Compose, RowFeedback, RowTarget},
    cronboard::{TEST_NOW, test_cron, test_view},
    home::paint,
    home_leads::{Kind, LeadPreview},
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, crossterm::event::KeyCode::*};
use std::collections::BTreeMap;
use tmt_squad::cron::ClockStatus;

/// Tall enough to hold the whole HOME stream of the rich model at every width.
const HEIGHT: u16 = 60;
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
fn rich() -> App {
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
    let mut job = test_view("lead-a", "merge queue sweep", Some(TEST_NOW + 3_600_000));
    job.job.squad = "alpha".into();
    app.cron
        .replace(Ok(test_cron(vec![job], ClockStatus::NoClock)));
    app.select(0);
    app
}

fn select(app: &mut App, section: &str, squad: &str) {
    let index = app
        .home_entries()
        .iter()
        .position(|entry| entry.target.section == section && entry.target.squad == squad)
        .unwrap_or_else(|| panic!("no {section}/{squad} entry"));
    app.select(index);
}

fn sent_on(app: &mut App, section: &str, squad: &str) {
    select(app, section, squad);
    let target = app.home_entries()[app.selected].target.clone();
    app.sent = Some(RowFeedback {
        target: RowTarget::Home(target),
        home: None,
    });
}

/// Each state names the keys and selection that produce it.
const STATES: &[&str] = &[
    "base",
    "lead-selected",
    "squad-selected",
    "replies-hidden",
    "cron-selected",
    "all-leads-selected",
    "composer-needs-you",
    "composer-lead",
    "composer-squad",
    "sent-needs-you",
    "sent-lead",
    "sent-squad",
    "read-lead",
    "searching",
    "no-user",
];

/// The widths each state is captured at in the `tmt` look: every boundary for
/// the states that change with width, the narrowest and widest for the rest.
fn widths(state: &str) -> &'static [u16] {
    match state {
        "base" | "replies-hidden" => &[160, 140, 139, 120, 100, 99, 80, 79],
        "squad-selected" => &[160, 100, 99, 80],
        _ => &[160, 80],
    }
}

/// The other looks repeat the states whose styles differ the most.
const LOOK_STATES: &[&str] = &["base", "lead-selected", "composer-lead", "read-lead"];

fn state(name: &str) -> App {
    let mut app = rich();
    match name {
        "base" => (),
        "lead-selected" => select(&mut app, "leads", "beta"),
        "squad-selected" => select(&mut app, "squads", "gamma"),
        "replies-hidden" => {
            press(&mut app, Char('t'));
            select(&mut app, "leads", "alpha");
        }
        "cron-selected" => {
            let index = app
                .home_entries()
                .iter()
                .position(|entry| entry.target.section == super::super::CRON)
                .unwrap();
            app.select(index);
        }
        "all-leads-selected" => {
            let index = app
                .home_entries()
                .iter()
                .position(|entry| entry.target.section == super::super::ALL_LEADS)
                .unwrap();
            app.select(index);
        }
        "composer-needs-you" => {
            select(&mut app, "needs-you", "alpha");
            press(&mut app, Char('a'));
        }
        "composer-lead" => {
            select(&mut app, "leads", "alpha");
            press(&mut app, Char('a'));
        }
        "composer-squad" => {
            select(&mut app, "squads", "beta");
            press(&mut app, Char('a'));
        }
        "sent-needs-you" => sent_on(&mut app, "needs-you", "alpha"),
        "sent-lead" => sent_on(&mut app, "leads", "beta"),
        "sent-squad" => sent_on(&mut app, "squads", "gamma"),
        "read-lead" => {
            select(&mut app, "leads", "alpha");
            press(&mut app, Char('e'));
            assert!(matches!(
                app.input.as_ref().map(|input| &input.compose),
                Some(Compose::ReadLead { .. })
            ));
        }
        "searching" => {
            keys(&mut app, &[Char('/'), Char('l'), Char('e'), Char('a')]);
        }
        "no-user" => app.view.as_mut().unwrap().me = None,
        other => panic!("unknown state {other}"),
    }
    app
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

fn looks() -> [(&'static str, crate::look::Look); 3] {
    use tmt_cli_style::{Depth, Theme, theme::Base};
    let look = |base, depth| crate::look::Look {
        theme: Theme::new(Base::parse(base).unwrap()),
        depth,
    };
    [
        ("tmt", look("tmt", Depth::TrueColor)),
        ("tmt-light", look("tmt-light", Depth::TrueColor)),
        ("NO_COLOR", look("tmt", Depth::None)),
    ]
}

fn frame(
    app: &App,
    width: u16,
    usage: Option<ratatui::text::Line<'static>>,
    styles: &mut Styles,
) -> Value {
    app.hits.borrow_mut().clear();
    let mut terminal = Terminal::new(TestBackend::new(width, HEIGHT)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render_frame(frame, app, usage))
        .unwrap();
    let mut captured = buffer_json(terminal.backend().buffer(), styles);
    captured["hits"] = json!(
        app.hits
            .borrow()
            .iter()
            .map(|hit| format!("{}:{}+{}#{}", hit.y, hit.x, hit.width, hit.row))
            .collect::<Vec<_>>()
            .join(" ")
    );
    captured["starts"] = json!(*app.row_starts.borrow());
    captured["tabs"] = json!(format!("{:?}", app.tab_hits.borrow()));
    captured["input"] = json!(format!("{:?}", app.input_band.get()));
    captured
}

fn usage_line(app: &App, width: u16) -> Option<ratatui::text::Line<'static>> {
    paint::usage(&super::interaction::header_usage(), width, app.look())
}

fn tiles(width: u16, look: crate::look::Look, styles: &mut Styles) -> Vec<Value> {
    use crate::board::{
        app::{HomeUsage, UsageShare},
        home::tiles::{self, TileItem},
        rate::Reading,
    };
    let counts = |waiting, blocked, review, working, idle, members| Counts {
        members,
        waiting,
        blocked,
        review,
        working,
        idle,
    };
    let squads = [
        SquadLine {
            squad: "alpha".into(),
            lead: Some(json!({"name": "lead-a"})),
            counts: counts(2, 1, 0, 0, 0, 4),
            members: counts(2, 1, 0, 0, 0, 3),
        },
        SquadLine {
            squad: "delta-with-a-long-squad-name".into(),
            lead: Some(json!({"name": "lead-delta-with-a-long-name"})),
            counts: counts(0, 1, 1, 3, 2, 7),
            members: counts(0, 1, 1, 3, 2, 7),
        },
        SquadLine {
            squad: "quiet".into(),
            lead: None,
            counts: counts(0, 0, 0, 0, 0, 0),
            members: counts(0, 0, 0, 0, 0, 0),
        },
        SquadLine {
            squad: "custom".into(),
            lead: Some(json!({"name": "lead-c"})),
            counts: counts(0, 0, 0, 1, 0, 5),
            members: counts(0, 0, 0, 1, 0, 5),
        },
    ];
    let reading = |tokens, partial| {
        Some(Reading {
            tokens,
            partial,
            span: 60_000,
        })
    };
    let uniform = crate::config::TokenWindow::DEFAULTS;
    let other = ["5m", "1h", "24h"].map(|text| crate::config::TokenWindow::parse(text).unwrap());
    let usage = |windows, lead, share| HomeUsage {
        lead_model: Some("claude-opus-4-6"),
        windows,
        lead,
        squad: [None; 3],
        share,
    };
    let share = |fraction, partial| Some(UsageShare { fraction, partial });
    let cases: Vec<(&str, Vec<Option<HomeUsage<'static>>>)> = vec![
        ("none", vec![None, None, None, None]),
        (
            "uniform",
            vec![
                Some(usage(
                    uniform,
                    [
                        reading(1_000, false),
                        reading(2_000_000, true),
                        reading(0, false),
                    ],
                    share(0.375, true),
                )),
                Some(usage(
                    uniform,
                    [None, reading(1_500, false), reading(9_000, false)],
                    None,
                )),
                Some(usage(uniform, [None; 3], None)),
                None,
            ],
        ),
        (
            "mixed",
            vec![
                Some(usage(
                    uniform,
                    [
                        reading(1_000, false),
                        reading(2_000, false),
                        reading(3_000, false),
                    ],
                    share(0.5, false),
                )),
                Some(usage(
                    other,
                    [
                        reading(4_000, true),
                        reading(5_000, false),
                        reading(6_000, false),
                    ],
                    share(0.25, false),
                )),
                None,
                None,
            ],
        ),
    ];
    let mut frames = Vec::new();
    for (name, usages) in cases {
        let items = squads
            .iter()
            .zip(usages)
            .map(|(squad, usage)| TileItem {
                squad,
                members: &squad.members,
                lead_model: Some("claude-opus-4-6"),
                usage,
            })
            .collect::<Vec<_>>();
        for selected in [None, Some(1)] {
            let painted = tiles::paint(&items, width, look, selected);
            let mut buffer = Buffer::empty(ratatui::layout::Rect::new(
                0,
                0,
                width,
                painted.lines.len() as u16,
            ));
            for (y, line) in painted.lines.iter().enumerate() {
                tmt_tui::components::strip::paint_left(
                    &mut buffer,
                    ratatui::layout::Rect::new(0, y as u16, width, 1),
                    line.clone(),
                );
            }
            let mut captured = buffer_json(&buffer, styles);
            captured["case"] = json!(name);
            captured["selected"] = json!(selected);
            captured["width"] = json!(width);
            captured["legend"] = json!(tiles::legend(&items, width));
            captured["regions"] = json!(format!("{:?}", painted.regions));
            frames.push(captured);
        }
    }
    frames
}

fn captures() -> Value {
    let mut styles = Styles::default();
    let mut frames = Vec::new();
    for (look_name, look) in looks() {
        for name in STATES {
            let widths: &[u16] = match look_name {
                "tmt" => widths(name),
                _ if LOOK_STATES.contains(name) => &[160, 80],
                _ => &[],
            };
            for &width in widths {
                let mut app = state(name);
                app.view.as_mut().unwrap().look = look;
                let mut frame = frame(&app, width, None, &mut styles);
                frame["state"] = json!(name);
                frame["look"] = json!(look_name);
                frame["width"] = json!(width);
                frames.push(frame);
            }
        }
        // The header usage row and the key line's own widths.
        for width in if look_name == "tmt" {
            &[160, 140, 139, 100, 99, 80][..]
        } else {
            &[160, 80][..]
        } {
            let width = *width;
            let mut app = state("base");
            app.view.as_mut().unwrap().look = look;
            let line = usage_line(&app, width);
            let mut frame = frame(&app, width, line, &mut styles);
            frame["state"] = json!("header-usage");
            frame["look"] = json!(look_name);
            frame["width"] = json!(width);
            frames.push(frame);
        }
    }
    let tmt = looks()[0].1;
    let mut table = Vec::new();
    for width in [160, 140, 139, 120, 100, 99, 80] {
        table.extend(tiles(width, tmt, &mut styles));
    }
    let hints = [
        160usize, 140, 139, 118, 117, 110, 109, 100, 99, 95, 94, 86, 85, 80, 79, 55, 20,
    ]
    .into_iter()
    .flat_map(|width| [false, true].map(|cron| (width, cron, paint::hints(width, cron))))
    .map(|(width, cron, text)| json!({"width": width, "cron": cron, "text": text}))
    .collect::<Vec<_>>();
    let styles = styles
        .0
        .into_iter()
        .map(|(text, id)| (id, text))
        .collect::<BTreeMap<_, _>>();
    json!({"styles": styles, "frames": frames, "tiles": table, "hints": hints})
}

#[test]
fn home_oracle_keeps_every_cell_style_and_hit() {
    assert_eq!(
        captures(),
        serde_json::from_str::<Value>(include_str!("oracle.json")).unwrap()
    );
}

#[test]
#[ignore = "regenerates the HOME oracle; an own commit approved by the Squad lead"]
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
#[ignore = "writes the current captures to $HOME_ORACLE_OUT, for diffing against oracle.json"]
fn dump_home_oracle() {
    fs::write(
        std::env::var("HOME_ORACLE_OUT").expect("HOME_ORACLE_OUT"),
        serde_json::to_string_pretty(&captures()).unwrap() + "\n",
    )
    .unwrap();
}
