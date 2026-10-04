use super::{
    detail::{detail_represents, render_detail},
    footer::hints,
    header::{SPINNER_TICK, meter_region, summary_line},
    overlays::render_switcher,
    replies::reply_lines,
    rows::{GAP, grid_line},
    tabs::{pane_tab, tab, tab_window},
};
use crate::board::app::Switcher;
use crate::{
    attention::Attention,
    config::{NotesRender, TabColors},
    requests::{BODIES, age},
    rows::Rows,
};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
};
use serde_json::Value;
use tmt_cli_style::{Role, mark::Mark};
use unicode_width::UnicodeWidthStr;

mod help;
mod meter;
mod parity;
use super::*;
use crate::board::app::{Effect, Notes, Snapshot, View};
use crate::config::{BoardMode, Direction, Pane};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use std::collections::BTreeMap;
use unicode_width::UnicodeWidthChar;

#[test]
fn footer_omits_whole_hints_instead_of_clipping_words() {
    let mut app = App::new(Some("product".into()));
    app.apply(crate::board::app::tests::snapshot("product", json!([])));
    let full = hints(&app, usize::MAX);
    assert!(full.contains("T theme"));
    for width in [0, 1, 20, 40, 108, 112] {
        let shown = hints(&app, width);
        assert!(shown.width() <= width);
        assert!(full.starts_with(&shown));
        assert!(
            shown.is_empty() || full == shown || full[shown.len()..].starts_with("  "),
            "partial hint at {width}: {shown}"
        );
    }
    assert!(!draw(&app, 108, 8).last().unwrap().ends_with("◆ ne"));
}

/// Rows read from a squad config snippet, as `squad.toml` would give them.
fn rows_from(text: &str) -> Rows {
    let config: toml_edit::DocumentMut = text.parse().unwrap();
    crate::rows::read(config["p"].as_table_like(), "p").unwrap()
}

fn columns() -> Rows {
    rows_from(
        "[p.columns]\nshow = [\"member\", \"state\", \"task\"]\n\
             member = { width = 10 }\nstate = { width = 8 }\n",
    )
}

fn draw(app: &App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0;
            while x < width {
                let symbol = buffer[(x, y)].symbol().to_owned();
                x += symbol
                    .chars()
                    .map(|c| c.width().unwrap_or(0) as u16)
                    .sum::<u16>()
                    .max(1);
                line.push_str(&symbol);
            }
            line.trim_end().to_owned()
        })
        .collect()
}

fn board(sections: Value) -> App {
    let mut app = App::new(Some("product".into()));
    app.apply(Snapshot {
            squad_keys: Vec::new(),
            tabs: vec!["product".into(), "reviews".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some("product".into()),
            view: Ok(View {
                token_rate: None,
                home: None,
            derived: Default::default(),
                document: json!({"squad": {"name": "product", "lead": {"name": "sol"}}, "sections": sections}),
                rows: columns(),
                refresh: Some(crate::config::DEFAULT_REFRESH),
                board: crate::config::Board::simple(
                    crate::config::BoardMode::Split,
                    crate::config::Direction::LeftRight,
                    vec![crate::config::Pane::Rows],
                    &[100],
                ),
            notes: crate::board::app::Notes::NotShown,
                render: crate::config::NotesRender::Markdown,
                bindings: crate::action::preset(true, &[]),
                section_bindings: Vec::new(),
                configured_bindings: Default::default(),
                opener: None,
                clipboard: None,
                links: Default::default(),
                tab_colors: Default::default(),
                look: Default::default(),
                theme_notice: None,
                me: None,
                replies: Vec::new(),
            }),
        });
    app
}

fn row(name: &str, state: &str, task: &str, extra: Value) -> Value {
    let mut row = json!({"name": name, "fields": {"state": state, "task": task}, "pending": null, "colors": {"state": if state == "blocked" { "amber" } else { "default" }}});
    for (key, value) in extra.as_object().unwrap() {
        row[key] = value.clone();
    }
    row
}

#[test]
fn admitted_ids_belong_to_the_shown_view_during_load_resize_and_search() {
    use crate::board::app::tests::snapshot;
    let sections =
        json!([{"title":null,"rows":[{"id":"member-a","name":"worker","fields":{"task":"work"}}]}]);
    let mut app = App::new(Some("product".into()));
    assert_eq!(app.shown_tab(), None);
    draw(&app, 120, 30);
    assert!(app.view.is_none());
    app.apply(snapshot("product", sections.clone()));
    let id = |app: &App| {
        let view = app.view.as_ref().unwrap();
        let derived = view.derived.borrow();
        derived.grid.as_ref().unwrap().cells[0].id.clone().unwrap()
    };
    draw(&app, 120, 30);
    let a = id(&app);
    assert_eq!(&a[..3], &["tab:product", "section-0", "squad:product"]);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        Effect::Load("infra".into())
    );
    assert!(app.loading());
    app.search = "worker".into();
    for width in [80, 120] {
        draw(&app, width, 30);
        assert_eq!(id(&app), a);
    }
    app.apply(snapshot("infra", sections));
    draw(&app, 80, 30);
    let b = id(&app);
    assert_eq!(&b[..3], &["tab:infra", "section-0", "squad:infra"]);
    assert_ne!(a, b);
    assert!(!app.loading());
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    draw(&app, 120, 30);
    assert_eq!(id(&app), a);
    assert!(!app.loading(), "cached switch restores the view's identity");
}

#[test]
fn admitted_rows_reuse_selection_cache_and_resize_at_scale() {
    use std::time::Instant;
    for sample in 0..7 {
        for count in [10, 100, 1000] {
            let members: Vec<_> = (0..count)
                .map(|i| {
                    row(
                        &format!("worker-{i}"),
                        "working",
                        "wide 文件 e\u{301}👩‍💻 task",
                        json!({"id":format!("member-{i}")}),
                    )
                })
                .collect();
            let mut app = board(json!([{"title":null,"rows":members}]));
            let start = Instant::now();
            draw(&app, 120, 30);
            let cold = start.elapsed();
            let (pointer, ids) = {
                let derived = app.view.as_ref().unwrap().derived.borrow();
                let cells = &derived.grid.as_ref().unwrap().cells;
                assert_eq!(cells.len(), count);
                (
                    cells.as_ptr(),
                    cells.iter().map(|cell| cell.id.clone()).collect::<Vec<_>>(),
                )
            };
            app.selected = count - 1;
            let start = Instant::now();
            draw(&app, 120, 30);
            let selected = start.elapsed();
            assert_eq!(
                app.view
                    .as_ref()
                    .unwrap()
                    .derived
                    .borrow()
                    .grid
                    .as_ref()
                    .unwrap()
                    .cells
                    .as_ptr(),
                pointer
            );
            let start = Instant::now();
            draw(&app, 80, 30);
            let resize = start.elapsed();
            let derived = app.view.as_ref().unwrap().derived.borrow();
            let cells = &derived.grid.as_ref().unwrap().cells;
            assert_eq!(
                cells.iter().map(|cell| cell.id.clone()).collect::<Vec<_>>(),
                ids
            );
            assert_eq!(
                cells.last().unwrap().row_id.as_deref(),
                Some(format!("member-{}", count - 1).as_str())
            );
            println!(
                "markup rows sample={sample} count={count} cold={cold:?} selection={selected:?} resize={resize:?}"
            );
        }
    }
}

#[test]
fn uncovered_source_columns_do_not_squeeze_the_drawn_grid() {
    let config: toml_edit::DocumentMut = include_str!("../../rows/fixtures/uncovered-tracks.toml")
        .parse()
        .unwrap();
    let rows = crate::rows::read(config["squad"]["checkout"].as_table_like(), "checkout").unwrap();
    let task = "long task ".repeat(60);
    let sections = json!([{"title": null, "rows": [row("worker", "working", &task, json!({
        "fields": {"state": "working", "task": task, "pr_state": "#42 OPEN", "ctx": "487k", "model": "test-model"}
    }))]}]);
    for width in [146, 200] {
        let mut full = board(sections.clone());
        full.view.as_mut().unwrap().rows = rows.clone();
        let mut covered = board(sections.clone());
        let mut trimmed = rows.clone();
        trimmed.columns.truncate(4);
        covered.view.as_mut().unwrap().rows = trimmed;
        let actual = draw(&full, width, 20);
        assert_eq!(actual, draw(&covered, width, 20));
        assert!(
            actual
                .iter()
                .any(|line| line.contains("487k") && line.contains("test-model"))
        );
        assert_eq!(
            full.view
                .as_ref()
                .unwrap()
                .derived
                .borrow()
                .grid
                .as_ref()
                .unwrap()
                .layout
                .columns[4..],
            [None, None]
        );
    }
}

/// Explicit crew keeps its original columns and rendering byte for byte.
fn preset_board() -> App {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("squad-golden-{}-{serial}.toml", std::process::id()));
    std::fs::write(&path, "[squad.product]\nlayout = \"crew\"\n").unwrap();
    let config = crate::config::Config::read(path.clone()).unwrap();
    std::fs::remove_file(&path).unwrap();
    let mut app = board(json!([
        {"title": "Needs me", "rows": [
            row("auth-fix", "blocked", "rotate session tokens without logging everyone out", json!({
                "pending": "approve",
                "fields": {"state": "blocked", "task": "rotate session tokens without logging everyone out", "pr_link": "https://github.com/wkh237/tmt/pull/4242"}
            }))
        ]},
        {"title": "Everyone", "rows": [
            row("文件-sweep-long-name", "working", "整理安装指南和常见问题", json!({})),
            row("perf", "", "", json!({"fields": {}, "annotation": {"to": "sol", "text": "check the cache hit rate"}})),
        ]}
    ]));
    app.view.as_mut().unwrap().rows = config.rows("product").unwrap();
    app
}

