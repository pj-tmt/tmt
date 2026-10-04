//! Reproducible per-frame timing of the board at 160x50 (not a CI gate):
//!
//! ```text
//! CARGO_BUILD_JOBS=2 cargo test --release -p tmt-squad frame_timing -- --ignored --nocapture
//! ```
//!
//! Each scene (crew rows, team rows, home; the token meter is on in every scene)
//! is drawn through `Terminal::draw` on a `TestBackend`, so ratatui's buffer diff
//! is part of the cost. The surface split times the phases `view::render` runs, in
//! its order; `render_replica_matches_render` keeps the replica honest. Pane
//! painters, the rows scene and the outlines are timed again on their own, and the
//! microbench times `strip::paint_*` alone. Numbers vary with the machine: compare
//! runs from one machine and one build.
use super::*;
use crate::board::{
    home::{Age, AgeSource, Counts, Home, MemberRow, MemberSection, SquadLine},
    meter::Meter,
    rate::tests::input,
};
use crate::config::{Config, Layout, TokenRate};
use ratatui::{buffer::Buffer, style::Style, text::Span};
use std::time::{Duration, Instant};
use tmt_tui::components::{Outline, strip};

const WIDTH: u16 = 160;
const HEIGHT: u16 = 50;
const FRAMES: usize = 200;
const WARMUP: usize = 20;
/// The cadence under discussion for an animated token counter.
const TARGET_HZ: f64 = 10.0;

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e6
}

#[derive(Default)]
struct Samples(Vec<Duration>);
impl Samples {
    fn push(&mut self, sample: Duration) {
        self.0.push(sample);
    }
    fn at(&self, fraction: f64) -> f64 {
        let mut sorted = self.0.clone();
        sorted.sort();
        micros(sorted[((sorted.len() - 1) as f64 * fraction).round() as usize])
    }
    fn median(&self) -> f64 {
        self.at(0.5)
    }
}

/// Records the time since the previous mark under a phase name.
struct Lap {
    last: Instant,
    laps: Vec<(&'static str, Duration)>,
}
impl Lap {
    fn start() -> Self {
        Self {
            last: Instant::now(),
            laps: Vec::new(),
        }
    }
    fn mark(&mut self, phase: &'static str) {
        let now = Instant::now();
        self.laps.push((phase, now - self.last));
        self.last = now;
    }
}

/// The phases of `view::render`, in its order. Keep it in step with `render`.
fn render_replica(frame: &mut Frame, app: &App, lap: &mut Lap) {
    let look = app.look();
    app.input_band.set(None);
    app.hits.borrow_mut().clear();
    app.note_hits.borrow_mut().clear();
    app.link_hits.borrow_mut().clear();
    app.row_starts.borrow_mut().clear();
    app.tab_hits.borrow_mut().clear();
    app.unpicked_hit.set(None);
    app.title_hits.borrow_mut().clear();
    app.jobs_area.set(Rect::default());
    app.scrolls.begin_frame();
    let [tabs, summary, meter_status, body, footer] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Length(u16::from(header::meter_enabled(app))),
        ratatui::layout::Constraint::Min(1),
        ratatui::layout::Constraint::Length(1),
    ])
    .areas(frame.area());
    lap.mark("frame state reset");
    strip::paint_left(
        frame.buffer_mut(),
        tabs,
        tabs::paint(app, tabs),
        &look.theme,
        look.depth,
    );
    lap.mark("tab line strip");
    let summary_text = app
        .view
        .as_ref()
        .filter(|_| !app.loading())
        .and_then(|view| view.home.as_ref())
        .map_or_else(
            || header::summary_line(app),
            |home| crate::board::home::summary(home, summary.width, look),
        );
    let summary_area = header::meter_region(app, summary).map_or(summary, |(meter, _)| Rect {
        width: meter.x.saturating_sub(summary.x).saturating_sub(2),
        ..summary
    });
    strip::paint_left(
        frame.buffer_mut(),
        summary_area,
        summary_text,
        &look.theme,
        look.depth,
    );
    lap.mark("summary strip");
    header::render_meter(frame, app, summary);
    header::render_meter_status(frame, app, meter_status);
    lap.mark("meter strips");
    panes::render_body(frame, app, body);
    lap.mark("body (rows scene, outlines, pane strips, home)");
    waiting::inline_prompt(frame, app, body);
    footer::render(frame, app, footer, look);
    lap.mark("footer strip");
    overlays::render(frame, app, body, look);
    waiting::prompt(frame, app, body);
    lap.mark("overlays and prompts");
}