#[test]
fn factory_views_render_crew_team_and_custom_rows_at_each_width() {
    let path =
        std::env::temp_dir().join(format!("squad-factory-render-{}.toml", std::process::id()));
    for workflow in ["crew", "team"] {
        for custom_rows in [false, true] {
            for name in crate::view::ViewName::ALL {
                let rows = if custom_rows {
                    "[squad.product.rows]\ncolumns = [{ name = 'member', width = 14 }, { name = 'task', grow = 1 }]\nlines = [[{ field = 'member' }, { field = 'task' }]]\n"
                } else {
                    ""
                };
                std::fs::write(&path, format!("[squad.product]\nlayout = '{workflow}'\n[squad.product.board]\nview = '{}'\n{rows}", name.name())).unwrap();
                let config = crate::config::Config::read(path.clone()).unwrap();
                let mut app = board(
                    json!([{ "title": null, "rows": [row("view-worker", "working", "visible task", json!({}))] }]),
                );
                let view = app.view.as_mut().unwrap();
                view.board = config.board("product").unwrap();
                view.rows = config.rows("product").unwrap();
                view.notes = Notes::Text("# Notebook\nLead notebook sentinel".into());
                for width in [80, 120, 200] {
                    app.set_body_width(width);
                    let screen = draw(&app, width, 42);
                    assert!(
                        screen.iter().any(|line| line.contains("view-worker")),
                        "{} / {workflow} / custom={custom_rows} at {width}",
                        name.name()
                    );
                    assert!(!app.hits.borrow().is_empty(), "rows remain interactive");
                    assert!(
                        app.title_hits
                            .borrow()
                            .iter()
                            .all(|hit| hit.area.right() <= width && hit.area.bottom() <= 42)
                    );
                    if !app.collapsed_panes().contains(&Pane::Notes) {
                        assert!(
                            screen
                                .iter()
                                .any(|line| line.contains("Lead notebook sentinel"))
                        );
                    }
                }
            }
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn default_team_is_readable_at_80_120_and_200_columns() {
    let path =
        std::env::temp_dir().join(format!("squad-team-responsive-{}.toml", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let config = crate::config::Config::read(path).unwrap();
    let mut app = preset_board();
    let view = app.view.as_mut().unwrap();
    view.rows = config.rows("product").unwrap();
    view.board = config.board("product").unwrap();
    for width in [80, 120, 200] {
        app.set_body_width(width);
        let screen = draw(&app, width, 42);
        let widths = app
            .view
            .as_ref()
            .unwrap()
            .derived
            .borrow()
            .grid
            .as_ref()
            .unwrap()
            .layout
            .columns
            .clone();
        assert!(
            widths[..3].iter().all(Option::is_some),
            "essential columns at {width}"
        );
        assert!(widths[3].is_some(), "PR remains visible at {width}");
        assert_eq!(widths[4].is_some(), width == 200, "model steps aside first");
        assert!(
            screen.iter().any(|line| line.contains("TASK")),
            "task title at {width}"
        );
        assert_eq!(app.collapsed_panes().contains(&Pane::Detail), width < 100);
        assert_eq!(app.collapsed_panes().contains(&Pane::Replies), width < 100);
    }
    // Manual expansion at 80 keeps essentials readable even without auto-fold.
    app.set_body_width(80);
    app.perform(&crate::action::Action::parse("toggle detail").unwrap());
    app.perform(&crate::action::Action::parse("toggle replies").unwrap());
    draw(&app, 80, 42);
    let view = app.view.as_ref().unwrap();
    let derived = view.derived.borrow();
    let widths = &derived.grid.as_ref().unwrap().layout.columns;
    assert!(widths[..3].iter().all(Option::is_some));
    assert!(widths[3..].iter().all(Option::is_none));
}

/// Golden: every preset's board as drawn before the rows moved onto the
/// shared grid solver. The layout engine must keep these byte for byte.
#[test]
fn preset_columns_draw_exactly_as_before_at_every_width() {
    let app = preset_board();
    let golden: [(u16, [&str; 7]); 4] = [
        (
            48,
            [
                "  MEMBER         STATE      TASK    PR",
                "NEEDS ME",
                "◆ auth-fix       blocked    rotate… https://git…",
                "EVERYONE",
                "  文件-sweep-lo… working    整理安… –",
                "  perf           –          –       –",
                "    ✎ sent to sol: check the cache hit rate",
            ],
        ),
        (
            60,
            [
                "  MEMBER         STATE      TASK                PR",
                "NEEDS ME",
                "◆ auth-fix       blocked    rotate session tok… https://git…",
                "EVERYONE",
                "  文件-sweep-lo… working    整理安装指南和常见… –",
                "  perf           –          –                   –",
                "    ✎ sent to sol: check the cache hit rate",
            ],
        ),
        (
            80,
            [
                "  MEMBER         STATE      TASK                                    PR",
                "NEEDS ME",
                "◆ auth-fix       blocked    rotate session tokens without logging … https://git…",
                "EVERYONE",
                "  文件-sweep-lo… working    整理安装指南和常见问题                  –",
                "  perf           –          –                                       –",
                "    ✎ sent to sol: check the cache hit rate",
            ],
        ),
        (
            120,
            [
                "  MEMBER         STATE      TASK                                                                            PR",
                "NEEDS ME",
                "◆ auth-fix       blocked    rotate session tokens without logging everyone out                              https://git…",
                "EVERYONE",
                "  文件-sweep-lo… working    整理安装指南和常见问题                                                          –",
                "  perf           –          –                                                                               –",
                "    ✎ sent to sol: check the cache hit rate",
            ],
        ),
    ];
    for (width, lines) in golden {
        assert_eq!(draw(&app, width, 11)[2..9], lines, "at {width} columns");
    }
}

#[test]
fn a_narrow_preset_board_drops_the_link_instead_of_clipping() {
    let screen = draw(&preset_board(), 44, 11);
    assert_eq!(screen[2], "  MEMBER         STATE      TASK");
    assert_eq!(screen[4], "◆ auth-fix       blocked    rotate session …");
    assert!(screen[2..10].iter().all(|line| line.width() <= 44));
}

#[test]
fn handbook_rows_take_a_second_line_span_align_and_step_aside() {
    let mut app = board(json!([{"title": null, "rows": [
        row("auth-fix", "blocked", "rotate session tokens", json!({
            "pending": "approve the rollout plan",
            "fields": {"state": "blocked", "task": "rotate session tokens", "pr": "#4242"},
        })),
        row("docs", "working", "guide", json!({"fields": {"state": "working", "task": "guide", "pr": "#7"}})),
    ]}]));
    app.view.as_mut().unwrap().rows = rows_from(
        r#"[p.rows]
columns = [
  { name = "member", min = 10 },
  { name = "state",  width = 9 },
  { name = "task",   grow = 1, min = 12 },
  { name = "pr",     width = 10, align = "right", priority = 2 },
]
lines = [
  ["member", "state", "task", "pr"],
  ["",       { field = "pending", span = 3 }],
]
"#,
    );
    let wide = draw(&app, 60, 9);
    assert_eq!(
        wide[2],
        "  MEMBER     STATE     TASK                               PR"
    );
    assert_eq!(
        wide[3],
        "◆ auth-fix   blocked   rotate session tokens           #4242"
    );
    assert_eq!(wide[4], "             approve the rollout plan");
    // Nothing to show on the second line: the row keeps one line.
    assert_eq!(
        wide[5],
        "  docs       working   guide                              #7"
    );
    // Narrow: the prioritized column steps aside and the span shrinks.
    let narrow = draw(&app, 36, 9);
    assert_eq!(narrow[2], "  MEMBER     STATE     TASK");
    assert_eq!(narrow[3], "◆ auth-fix   blocked   rotate sessi…");
    assert_eq!(narrow[4], "             approve the rollout pl…");
}

#[test]
fn percentage_wrapping_keeps_continuation_hits_selection_and_visual_paging() {
    let mut app = board(json!([{ "title": null, "rows": [
            row("a", "", "alpha beta gamma delta", json!({})),
            row("b", "", "alpha beta gamma delta", json!({})),
            row("c", "", "alpha beta gamma delta", json!({})),
            row("d", "", "alpha beta gamma delta", json!({})),
        ] }]));
    app.view.as_mut().unwrap().rows = rows_from(
        r#"[p.rows]
columns = [{ name = "member", width = "30%" },
           { name = "task", width = "70%", overflow = "wrap", max_lines = 2 }]
"#,
    );
    let screen = draw(&app, 20, 10);
    assert_eq!(screen[3], "  a     alpha beta");
    assert_eq!(screen[4], "        gamma delta");
    assert_eq!(*app.row_starts.borrow(), [1, 3, 5, 7]);
    let hits: Vec<_> = app
        .hits
        .borrow()
        .iter()
        .map(|hit| (hit.y, hit.row))
        .collect();
    assert!(hits.contains(&(3, 0)) && hits.contains(&(4, 0)));
    app.selected = 1;
    app.mouse(
        ratatui::crossterm::event::MouseEvent {
            kind: ratatui::crossterm::event::MouseEventKind::Down(
                ratatui::crossterm::event::MouseButton::Left,
            ),
            column: 8,
            row: 4,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert_eq!(app.selected, 0, "a continuation click selects its record");
    app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(
        app.selected, 2,
        "a page crosses visual lines, not six records"
    );
    draw(&app, 20, 10);
    app.key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
    assert_eq!(app.selected, 0);
    app.selected = 3;
    let screen = draw(&app, 20, 7);
    assert!(
        screen.iter().any(|line| line.contains("d     alpha beta")),
        "{screen:?}"
    );
    assert!(
        screen.iter().any(|line| line.contains("gamma delta")),
        "{screen:?}"
    );
    assert!(app.hits.borrow().iter().filter(|hit| hit.row == 3).count() == 2);
    for width in [8, 12, 18] {
        let screen = draw(&app, width, 7);
        assert!(screen.iter().all(|line| line.width() <= usize::from(width)));
        assert!(app.hits.borrow().iter().all(|hit| hit.row < 4));
    }
}

#[test]
fn middle_truncation_keeps_both_ends_of_a_link() {
    let mut app = board(json!([{"title": null, "rows": [
        row("docs", "working", "", json!({"fields": {"link": "https://github.com/wkh237/tmt/pull/4242"}})),
    ]}]));
    app.view.as_mut().unwrap().rows = rows_from(
        "[p.rows]\ncolumns = [{ name = \"member\", width = 6 }, { name = \"link\", width = 20, truncate = \"middle\" }]\n",
    );
    assert_eq!(draw(&app, 40, 4)[2], "  docs   https://gi…pull/4242");
}

#[test]
fn rows_ignore_retired_notes_and_show_pending_sections_and_aligned_wide_text() {
    let app = board(json!([
        {"title": "Needs me", "rows": [row("auth-fix", "blocked", "rotate session tokens", json!({"pending": "approve", "note": "needs a call"}))]},
        {"title": "Everyone", "rows": [row("文件-sweep", "working", "整理安装指南", json!({}))]}
    ]));
    let screen = draw(&app, 48, 10);
    // The tab line holds only the tabs; the summary has its own line.
    assert_eq!(screen[0], "  product    reviews");
    assert_eq!(screen[1], "lead sol · 2 members");
    assert_eq!(screen[2], "  MEMBER     STATE    TASK");
    assert_eq!(screen[3], "NEEDS ME");
    assert_eq!(screen[4], "◆ auth-fix   blocked  rotate session tokens");
    assert_eq!(screen[5], "EVERYONE");
    assert_eq!(screen[6], "  文件-sweep working  整理安装指南");
    assert!(screen.iter().all(|line| !line.contains("needs a call")));
    assert!(screen[9].starts_with("⏎ jump"));
}

#[test]
fn a_squad_of_one_has_one_member() {
    let app = board(json!([{"title": null, "rows": [row("docs", "working", "guide", json!({}))]}]));
    assert_eq!(draw(&app, 48, 5)[1], "lead sol · 1 member");
}

#[test]
fn drawn_rows_are_clickable_and_the_menu_and_help_show_bindings() {
    let mut app = board(json!([
        {"title": "Needs me", "rows": [row("auth-fix", "blocked", "rotate", json!({"note": "needs a call"}))]},
        {"title": "Everyone", "rows": [row("docs", "working", "guide", json!({}))]}
    ]));
    draw(&app, 48, 9);
    let lines: Vec<(u16, usize)> = app.hits.borrow().iter().map(|h| (h.y, h.row)).collect();
    assert_eq!(
        lines,
        [(4, 0), (6, 1)],
        "a retired note adds no clickable line"
    );
    // Scrolled: only visible lines are clickable, at their screen rows.
    app.selected = 1;
    draw(&app, 48, 6);
    let lines: Vec<(u16, usize)> = app.hits.borrow().iter().map(|h| (h.y, h.row)).collect();
    // An overflowing pane keeps its last line for the indicator, so two of
    // the three lines show rows.
    assert_eq!(lines, [(3, 1)]);

    app.view.as_mut().unwrap().bindings = crate::action::preset(false, &[]);
    app.help = true;
    // Notes cursor guidance and the settings binding need one more help row.
    let help = help_lines(&app);
    assert!(help.iter().any(|line| line.starts_with("g G")));
    assert!(help.iter().any(|line| line == ",  show settings"));
    assert!(
        help.iter()
            .any(|line| line == "y  copy from the selected row"),
        "{help:#?}"
    );
    assert!(
        help.iter()
            .any(|line| line == "reload  automatically every 5s"),
        "{help:#?}"
    );
    app.view.as_mut().unwrap().refresh = None;
    let help = help_lines(&app);
    assert!(
        help.iter()
            .any(|line| line == "reload  automatic reload is off"),
        "{help:#?}"
    );
    app.help = false;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let screen = draw(&app, 60, 16);
    assert!(
        screen
            .iter()
            .any(|line| line.contains("docs · Enter runs, Esc closes"))
    );
    assert!(screen.iter().any(|line| line.contains("backspace back")));
    assert!(draw(&App::new(None), 60, 4)[3].starts_with("/ search"));
}

#[test]
fn help_groups_view_lead_and_theme_bindings() {
    let mut app = board(json!([]));
    app.view.as_mut().unwrap().bindings = crate::action::preset(true, &[]);
    let help = help_lines(&app);
    assert!(
        help.iter()
            .any(|line| line == "double-click  go to the member's pane")
    );
    let at = help
        .iter()
        .position(|line| line == "l  pick a pane layout")
        .unwrap();
    assert_eq!(
        &help[at..at + 3],
        [
            "l  pick a pane layout",
            "L  go to the lead's pane",
            "T  pick a theme"
        ]
    );
}

#[test]
fn fit_truncates_by_display_width_and_selection_scrolls_into_view() {
    assert_eq!(fit("整理安装指南", 7), "整理安…");
    assert_eq!(fit("abc", 5), "abc  ");
    assert_eq!(fit("abcdef", 4), "abc…");
    let rows: Vec<Value> = (0..20)
        .map(|i| row(&format!("m{i:02}"), "working", "", json!({})))
        .collect();
    let mut app = board(json!([{"title": null, "rows": rows}]));
    app.selected = 15;
    let screen = draw(&app, 40, 8);
    assert!(screen.iter().any(|line| line.contains("m15")), "{screen:?}");
    assert!(!screen.iter().any(|line| line.contains("m00")));
}

#[test]
fn refresh_hint_and_help_render_exact_lowercase_ctrl_r() {
    let mut app = board(json!([{"title": null, "rows": [row("a", "idle", "", json!({}))]}]));
    let screen = draw(&app, 160, 12);
    assert!(screen[11].contains("ctrl-r refresh"), "{:?}", screen[11]);
    assert!(!screen[11].contains("f5") && !screen[11].contains("F5"));
    app.help = true;
    let screen = help_lines(&app);
    assert!(
        screen
            .iter()
            .any(|line| line == "ctrl-r  refresh the board now"),
        "{screen:?}"
    );
    assert!(
        !screen
            .iter()
            .any(|line| line.contains("Ctrl-R") || line.contains("F5") || line.contains("f5"))
    );
}

#[test]
fn search_help_and_errors_use_the_footer_and_overlay() {
    let mut app = board(json!([{"title": null, "rows": [row("a", "idle", "", json!({}))]}]));
    app.searching = true;
    app.search = "zz".into();
    let screen = draw(&app, 40, 6);
    assert!(
        screen
            .iter()
            .any(|line| line.contains("(no matching members)"))
    );
    assert_eq!(screen[5], "/zz▏");
    app.searching = false;
    app.search.clear();
    app.help = true;
    assert!(
        draw(&app, 60, 12)
            .iter()
            .any(|line| line.contains("↑↓ scroll"))
    );
    let mut failed = App::new(Some("product".into()));
    failed.error = Some("tmt did not finish in time".into());
    assert_eq!(draw(&failed, 40, 4)[3], "tmt did not finish in time");
}

fn paned(board: crate::config::Board, notes: Notes) -> App {
    let bindings = crate::action::preset(true, &board.panes);
    let mut app = App::new(Some("product".into()));
    app.apply(Snapshot {
            squad_keys: Vec::new(),
            tabs: vec!["product".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some("product".into()),
            view: Ok(View {
                token_rate: None,
                home: None,
            derived: Default::default(),
                document: json!({"squad": {"name": "product", "lead": {"name": "sol"}}, "sections": [
                    {"title": null, "rows": [row("auth-fix", "blocked", "rotate tokens", json!({
                        "pending": "approve the plan", "note": "needs a call", "presence": "active", "state": "blocked",
                        "pane": {"target": "crew:2.0", "cwd": "/w/app-3"}
                    }))]}
                ]}),
                rows: columns(),
                refresh: None,
                board,
                notes,
                render: NotesRender::Markdown,
                bindings,
                section_bindings: Vec::new(),
                configured_bindings: Default::default(),
                opener: None,
                clipboard: None,
                links: Default::default(),
                tab_colors: Default::default(),
                look: Default::default(),
                theme_notice: None,
                me: None,
                replies: Vec::new(),
            }),
        });
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["fields"]["pr_link"] =
        json!("https://example.com/pull/412");
    app
}

fn split(direction: Direction, panes: Vec<Pane>, sizes: Vec<u16>) -> crate::config::Board {
    crate::config::Board::simple(BoardMode::Split, direction, panes, &sizes)
}

#[test]
fn state_cells_use_the_projected_token_for_pattern_and_exact_states() {
    let rows = rows_from("[p.columns]\nshow = ['state']\nstate = { width = 20 }\n");
    let look = crate::look::Look::default();
    for state in ["blocked", "blocked-on-ci"] {
        let row =
            json!({"state": state, "fields": {"state": state}, "colors": {"state": "review"}});
        let spans = grid_line(
            look,
            &rows,
            &crate::markup::Grid::compile(&rows, |_| 20, 20).unwrap(),
            &crate::markup::row_values(&rows, "product", vec![(0, &row)]).unwrap()[0],
            &row,
            0,
            false,
        )
        .unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0][0].style.fg, look.named("review").fg);
        assert_eq!(spans[0][0].content.trim(), state);
        let plain = json!({"state": state, "fields": {"state": state}});
        let spans = grid_line(
            look,
            &rows,
            &crate::markup::Grid::compile(&rows, |_| 20, 20).unwrap(),
            &crate::markup::row_values(&rows, "product", vec![(0, &plain)]).unwrap()[0],
            &plain,
            0,
            false,
        )
        .unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0][0].style, Style::new());
    }
}

#[test]
fn selected_reverse_rows_are_uniform_across_wrapping_empty_pending_and_age() {
    let mut app = board(json!([{"title": null, "rows": [
        row("auth-fix", "working", "rotate session tokens for the rollout", json!({
            "pending": "approve the rollout plan",
            "fields": {"state": "working", "task": "rotate session tokens for the rollout", "pr": "#42"},
            "colors": {"state": "working", "task": "waiting", "pr": "review"},
            "staleness": {"state": "stale", "ageMs": 3 * 3_600_000},
        })),
        row("docs", "review", "guide", json!({"colors": {"state": "review"}})),
    ]}]));
    app.view.as_mut().unwrap().rows = rows_from(
        r#"[p.rows]
columns = [
    { name = "member", width = 12 },
    { name = "state", width = 9 },
    { name = "task", width = 12, overflow = "wrap", max_lines = 2 },
    { name = "pr", width = 8 },
    { name = "empty", width = 5 },
]
lines = [
    ["member", "state", "task", "pr", "empty"],
    ["", { field = "pending", span = 4, token = "waiting" }],
]
"#,
    );
    let text = draw(&app, 80, 12);
    for base in tmt_cli_style::Base::ALL {
        for depth in [
            tmt_cli_style::Depth::TrueColor,
            tmt_cli_style::Depth::Ansi16,
            tmt_cli_style::Depth::None,
        ] {
            let look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            };
            if look.selection().bg.is_some() {
                continue;
            }
            app.view.as_mut().unwrap().look = look;
            let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            let screen = draw(&app, 80, 12);
            assert_eq!(
                screen, text,
                "{base:?} {depth:?}: text and geometry stay exact"
            );
            let first = screen
                .iter()
                .position(|line| line.contains("auth-fix"))
                .unwrap();
            let end = screen
                .iter()
                .position(|line| line.contains("docs "))
                .unwrap();
            assert!(end - first >= 3, "wrapping and pending line are present");
            assert!(screen[first].contains('–'));
            assert!(screen[first].ends_with("stale 3h"));
            let selection = look.selection();
            let view = app.view.as_ref().unwrap();
            let derived = view.derived.borrow();
            let widths = &derived.grid.as_ref().unwrap().layout.columns;
            let grid_width = 2
                + widths.iter().flatten().sum::<usize>()
                + GAP * widths.iter().flatten().count().saturating_sub(1);
            for y in first..end {
                // The first line's age extends to the edge; later lines
                // keep the existing fixed-width grid extent.
                let width = if y == first { 80 } else { grid_width as u16 };
                for x in 0..width {
                    let cell = &buffer[(x, y as u16)];
                    assert_eq!(
                        cell.fg,
                        selection.fg.unwrap_or_default(),
                        "{base:?} {depth:?} {x},{y}"
                    );
                    assert_eq!(cell.bg, selection.bg.unwrap_or_default());
                    assert!(
                        cell.modifier.contains(Modifier::REVERSED),
                        "{base:?} {depth:?} {x},{y}: {cell:?}"
                    );
                    assert!(!cell.modifier.contains(Modifier::DIM));
                }
            }
            for word in ["◆", "working", "rotate", "approve"] {
                let y = screen.iter().position(|line| line.contains(word)).unwrap();
                let x = screen[y][..screen[y].find(word).unwrap()].chars().count() as u16;
                assert!(
                    buffer[(x, y as u16)].modifier.contains(Modifier::BOLD),
                    "{word}"
                );
            }
        }
    }
}

#[test]
fn cell_tokens_override_decoration_but_keep_missing_failure_and_reverse_rules() {
    let rows = rows_from(
        "[p.rows]\ncolumns=[{name='task',width=12,overflow='wrap',max_lines=2}]\nlines=[[{field='task',token='waiting'}]]\n",
    );
    let grid = crate::markup::Grid::compile(&rows, |_| 20, 12).unwrap();
    for depth in [
        tmt_cli_style::Depth::TrueColor,
        tmt_cli_style::Depth::Ansi16,
        tmt_cli_style::Depth::None,
    ] {
        let look = crate::look::Look {
            depth,
            ..Default::default()
        };
        for (row, role) in [
            (
                json!({"fields":{"task":"alpha beta gamma"},"colors":{"task":"review"}}),
                Role::Waiting,
            ),
            (json!({"fields":{"task":"?"},"failed":["task"]}), Role::Dim),
            (json!({"fields":{"task":""}}), Role::Dim),
            (json!({"fields":{}}), Role::Dim),
        ] {
            let scene = crate::markup::row_values(&rows, "product", vec![(0, &row)]).unwrap();
            for selected in [false, true] {
                let lines = grid_line(look, &rows, &grid, &scene[0], &row, 0, selected).unwrap();
                for spans in lines {
                    assert_eq!(spans.len(), 1);
                    assert_eq!(
                        spans[0].style,
                        look.row_span(selected, look.role(role), role == Role::Waiting)
                    );
                }
            }
        }
    }
}

#[test]
fn declarative_style_preserves_stale_row_inheritance() {
    let mut app = board(json!([{"title":null,"rows":[
        row("worker", "working", "ship", json!({
            "pending":"approve",
            "staleness":{"state":"stale","ageMs":3_600_000},
        })),
        row("other", "working", "docs", json!({})),
    ]}]));
    app.view.as_mut().unwrap().rows = rows_from(
        "[p.rows]\ncolumns=[{name='member',width=10},{name='task',width=15}]\nlines=[['member','task'],['',{field='pending',token='waiting'}]]\n",
    );
    app.selected = 1;
    for depth in [
        tmt_cli_style::Depth::TrueColor,
        tmt_cli_style::Depth::Ansi16,
        tmt_cli_style::Depth::None,
    ] {
        let look = crate::look::Look {
            depth,
            ..Default::default()
        };
        app.view.as_mut().unwrap().look = look;
        let screen = draw(&app, 60, 10);
        let y = screen
            .iter()
            .position(|line| line.contains("approve"))
            .unwrap();
        let x = screen[y].find("approve").unwrap();
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let cell = &terminal.backend().buffer()[(x as u16, y as u16)];
        let expected = look.role(Role::Dim).patch(look.role(Role::Waiting));
        assert_eq!(cell.fg, expected.fg.unwrap_or_default());
        assert_eq!(cell.modifier, expected.add_modifier);
        assert!(screen.iter().any(|line| line.ends_with("stale 1h")));
    }
}

#[test]
fn team_pending_line_style_snapshots_and_unstyled_control() {
    // Literal text/style snapshots of the real rendered pending span, including padding.
    const TRUE_COLOR: &str = r#""                               approve rollout                                  "
[(2, "Reset/Reset/Reset/NONE/None"), (17, "Rgb(122, 131, 174)/Reset/Reset/NONE/None"), (1, "Reset/Reset/Reset/NONE/None"), (10, "Rgb(122, 131, 174)/Reset/Reset/NONE/None"), (1, "Reset/Reset/Reset/NONE/None"), (49, "Rgb(255, 158, 100)/Reset/Reset/NONE/None")]
"                               approve rollout                                  "
[(2, "Rgb(192, 202, 245)/Rgb(40, 52, 87)/Reset/NONE/None"), (17, "Rgb(122, 131, 174)/Rgb(40, 52, 87)/Reset/NONE/None"), (1, "Rgb(192, 202, 245)/Rgb(40, 52, 87)/Reset/NONE/None"), (10, "Rgb(122, 131, 174)/Rgb(40, 52, 87)/Reset/NONE/None"), (1, "Rgb(192, 202, 245)/Rgb(40, 52, 87)/Reset/NONE/None"), (49, "Rgb(255, 158, 100)/Rgb(40, 52, 87)/Reset/NONE/None")]"#;
    const ANSI16: &str = r#""                               approve rollout                                  "
[(2, "Reset/Reset/Reset/NONE/None"), (17, "Reset/Reset/Reset/DIM/None"), (1, "Reset/Reset/Reset/NONE/None"), (10, "Reset/Reset/Reset/DIM/None"), (1, "Reset/Reset/Reset/NONE/None"), (49, "Yellow/Reset/Reset/NONE/None")]
"                               approve rollout                                  "
[(31, "Reset/Reset/Reset/REVERSED/None"), (49, "Reset/Reset/Reset/BOLD | REVERSED/None")]"#;
    let config: toml_edit::DocumentMut = "[squad.product]\nlayout='team'\n".parse().unwrap();
    let path = std::env::temp_dir().join(format!("tmt-line-style-{}.toml", std::process::id()));
    std::fs::write(&path, config.to_string()).unwrap();
    let rows = crate::config::Config::read(path.clone())
        .unwrap()
        .rows("product")
        .unwrap();
    std::fs::remove_file(path).unwrap();
    let mut app = board(json!([{"title":null,"rows":[
        row("worker", "working", "ship", json!({"pending":"approve rollout"})),
        row("other", "working", "docs", json!({})),
    ]}]));
    app.view.as_mut().unwrap().rows = rows;
    let snapshot = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let y = draw(app, 80, 12)
            .iter()
            .position(|line| line.contains("approve rollout"))
            .unwrap() as u16;
        let cells: Vec<_> = (0..80).map(|x| &buffer[(x, y)]).collect();
        let text: String = cells.iter().map(|cell| cell.symbol()).collect();
        let mut runs: Vec<(usize, String)> = Vec::new();
        for cell in cells {
            let state = format!(
                "{:?}/{:?}/{:?}/{:?}/{:?}",
                cell.fg, cell.bg, cell.underline_color, cell.modifier, cell.diff_option
            );
            match runs.last_mut().filter(|last| last.1 == state) {
                Some(last) => last.0 += 1,
                None => runs.push((1, state)),
            }
        }
        format!("{text:?}\n{runs:?}")
    };
    for depth in [
        tmt_cli_style::Depth::TrueColor,
        tmt_cli_style::Depth::Ansi16,
    ] {
        app.view.as_mut().unwrap().look = crate::look::Look {
            depth,
            ..Default::default()
        };
        app.selected = 1;
        let normal = snapshot(&app);
        app.selected = 0;
        let selected = snapshot(&app);
        let view = app.view.as_mut().unwrap();
        view.rows.lines[1][2].token = None;
        *view.derived.borrow_mut() = Default::default();
        app.selected = 1;
        let control = snapshot(&app);
        assert_ne!(
            normal, control,
            "removing the token must break the pending snapshot"
        );
        app.view.as_mut().unwrap().rows.lines[1][2].token = Some(Role::Waiting);
        *app.view.as_ref().unwrap().derived.borrow_mut() = Default::default();
        let expected = match depth {
            tmt_cli_style::Depth::TrueColor => TRUE_COLOR,
            tmt_cli_style::Depth::Ansi16 => ANSI16,
            _ => unreachable!("snapshot covers colored terminal depths"),
        };
        assert_eq!(format!("{normal}\n{selected}"), expected, "{depth:?}");
    }
}

#[test]
fn a_cell_shows_its_resolved_color_token_as_decoration() {
    let mut app = board(json!([{"title": null, "rows": [
        row("auth-fix", "working", "rotate", json!({"colors": {"task": "blocked"}})),
        row("docs", "working", "write", json!({
            "colors": {"task": "review"},
            "staleness": {"state": "stale", "ageMs": 3_600_000},
        })),
        row("ci", "working", "fix", json!({})),
    ]}]));
    app.selected = 0;
    let cell = |app: &App, name: &str| {
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen = draw(app, 60, 8);
        let y = screen.iter().position(|line| line.contains(name)).unwrap();
        let x = screen[y].find(match name {
            "auth-fix" => "rotate",
            "docs" => "write",
            _ => "fix",
        });
        buffer[(x.unwrap() as u16, y as u16)].clone()
    };
    let role = |app: &App, role: Role| app.look().role(role).fg.unwrap_or_default();
    // The selected row keeps its background and the cell keeps its token.
    let selected = cell(&app, "auth-fix");
    assert_eq!(selected.fg, role(&app, Role::Blocked));
    assert_eq!(Some(selected.bg), app.look().selection().bg);
    assert!(!selected.modifier.contains(Modifier::REVERSED));
    // On a stale (dim) row the cell's own color still shows.
    assert_eq!(cell(&app, "docs").fg, role(&app, Role::Review));
    // No token: no color of its own.
    assert_ne!(cell(&app, "ci").fg, role(&app, Role::Blocked));
    // Without color the text is all there is.
    app.view.as_mut().unwrap().look = crate::look::Look {
        theme: tmt_cli_style::Theme::default(),
        depth: tmt_cli_style::Depth::None,
    };
    assert_eq!(cell(&app, "auth-fix").fg, ratatui::style::Color::Reset);
}

#[test]
fn default_look_keeps_unselected_body_at_terminal_foreground() {
    let mut app = board(json!([{ "title": null, "rows": [
            row("docs", "working", "write", json!({})),
            row("ci", "working", "fix", json!({})),
        ] }]));
    app.view.as_mut().unwrap().look = crate::look::Look::default();
    let mut terminal = Terminal::new(TestBackend::new(60, 7)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    for x in [2, 22] {
        assert_eq!(buffer[(x, 4)].fg, ratatui::style::Color::Reset);
    }
    assert_eq!(buffer[(2, 3)].fg, app.look().selection().fg.unwrap());
    assert_eq!(buffer[(2, 3)].bg, app.look().selection().bg.unwrap());
}

#[test]
fn light_body_chrome_and_selection_use_the_theme_and_no_color_keeps_focus() {
    let mut app = board(json!([{ "title": null, "rows": [
            row("docs", "working", "write", json!({})),
            row("ci", "working", "fix", json!({})),
            row("empty", "", "", json!({"fields": {}})),
        ] }]));
    for depth in [
        tmt_cli_style::Depth::TrueColor,
        tmt_cli_style::Depth::Ansi16,
        tmt_cli_style::Depth::None,
    ] {
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::TmtLight),
            depth,
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 7)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let fg = |role| app.look().role(role).fg.unwrap_or_default();
        // Chrome uses the theme; unselected body keeps the terminal foreground.
        for (x, y) in [(1, 1), (2, 2), (1, 6), (12, 0)] {
            assert_eq!(buffer[(x, y)].fg, fg(Role::Muted), "chrome {x},{y}");
        }
        assert_eq!(buffer[(2, 4)].fg, ratatui::style::Color::Reset);
        assert_eq!(buffer[(13, 5)].fg, fg(Role::Dim));
        assert_eq!(buffer[(22, 5)].fg, fg(Role::Dim));
        assert_eq!(buffer[(1, 0)].fg, fg(Role::Accent));
        assert!(buffer[(2, 0)].modifier.contains(Modifier::BOLD));
        let selected = &buffer[(2, 3)];
        assert_eq!(selected.fg, fg(Role::Text));
        assert_eq!(selected.bg, app.look().selection().bg.unwrap_or_default());
        assert_eq!(
            selected.modifier.contains(Modifier::REVERSED),
            depth != tmt_cli_style::Depth::TrueColor
        );
        assert_eq!(
            buffer[(2, 0)].modifier.contains(Modifier::REVERSED),
            depth != tmt_cli_style::Depth::TrueColor
        );
    }
}

#[test]
fn selected_tabs_keep_foregrounds_and_geometry_with_selection_background() {
    for base in tmt_cli_style::Base::ALL {
        for depth in [
            tmt_cli_style::Depth::TrueColor,
            tmt_cli_style::Depth::Ansi16,
            tmt_cli_style::Depth::None,
        ] {
            let look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            };
            let selection = look.selection();
            for (attention, text) in [
                (Attention::default(), "  product "),
                (
                    Attention {
                        waiting: 2,
                        blocked: 0,
                    },
                    "◆ product 2 ",
                ),
                (
                    Attention {
                        waiting: 0,
                        blocked: 1,
                    },
                    "✗ product 1 ",
                ),
                (
                    Attention {
                        waiting: 2,
                        blocked: 1,
                    },
                    "◆ product 2 ✗1 ",
                ),
            ] {
                let selected = tab(look, "product", true, attention, &TabColors::default());
                let unselected = tab(look, "product", false, attention, &TabColors::default());
                assert_eq!(selected.to_string(), text);
                assert_eq!(selected.to_string(), unselected.to_string());
                assert_eq!(selected.width(), unselected.width());
                for (label, chosen) in [(selected, true), (unselected, false)] {
                    let normal = if chosen {
                        Style {
                            bg: selection.bg,
                            ..look
                                .role(Role::Accent)
                                .add_modifier(Modifier::BOLD | selection.add_modifier)
                        }
                    } else {
                        look.role(Role::Muted)
                    };
                    let width = label.width() as u16;
                    let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
                    terminal
                        .draw(|frame| {
                            frame.render_widget(Paragraph::new(label), Rect::new(0, 0, width, 1))
                        })
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    assert_eq!(
                        buffer[(2, 0)].symbol(),
                        "p",
                        "names always start after the two-cell slot"
                    );
                    for x in 0..width {
                        let role = if x == 0 && attention.waiting > 0 {
                            Some(Role::Waiting)
                        } else if (x == 0 && attention.blocked > 0)
                            || (attention.waiting > 0
                                && attention.blocked > 0
                                && (12..14).contains(&x))
                        {
                            Some(Role::Blocked)
                        } else {
                            None
                        };
                        let expected = role.map_or(normal, |role| Style {
                            bg: normal.bg,
                            ..look.role(role).add_modifier(
                                Modifier::BOLD | (normal.add_modifier & Modifier::REVERSED),
                            )
                        });
                        let cell = &buffer[(x, 0)];
                        assert_eq!(
                            cell.fg,
                            expected.fg.unwrap_or_default(),
                            "{base:?} {depth:?} {attention:?} selected={chosen} at {x}"
                        );
                        assert_eq!(cell.bg, expected.bg.unwrap_or_default());
                        assert_eq!(cell.modifier, expected.add_modifier);
                    }
                }
            }
            let selected = pane_tab(look, "detail", true);
            let unselected = pane_tab(look, "detail", false);
            assert_eq!(selected.content, "[detail]");
            assert_eq!(unselected.content, " detail ");
            assert_eq!(selected.width(), unselected.width());
            assert_eq!(selected.style.fg, look.role(Role::Accent).fg);
            assert_eq!(selected.style.bg, selection.bg);
            assert!(selected.style.add_modifier.contains(Modifier::BOLD));
            assert_eq!(
                selected.style.add_modifier.contains(Modifier::REVERSED),
                selection.bg.is_none()
            );
            assert_eq!(unselected.style, look.role(Role::Muted));
        }
    }
}

#[test]
fn tab_fallback_depends_on_background_even_with_an_accent_foreground() {
    let look = crate::look::Look {
        theme: tmt_cli_style::Theme::parse("theme", [("base", "terminal"), ("accent", "blue")])
            .unwrap(),
        depth: tmt_cli_style::Depth::TrueColor,
    };
    assert!(look.role(Role::Accent).fg.is_some());
    assert!(look.selection().bg.is_none());
    for style in [
        tab(
            look,
            "product",
            true,
            Attention::default(),
            &TabColors::default(),
        )
        .style,
        pane_tab(look, "detail", true).style,
    ] {
        assert_eq!(style.fg, look.role(Role::Accent).fg);
        assert!(style.add_modifier.contains(Modifier::REVERSED));
    }
}

#[test]
fn stale_rows_and_notes_are_quiet_and_say_how_old() {
    let age = |state: &str, ms: u64| json!({"state": state, "ageMs": ms});
    let mut app = board(json!([
        {"title": "working", "rows": [
            row("auth-fix", "working", "rotate", json!({"staleness": age("stale", 3 * 3_600_000)})),
            row("docs", "working", "write", json!({"staleness": age("fresh", 60_000)})),
            row("ci", "working", "fix", json!({"staleness": {"state": "unknown"}})),
        ]},
        // The same member twice: every row of it carries the same age.
        {"title": "again", "rows": [
            row("auth-fix", "working", "rotate", json!({"staleness": age("stale", 3 * 3_600_000)})),
        ]},
    ]));
    app.selected = 1;
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let screen = draw(&app, 60, 12);
    let line_of = |name: &str, from: usize| {
        from + screen[from..]
            .iter()
            .position(|line| line.contains(name))
            .unwrap()
    };
    let stale = line_of("auth-fix", 0);
    assert!(screen[stale].ends_with("stale 3h"), "{screen:#?}");
    let dim = app.look().role(Role::Dim).fg;
    assert_eq!(Some(buffer[(4, stale as u16)].fg), dim, "the row is quiet");
    let again = line_of("auth-fix", stale + 1);
    assert!(screen[again].ends_with("stale 3h"), "repeated rows agree");
    for name in ["docs", "ci"] {
        let line = line_of(name, 0);
        assert!(!screen[line].contains("stale"), "{name}: no mark");
        assert_ne!(Some(buffer[(4, line as u16)].fg), dim);
    }

    // Narrow: the age goes first, the cells stay.
    let narrow = draw(&app, 24, 12);
    let line = narrow.iter().find(|line| line.contains("auth")).unwrap();
    assert!(!line.contains("stale"), "{narrow:#?}");

    // Without color the age is still there in words.
    app.view.as_mut().unwrap().look = crate::look::Look {
        theme: tmt_cli_style::Theme::default(),
        depth: tmt_cli_style::Depth::None,
    };
    assert!(draw(&app, 60, 12)[stale].ends_with("stale 3h"));
}