fn meter(app: &mut App) {
    let now = Instant::now();
    let settings = TokenRate {
        enabled: true,
        reduced_motion: true,
        ..Default::default()
    };
    let mut meter = Meter::new(settings, &input(100), now);
    meter.sample(Ok(&input(200)), now + Duration::from_secs(10));
    meter.tick(now + Duration::from_secs(11));
    app.meter = Some(meter);
}

/// Crew or team rows with every column of the preset, a lead notebook and the meter.
fn member_board(layout: Layout) -> App {
    let states = ["working", "blocked", "review", "idle", "working", "working"];
    let section = |title: &str, count: usize, offset: usize| {
        let rows: Vec<Value> = (0..count)
            .map(|index| {
                let n = offset + index;
                let state = states[n % states.len()];
                let task = format!("task {n}: rotate session tokens and update the handbook pages");
                row(
                    &format!("member-{n:02}"),
                    state,
                    &task,
                    json!({"fields": {
                        "state": state, "task": task, "model": "test-model", "ctx": "487k",
                        "pr_state": format!("#{n} OPEN"),
                        "pr_link": format!("https://github.com/pj-tmt/tmt/pull/{}", 1000 + n),
                    }, "pending": if state == "blocked" { json!("approve") } else { Value::Null }}),
                )
            })
            .collect();
        json!({"title": title, "rows": rows})
    };
    let mut app = board(json!([
        section("Needs me", 4, 0),
        section("Working", 12, 4),
        section("Everyone", 10, 16)
    ]));
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/markup-parity.toml");
    let config = Config::read(path).unwrap();
    let view = app.view.as_mut().unwrap();
    view.rows = config.rows(layout.as_str()).unwrap();
    view.board = config.board(layout.as_str()).unwrap();
    view.bindings = config.bindings(true, &view.board.panes).unwrap();
    view.document["squad"]["layout"] = json!(layout.as_str());
    let projected = view.rows.value();
    view.document["columns"] = projected["columns"].clone();
    view.document["lines"] = projected["lines"].clone();
    view.notes = Notes::Text(
        "# Notebook\n\nLead notes for the squad.\n\n- ship the handbook batch\n- review the open PRs\n- keep the glossary\n"
            .repeat(6),
    );
    meter(&mut app);
    app.set_body_width(WIDTH);
    app
}