#[test]
fn stale_lead_notes_say_so_on_the_pane_title() {
    let mut app = paned(
        split(
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            vec![50, 50],
        ),
        Notes::Text("ship it".into()),
    );
    let title = |app: &App| draw(app, 70, 8)[2].clone();
    assert!(!title(&app).contains("stale"), "unknown notes: no mark");
    app.view.as_mut().unwrap().document["squad"]["notesStaleness"] =
        json!({"state": "stale", "ageMs": 2 * 3_600_000});
    let line = title(&app);
    assert!(line.contains("notes · sol · stale 2h"), "{line}");
    let mut terminal = Terminal::new(TestBackend::new(70, 8)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let at = line[..line.find("stale 2h").unwrap()].chars().count() as u16;
    assert_eq!(
        Some(buffer[(at, 2)].fg),
        app.look().role(Role::Waiting).fg,
        "the notes' age asks for attention"
    );
}

#[test]
fn nested_splits_draw_rows_beside_detail_over_notes() {
    use crate::split::{Size, Split};
    let board = crate::config::Board {
        mode: BoardMode::Split,
        collapsed: Default::default(),
        fold_below: None,
        panes: vec![Pane::Rows, Pane::Detail, Pane::Notes],
        split: Split::Group {
            direction: Direction::LeftRight,
            children: vec![
                (Size::Percent(60), Split::Pane(Pane::Rows)),
                (
                    Size::Percent(40),
                    Split::simple(
                        Direction::TopBottom,
                        &[Pane::Detail, Pane::Notes],
                        &[40, 60],
                    ),
                ),
            ],
        },
    };
    let mut app = paned(board, Notes::Text("## Now\n- tokens".into()));
    let screen = draw(&app, 100, 23);
    // Rows take 60 of 100 columns; detail sits over notes in the rest.
    let right = |line: &str| line.chars().skip(60).collect::<String>();
    assert!(screen[2].starts_with("┌ rows"), "{screen:#?}");
    assert!(right(&screen[2]).starts_with("┌ detail"), "{screen:#?}");
    let notes_top = screen
        .iter()
        .position(|line| right(line).starts_with("┌ notes · sol"))
        .expect("notes block");
    // 40% of the 20 body lines is detail: notes start 8 lines below it.
    assert_eq!(notes_top, 2 + 8, "{screen:#?}");
    assert!(screen.iter().any(|line| right(line).contains("auth-fix")));
    // Tab walks the panes in reading order.
    for expected in [Pane::Detail, Pane::Notes, Pane::Rows] {
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focused(), expected);
    }
}

#[test]
fn detail_notebook_uses_safe_markdown_and_existing_scroll() {
    let app = board(
        json!([{"rows":[row("worker", "working", "ship", json!({"id":"W", "lifetime":"saved"}))]}]),
    );
    let safe = super::super::notes::sanitize(
        "## Current state\n- Now: **testing**\n\u{1b}[31mSafe\nNext: review\nBlocked: none",
    );
    app.notebooks
        .borrow_mut()
        .keep("W".into(), Notes::Text(safe));
    let full = detail_text(&detail_buffer(&app, 50, 20)).join("\n");
    assert!(
        full.contains("─ notebook ─") && full.contains("Current state") && full.contains("testing")
    );
    assert!(!full.contains("**") && !full.contains('\u{1b}'));
    let buffer = detail_buffer(&app, 50, 20);
    assert!(!buffer[(0, 4)].modifier.contains(Modifier::BOLD));
    assert_eq!(buffer[(49, 4)].symbol(), "─");
    detail_buffer(&app, 16, 5);
    app.scrolls
        .scroll(Pane::Detail, super::super::scroll::Step::Bottom);
    assert!(
        detail_text(&detail_buffer(&app, 16, 5))
            .join("")
            .contains("none")
    );
    app.notebooks.borrow_mut().keep("W".into(), Notes::Missing);
    app.scrolls
        .scroll(Pane::Detail, super::super::scroll::Step::Top);
    assert!(detail_text(&detail_buffer(&app, 50, 20)).contains(&"(no notes yet)".into()));
}

fn detail_buffer(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render_detail(frame, app, Rect::new(0, 0, width, height)))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn detail_text(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
fn detail_ignores_retired_notes_and_represented_fields_do_not_repeat() {
    let mut app = board(json!([{"title": null, "rows": [row(
        "worker", "working", "rotate tokens", json!({
            "state": "working", "presence": "active", "pending": "approve",
            "pane": {"target": "crew:2.0", "cwd": "/work"},
            "note": "needs a call", "activity": {"activity": "testing"},
            "fields": {"state": "working", "task": "rotate tokens", "pr_link": "https://example.com/412"}
        })
    )]}]));
    // Includes all represented fields, even those missing from fields.
    app.view.as_mut().unwrap().rows = rows_from(
        "[p.columns]\nshow = ['member', 'state', 'task', 'pending', 'note', 'activity', 'presence', 'target', 'cwd', 'link', 'pr_link']\n",
    );
    assert_eq!(
        detail_text(&detail_buffer(&app, 100, 20)),
        [
            "worker",
            "waiting on you: approve",
            "active · working · crew:2.0 · /work",
            "task: rotate tokens",
            "activity: testing",
            "links: pr_link https://example.com/412",
            &format!("─ notebook {}", "─".repeat(89)),
            "(temporary identity: no notebook)",
        ]
    );
    for column in &app.view.as_ref().unwrap().rows.columns {
        assert!(detail_represents(&column.field), "{}", column.field);
    }
    for field in ["pr", "model", "build", "review", "location"] {
        assert!(!detail_represents(field), "{field}");
    }
    // The actual default preset is identical.
    let mut default = preset_board();
    let before = detail_text(&detail_buffer(&default, 100, 20));
    default.view.as_mut().unwrap().rows = columns();
    assert_eq!(detail_text(&detail_buffer(&default, 100, 20)), before);
}

#[test]
fn detail_appends_full_projected_provider_and_bound_values_in_column_order() {
    let mut app = board(json!([{"title": null, "rows": [row(
        "worker", "working", "rotate tokens", json!({
            "fields": {"task": "rotate tokens", "pr": "#412 open · changes requested", "model": "a full session model name"}
        })
    )]}]));
    app.view.as_mut().unwrap().rows = rows_from(
        "[p.fields.pr]\npreset = 'github-pr'\n[p.rows]\ncolumns = [{name = 'pr', width = 4, title = 'Pull request', from = 'fields.pr'}, {name = 'model', width = 4, from = 'session.model'}]\n",
    );
    let buffer = detail_buffer(&app, 80, 20);
    assert_eq!(
        detail_text(&buffer),
        [
            "worker",
            "– · –",
            "task: rotate tokens",
            "pr: #412 open · changes requested",
            "model: a full session model name",
            &format!("─ notebook {}", "─".repeat(69)),
            "(temporary identity: no notebook)",
        ]
    );
    // New lines inherit the terminal foreground; no grid/provider tint.
    assert_eq!(buffer[(0, 3)].fg, ratatui::style::Color::Reset);
    assert_eq!(buffer[(4, 3)].fg, ratatui::style::Color::Reset);
}

#[test]
fn detail_wraps_long_values_without_grid_truncation() {
    let value = "abcdefghijklmnopqrstuvwxyz0123456789";
    let mut app = board(json!([{"title": null, "rows": [row(
        "worker", "working", "", json!({"fields": {"model": value}})
    )]}]));
    app.view.as_mut().unwrap().rows =
        rows_from("[p.rows]\ncolumns = [{name = 'model', width = 4}]\n");
    let text = detail_text(&detail_buffer(&app, 9, 20));
    assert_eq!(
        text[2..]
            .iter()
            .take_while(|line| !line.starts_with("─ note"))
            .cloned()
            .collect::<String>(),
        format!("model:{value}")
    );
    assert!(!text.join("").contains('…'));
    // Existing bounds handle zero area and single-cell panes.
    detail_buffer(&app, 0, 0);
    detail_buffer(&app, 1, 1);
}

#[test]
fn detail_uses_failed_missing_and_shared_cell_escaping() {
    let mut app = board(json!([{"title": null, "rows": [row(
        "worker", "working", "", json!({
            "fields": {"pr": "stale provider value", "model": "", "build": "one\ntwo\t\u{1b}[31m"},
            "failed": ["pr"]
        })
    )]}]));
    app.view.as_mut().unwrap().rows =
        rows_from("[p.columns]\nshow = ['pr', 'model', 'review', 'build']\n");
    assert_eq!(
        detail_text(&detail_buffer(&app, 100, 20))[2..],
        [
            "pr: ?",
            "model: –",
            "review: –",
            &format!(
                "build: {}",
                tmt_cli_style::table::escape("one\ntwo\t\u{1b}[31m")
            ),
            &format!("─ notebook {}", "─".repeat(89)),
            "(temporary identity: no notebook)",
        ]
    );
}

#[test]
fn split_panes_follow_direction_and_sizes() {
    let app = paned(
        split(
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            vec![60, 40],
        ),
        Notes::Text("## Now\n- tokens: waiting on Ben".into()),
    );
    let screen = draw(&app, 100, 11);
    // 60% of 100 columns: the notes block starts at column 60.
    let notes_at = screen[2].find("┌ notes · sol").expect("notes block title");
    assert_eq!(screen[2][..notes_at].chars().count(), 60, "{screen:#?}");
    assert!(screen[2].starts_with("┌ rows"));
    assert!(
        screen.iter().any(|line| line.contains("│  Now")),
        "markdown heading"
    );
    assert!(screen.iter().any(|line| line.contains("◆ auth-fix")));

    let app = paned(
        split(
            Direction::TopBottom,
            vec![Pane::Rows, Pane::Detail],
            vec![50, 50],
        ),
        Notes::NotShown,
    );
    let screen = draw(&app, 70, 23);
    let detail_row = screen
        .iter()
        .position(|line| line.starts_with("┌ detail"))
        .unwrap();
    assert_eq!(
        detail_row, 12,
        "detail starts halfway down the 20-line body"
    );
    let detail = screen[detail_row..].join("\n");
    assert!(
        !detail.contains("needs a call"),
        "retired note stays hidden"
    );
    for expected in [
        "waiting on you: approve the plan",
        "active · blocked · crew:2.0 · /w/app-3",
        "task: rotate tokens",
        "links: pr_link https://example.com/pull/412",
    ] {
        assert!(detail.contains(expected), "{expected}\n{detail}");
    }
}

#[test]
fn replies_show_recipient_age_prompt_full_sanitized_body_and_result_hints() {
    assert_eq!(age(100_000, 55_000), "45s");
    assert_eq!(age(3_600_000, 0), "1h");
    assert_eq!(age(200_000_000, 0), "2d");
    assert_eq!(
        age(0, 5_000),
        "0s",
        "a clock behind the final is not negative"
    );
    let body = "```text\nline 1\n\u{1b}[31mred\u{1b}[0m\n3\n4\n5\n6\n7\n8\n```";
    let mut replies = vec![
        json!({"requestId": "r1", "to": "sol", "prompt": "[product · auth-fix] split", "status": "retained", "submittedAtMs": 40_000, "response": body}),
        json!({"requestId": "r2", "to": "docs", "prompt": "check", "status": "expired", "submittedAtMs": 30_000, "response": null}),
    ];
    for index in 0..BODIES {
        replies.push(json!({"requestId": format!("old{index}"), "to": "sol", "prompt": "p", "status": "retained", "submittedAtMs": 0, "response": null}));
    }
    let lines: Vec<String> = reply_lines(
        crate::look::Look::default(),
        &replies,
        40,
        100_000,
        &mut Default::default(),
    )
    .iter()
    .map(|line| {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
            .trim_end()
            .to_owned()
    })
    .collect();
    assert_eq!(lines[0], "sol · 1m");
    assert_eq!(lines[1], "  › [product · auth-fix] split");
    assert_eq!(
        lines[2..12],
        [
            "  ```text",
            "  line 1",
            "  red",
            "  3",
            "  4",
            "  5",
            "  6",
            "  7",
            "  8",
            "  ```"
        ],
        "the full code source survives and escapes are removed"
    );
    assert!(!lines.iter().any(|line| line.contains('…')));
    assert!(lines.contains(&"  (final expired)".to_owned()));
    assert!(
        lines.contains(&format!("  tmt result old{}", BODIES - 1)),
        "older finals point to tmt result"
    );
    assert!(!lines.iter().any(|line| line.contains('\u{1b}')));
}

fn reply_text(lines: &[Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn long_reply() -> Value {
    json!({
        "requestId": "long-reply", "to": "lead", "submittedAtMs": 0,
        "prompt": format!("Please review {} PROMPT-END", "the complete implementation and verification evidence ".repeat(5)),
        "response": format!("# Full reply\n\n- **First** item with `code`\n- second item\n\n```text\n{}\n```\n\nFINAL-SENTINEL", (1..=40).map(|n| format!("line {n:02}")).collect::<Vec<_>>().join("\n"))
    })
}

#[test]
fn full_markdown_replies_wrap_prompts_and_neutralize_hostile_input_at_all_widths() {
    for width in [40, 80, 120] {
        let mut reply = long_reply();
        let body = reply["response"]
            .as_str()
            .unwrap()
            .replace("Full reply", "\u{1b}]0;bad\u{7}Full reply\u{202e}")
            .replace("First", "\u{9b}31mFirst\u{0}");
        reply["response"] = json!(body);
        let mut derived = Default::default();
        let lines = reply_lines(
            crate::look::Look::default(),
            &[reply],
            width,
            60_000,
            &mut derived,
        );
        let text = reply_text(&lines);
        let all = text.join("\n");
        for expected in [
            "Full reply",
            "• First item with code",
            "• second item",
            "```text",
            "line 40",
            "FINAL-SENTINEL",
            "PROMPT-END",
        ] {
            assert!(all.contains(expected), "{width}: missing {expected}: {all}");
        }
        assert!(!all.contains('…'));
        assert!(
            !all.chars()
                .any(|c| (c.is_control() && c != '\n') || c == '\u{202e}')
        );
        assert!(text.iter().all(|line| line.width() <= width));
        let prompt_end = text
            .iter()
            .position(|line| line.contains("PROMPT-END"))
            .unwrap();
        assert!(prompt_end > 1);
        assert!(
            text[2..=prompt_end]
                .iter()
                .all(|line| line.starts_with("    "))
        );
        let heading = lines
            .iter()
            .find(|line| reply_text(std::slice::from_ref(line))[0].contains("Full reply"))
            .unwrap();
        assert_eq!(heading.spans[0].content, "  ");
        assert!(
            heading
                .spans
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(
            lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content == "code"
                    && span.style == crate::look::Look::default().role(Role::Accent))
        );
    }
}

#[test]
fn replies_scroll_with_keys_pages_and_wheel_to_the_complete_end() {
    for width in [40, 80, 120] {
        let mut app = board(json!([]));
        let view = app.view.as_mut().unwrap();
        view.board = crate::config::Board::simple(
            BoardMode::Tabs,
            Direction::LeftRight,
            vec![Pane::Replies],
            &[],
        );
        view.replies = vec![long_reply()];
        let top = draw(&app, width, 16).join("\n");
        assert!(top.contains("Please review"), "{top}");
        assert!(!top.contains("FINAL-SENTINEL"));
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.scrolls.offset(Pane::Replies), 1);
        app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert!(app.scrolls.offset(Pane::Replies) > 1);
        app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(app.scrolls.offset(Pane::Replies), 0);
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 1,
                row: 5,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        assert_eq!(
            app.scrolls.offset(Pane::Replies),
            crate::board::scroll::WHEEL_LINES
        );
        app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let bottom = draw(&app, width, 16).join("\n");
        assert!(bottom.contains("FINAL-SENTINEL"), "{bottom}");
        assert!(bottom.contains("↑ "));
        assert!(!bottom.contains("↓ "));
        app.key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert!(!draw(&app, width, 16).join("\n").contains("FINAL-SENTINEL"));
    }
}

#[test]
fn reply_bodies_reuse_cache_while_ages_change_and_invalidate_on_width_look_and_view() {
    let replies = vec![long_reply()];
    let look = crate::look::Look::default();
    let mut derived = super::super::derived::Derived::default();
    let first = reply_lines(look, &replies, 80, 60_000, &mut derived);
    let cached = derived.replies.as_ref().unwrap().bodies["long-reply"].as_ptr();
    let later = reply_lines(look, &replies, 80, 120_000, &mut derived);
    assert_ne!(first[0], later[0], "ages remain live");
    assert_eq!(
        cached,
        derived.replies.as_ref().unwrap().bodies["long-reply"].as_ptr(),
        "age repaint reuses Markdown"
    );
    assert_eq!(first[1..], later[1..]);
    let narrow = reply_lines(look, &replies, 40, 120_000, &mut derived);
    assert_eq!(derived.replies.as_ref().unwrap().width, 40);
    assert_ne!(narrow.len(), later.len());
    let mut light = look;
    light.theme = tmt_cli_style::Theme::new(tmt_cli_style::Base::TmtLight);
    let light_lines = reply_lines(light, &replies, 40, 120_000, &mut derived);
    assert_eq!(derived.replies.as_ref().unwrap().look, light);
    assert_ne!(light_lines, narrow);
    let mut app = board(json!([]));
    app.view.as_mut().unwrap().derived.replace(derived);
    app.apply(crate::board::app::tests::snapshot("product", json!([])));
    assert!(
        app.view
            .as_ref()
            .unwrap()
            .derived
            .borrow()
            .replies
            .is_none()
    );
}

#[test]
fn reply_empty_hints_and_consecutive_body_headings_keep_their_meaning() {
    let mut app = board(json!([]));
    app.view.as_mut().unwrap().board = crate::config::Board::simple(
        BoardMode::Tabs,
        Direction::LeftRight,
        vec![Pane::Replies],
        &[],
    );
    assert!(
        draw(&app, 120, 16)
            .join("\n")
            .contains("(tmt squad me <name> shows the replies to your requests)")
    );
    app.view.as_mut().unwrap().me = Some("user".into());
    assert!(
        draw(&app, 120, 16)
            .join("\n")
            .contains("(no replies to your squad requests yet)")
    );
    for ending in ["- final item", "```text\nlast code\n```"] {
        let replies = vec![
            json!({"requestId":"first", "to":"first lead", "response":ending}),
            json!({"requestId":"second", "to":"second lead", "response":"# Body heading"}),
        ];
        let lines = reply_lines(
            crate::look::Look::default(),
            &replies,
            40,
            0,
            &mut Default::default(),
        );
        let text = reply_text(&lines);
        let header = text.iter().position(|line| line == "second lead").unwrap();
        assert_eq!(text[header - 1], "");
        assert_eq!(text[header + 2], "  Body heading");
        assert!(text[header - 2].starts_with("  "));
    }
}

#[test]
fn every_row_is_one_line_cut_by_display_width_at_any_width() {
    let long = "rotate session tokens without logging everyone out of every device";
    let mut app = board(json!([{"title": null, "rows": [
        row("ascii-member-with-a-long-name", "blocked", long, json!({
            "fields": {"state": "blocked", "task": long, "pr_link": "https://github.com/wkh237/tmt/pull/4242"}
        })),
        row("文件整理小组成员", "进行中", "整理安装指南和常见问题并补充截图说明", json!({})),
        row("mix-混合-🚀", "review", "修 bug in 登录 flow 🚀 then ship", json!({})),
    ]}]));
    app.view.as_mut().unwrap().rows = crate::rows::Rows::preset();
    for width in [30u16, 44, 60, 100] {
        let screen = draw(&app, width, 9);
        // Tabs, summary, header, three rows, blank, blank, footer: nothing
        // wrapped.
        for (line, name) in screen[3..6].iter().zip(["ascii-", "文件", "mix-"]) {
            assert!(line.contains(name), "at {width}: {screen:#?}");
        }
        assert_eq!(screen[6], "", "at {width}: a row spilled: {screen:#?}");
        for line in &screen[2..6] {
            assert!(line.width() <= usize::from(width), "at {width}: {line:?}");
        }
    }
}

#[test]
fn tabs_carry_attention_by_color_and_count_and_the_summary_has_its_own_line() {
    let mut app = board(json!([{"title": null, "rows": [
        row("auth-fix", "blocked", "rotate", json!({"pending": "approve"})),
    ]}]));
    app.attention = BTreeMap::from([
        (
            "product".into(),
            Attention {
                waiting: 1,
                blocked: 1,
            },
        ),
        (
            "reviews".into(),
            Attention {
                waiting: 0,
                blocked: 2,
            },
        ),
    ]);
    let mut terminal = Terminal::new(TestBackend::new(60, 6)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let tabs: String = (0..60)
        .map(|x| buffer[(x, 0)].symbol().to_owned())
        .collect();
    // Counts say what the color says, so no meaning is color-only.
    assert_eq!(tabs.trim_end(), "◆ product 1 ✗1  ✗ reviews 2");
    let column = |name: &str| tabs[..tabs.find(name).unwrap()].chars().count() as u16;
    let product = &buffer[(column("product"), 0)];
    // Waiting wins over blocked; selection is bold without moving the tab.
    let fg = |role| app.look().role(role).fg.unwrap_or_default();
    assert_eq!(product.fg, fg(Role::Accent));
    assert_eq!(buffer[(0, 0)].fg, fg(Role::Waiting));
    assert!(product.modifier.contains(Modifier::BOLD));
    assert!(!product.modifier.contains(Modifier::REVERSED));
    let reviews = &buffer[(column("reviews"), 0)];
    assert_eq!(reviews.fg, fg(Role::Muted));
    assert_eq!(buffer[(column("reviews") - 2, 0)].fg, fg(Role::Blocked));
    assert!(!reviews.modifier.contains(Modifier::REVERSED));

    let screen = draw(&app, 60, 6);
    assert_eq!(screen[1], "lead sol · 1 member · 1 waiting on you");
    // Colors come from [tabs.colors]; without a lead the summary says so.
    let view = app.view.as_mut().unwrap();
    view.tab_colors.blocked = "review".into();
    view.document["squad"]["lead"] = Value::Null;
    app.attention.clear();
    app.attention.insert(
        "reviews".into(),
        Attention {
            waiting: 0,
            blocked: 2,
        },
    );
    let mut terminal = Terminal::new(TestBackend::new(60, 6)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let tabs: String = (0..60)
        .map(|x| buffer[(x, 0)].symbol().to_owned())
        .collect();
    let reviews = tabs[..tabs.find("reviews").unwrap()].chars().count() as u16;
    assert_eq!(
        buffer[(reviews - 2, 0)].fg,
        app.look().role(Role::Review).fg.unwrap_or_default()
    );
    assert_eq!(draw(&app, 60, 6)[1], "no lead · 1 member");
}

#[test]
fn attention_counts_keep_shared_marks_and_tab_width_without_color() {
    let mut app = board(json!([{"title": null, "rows": []}]));
    app.view.as_mut().unwrap().look.depth = tmt_cli_style::Depth::None;
    let counts = Attention {
        waiting: 2,
        blocked: 1,
    };
    app.attention.insert("product".into(), counts);
    assert_eq!(Mark::Failed.symbol().width(), 1);
    assert_eq!(
        tab(
            app.look(),
            "product",
            false,
            Attention::default(),
            &TabColors::default()
        )
        .to_string(),
        "  product "
    );
    let colors = TabColors::default();
    let plain = tab(app.look(), "product", false, counts, &colors);
    let selected = tab(app.look(), "product", true, counts, &colors);
    assert_eq!(selected.to_string(), "◆ product 2 ✗1 ");
    assert_eq!(selected.to_string(), plain.to_string());
    assert_eq!(selected.width(), "◆ product 2 ✗1 ".width());
    assert_eq!(selected.style.fg, None);
    assert!(draw(&app, 60, 8)[0].contains("◆ product 2 ✗1"));
    app.switcher = Some(Switcher::new("product".into()));
    assert!(
        draw(&app, 60, 8)
            .iter()
            .any(|line| line.contains("◆ product 2 ✗1"))
    );
}

#[test]
fn tab_hits_cover_the_slot_name_and_trailing_cell_of_the_rendered_label() {
    use crate::board::app::Effect;
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    for attention in [
        Attention::default(),
        Attention {
            waiting: 1,
            blocked: 2,
        },
    ] {
        for offset in [0, 2, 14] {
            let mut app = board(json!([{"title": null, "rows": []}]));
            app.attention.insert("reviews".into(), attention);
            draw(&app, 80, 8);
            let hit = app.tab_hits.borrow()[1];
            let label = tab(
                app.look(),
                "reviews",
                false,
                attention,
                &TabColors::default(),
            );
            assert_eq!(usize::from(hit.width), label.width());
            let x = hit.x + offset.min(hit.width - 1);
            assert_eq!(
                app.mouse(
                    MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: x,
                        row: hit.y,
                        modifiers: KeyModifiers::NONE,
                    },
                    std::time::Instant::now()
                ),
                Effect::Load("reviews".into())
            );
            assert_eq!(app.current.as_deref(), Some("reviews"));
        }
    }
}

#[test]
fn default_attention_color_does_not_inherit_the_selected_name_foreground() {
    let look = crate::look::Look::default();
    let label = tab(
        look,
        "product",
        true,
        Attention {
            waiting: 1,
            blocked: 1,
        },
        &TabColors {
            waiting: "default".into(),
            blocked: "default".into(),
        },
    );
    for rendered in [label.clone(), Line::from(label.spans)] {
        let width = rendered.width() as u16;
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(Paragraph::new(rendered), frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].fg, Style::new().fg.unwrap_or_default());
        assert_eq!(buffer[(12, 0)].fg, Style::new().fg.unwrap_or_default());
        assert_eq!(buffer[(2, 0)].fg, look.role(Role::Accent).fg.unwrap());
        for x in 0..width {
            assert_eq!(buffer[(x, 0)].bg, look.selection().bg.unwrap());
        }
    }
}

#[test]
fn switcher_fitting_keeps_mark_styles_alignment_and_its_selected_row() {
    for name in ["product", "wide-界界界界界界", "literal…name"] {
        for width in [1, 2, 8, 12, 40, 80, 160] {
            for depth in [tmt_cli_style::Depth::TrueColor, tmt_cli_style::Depth::None] {
                let mut app = board(json!([{ "title": null, "rows": [] }]));
                app.tabs = vec![name.into()];
                app.hidden.clear();
                let view = app.view.as_mut().unwrap();
                view.look.depth = depth;
                view.tab_colors = TabColors {
                    waiting: "review".into(),
                    blocked: "link".into(),
                };
                app.attention.insert(
                    name.into(),
                    Attention {
                        waiting: 1,
                        blocked: 2,
                    },
                );
                app.switcher = Some(Switcher::default());
                let look = app.look();
                let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
                terminal
                    .draw(|frame| {
                        render_switcher(frame, &app, app.switcher.as_ref().unwrap(), frame.area())
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let surface = app.switcher.as_ref().unwrap().surface.borrow();
                let geometry = &surface
                    .frame
                    .as_ref()
                    .unwrap()
                    .list
                    .as_ref()
                    .unwrap()
                    .geometry;
                for row in geometry {
                    if row.visible.width == 0 || row.visible.height == 0 {
                        continue;
                    }
                    for x in [row.visible.x, row.visible.right() - 1] {
                        let cell = &buffer[(x, row.visible.y)];
                        assert_eq!(cell.bg, look.selection().bg.unwrap_or_default());
                        assert_eq!(
                            cell.modifier.contains(Modifier::REVERSED),
                            look.selection().add_modifier.contains(Modifier::REVERSED)
                        );
                    }
                }
                for cell in &buffer.content {
                    let role = match cell.symbol() {
                        "◆" => Role::Review,
                        "✗" => Role::Link,
                        _ => continue,
                    };
                    assert!(cell.modifier.contains(Modifier::BOLD));
                    let expected = if look.selection().bg.is_some() {
                        look.role(role).fg.unwrap_or_default()
                    } else {
                        look.role(Role::Text).fg.unwrap_or_default()
                    };
                    assert_eq!(cell.fg, expected);
                }
            }
        }
    }
}

#[test]
fn the_leads_tab_is_labelled_leads_and_counts_squad_leads() {
    let mut view = board(json!([{"title": null, "rows": [
            row("sol", "working", "plan", json!({"fields": {"squad": "product", "state": "working", "task": "plan"}})),
            row("rin", "blocked", "ci", json!({"fields": {"squad": "infra", "state": "blocked", "task": "ci"}})),
        ]}]))
        .view
        .take()
        .unwrap();
    view.rows = crate::rows::Rows::leads();
    let mut app = App::new(Some(crate::board::LEADS.into()));
    app.apply(Snapshot {
        squad_keys: Vec::new(),
        tabs: vec!["product".into(), crate::board::LEADS.into()],
        hidden: Vec::new(),
        pinned: 0,
        attention: Default::default(),
        squad: Some(crate::board::LEADS.into()),
        view: Ok(view),
    });
    let screen = draw(&app, 60, 6);
    assert_eq!(screen[0], "  product    leads");
    assert_eq!(screen[1], "2 squad leads");
    assert_eq!(screen[2], "  SQUAD          LEAD           STATE      TASK");
    assert_eq!(screen[3], "  product        sol            working    plan");
}

#[test]
fn tabs_move_with_shift_arrows_or_a_drag_and_the_order_is_saved() {
    use crate::board::app::{Effect, Request};
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = board(json!([{"title": null, "rows": []}]));
    app.tabs.push(crate::board::LEADS.into());
    let order = |app: &App| app.tabs.clone();
    let shift = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
    assert_eq!(
        app.key(shift(KeyCode::Right)),
        Effect::Act(Request::Reorder(
            ["reviews", "product", crate::board::LEADS]
                .map(String::from)
                .to_vec()
        ))
    );
    assert_eq!(app.current.as_deref(), Some("product"), "still shown");
    assert_eq!(
        app.key(shift(KeyCode::Right)),
        Effect::Act(Request::Reorder(order(&app)))
    );
    assert_eq!(
        app.key(shift(KeyCode::Right)),
        Effect::None,
        "no wrap at the end"
    );

    // Drag: press on the first tab (showing it), release over the last.
    let screen = draw(&app, 60, 6);
    assert_eq!(screen[0], "  reviews    leads    product");
    let mouse = |kind, column| MouseEvent {
        kind,
        column,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    assert_eq!(
        app.mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 2),
            std::time::Instant::now()
        ),
        Effect::Load("reviews".into())
    );
    assert_eq!(
        app.mouse(
            mouse(MouseEventKind::Up(MouseButton::Left), 22),
            std::time::Instant::now()
        ),
        Effect::Act(Request::Reorder(
            ["leads", "product", "reviews"]
                .map(|name| if name == "leads" {
                    crate::board::LEADS.to_owned()
                } else {
                    name.to_owned()
                })
                .to_vec()
        ))
    );
    // Releasing off the tab line moves nothing.
    app.mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), 2),
        std::time::Instant::now(),
    );
    let mut away = mouse(MouseEventKind::Up(MouseButton::Left), 2);
    away.row = 4;
    assert_eq!(app.mouse(away, std::time::Instant::now()), Effect::None);
}