/// Four squads, a needs-you and a blocked section, and squad tiles.
fn home_board() -> App {
    let names = ["product", "reviews", "infra", "colab"];
    let member = |squad: &str, n: usize, state: &str| {
        json!({"id": format!("{squad}-{n}"), "name": format!("{squad}-member-{n}"), "squad": squad,
            "state": state, "pending": null, "fields": {"state": state}, "waitingOnYou": [],
            "staleness": {}})
    };
    let section = |key: &str, state: &str, per_squad: usize, request: bool| MemberSection {
        key: key.into(),
        rows: names
            .iter()
            .flat_map(|squad| {
                (0..per_squad).map(move |n| MemberRow {
                    squad: (*squad).into(),
                    member: member(squad, n, state),
                    lead: Some(format!("{squad}-lead")),
                    age: Some(Age {
                        source: if request {
                            AgeSource::Request
                        } else {
                            AgeSource::Observed
                        },
                        since_ms: crate::status::now_ms().saturating_sub(720_000 * (n as u64 + 1)),
                    }),
                })
            })
            .collect(),
    };
    let counts = |members| Counts {
        members,
        waiting: 2,
        blocked: 1,
        review: 1,
        working: members.saturating_sub(5),
        idle: 1,
    };
    let home = Home {
        windows: crate::config::TokenWindow::DEFAULTS,
        summary: counts(names.len() * 8),
        sections: vec![
            section("needs-you", "working", 2, true),
            section("blocked", "blocked", 1, false),
        ],
        squads: names
            .iter()
            .map(|squad| SquadLine {
                squad: (*squad).into(),
                lead: Some(json!({"id": format!("{squad}-lead"), "name": format!("{squad}-lead")})),
                counts: counts(8),
                members: counts(7),
            })
            .collect(),
        failures: Vec::new(),
        incomplete: false,
    };
    let mut app = board(json!([]));
    let view = app.view.as_mut().unwrap();
    view.home = Some(home);
    view.me = Some("ben".into());
    view.bindings = crate::action::all_preset();
    meter(&mut app);
    app.set_body_width(WIDTH);
    app
}

fn terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap()
}

/// Median microseconds of `work` over `FRAMES` runs after a warm-up.
fn time(mut work: impl FnMut()) -> Samples {
    for _ in 0..WARMUP {
        work();
    }
    let mut samples = Samples::default();
    for _ in 0..FRAMES {
        let start = Instant::now();
        work();
        samples.push(start.elapsed());
    }
    samples
}

fn rows_pane(app: &App) -> Option<Rect> {
    let view = app.view.as_ref()?;
    let board = app.effective_board()?;
    let body = Rect::new(0, 3, WIDTH, HEIGHT - 4);
    let slots = crate::board::composition::layout(
        &mut view.derived.borrow_mut().composition,
        board,
        &app.collapsed_panes(),
        app.focused(),
        body,
    )
    .ok()?;
    let (_, area) = slots
        .iter()
        .find(|(id, _)| id.last().map(String::as_str) == Some("rows"))
        .or_else(|| slots.first())?;
    Some(Outline::inner(
        &Outline {
            title: Line::default(),
            border: Style::new(),
            title_style: Style::new(),
        },
        *area,
    ))
}