#[test]
fn many_tabs_scroll_to_keep_the_current_one_and_count_the_rest() {
    // Every tab is 8 columns with its gap; 40 columns hold four, or three
    // beside one count.
    let widths = [8u16; 10];
    assert_eq!(tab_window(&[8, 8], Some(1), 0, 40), (0, 2), "all fit");
    assert_eq!(tab_window(&widths, Some(0), 0, 40), (0, 4));
    assert_eq!(tab_window(&widths, Some(3), 0, 40), (0, 4));
    // Moving right scrolls only as far as needed, then left keeps it.
    let (start, end) = tab_window(&widths, Some(4), 0, 40);
    assert!(start > 0 && (start..end).contains(&4), "{start}..{end}");
    assert_eq!(tab_window(&widths, Some(4), start, 40), (start, end));
    assert_eq!(tab_window(&widths, Some(9), start, 40).1, 10);
    assert_eq!(tab_window(&widths, Some(2), 5, 40).0, 2);
    // A tab wider than the line still shows.
    assert_eq!(tab_window(&[80, 8], Some(0), 0, 40), (0, 1));

    let names: Vec<String> = (0..9).map(|n| format!("sq{n}")).collect();
    let mut app = board(json!([{"title": null, "rows": []}]));
    app.tabs = names.clone();
    app.current = Some("sq7".into());
    app.attention.insert(
        "sq1".into(),
        Attention {
            waiting: 0,
            blocked: 1,
        },
    );
    app.attention.insert(
        "sq8".into(),
        Attention {
            waiting: 2,
            blocked: 0,
        },
    );
    let mut terminal = Terminal::new(TestBackend::new(32, 6)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let line: String = (0..32)
        .map(|x| buffer[(x, 0)].symbol().to_owned())
        .collect();
    assert!(line.starts_with("‹ "), "{line:?}");
    assert!(
        line.contains(" sq7 "),
        "the current tab stays in view: {line:?}"
    );
    assert!(line.trim_end().ends_with("1 ›"), "{line:?}");
    // The left count hides a blocked tab, the right one a waiting tab.
    assert_eq!(
        buffer[(0, 0)].fg,
        app.look().role(Role::Blocked).fg.unwrap()
    );
    let right = line.trim_end().chars().count() as u16 - 1;
    assert_eq!(
        buffer[(right, 0)].fg,
        app.look().role(Role::Waiting).fg.unwrap()
    );
    // Only shown tabs can be clicked, at their drawn places.
    let hits = app.tab_hits.borrow().clone();
    assert!(hits.iter().all(|hit| hit.tab >= app.tab_start.get()));
    let seven = hits.iter().find(|hit| hit.tab == 7).unwrap();
    let at = line[..line.find("  sq7").unwrap()].chars().count() as u16;
    assert_eq!(seven.x, at);
}

#[test]
fn a_hidden_squad_being_shown_leads_the_tab_line_selected() {
    let mut app = board(json!([{"title": null, "rows": []}]));
    app.tabs = (0..9).map(|n| format!("sq{n}")).collect();
    app.hidden = vec!["quiet".into()];
    app.current = Some("quiet".into());
    app.attention.insert(
        "quiet".into(),
        Attention {
            waiting: 1,
            blocked: 2,
        },
    );
    let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let line: String = (0..40)
        .map(|x| buffer[(x, 0)].symbol().to_owned())
        .collect();
    assert!(line.starts_with("◆ quiet (hidden) 1 ✗2 "), "{line:?}");
    assert!(buffer[(2, 0)].modifier.contains(Modifier::BOLD));
    assert_eq!(buffer[(2, 0)].fg, app.look().role(Role::Accent).fg.unwrap());
    assert_eq!(
        buffer[(0, 0)].fg,
        app.look().role(Role::Waiting).fg.unwrap()
    );
    assert_eq!(
        buffer[(19, 0)].fg,
        app.look().role(Role::Blocked).fg.unwrap()
    );
    assert!(line.trim_end().ends_with(" ›"), "{line:?}");
    // It is not one of the tabs, so it cannot be clicked or dragged, and
    // the tabs after it are hit where they are drawn.
    let hits = app.tab_hits.borrow().clone();
    let first = hits.iter().find(|hit| hit.tab == 0).unwrap();
    let at = line[..line.find("  sq0").unwrap()].chars().count() as u16;
    assert_eq!(first.x, at, "{line:?}");
    assert!(hits.iter().all(|hit| hit.x >= at));
}

#[test]
fn the_switcher_filters_every_tab_and_opens_the_chosen_one() {
    use crate::board::app::Effect;
    let mut app = board(json!([{"title": null, "rows": []}]));
    app.tabs.push(crate::board::LEADS.into());
    app.hidden = vec!["quiet".into()];
    app.attention.insert(
        "reviews".into(),
        Attention {
            waiting: 1,
            blocked: 0,
        },
    );
    let press = |app: &mut App, code| app.key(KeyEvent::new(code, KeyModifiers::NONE));
    press(&mut app, KeyCode::Char('s'));
    let mut screen = draw(&app, 60, 12);
    // The guideline caps centered overlays at 80% of the body. Verify
    // every offered tab by scrolling the shared viewport, not a taller box.
    for _ in 0..app.switchable().len() {
        press(&mut app, KeyCode::Down);
        screen.extend(draw(&app, 60, 12));
    }
    let body = screen.join("\n");
    for expected in [
        "Enter opens · Esc closes",
        " product",
        "◆ reviews 1",
        " leads",
        " quiet (hidden)",
    ] {
        assert!(body.contains(expected), "{expected}: {body}");
    }
    for character in "qt".chars() {
        press(&mut app, KeyCode::Char(character));
    }
    let screen = draw(&app, 60, 12);
    let listed: Vec<&str> = screen
        .iter()
        .filter_map(|line| line.split('│').nth(1))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        listed,
        [
            "› qt▏",
            "quiet (hidden)",
            "1–1 of 1",
            "↑↓ choose · Enter opens · Esc closes"
        ],
        "{screen:#?}"
    );
    // Enter shows the hidden squad; the switcher closes.
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        Effect::Load("quiet".into())
    );
    assert!(app.switcher.is_none());
    // Esc closes without switching; `s` bound by the user runs the binding.
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.switcher.is_none());
    app.view.as_mut().unwrap().bindings =
        crate::action::parse_bindings([("s", Some("refresh"))].into_iter(), "bind").unwrap();
    assert_eq!(press(&mut app, KeyCode::Char('s')), Effect::Refresh);
    assert!(app.switcher.is_none());
}

#[test]
fn pinned_tabs_stay_in_view_and_keep_their_pin_order() {
    use crate::board::app::Effect;
    let mut app = board(json!([{"title": null, "rows": []}]));
    app.tabs = std::iter::once(crate::board::ALL.to_owned())
        .chain((0..9).map(|n| format!("sq{n}")))
        .collect();
    app.pinned = 1;
    app.current = Some("sq8".into());
    let line = draw(&app, 36, 6)[0].clone();
    assert!(
        line.starts_with("  all  ‹ 6 "),
        "the pin stays first: {line:?}"
    );
    assert!(
        line.ends_with(" sq8"),
        "the current tab is in view: {line:?}"
    );
    let hits = app.tab_hits.borrow().clone();
    assert_eq!(hits[0].tab, 0);
    assert_eq!(hits[0].x, 0);
    // A pin neither moves nor is passed; the other tabs move among
    // themselves. (A saved `order` could not reorder the pins.)
    let shift = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
    let refused = Some("Pinned tabs keep the order in [tabs] pin.");
    app.current = Some("sq0".into());
    assert_eq!(app.key(shift(KeyCode::Left)), Effect::None);
    assert_eq!(app.notice.as_deref(), refused);
    app.pinned = 2;
    app.current = Some(crate::board::ALL.into());
    app.notice = None;
    assert_eq!(app.key(shift(KeyCode::Right)), Effect::None);
    assert_eq!(app.notice.as_deref(), refused);
    assert_eq!(app.tabs[..2], [crate::board::ALL, "sq0"]);
    app.pinned = 1;
    app.current = Some("sq0".into());
    assert!(matches!(app.key(shift(KeyCode::Right)), Effect::Act(_)));
    assert_eq!(app.tabs[..3], [crate::board::ALL, "sq1", "sq0"]);
}

#[test]
fn switching_squads_never_moves_a_tab_or_blanks_the_frame() {
    let mut app = board(json!([
        {"title": null, "rows": [row("auth-fix", "blocked", "rotate", json!({}))]}
    ]));
    let before = draw(&app, 60, 6);
    let place = |line: &str, name: &str| line.find(name).unwrap();
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    app.loading_since = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    let during = draw(&app, 60, 6);
    for name in ["product", "reviews"] {
        assert_eq!(
            place(&before[0], name),
            place(&during[0], name),
            "{name} moved: {before:?} / {during:?}"
        );
    }
    assert_eq!(app.current.as_deref(), Some("reviews"));
    // Selection is a style, so the tab text is the same either way.
    assert!(during[0].starts_with("  product    reviews "), "{during:?}");
    assert_eq!(before[0].trim_end(), "  product    reviews");
    assert!(
        during[1].contains("loading"),
        "a slow switch shows a spinner"
    );
    assert!(
        during.iter().any(|line| line.contains("auth-fix")),
        "the previous frame stays: {during:?}"
    );
    assert!(!during.iter().any(|line| line.contains("Loading…")));
}

#[test]
fn tabs_show_one_pane_and_tab_moves_focus() {
    let tabs = crate::config::Board::simple(
        BoardMode::Tabs,
        Direction::LeftRight,
        vec![Pane::Rows, Pane::Replies, Pane::Notes],
        &[],
    );
    let mut app = paned(tabs, Notes::Missing);
    let screen = draw(&app, 70, 11);
    assert!(
        screen[2].starts_with("[rows]  replies   notes"),
        "{screen:#?}"
    );
    assert!(screen.iter().any(|line| line.contains("auth-fix")));
    let tab = |app: &mut App| {
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    };
    tab(&mut app);
    let screen = draw(&app, 70, 11);
    // Same columns as before the switch: only the brackets move.
    assert!(
        screen[2].starts_with(" rows  [replies]  notes"),
        "{screen:#?}"
    );
    assert!(
        screen
            .iter()
            .any(|line| line.contains("tmt squad me <name> shows the replies"))
    );
    tab(&mut app);
    assert!(
        draw(&app, 70, 10)
            .iter()
            .any(|line| line.contains("(no notes yet)"))
    );
    tab(&mut app);
    assert_eq!(app.focused(), Pane::Rows, "focus wraps around");
}

fn wheel(app: &mut App, column: u16, row: u16, down: bool) {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    app.mouse(
        MouseEvent {
            kind: if down {
                MouseEventKind::ScrollDown
            } else {
                MouseEventKind::ScrollUp
            },
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
}

#[test]
fn the_wheel_scrolls_the_pane_under_the_pointer_whichever_is_focused() {
    let text = (1..=30)
        .map(|n| format!("line {n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = paned(
        split(
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            vec![50, 50],
        ),
        Notes::Text(text),
    );
    app.view.as_mut().unwrap().render = NotesRender::Plain;
    let before = draw(&app, 60, 11);
    assert!(before.iter().any(|line| line.contains("line 01")));
    assert!(
        before.iter().any(|line| line.contains("25 more ↓")),
        "an overflowing pane says how much is below: {before:#?}"
    );
    // Rows is focused; the wheel over the notes (right half) moves them.
    wheel(&mut app, 45, 5, true);
    wheel(&mut app, 45, 5, true);
    let after = draw(&app, 60, 11);
    assert!(
        !after.iter().any(|line| line.contains("line 06")),
        "{after:#?}"
    );
    assert!(after.iter().any(|line| line.contains("line 07")));
    assert!(after.iter().any(|line| line.contains("↑ 6  19 more ↓")));
    wheel(&mut app, 45, 5, false);
    assert!(
        draw(&app, 60, 11)
            .iter()
            .any(|line| line.contains("line 04"))
    );
    assert_eq!(app.selected, 0, "the rows' selection never moves");
}

#[test]
fn a_click_focuses_the_pane_under_it() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let text = (1..=30)
        .map(|n| format!("line {n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = paned(
        split(
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            vec![50, 50],
        ),
        Notes::Text(text),
    );
    app.view.as_mut().unwrap().render = NotesRender::Plain;
    let click = |app: &mut App, column, row| {
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        )
    };
    draw(&app, 60, 11);
    assert_eq!(app.focused(), Pane::Rows);
    // A click focuses notes and places the source-line cursor; Down moves it.
    assert_eq!(click(&mut app, 45, 5), crate::board::Effect::None);
    assert_eq!(app.focused(), Pane::Notes);
    assert_eq!(app.selected, 0);
    let clicked = app.note_cursors.borrow()["product"].source;
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let screen = draw(&app, 60, 11);
    assert_eq!(app.note_cursors.borrow()["product"].source, clicked + 1);
    assert!(
        screen
            .iter()
            .any(|line| line.contains(&format!("line {:02}", clicked + 2)))
    );
    // The rows border now shows the notes as focused.
    let mut terminal = Terminal::new(TestBackend::new(60, 11)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(buffer[(31, 2)].modifier.contains(Modifier::BOLD));
    assert!(!buffer[(1, 2)].modifier.contains(Modifier::BOLD));
    // A click on a row focuses the rows again and selects it.
    let hit = app.hits.borrow()[0];
    click(&mut app, hit.x, hit.y);
    assert_eq!(app.focused(), Pane::Rows);
    assert_eq!(app.selected, hit.row);
    // Outside every pane, or under the help, a click changes nothing.
    click(&mut app, 45, 5);
    assert_eq!(app.focused(), Pane::Notes);
    click(&mut app, 5, 0);
    assert_eq!(app.focused(), Pane::Notes, "the header is not a pane");
    app.help = true;
    click(&mut app, 5, 5);
    assert_eq!(app.focused(), Pane::Notes, "the help takes no clicks");
}

#[test]
fn the_wheel_scrolls_rows_away_from_the_selection_until_a_key_brings_it_back() {
    let rows: Vec<Value> = (0..20)
        .map(|i| row(&format!("m{i:02}"), "working", "", json!({})))
        .collect();
    let mut app = board(json!([{"title": null, "rows": rows}]));
    draw(&app, 40, 8);
    for _ in 0..3 {
        wheel(&mut app, 5, 4, true);
    }
    let screen = draw(&app, 40, 8);
    assert!(
        !screen.iter().any(|line| line.contains("m00")),
        "{screen:#?}"
    );
    assert!(screen.iter().any(|line| line.contains("m09")));
    assert_eq!(app.selected, 0);
    // Only visible rows take clicks, at their screen lines.
    assert!(app.hits.borrow().iter().all(|hit| hit.row >= 8));
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let screen = draw(&app, 40, 8);
    assert!(
        screen.iter().any(|line| line.contains("m01")),
        "{screen:#?}"
    );
    // PgDn and End page the selection when they are not bound.
    app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    assert_eq!(app.selected, 19);
    assert!(draw(&app, 40, 8).iter().any(|line| line.contains("m19")));
    app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    assert_eq!(app.selected, 0);
}

#[test]
fn a_bound_paging_key_runs_its_binding_instead_of_scrolling() {
    let rows: Vec<Value> = (0..20)
        .map(|i| row(&format!("m{i:02}"), "working", "", json!({})))
        .collect();
    let mut app = board(json!([{"title": null, "rows": rows}]));
    app.view.as_mut().unwrap().bindings =
        crate::action::parse_bindings([("pagedown", Some("copy {name}"))].into_iter(), "bind")
            .unwrap();
    draw(&app, 40, 8);
    let effect = app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(app.selected, 0, "no paging");
    assert!(
        matches!(effect, crate::board::app::Effect::Act(_)),
        "{effect:?}"
    );
}

#[test]
fn notebook_links_preview_before_click_and_honor_explicit_bindings() {
    use crate::board::app::{Effect, Request};
    let mut app = paned(
        split(
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            vec![50, 50],
        ),
        Notes::Text("[界 wide words](https://example.com) [other](tmt:back)".into()),
    );
    draw(&app, 60, 15);
    let (hit, _) = app.link_hits.borrow()[0];
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.x,
        row: hit.y,
        modifiers: KeyModifiers::NONE,
    };
    assert_eq!(app.mouse(click, std::time::Instant::now()), Effect::None);
    assert_eq!(app.focused_pane(), Some(Pane::Notes));
    assert!(
        draw(&app, 60, 15)
            .last()
            .unwrap()
            .contains("https://example.com")
    );
    assert!(
        matches!(app.mouse(click, std::time::Instant::now()), Effect::Act(Request::Open { link, .. }) if link == "https://example.com")
    );
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.selected_link().unwrap().target, "tmt:back");
    app.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.selected_link().unwrap().target, "https://example.com");
    app.view.as_mut().unwrap().configured_bindings = crate::action::parse_bindings(
        [("tab", Some("next-pane")), ("enter", Some("copy {name}"))].into_iter(),
        "bind",
    )
    .unwrap();
    assert!(matches!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::Act(Request::Copy { .. })
    ));
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
}

#[test]
fn notebook_cursor_click_resize_refresh_annotation_and_cancel() {
    use crate::board::app::{Effect, Request};
    let mut app = paned(
        split(
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            vec![50, 50],
        ),
        Notes::Text("# First\n\n- selected source line that wraps at narrow widths\n- last".into()),
    );
    app.view.as_mut().unwrap().me = Some("Ben".into());
    draw(&app, 60, 15);
    let (hit, _) = *app
        .note_hits
        .borrow()
        .iter()
        .find(|(_, source)| *source == 2)
        .unwrap();
    assert_eq!(
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: hit.x,
                row: hit.y,
                modifiers: KeyModifiers::NONE
            },
            std::time::Instant::now()
        ),
        Effect::None
    );
    assert_eq!(app.note_cursors.borrow()["product"].source, 2);
    assert_eq!(app.selected, 0);
    draw(&app, 30, 15);
    assert_eq!(app.note_cursors.borrow()["product"].source, 2);
    app.view.as_mut().unwrap().notes = Notes::Text(
        "new\n# First\n\n- selected source line that wraps at narrow widths\n- last".into(),
    );
    app.view.as_mut().unwrap().derived = Default::default();
    draw(&app, 60, 15);
    assert_eq!(app.note_cursors.borrow()["product"].source, 3);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        Effect::None
    );
    let input = app.input.as_ref().unwrap();
    assert!(
        input
            .prompt
            .starts_with("note for sol · L4 “selected source")
    );
    assert!(input.text.is_empty());
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Effect::None
    );
    assert!(app.input.is_none());
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(
        matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Effect::Act(Request::Annotate { me, squad, to, row, text })
            if me == "Ben" && squad == "product" && to == "sol" && row.starts_with("notes L4 ") && text == "x")
    );
    app.key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
    assert_eq!(app.note_cursors.borrow()["product"].source, 4);
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.note_cursors.borrow()["product"].source, 0);
    app.current = Some("loading".into());
    draw(&app, 60, 15);
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    draw(&app, 60, 15);
    assert!(!app.note_cursors.borrow().contains_key("loading"));
}

#[test]
fn sent_marker_reserves_space_without_clipping_wrapped_source_text() {
    let text = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut app = paned(
        split(
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            vec![50, 50],
        ),
        Notes::Text(text.into()),
    );
    app.view.as_mut().unwrap().render = NotesRender::Plain;
    app.view.as_mut().unwrap().document["squad"]["noteAnnotations"] =
        json!([{"line": 0, "quote": text, "requestId": "open"}]);
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let mut content = String::new();
    for (area, _) in app.note_hits.borrow().iter() {
        for x in area.x..area.x + area.width {
            content.extend(
                terminal.backend().buffer()[(x, area.y)]
                    .symbol()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric()),
            );
        }
    }
    assert_eq!(content, text, "the marker cannot hide any wrapped content");
}

#[test]
fn notebook_cursor_uses_existing_selection_and_sent_marker_clears_on_answer() {
    for (base, depth) in [
        (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
        (
            tmt_cli_style::Base::TmtLight,
            tmt_cli_style::Depth::TrueColor,
        ),
        (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
    ] {
        let mut app = paned(
            split(
                Direction::LeftRight,
                vec![Pane::Rows, Pane::Notes],
                vec![50, 50],
            ),
            Notes::Text(
                "selected source line that wraps across multiple painted rows\nother".into(),
            ),
        );
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(base),
            depth,
        };
        app.view.as_mut().unwrap().render = NotesRender::Plain;
        app.focus = 1;
        app.view.as_mut().unwrap().document["squad"]["noteAnnotations"] =
            json!([{"line": 0, "quote": "selected", "requestId": "open"}]);
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let (hit, _) = app.note_hits.borrow()[0];
        let cell = &terminal.backend().buffer()[(hit.x + hit.width - 1, hit.y)];
        let selection = app.look().selection();
        assert_eq!(cell.bg, selection.bg.unwrap_or_default());
        assert_eq!(
            cell.modifier.contains(Modifier::REVERSED),
            selection.bg.is_none()
        );
        let sources = app
            .view
            .as_ref()
            .unwrap()
            .derived
            .borrow()
            .notes
            .as_ref()
            .unwrap()
            .sources
            .clone();
        for (area, at) in app.note_hits.borrow().iter() {
            if sources[*at] == 0 {
                let cell = &terminal.backend().buffer()[(area.x + area.width - 1, area.y)];
                assert_eq!(cell.bg, selection.bg.unwrap_or_default());
                assert_eq!(
                    cell.modifier.contains(Modifier::REVERSED),
                    selection.bg.is_none()
                );
            }
        }
        assert!(
            draw(&app, 60, 12)
                .iter()
                .any(|line| line.contains("✎ selected"))
        );
        app.view.as_mut().unwrap().document["squad"]
            .as_object_mut()
            .unwrap()
            .remove("noteAnnotations");
        assert!(
            !draw(&app, 60, 12)
                .iter()
                .any(|line| line.contains("✎ selected"))
        );
    }
}

#[test]
fn focused_notes_scroll_while_rows_keep_their_selection() {
    let text = (1..=30)
        .map(|n| format!("line {n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = paned(
        split(
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            vec![50, 50],
        ),
        Notes::Text(text),
    );
    // Plain rendering keeps one note line per display line to scroll by.
    app.view.as_mut().unwrap().render = NotesRender::Plain;
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    for _ in 0..5 {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    let screen = draw(&app, 60, 10);
    assert!(
        screen.iter().any(|line| line.contains("line 06")),
        "{screen:#?}"
    );
    assert!(!screen.iter().any(|line| line.contains("line 01")));
    assert_eq!(app.selected, 0);
}
#[test]
fn loading_is_delayed_animated_and_absent_on_a_cached_switch() {
    let mut app = board(json!([]));
    let started = std::time::Instant::now();
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    app.loading_since = Some(started);
    assert_eq!(
        spinner_frame(
            &app,
            started + SPINNER_DELAY - std::time::Duration::from_millis(1)
        ),
        None
    );
    assert_eq!(spinner_frame(&app, started + SPINNER_DELAY), Some(0));
    assert_eq!(
        spinner_frame(&app, started + SPINNER_DELAY + SPINNER_TICK),
        Some(1)
    );
    app.apply(crate::board::app::tests::snapshot("reviews", json!([])));
    app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert!(!app.loading());
    assert_eq!(spinner_frame(&app, std::time::Instant::now()), None);
    assert!(
        draw(&app, 60, 8)
            .iter()
            .all(|line| !line.contains("loading"))
    );
}

#[test]
fn derivations_survive_selection_and_change_with_width_search_and_snapshot() {
    let mut app = board(
        json!([{ "title": null, "rows": [row("first", "working", "one", json!({})), row("second", "working", "two", json!({}))] }]),
    );
    let view = app.view.as_mut().unwrap();
    view.board = crate::config::Board::simple(
        BoardMode::Tabs,
        Direction::LeftRight,
        vec![Pane::Rows, Pane::Notes],
        &[],
    );
    view.notes = Notes::Text("# Notes\nA sentence that wraps at a narrow width.".into());
    draw(&app, 60, 12);
    assert_eq!(
        app.view
            .as_ref()
            .unwrap()
            .derived
            .borrow()
            .grid
            .as_ref()
            .unwrap()
            .width,
        56
    );
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(
        draw(&app, 60, 12)
            .iter()
            .any(|line| line.contains("second"))
    );
    app.search = "second".into();
    draw(&app, 35, 12);
    let grid = app.view.as_ref().unwrap().derived.borrow();
    assert_eq!(grid.grid.as_ref().unwrap().search, "second");
    assert_eq!(grid.grid.as_ref().unwrap().width, 31);
    drop(grid);
    app.focus = 1;
    draw(&app, 60, 12);
    let lines = app
        .view
        .as_ref()
        .unwrap()
        .derived
        .borrow()
        .notes
        .as_ref()
        .unwrap()
        .lines
        .clone();
    draw(&app, 25, 12);
    assert_ne!(
        app.view
            .as_ref()
            .unwrap()
            .derived
            .borrow()
            .notes
            .as_ref()
            .unwrap()
            .lines,
        lines
    );
    app.apply(crate::board::app::tests::snapshot("product", json!([])));
    assert!(app.view.as_ref().unwrap().derived.borrow().notes.is_none());
    assert!(app.view.as_ref().unwrap().derived.borrow().grid.is_none());
}
#[test]
fn only_visible_reply_age_text_invalidates_the_clock() {
    let mut app = board(json!([]));
    let view = app.view.as_mut().unwrap();
    view.board = crate::config::Board::simple(
        BoardMode::Tabs,
        Direction::LeftRight,
        vec![Pane::Rows, Pane::Replies],
        &[],
    );
    view.replies = vec![json!({ "submittedAtMs": 1_000 })];
    assert!(time_marks(&app, 61_000).is_empty());
    app.focus = 1;
    let first = time_marks(&app, 61_000);
    assert!(!first.is_empty());
    assert_eq!(first, time_marks(&app, 61_200));
    assert_ne!(first, time_marks(&app, 121_000));
}
fn fold(app: &mut App, pane: Pane) {
    app.perform(&crate::action::Action::parse(&format!("toggle {}", pane.title())).unwrap());
}
fn click_title(app: &mut App, pane: Pane) {
    let hit = *app
        .title_hits
        .borrow()
        .iter()
        .find(|hit| hit.pane == pane)
        .unwrap();
    assert_eq!(
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: hit.area.x,
                row: hit.area.y,
                modifiers: KeyModifiers::NONE
            },
            std::time::Instant::now()
        ),
        Effect::None
    );
}

#[test]
fn fold_render_restores_both_directions_and_keeps_the_mark_muted() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        for base in [tmt_cli_style::Base::Tmt, tmt_cli_style::Base::TmtLight] {
            let mut app = paned(
                split(direction, vec![Pane::Rows, Pane::Detail], vec![60, 40]),
                Notes::NotShown,
            );
            app.view.as_mut().unwrap().look.theme = tmt_cli_style::Theme::new(base);
            let expanded = draw(&app, 80, 23);
            assert!(expanded.iter().any(|line| line.contains("┌ detail")));
            fold(&mut app, Pane::Detail);
            let folded = draw(&app, 80, 23);
            let hit = *app
                .title_hits
                .borrow()
                .iter()
                .find(|hit| hit.pane == Pane::Detail)
                .unwrap();
            assert!(
                folded[hit.area.y as usize].contains("▸ detail"),
                "{folded:#?}"
            );
            assert_eq!(hit.area.height, 1);
            if direction == Direction::LeftRight {
                assert_eq!(hit.area.x, 72);
                let title = &folded[hit.area.y as usize];
                assert_eq!(title.chars().nth(hit.area.x as usize - 1), Some(' '));
                assert_eq!(title.chars().nth(hit.area.x as usize - 2), Some('┐'));
            } else {
                assert_eq!(hit.area.y, 22 - 1);
            }
            assert!(!folded.iter().any(|line| line.contains("waiting on you:")));
            assert!(app.scrolls.pane_at(hit.area.x, hit.area.y).is_none());
            let mut terminal = Terminal::new(TestBackend::new(80, 23)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            assert_eq!(
                terminal.backend().buffer()[(hit.area.x, hit.area.y)].fg,
                app.look().role(Role::Muted).fg.unwrap()
            );
            click_title(&mut app, Pane::Detail);
            assert_eq!(draw(&app, 80, 23), expanded);
        }
    }
}