struct Report {
    scene: &'static str,
    cold: f64,
    draw: Samples,
    render: Samples,
    phases: Vec<(&'static str, Samples)>,
    pane: Option<(Samples, Samples)>,
    outlines: Option<Samples>,
}

fn measure(scene: &'static str, app: App) -> Report {
    // The first frame after a resize builds the cached row scene.
    let cold = {
        let mut terminal = terminal();
        let start = Instant::now();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        micros(start.elapsed())
    };
    let mut terminal = terminal();
    let draw = time(|| {
        terminal.draw(|frame| render(frame, &app)).unwrap();
    });
    let mut render_only = Samples::default();
    let mut phases: Vec<(&'static str, Samples)> = Vec::new();
    let mut terminal = self::terminal();
    for index in 0..WARMUP + FRAMES {
        let mut lap = Lap::start();
        let start = Instant::now();
        terminal
            .draw(|frame| render_replica(frame, &app, &mut lap))
            .unwrap();
        if index >= WARMUP {
            // Draw time minus the closure is ratatui's diff and backend write.
            let total = start.elapsed();
            let closure: Duration = lap.laps.iter().map(|(_, d)| *d).sum();
            render_only.push(closure);
            if phases.is_empty() {
                phases = lap
                    .laps
                    .iter()
                    .map(|(phase, _)| (*phase, Samples::default()))
                    .collect();
                phases.push(("ratatui diff and backend write", Samples::default()));
            }
            for ((_, samples), (_, sample)) in phases.iter_mut().zip(&lap.laps) {
                samples.push(*sample);
            }
            phases.last_mut().unwrap().1.push(total - closure);
        }
    }
    // Pane painters on their own: the rows pane and its cached scene.
    let look = app.look();
    let area = rows_pane(&app).filter(|_| app.view.as_ref().is_some_and(|v| v.home.is_none()));
    let pane = area.map(|area| {
        let mut terminal = self::terminal();
        // Timed inside the draw closure, so the buffer diff is not part of it.
        let mut painter = Samples::default();
        for index in 0..WARMUP + FRAMES {
            app.hits.borrow_mut().clear();
            app.row_starts.borrow_mut().clear();
            terminal
                .draw(|frame| {
                    let start = Instant::now();
                    rows::render_rows(frame, &app, area);
                    if index >= WARMUP {
                        painter.push(start.elapsed());
                    }
                })
                .unwrap();
        }
        let mut buffer = Buffer::empty(Rect::new(0, 0, WIDTH, HEIGHT));
        let derived = app.view.as_ref().unwrap().derived.borrow();
        let scene = &derived.grid.as_ref().expect("a prepared grid").scene;
        let paint = time(|| {
            scene.paint(&mut buffer, area, 0, app.selected, look);
        });
        (painter, paint)
    });
    // Outlines: one representative bordered, titled pane per placed slot.
    let outlines = app
        .view
        .as_ref()
        .filter(|view| view.home.is_none())
        .and_then(|view| {
            let board = app.effective_board()?;
            let slots = crate::board::composition::layout(
                &mut view.derived.borrow_mut().composition,
                board,
                &app.collapsed_panes(),
                app.focused(),
                Rect::new(0, 3, WIDTH, HEIGHT - 4),
            )
            .ok()?;
            let outline = Outline {
                title: Line::from(vec![Span::raw(" rows ")]),
                border: look.role(tmt_cli_style::Role::Dim),
                title_style: look.role(tmt_cli_style::Role::Muted),
            };
            let mut buffer = Buffer::empty(Rect::new(0, 0, WIDTH, HEIGHT));
            Some(time(|| {
                for (_, area) in &slots {
                    outline.paint(*area, &mut buffer);
                }
            }))
        });
    Report {
        scene,
        cold,
        draw,
        render: render_only,
        phases,
        pane,
        outlines,
    }
}

fn print(report: &Report) {
    let share = report.draw.median() / 1e6 * TARGET_HZ * 100.0;
    println!(
        "\n== {} at {WIDTH}x{HEIGHT} ({FRAMES} warm frames) ==",
        report.scene
    );
    println!(
        "full draw (render + diff)  median {:8.1} us   min {:8.1}   p90 {:8.1}   cold first frame {:8.1}",
        report.draw.median(),
        report.draw.at(0.0),
        report.draw.at(0.9),
        report.cold
    );
    println!(
        "render closure only        median {:8.1} us   -> {:.2}% of one core at {TARGET_HZ} Hz (full draw)",
        report.render.median(),
        share
    );
    let sum: f64 = report.phases.iter().map(|(_, s)| s.median()).sum();
    for (phase, samples) in &report.phases {
        println!(
            "  {:<48} {:8.1} us  {:5.1}%",
            phase,
            samples.median(),
            samples.median() / sum * 100.0
        );
    }
    if let Some((painter, paint)) = &report.pane {
        println!(
            "  rows pane painter alone (scene paint + scroll strips)   {:8.1} us",
            painter.median()
        );
        println!(
            "    of which cached rows scene paint                      {:8.1} us",
            paint.median()
        );
        let outlines = report.outlines.as_ref().map_or(0.0, Samples::median);
        println!(
            "  outlines for all placed panes alone                     {:8.1} us",
            outlines
        );
        let body = report
            .phases
            .iter()
            .find(|(phase, _)| phase.starts_with("body"))
            .map_or(0.0, |(_, samples)| samples.median());
        println!(
            "  body minus rows pane and outlines (other panes, remainder) {:5.1} us",
            body - painter.median() - outlines
        );
    }
}

/// One-line strips of typical board shapes: `spans` styled spans filling `width`.
fn strip_line(spans: usize, width: usize, look: crate::look::Look) -> Line<'static> {
    let each = width / spans;
    let roles = [
        tmt_cli_style::Role::Text,
        tmt_cli_style::Role::Muted,
        tmt_cli_style::Role::Dim,
        tmt_cli_style::Role::Working,
    ];
    Line::from(
        (0..spans)
            .map(|n| {
                let mut text = format!("span{n} ");
                text.push_str(&"x".repeat(each.saturating_sub(text.len())));
                Span::styled(text, look.role(roles[n % roles.len()]))
            })
            .collect::<Vec<_>>(),
    )
}

fn print_strip_microbench(app: &App) {
    let look = app.look();
    println!("\n== strip::paint_left alone (cost per strip, {FRAMES} runs of 100 strips) ==");
    for (label, spans, width) in [
        ("message, 1 span", 1, 80),
        ("tab line shape, 6 spans", 6, 100),
        ("summary shape, 8 spans", 8, 120),
        ("home row, 5 spans", 5, 160),
        ("footer hints, 12 spans", 12, 160),
    ] {
        let line = strip_line(spans, width, look);
        let area = Rect::new(0, 0, width as u16, 1);
        let mut buffer = Buffer::empty(Rect::new(0, 0, WIDTH, 1));
        let samples = time(|| {
            for _ in 0..100 {
                strip::paint_left(&mut buffer, area, line.clone(), &look.theme, look.depth);
            }
        });
        println!(
            "  {:<28} {:7.2} us per strip   {:6.2} us per span",
            label,
            samples.median() / 100.0,
            samples.median() / 100.0 / spans as f64
        );
    }
    let area = Rect::new(0, 0, WIDTH, 1);
    let mut buffer = Buffer::empty(Rect::new(0, 0, WIDTH, 1));
    let line = tabs::paint(app, area);
    let spans = line.spans.len();
    let samples = time(|| {
        for _ in 0..100 {
            strip::paint_left(&mut buffer, area, line.clone(), &look.theme, look.depth);
        }
    });
    println!(
        "  {:<28} {:7.2} us per strip   {:6.2} us per span",
        format!("this board's tab line, {spans} spans"),
        samples.median() / 100.0,
        samples.median() / 100.0 / spans as f64
    );
}

#[test]
#[ignore = "timing harness; see the module comment"]
fn frame_timing_report() {
    println!(
        "\nboard frame timing; build profile: {}",
        if cfg!(debug_assertions) {
            "DEBUG (use --release)"
        } else {
            "release"
        }
    );
    let crew = member_board(Layout::Crew);
    print_strip_microbench(&crew);
    for report in [
        measure("crew rows + meter", crew),
        measure("team rows + meter", member_board(Layout::Team)),
        measure("home + meter", home_board()),
    ] {
        print(&report);
    }
}

#[test]
fn render_replica_matches_render() {
    for (app, sentinel) in [
        (member_board(Layout::Crew), "member-05"),
        (member_board(Layout::Team), "member-05"),
        (home_board(), "needs you"),
    ] {
        let mut expected = terminal();
        expected.draw(|frame| render(frame, &app)).unwrap();
        let mut actual = terminal();
        actual
            .draw(|frame| render_replica(frame, &app, &mut Lap::start()))
            .unwrap();
        assert_eq!(expected.backend().buffer(), actual.backend().buffer());
        let screen: String = expected
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains(sentinel), "the scene shows {sentinel}");
        assert!(screen.contains("tok"), "the meter is on in every scene");
        assert!(
            !app.hits.borrow().is_empty(),
            "the scene has interactive rows"
        );
    }
}