#[test]
fn nested_and_all_folded_render_titles_only_and_tiny_hits_stay_disjoint() {
    use crate::split::{Size, Split};
    let panes = vec![Pane::Rows, Pane::Detail, Pane::Notes, Pane::Replies];
    let board = crate::config::Board {
        mode: BoardMode::Split,
        panes: panes.clone(),
        collapsed: Default::default(),
        fold_below: None,
        split: Split::Group {
            direction: Direction::LeftRight,
            children: vec![
                (Size::Percent(60), Split::Pane(Pane::Rows)),
                (
                    Size::Percent(40),
                    Split::simple(Direction::TopBottom, &panes[1..], &[30, 30, 40]),
                ),
            ],
        },
    };
    let mut app = paned(board, Notes::Text("SECRET NOTE BODY".into()));
    for pane in &panes[1..] {
        fold(&mut app, *pane);
    }
    let screen = draw(&app, 100, 23);
    assert_eq!(
        app.title_hits
            .borrow()
            .iter()
            .find(|hit| hit.pane == Pane::Detail)
            .unwrap()
            .area
            .x,
        91
    );
    assert!(!screen.iter().any(|line| line.contains("SECRET")));
    fold(&mut app, Pane::Rows);
    let screen = draw(&app, 100, 23);
    assert!(
        screen
            .iter()
            .any(|line| line.starts_with("▸ rows ▸ detail"))
    );
    assert!(app.hits.borrow().is_empty());
    assert_eq!(app.focused_pane(), None);
    for (w, h) in [(20, 9), (10, 6), (3, 4), (1, 1), (0, 0)] {
        draw(&app, w, h);
        let hits = app.title_hits.borrow();
        for (i, hit) in hits.iter().enumerate() {
            assert!(hit.area.right() <= w && hit.area.bottom() <= h);
            for other in &hits[..i] {
                assert!(hit.area.intersection(other.area).is_empty());
            }
        }
    }
}

#[test]
fn title_clicks_bypass_row_bindings_overlays_and_double_click_history() {
    let mut app = paned(
        split(
            Direction::TopBottom,
            vec![Pane::Rows, Pane::Detail],
            vec![50, 50],
        ),
        Notes::NotShown,
    );
    app.view.as_mut().unwrap().bindings.extend([
        (
            "click".into(),
            crate::action::Action::parse("refresh").unwrap(),
        ),
        (
            "double-click".into(),
            crate::action::Action::parse("refresh").unwrap(),
        ),
    ]);
    draw(&app, 80, 23);
    let selected = app.selected;
    app.help = true;
    click_title(&mut app, Pane::Detail);
    assert!(app.collapsed_panes().is_empty());
    app.help = false;
    click_title(&mut app, Pane::Rows);
    assert_eq!(app.selected, selected);
    draw(&app, 80, 23);
    assert!(app.hits.borrow().is_empty());
    click_title(&mut app, Pane::Rows);
    draw(&app, 80, 23);
    let hit = app.hits.borrow()[0];
    let result = app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert_eq!(
        result,
        Effect::Refresh,
        "title clicks never become a row action"
    );
}

#[test]
fn fold_preserves_scroll_and_the_borderless_single_pane_path() {
    let mut app = paned(
        split(
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            vec![60, 40],
        ),
        Notes::Text(
            (0..100)
                .map(|n| format!("line {n}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    );
    draw(&app, 80, 23);
    app.scrolls
        .scroll(Pane::Notes, super::super::scroll::Step::Lines(12));
    fold(&mut app, Pane::Notes);
    draw(&app, 80, 23);
    fold(&mut app, Pane::Notes);
    draw(&app, 80, 23);
    assert_eq!(app.scrolls.offset(Pane::Notes), 12);
    let mut app = paned(
        split(Direction::LeftRight, vec![Pane::Rows], vec![100]),
        Notes::NotShown,
    );
    let before = draw(&app, 60, 12);
    assert!(app.title_hits.borrow().is_empty());
    fold(&mut app, Pane::Rows);
    assert_eq!(draw(&app, 60, 12)[2], "▸ rows");
    click_title(&mut app, Pane::Rows);
    assert_eq!(draw(&app, 60, 12), before);
    assert!(app.title_hits.borrow().is_empty());
}

#[test]
fn folded_rows_clear_visual_positions_and_resume_wrapped_paging_after_expansion() {
    let mut app = board(json!([{ "title": null, "rows": [
            row("a", "", "alpha beta gamma delta", json!({})),
            row("b", "", "alpha beta gamma delta", json!({})),
            row("c", "", "alpha beta gamma delta", json!({})),
            row("d", "", "alpha beta gamma delta", json!({})),
        ] }]));
    let view = app.view.as_mut().unwrap();
    view.board = split(
        Direction::TopBottom,
        vec![Pane::Rows, Pane::Detail],
        vec![50, 50],
    );
    view.rows = rows_from(
        r#"[p.rows]
columns = [{ name = "member", width = "30%" },
           { name = "task", width = "70%", overflow = "wrap", max_lines = 2 }]
"#,
    );
    app.selected = 1;
    draw(&app, 20, 10);
    assert_eq!(*app.row_starts.borrow(), [1, 3, 5, 7]);
    fold(&mut app, Pane::Rows);
    draw(&app, 20, 10);
    assert!(app.row_starts.borrow().is_empty());
    app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(app.selected, 1, "paging detail cannot move hidden rows");
    fold(&mut app, Pane::Detail);
    draw(&app, 20, 10);
    assert_eq!(app.focused_pane(), None);
    for key in [KeyCode::PageDown, KeyCode::Home, KeyCode::End] {
        app.key(KeyEvent::new(key, KeyModifiers::NONE));
        assert_eq!(
            app.selected, 1,
            "all-folded navigation cannot page stale rows"
        );
    }
    fold(&mut app, Pane::Rows);
    draw(&app, 20, 10);
    assert_eq!(*app.row_starts.borrow(), [1, 3, 5, 7]);
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert!(app.selected > 0, "expanded rows resume visual paging");
}

#[test]
fn group_folding_reclaims_row_space_and_unfolding_restores_exact_geometry() {
    let config = crate::config::Config::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/markup-parity.toml"),
    )
    .unwrap();
    let mut app = paned(config.board("team").unwrap(), Notes::Missing);
    app.set_body_width(120);
    let expanded = draw(&app, 120, 42);
    let width = app.hits.borrow()[0].width;
    app.focus = 2;
    app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    draw(&app, 120, 42);
    assert!(
        app.hits.borrow()[0].width > width,
        "rows take the group's space"
    );
    app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    assert_eq!(
        draw(&app, 120, 42),
        expanded,
        "all text geometry is restored exactly"
    );
    assert_eq!(app.hits.borrow()[0].width, width);
    app.focus = 1;
    draw(&app, 120, 42);
    click_title(&mut app, Pane::Detail);
    assert_eq!(
        app.focused_pane(),
        Some(Pane::Rows),
        "mouse folding also returns focus to rows"
    );
    click_title(&mut app, Pane::Detail);
    assert_eq!(
        app.focused_pane(),
        Some(Pane::Rows),
        "mouse unfolding never steals focus"
    );
}

#[test]
fn toggle_footer_and_help_show_current_state_and_drop_the_whole_hint() {
    for panes in [
        vec![Pane::Rows, Pane::Detail, Pane::Replies],
        vec![Pane::Rows, Pane::Detail],
        vec![Pane::Rows, Pane::Replies],
        vec![Pane::Rows],
    ] {
        let mut app = paned(
            split(Direction::LeftRight, panes.clone(), vec![]),
            Notes::NotShown,
        );
        let targets: Vec<_> = panes
            .iter()
            .filter(|pane| [Pane::Detail, Pane::Replies].contains(pane))
            .copied()
            .collect();
        if targets.is_empty() {
            assert!(!app.bindings().contains_key("d"));
            assert!(!help_lines(&app).iter().any(|line| line.starts_with("d ")));
            continue;
        }
        let label = targets
            .iter()
            .map(|pane| pane.title())
            .collect::<Vec<_>>()
            .join("+");
        for folded in [false, true] {
            if folded {
                app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
            }
            let state = if folded { "▸" } else { "▾" };
            let hint = format!("d {label} {state}");
            let full = hints(&app, usize::MAX);
            assert!(full.contains(&hint), "{full}");
            let description = app.bindings()["d"].description();
            assert!(help_lines(&app).contains(&format!("d  {description} (▾ open, ▸ folded)")));
            for width in 0..160 {
                let shown = hints(&app, width);
                if shown.contains(&format!("d {label}")) {
                    assert!(shown.contains(&hint), "{width}: {shown}");
                }
                assert!(!shown.contains('…'));
                assert!(shown.is_empty() || shown == full || full[shown.len()..].starts_with("  "));
            }
        }
        if targets.len() == 2 {
            fold(&mut app, Pane::Detail);
            assert!(
                hints(&app, usize::MAX).contains("d detail+replies ▾"),
                "mixed state uses open mark"
            );
        }
    }
}

#[test]
fn footer_hints_are_conditional_and_effective_bindings_remain_visible() {
    let mut app = paned(
        split(Direction::LeftRight, vec![Pane::Rows], vec![100]),
        Notes::NotShown,
    );
    assert!(!hints(&app, usize::MAX).contains("d detail"));
    app.view.as_mut().unwrap().board = split(
        Direction::TopBottom,
        vec![Pane::Rows, Pane::Detail],
        vec![60, 40],
    );
    let view = app.view.as_mut().unwrap();
    view.bindings = crate::action::preset(true, &view.board.panes);
    assert!(hints(&app, usize::MAX).contains("d detail ▾"));
    assert!(draw(&app, 48, 12)[11].contains("d detail ▾"));
    app.view
        .as_mut()
        .unwrap()
        .bindings
        .insert("d".into(), crate::action::Action::parse("refresh").unwrap());
    assert!(hints(&app, usize::MAX).contains("d refresh"));
    assert!(
        help_lines(&app)
            .iter()
            .any(|line| line.trim_end() == "d  refresh the board now")
    );
}
#[test]
fn folded_reply_age_never_invalidates_the_visible_frame() {
    let mut app = paned(
        split(
            Direction::TopBottom,
            vec![Pane::Rows, Pane::Replies],
            vec![60, 40],
        ),
        Notes::NotShown,
    );
    app.view.as_mut().unwrap().replies = vec![json!({"submittedAtMs":40_000})];
    assert!(!time_marks(&app, 60_000).is_empty());
    fold(&mut app, Pane::Replies);
    assert!(time_marks(&app, 60_000).is_empty());
    fold(&mut app, Pane::Replies);
    assert!(!time_marks(&app, 60_000).is_empty());
}

fn help_lines(app: &App) -> Vec<String> {
    crate::board::help::model(app)
        .sections
        .into_iter()
        .flat_map(|section| section.entries)
        .map(|entry| format!("{}  {}", entry.keys, entry.description))
        .collect()
}
