use super::row_paint::GAP;
use super::{
    detail::{detail_represents, render_detail},
    footer::hints,
    header::{SPINNER_TICK, meter_region, summary_line},
    overlays::render_switcher,
    replies::reply_lines,
    tabs::{pane_tab, tab},
};
use crate::board::app::Switcher;
use crate::{
    attention::Attention,
    config::{NotesRender, TabColors},
    requests::{BODIES, age},
    rows::Rows,
};
use ratatui::widgets::Paragraph;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
};
use serde_json::Value;
use tmt_cli_style::{Role, mark::Mark};
use unicode_width::UnicodeWidthStr;

mod cron;
mod frame_timing;
mod help;
mod menu;
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
fn home_pending_history_has_a_status_row_before_any_usage_is_available() {
    for width in [80, 100, 160] {
        let mut app = board(json!([]));
        let view = app.view.as_mut().unwrap();
        view.home = Some(crate::board::home::Home {
            summary: Default::default(),
            windows: crate::config::TokenRate::default().windows,
            sections: Vec::new(),
            squads: Vec::new(),
            failures: Vec::new(),
            incomplete: false,
        });
        view.history_pending = true;
        let pending = draw(&app, width, 24);
        assert!(pending[2].contains("Updating usage…"));
        assert!(pending[1].contains("0 squads"));
        app.view.as_mut().unwrap().history_pending = false;
        assert!(
            draw(&app, width, 24)
                .iter()
                .all(|line| !line.contains("Updating usage"))
        );
    }
}

#[test]
fn waiting_rows_detail_and_ask_prompt_fit_each_width_and_theme() {
    for width in [160, 100, 80] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            let now = crate::status::now_ms();
            let question =
                "Ship #412 tonight or wait for CI? Token rotation is risky if CI is red.";
            let mut app = board(json!([{"title": null, "rows": [
                row("auth-fix", "blocked", "rotate tokens", json!({"waitingOnYou": [{"requestId": "q", "preview": question, "preparedAtMs": now - 720000}]})),
                row("docs-request", "working", "handbook", json!({"pending": "Keep the glossary?", "waitingOnYou": [{"requestId": "docs-q", "preview": "fallback", "preparedAtMs": now - 720000}]})),
                row("docs", "working", "copy", json!({"pending": "Choose the tone?"})),
            ]}]));
            lead_sol(&mut app);
            let view = app.view.as_mut().unwrap();
            view.bindings = crate::action::preset(true, &[]);
            view.look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::theme::Base::parse(base).unwrap()),
                depth,
            };
            view.me = Some("Ben".into());
            let screen = draw(&app, width, 24);
            let asks = screen
                .iter()
                .find(|line| line.contains("Ship #412"))
                .unwrap();
            assert!(asks.contains("12m"), "{base}/{width}: {asks}");
            let pending = screen
                .iter()
                .find(|line| line.contains("Keep the glossary?"))
                .unwrap();
            assert_eq!(pending.find("12m"), asks.find("12m"), "request ages align");
            let pending_only = screen
                .iter()
                .find(|line| line.contains("Choose the tone?"))
                .unwrap();
            assert!(!pending_only.contains("12m"), "pending-only has no age");
            assert!(screen.last().unwrap().contains("◆ 3 waiting"));
            assert_eq!(screen.last().unwrap().contains("A ask lead"), width >= 100);
            assert!(
                app.hits
                    .borrow()
                    .iter()
                    .any(|hit| hit.row == 1 && screen[usize::from(hit.y)].contains("Ship #412"))
            );
            app.perform(&crate::action::Action::parse("ask-lead").unwrap());
            let prompt = draw(&app, width, 24);
            let title = prompt
                .iter()
                .find(|line| line.contains("→ lead sol"))
                .unwrap();
            assert!(title.starts_with("│ → lead sol"));
            assert!(title.ends_with('│'), "opaque prompt spans the whole band");
            assert_eq!(title.width(), usize::from(width));
            let top = prompt
                .iter()
                .position(|line| line.contains("→ lead sol"))
                .unwrap()
                - 1;
            assert_eq!(
                prompt[top],
                format!("┌{}┐", "─".repeat(usize::from(width) - 2))
            );
            assert!(
                prompt
                    .iter()
                    .any(|line| line.contains("Enter send · Esc cancel"))
            );
            assert!(
                prompt
                    .iter()
                    .any(|line| line.contains("List what waits on me"))
            );
            app.input = None;
            app.select(1);
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal
                .draw(|frame| render_detail(frame, &app, frame.area()))
                .unwrap();
            let detail = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                detail.contains(question),
                "full available preview in detail"
            );
        }
    }
}

#[test]
fn waiting_hint_uses_rebound_key_and_drops_oldest_before_actions() {
    let now = crate::status::now_ms();
    let mut app = board(
        json!([{"title": null, "rows": [row("auth-fix", "working", "", json!({"pending": "approve", "waitingOnYou": [{"preparedAtMs": now - 720000, "preview": "fallback"}]}))]}]),
    );
    app.view.as_mut().unwrap().bindings.remove("A");
    app.view.as_mut().unwrap().bindings.insert(
        "z".into(),
        crate::action::Action::parse("ask-lead").unwrap(),
    );
    assert!(hints(&app, 160).contains("oldest auth-fix 12m"));
    assert!(hints(&app, 120).contains("z ask lead"));
    assert!(!hints(&app, 100).contains("z ask lead"));
    assert!(!hints(&app, 50).contains("oldest"));
    assert!(!hints(&app, 80).contains("oldest"));
    assert!(hints(&app, 100).contains("r reply"));
    assert!(hints(&app, 100).ends_with("? more  q quit"));
    assert!(!hints(&app, 50).contains("A ask lead"));
    assert_eq!(
        super::waiting::text(app.selected_row().unwrap()),
        Some("approve")
    );
}

#[test]
fn footer_omits_whole_hints_instead_of_clipping_words() {
    let mut app = App::new(Some("product".into()));
    app.apply(crate::board::app::tests::snapshot("product", json!([])));
    let full = hints(&app, usize::MAX);
    assert!(
        full.starts_with("Focus: rows  ↑↓ move  A ask lead  / search"),
        "{full}"
    );
    for width in [0, 1, 6, 20, 40, 80, 100, 108, 112, 120, 160] {
        let shown = hints(&app, width);
        assert!(shown.width() <= width);
        if width >= "? more".width() {
            assert!(shown.contains("? more"), "help stays discoverable: {width}");
        }
        assert!(
            shown
                .split("  ")
                .filter(|hint| !hint.is_empty())
                .all(|hint| full.split("  ").any(|whole| whole == hint)),
            "partial hint at {width}: {shown}"
        );
    }
    assert!(!draw(&app, 108, 8).last().unwrap().ends_with("◆ ne"));
}

/// The shown hints without the reserved tail.
fn shown_hints(text: &str) -> Vec<&str> {
    text.split("  ")
        .filter(|hint| !hint.is_empty() && !["q quit", "? more", "Focus: rows"].contains(hint))
        .collect()
}

#[test]
fn footer_orders_hints_by_priority_and_always_keeps_quit_and_more() {
    let mut app = paned(
        split(Direction::LeftRight, vec![Pane::Rows], vec![100]),
        Notes::NotShown,
    );
    app.select(1);
    assert_eq!(app.selected_row().unwrap()["name"], "auth-fix");
    let full = hints(&app, usize::MAX);
    let order = [
        "↑↓ move",
        "⏎ open",
        "a write",
        "r reply",
        "t talk",
        "A ask lead",
        "/ search",
    ];
    let at = |hint: &str| full.find(hint).unwrap_or_else(|| panic!("{hint}: {full}"));
    assert!(
        order.windows(2).all(|pair| at(pair[0]) < at(pair[1])),
        "{full}"
    );
    assert!(!full.contains("next-pane"), "{full}");
    assert!(full.ends_with("? more  q quit"));
    let whole = shown_hints(&full);
    for width in 0..=full.width() {
        let shown = hints(&app, width);
        assert!(shown.width() <= width, "{width}: {shown}");
        match width {
            0..=5 => assert_eq!(shown, ""),
            6..=13 => assert_eq!(shown, "? more", "{width}"),
            _ => assert!(shown.ends_with("? more  q quit"), "{width}: {shown}"),
        }
        // Whole hints only, always a prefix of the priority order.
        let kept = shown_hints(&shown);
        assert_eq!(kept, whole[..kept.len()], "{width}: {shown}");
    }
    // The other keys are help-only, never in the footer.
    for hint in [
        "⌫ back",
        "o open",
        "y copy",
        "tab pane",
        "ctrl-r refresh",
        "T theme",
    ] {
        assert!(!full.contains(hint), "{hint}: {full}");
    }
    // Focus and mandatory hints remain while optional actions fit whole.
    let narrow = hints(&app, 100);
    assert!(
        narrow.contains("⏎ open  a write  r reply  t talk"),
        "{narrow}"
    );
}

#[test]
fn footer_shows_only_the_row_actions_the_selected_row_allows() {
    let mut app = paned(
        split(Direction::LeftRight, vec![Pane::Rows], vec![100]),
        Notes::NotShown,
    );
    // The lead row waits on nothing and has no link: no reply, no open.
    app.select(0);
    assert_eq!(app.selected_row().unwrap()["name"], "sol");
    let lead = hints(&app, usize::MAX);
    for hint in ["⏎ open", "t talk", "a write"] {
        assert!(lead.contains(hint), "{lead}");
    }
    assert!(!lead.contains("r reply"), "{lead}");
    // Only a decision (pending text or an open request) enables reply.
    app.select(1);
    assert!(hints(&app, usize::MAX).contains("r reply"));
    let set = |app: &mut App, key: &str, value: Value| {
        app.view.as_mut().unwrap().document["sections"][0]["rows"][0][key] = value;
    };
    set(&mut app, "pending", Value::Null);
    assert!(!hints(&app, usize::MAX).contains("r reply"));
    set(
        &mut app,
        "waitingOnYou",
        json!([{"requestId": "q-1", "preview": "approve?"}]),
    );
    assert!(hints(&app, usize::MAX).contains("r reply"));
    // No row: board keys only, and the reserved tail.
    let mut empty = App::new(Some("product".into()));
    empty.apply(crate::board::app::tests::snapshot("product", json!([])));
    let text = hints(&empty, usize::MAX);
    for hint in ["⏎", "t talk", "r reply", "a write"] {
        assert!(!text.contains(hint), "{text}");
    }
    assert!(text.contains("/ search") && text.ends_with("? more  q quit"));
    // The plain host's Enter is the row menu; rebinding changes the word.
    let view = app.view.as_mut().unwrap();
    view.bindings =
        crate::action::with_action_keys(crate::action::preset(false, &view.board.panes));
    assert!(hints(&app, usize::MAX).contains("⏎ open"));
}

#[test]
fn footer_labels_use_only_registered_spaced_marks_and_structural_typography() {
    use crate::board::glyph_guard::glyph_error;
    for panes in [
        vec![Pane::Rows],
        vec![Pane::Rows, Pane::Detail],
        vec![Pane::Rows, Pane::Replies],
        vec![Pane::Rows, Pane::Detail, Pane::Replies, Pane::Notes],
    ] {
        for tmux in [true, false] {
            let mut app = paned(
                split(Direction::LeftRight, panes.clone(), vec![]),
                Notes::NotShown,
            );
            let view = app.view.as_mut().unwrap();
            view.bindings =
                crate::action::with_action_keys(crate::action::preset(tmux, &view.board.panes));
            for folded in [false, true] {
                if folded && panes.len() > 1 {
                    app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
                }
                for selected in [0, 1] {
                    app.select(selected);
                    for width in [usize::MAX, 160, 100, 80, 48] {
                        let shown = hints(&app, width);
                        assert_eq!(glyph_error(&shown), None, "{width}: {shown}");
                    }
                }
            }
        }
    }
}

#[test]
fn footer_hints_follow_rebound_keys_and_show_one_hint_per_action() {
    use crate::action::Action;
    let mut app = paned(
        split(Direction::LeftRight, vec![Pane::Rows], vec![100]),
        Notes::NotShown,
    );
    app.select(1);
    let default = hints(&app, usize::MAX);
    let view = app.view.as_mut().unwrap();
    // Rebinding talk moves its hint to the new key and keeps its rank.
    view.bindings.remove("t");
    view.bindings
        .insert("m".into(), Action::parse("talk").unwrap());
    let rebound = hints(&app, usize::MAX);
    assert_eq!(rebound, default.replace("t talk", "m talk"), "{rebound}");
    // An unbound action has no hint.
    let view = app.view.as_mut().unwrap();
    view.bindings.remove("m");
    let unbound = hints(&app, usize::MAX);
    assert_eq!(unbound, default.replace("t talk  ", ""), "{unbound}");
    // A second key for one action adds no hint, and Enter stays the shown key.
    let view = app.view.as_mut().unwrap();
    view.bindings
        .insert("J".into(), Action::parse("jump").unwrap());
    view.bindings
        .insert("x".into(), Action::parse("copy {name}").unwrap());
    let text = hints(&app, usize::MAX);
    let shown = shown_hints(&text);
    assert_eq!(
        shown.iter().filter(|hint| hint.ends_with(" open")).count(),
        1
    );
    assert!(shown.contains(&"⏎ open"), "{shown:?}");
    // Help-only verbs (`copy`, `run`, `notes`) never reach the footer.
    assert!(!shown.iter().any(|hint| hint.ends_with(" copy")));
    let view = app.view.as_mut().unwrap();
    view.bindings
        .insert("R".into(), Action::parse("run true").unwrap());
    assert!(!hints(&app, usize::MAX).contains("R run"));
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

pub(super) fn draw(app: &App, width: u16, height: u16) -> Vec<String> {
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

pub(super) fn board(sections: Value) -> App {
    let mut app = App::new(Some("product".into()));
    app.apply(Snapshot {
        squad_keys: Vec::new(),
        tabs: vec!["product".into(), "reviews".into()],
        hidden: Vec::new(),
        pinned: 0,
        attention: Default::default(),
        squad: Some("product".into()),
        view: Ok(View {
            history_pending: false,
            ask_lead: crate::config::DEFAULT_ASK_LEAD.into(),
            home_replies: true,
            token_rate: None,
            home_rate: Default::default(),
            exchanges: Vec::new(),
            home: None,
            derived: Default::default(),
            document: json!({"squad": {"name": "product", "lead": null}, "sections": sections}),
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
            bindings: crate::action::with_action_keys(crate::action::preset(true, &[])),
            section_bindings: Vec::new(),
            configured_bindings: Default::default(),
            opener: None,
            clipboard: None,
            links: Default::default(),
            tab_colors: Default::default(),
            look: Default::default(),
            theme_notice: None,
            me: None,
            me_id: None,
            replies: Vec::new(),
        }),
    });
    app
}

/// The fixture squad has no lead; this one has `sol`, shown as the first row.
pub(super) fn lead_sol(app: &mut App) {
    let lead = row("sol", "working", "coordinate", json!({"id": "LEAD"}));
    app.view.as_mut().unwrap().document["squad"]["lead"] = lead;
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

/// Legacy four-column fixture; real preset defaults are covered by frozen parity.
fn preset_board() -> App {
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
    app.view.as_mut().unwrap().rows = Rows::preset();
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
fn previous_team_view_is_readable_at_80_120_and_200_columns() {
    let path =
        std::env::temp_dir().join(format!("squad-team-responsive-{}.toml", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let config = crate::config::Config::read(path)
        .unwrap()
        .preview_view(
            &crate::view::ViewScope::Board,
            Some(crate::view::ViewName::Team),
            "product",
        )
        .unwrap();
    let mut app = preset_board();
    let view = app.view.as_mut().unwrap();
    view.rows = config.rows("product").unwrap();
    view.board = config.board("product").unwrap();
    for width in [80, 100, 120, 160, 200] {
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
        assert_eq!(widths.len(), 8);
        assert!(widths[4].is_none_or(|width| width <= 14), "model cap");
        for (earlier, later) in [(7, 6), (6, 3), (3, 4), (4, 5)] {
            assert!(widths[earlier].is_none() || widths[later].is_some());
        }
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

/// Golden: the legacy four-column grid before adoption of the shared solver.
#[test]
fn legacy_four_column_grid_draws_exactly_as_before_at_every_width() {
    let app = preset_board();
    let golden: [(u16, [&str; 8]); 4] = [
        (
            48,
            [
                "  MEMBER         STATE      TASK    PR",
                "NEEDS ME",
                "◆>auth-fix       blocked    rotate… https://git…",
                " >  approve",
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
                "◆>auth-fix       blocked    rotate session tok… https://git…",
                " >  approve",
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
                "◆>auth-fix       blocked    rotate session tokens without logging … https://git…",
                " >  approve",
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
                "◆>auth-fix       blocked    rotate session tokens without logging everyone out                              https://git…",
                " >  approve",
                "EVERYONE",
                "  文件-sweep-lo… working    整理安装指南和常见问题                                                          –",
                "  perf           –          –                                                                               –",
                "    ✎ sent to sol: check the cache hit rate",
            ],
        ),
    ];
    for (width, lines) in golden {
        assert_eq!(draw(&app, width, 12)[2..10], lines, "at {width} columns");
    }
}

#[test]
fn a_narrow_preset_board_drops_the_link_instead_of_clipping() {
    let screen = draw(&preset_board(), 44, 11);
    assert_eq!(screen[2], "  MEMBER         STATE      TASK");
    assert_eq!(screen[4], "◆>auth-fix       blocked    rotate session …");
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
        "◆>auth-fix   blocked   rotate session tokens           #4242"
    );
    assert_eq!(wide[4], " >           approve the rollout plan");
    // Nothing to show on the second line: the row keeps one line.
    assert_eq!(
        wide[5],
        "  docs       working   guide                              #7"
    );
    // Narrow: the prioritized column steps aside and the span shrinks.
    let narrow = draw(&app, 36, 9);
    assert_eq!(narrow[2], "  MEMBER     STATE     TASK");
    assert_eq!(narrow[3], "◆>auth-fix   blocked   rotate sessi…");
    assert_eq!(narrow[4], " >           approve the rollout pl…");
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
    assert_eq!(screen[3], " >a     alpha beta");
    assert_eq!(screen[4], " >      gamma delta");
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
    assert_eq!(draw(&app, 40, 4)[2], " >docs   https://gi…pull/4242");
}

#[test]
fn rows_ignore_retired_notes_and_show_pending_sections_and_aligned_wide_text() {
    let mut app = board(json!([
        {"title": "Needs me", "rows": [row("auth-fix", "blocked", "rotate session tokens", json!({"pending": "approve", "note": "needs a call"}))]},
        {"title": "Everyone", "rows": [row("文件-sweep", "working", "整理安装指南", json!({}))]}
    ]));
    lead_sol(&mut app);
    let screen = draw(&app, 48, 12);
    // The tab line holds only the tabs; the summary has its own line.
    assert_eq!(screen[0], "  [product]   reviews");
    assert_eq!(screen[1], "lead sol · 2 members");
    assert_eq!(screen[2], "  MEMBER     STATE    TASK");
    // The lead is the first row; the rule names what follows.
    assert_eq!(screen[3], " >sol  lead  working  coordinate");
    assert_eq!(screen[4], format!("── members · 2 {}", "─".repeat(33)));
    assert_eq!(screen[5], "NEEDS ME");
    assert_eq!(screen[6], "◆ auth-fix   blocked  rotate session tokens");
    assert_eq!(screen[7], "    approve");
    assert_eq!(screen[8], "EVERYONE");
    assert_eq!(screen[9], "  文件-sweep working  整理安装指南");
    assert!(screen.iter().all(|line| !line.contains("needs a call")));
    assert!(screen[11].starts_with("Focus: rows  ◆ 1 waiting"));
}

#[test]
fn the_lead_tag_sits_in_the_name_cell_and_is_the_first_thing_cut() {
    let draw_lead = |member_width: usize| {
        let mut app =
            board(json!([{"title": null, "rows": [row("docs", "working", "guide", json!({}))]}]));
        lead_sol(&mut app);
        app.view.as_mut().unwrap().rows = rows_from(&format!(
            "[p.rows]\ncolumns = [{{ name = \"member\", width = {member_width} }}, {{ name = \"state\", width = 8 }}]\n"
        ));
        draw(&app, 40, 8)
    };
    // Room for the tag: it follows the name by two cells; other rows are unchanged.
    let screen = draw_lead(10);
    assert_eq!(screen[3], " >sol  lead  working");
    assert_eq!(screen[5], "  docs       working");
    // Narrow cell: the tag shrinks (ellipsis) before the name does, then goes.
    assert_eq!(draw_lead(8)[3], " >sol  le… working");
    assert_eq!(draw_lead(7)[3], " >sol  l… working");
    assert_eq!(draw_lead(6)[3], " >sol    working");
    assert_eq!(draw_lead(3)[3], " >sol working");
}

#[test]
fn a_squad_of_one_has_one_member() {
    let mut app =
        board(json!([{"title": null, "rows": [row("docs", "working", "guide", json!({}))]}]));
    lead_sol(&mut app);
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

    app.view.as_mut().unwrap().bindings =
        crate::action::with_action_keys(crate::action::preset(false, &[]));
    app.help = true;
    // Notes cursor guidance and the settings binding need one more help row.
    let help = help_lines(&app);
    assert!(help.iter().any(|line| line.starts_with("g G")));
    assert!(
        help.iter()
            .any(|line| line == ",  show settings, theme, view and token window")
    );
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
    let screen = draw(&app, 60, 28);
    assert!(screen.iter().any(|line| line.contains("┌ docs ")));
    assert!(
        screen
            .iter()
            .any(|line| line.contains("Enter runs · Esc closes"))
    );
    assert!(screen.iter().any(|line| line.contains("backspace back")));
    assert!(draw(&App::new(None), 60, 4)[3].starts_with("↑↓ move  / search"));
}

#[test]
fn help_groups_view_and_theme_bindings_and_lists_no_default_jump_lead() {
    let mut app = board(json!([]));
    app.view.as_mut().unwrap().bindings =
        crate::action::with_action_keys(crate::action::preset(true, &[]));
    let help = help_lines(&app);
    assert!(help.iter().all(|line| !line.contains("lead's pane")));
    assert!(
        help.iter()
            .any(|line| line == "double-click  go to the member's pane")
    );
    let at = help
        .iter()
        .position(|line| line == "l  pick a pane layout")
        .unwrap();
    assert_eq!(
        &help[at..at + 2],
        ["l  pick a pane layout", "T  pick a theme",]
    );
    // A user who binds it still sees it described.
    app.view.as_mut().unwrap().bindings.insert(
        "L".into(),
        crate::action::Action::parse("jump lead").unwrap(),
    );
    assert!(
        help_lines(&app)
            .iter()
            .any(|line| line == "L  go to the squad lead's pane")
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
    assert!(!screen[11].contains("refresh"), "{:?}", screen[11]);
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
    let bindings = crate::action::with_action_keys(crate::action::preset(true, &board.panes));
    let mut app = App::new(Some("product".into()));
    app.apply(Snapshot {
            squad_keys: Vec::new(),
            tabs: vec!["product".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some("product".into()),
            view: Ok(View {
                history_pending: false,
                ask_lead: crate::config::DEFAULT_ASK_LEAD.into(),
                home_replies: true,
                token_rate: None,
                home_rate: Default::default(),
            exchanges: Vec::new(),
                home: None,
            derived: Default::default(),
                document: json!({"squad": {"name": "product", "lead": {"id": "LEAD", "name": "sol", "fields": {}}}, "sections": [
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
                me_id: None,
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

/// The painted cells of the first board row, from its first line down.
fn board_buffer(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

/// The foreground, background and modifiers a style gives a cell.
fn painted(style: Style) -> (ratatui::style::Color, ratatui::style::Color, Modifier) {
    let mut cell = ratatui::buffer::Cell::default();
    cell.set_style(style);
    (cell.fg, cell.bg, cell.modifier)
}

fn painted_at(
    buffer: &ratatui::buffer::Buffer,
    x: u16,
    y: u16,
) -> (ratatui::style::Color, ratatui::style::Color, Modifier) {
    let cell = &buffer[(x, y)];
    (cell.fg, cell.bg, cell.modifier)
}

#[test]
fn state_cells_use_the_projected_token_for_pattern_and_exact_states() {
    let rows = rows_from("[p.columns]\nshow = ['state']\nstate = { width = 20 }\n");
    let look = crate::look::Look::default();
    for state in ["blocked", "blocked-on-ci"] {
        let draw_row = |colors: Value| {
            let mut app = board(json!([{"title": null, "rows": [
                {"name": "x", "pending": null, "fields": {"state": state}, "colors": colors}
            ]}]));
            let view = app.view.as_mut().unwrap();
            view.rows = rows.clone();
            view.look = look;
            app.selected = 1;
            board_buffer(&app, 30, 8)
        };
        let colored = draw_row(json!({"state": "review"}));
        let plain = draw_row(json!({}));
        let y = (0..8u16)
            .find(|y| {
                (0..30u16)
                    .map(|x| colored[(x, *y)].symbol())
                    .collect::<String>()
                    .contains(state)
            })
            .unwrap();
        for x in 2..2 + state.len() as u16 {
            assert_eq!(
                painted_at(&colored, x, y),
                painted(look.named("review")),
                "{state}"
            );
            assert_eq!(painted_at(&plain, x, y), painted(Style::new()), "{state}");
        }
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
fn sent_feedback_sits_under_its_row_before_the_annotation_and_stays_clickable() {
    let mut app = board(json!([{"title": null, "rows": [
        row("alpha", "working", "one", json!({"id": "u1", "annotation": {"to": "sol", "text": "noted"}})),
        row("bravo", "working", "two", json!({"id": "u2"})),
    ]}]));
    app.view.as_mut().unwrap().look = Default::default();
    app.sent = Some(crate::board::app::RowFeedback {
        sent: true,
        target: app.row_target(0).unwrap(),
        home: None,
    });
    let screen = draw(&app, 60, 12);
    let alpha = screen.iter().position(|l| l.contains("alpha")).unwrap();
    assert_eq!(screen[alpha + 1], "    ✓ sent");
    assert!(screen[alpha + 2].starts_with("    ✎ sent to sol: noted"));
    assert!(screen[alpha + 3].contains("bravo"));
    // The feedback line is the row's, so a click on it selects the row, and the
    // sent mark takes the working role without the row's selection.
    let hit = app
        .hits
        .borrow()
        .iter()
        .copied()
        .find(|hit| usize::from(hit.y) == alpha + 1);
    assert_eq!(hit.map(|hit| hit.row), Some(0));
    let buffer = board_buffer(&app, 60, 12);
    let look = app.look();
    assert_eq!(
        painted_at(&buffer, 4, (alpha + 1) as u16),
        painted(look.role(Role::Working))
    );
    // The next frame without feedback restores the original stream.
    app.sent = None;
    assert_eq!(draw(&app, 60, 12)[alpha + 1], "    ✎ sent to sol: noted");
}

#[test]
fn cell_tokens_override_decoration_but_keep_missing_failure_and_reverse_rules() {
    let rows = rows_from(
        "[p.rows]\ncolumns=[{name='task',width=12,overflow='wrap',max_lines=2}]\nlines=[[{field='task',token='waiting'}]]\n",
    );
    for depth in [
        tmt_cli_style::Depth::TrueColor,
        tmt_cli_style::Depth::Ansi16,
        tmt_cli_style::Depth::None,
    ] {
        let look = crate::look::Look {
            depth,
            ..Default::default()
        };
        for (row, role, lines) in [
            (
                json!({"fields":{"task":"alpha beta gamma"},"colors":{"task":"review"}}),
                Role::Waiting,
                2,
            ),
            (
                json!({"fields":{"task":"?"},"failed":["task"]}),
                Role::Dim,
                1,
            ),
            (json!({"fields":{"task":""}}), Role::Dim, 1),
            (json!({"fields":{}}), Role::Dim, 1),
        ] {
            for selected in [false, true] {
                let mut app = board(json!([{"title": null, "rows": [row.clone(), row.clone()]}]));
                let view = app.view.as_mut().unwrap();
                view.rows = rows.clone();
                view.look = look;
                app.selected = usize::from(!selected);
                let buffer = board_buffer(&app, 30, 10);
                let base = if selected {
                    look.selection()
                } else {
                    Style::new()
                };
                let style =
                    base.patch(look.row_span(selected, look.role(role), role == Role::Waiting));
                // Waiting and dim words on a selection background paint in text.
                let expected = painted(if selected && look.selection().bg.is_some() {
                    style.fg(look.role(Role::Text).fg.unwrap())
                } else {
                    style
                });
                // The first row's cell spans its wrapped lines, padding included.
                let first = 3;
                for y in first..first + lines {
                    for x in 2..14 {
                        assert_eq!(
                            painted_at(&buffer, x, y),
                            expected,
                            "{depth:?} {role:?} selected={selected} ({x},{y})"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn the_waiting_row_mark_takes_the_waiting_token_like_the_tab_mark() {
    for base in ["tmt", "tmt-light"] {
        for depth in [
            tmt_cli_style::Depth::TrueColor,
            tmt_cli_style::Depth::Ansi16,
            tmt_cli_style::Depth::None,
        ] {
            let look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::theme::Base::parse(base).unwrap()),
                depth,
            };
            for selected in [false, true] {
                let mut app = board(json!([{"title": null, "rows": [
                    row("waits", "working", "ship", json!({"pending": "approve"})),
                    row("quiet", "working", "docs", json!({})),
                ]}]));
                app.view.as_mut().unwrap().look = look;
                app.selected = usize::from(!selected);
                let buffer = board_buffer(&app, 60, 10);
                // The tab line also carries a diamond: look only inside the rows.
                let (x, y) = (2..10u16)
                    .flat_map(|y| (0..4u16).map(move |x| (x, y)))
                    .find(|(x, y)| buffer[(*x, *y)].symbol() == "◆")
                    .expect("the waiting row has its mark");
                let token = if depth == tmt_cli_style::Depth::None {
                    Style::new()
                } else {
                    look.role(Role::Waiting)
                };
                let base_style = if selected {
                    look.selection()
                } else {
                    Style::new()
                };
                assert_eq!(
                    painted_at(&buffer, x, y),
                    painted(base_style.patch(look.row_span(selected, token, true))),
                    "{base} {depth:?} selected={selected}"
                );
                // The blank after the diamond and the quiet row's mark are unchanged.
                let blank = painted(base_style);
                assert_eq!(painted_at(&buffer, x + 1, y), blank, "{base} {depth:?}");
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
[(2, "Reset/Reset/Reset/NONE/None"), (17, "Rgb(133, 133, 133)/Reset/Reset/NONE/None"), (1, "Reset/Reset/Reset/NONE/None"), (10, "Rgb(133, 133, 133)/Reset/Reset/NONE/None"), (1, "Reset/Reset/Reset/NONE/None"), (49, "Rgb(255, 158, 100)/Reset/Reset/NONE/None")]
" >                             approve rollout                                  "
[(80, "Rgb(216, 216, 216)/Rgb(43, 43, 43)/Reset/NONE/None")]"#;
    const ANSI16: &str = r#""                               approve rollout                                  "
[(2, "Reset/Reset/Reset/NONE/None"), (17, "Reset/Reset/Reset/DIM/None"), (1, "Reset/Reset/Reset/NONE/None"), (10, "Reset/Reset/Reset/DIM/None"), (1, "Reset/Reset/Reset/NONE/None"), (49, "Yellow/Reset/Reset/NONE/None")]
" >                             approve rollout                                  "
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
    // The selected row keeps its background; a state-colored word there paints in text.
    let selected = cell(&app, "auth-fix");
    assert_eq!(selected.fg, role(&app, Role::Text));
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
        assert_eq!(buffer[(1, 1)].fg, fg(Role::Text));
        for (x, y) in [(2, 2), (1, 6), (14, 0)] {
            assert_eq!(buffer[(x, y)].fg, fg(Role::Muted), "chrome {x},{y}");
        }
        assert_eq!(buffer[(2, 4)].fg, ratatui::style::Color::Reset);
        assert_eq!(buffer[(13, 5)].fg, fg(Role::Dim));
        assert_eq!(buffer[(22, 5)].fg, fg(Role::Dim));
        // The selected tab is a word on a real selection background: text.
        let selected_tab = Role::Text;
        assert_eq!(buffer[(1, 0)].fg, fg(selected_tab));
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
fn selected_tabs_add_measured_name_brackets_and_keep_semantic_mark_styles() {
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
                (Attention::default(), "  product"),
                (
                    Attention {
                        waiting: 2,
                        blocked: 0,
                    },
                    "◆ product 2",
                ),
                (
                    Attention {
                        waiting: 0,
                        blocked: 1,
                    },
                    "✗ product 1",
                ),
                (
                    Attention {
                        waiting: 2,
                        blocked: 1,
                    },
                    "◆ product 2 ✗ 1",
                ),
            ] {
                let selected = tab(
                    look,
                    "product",
                    true,
                    false,
                    attention,
                    &TabColors::default(),
                );
                let unselected = tab(
                    look,
                    "product",
                    false,
                    false,
                    attention,
                    &TabColors::default(),
                );
                assert_eq!(selected.to_string(), text.replace("product", "[product]"));
                assert_eq!(unselected.to_string(), text);
                assert_eq!(selected.width(), unselected.width() + 2);
                for (label, chosen) in [(selected, true), (unselected, false)] {
                    let normal = if chosen {
                        Style {
                            bg: selection.bg,
                            ..look
                                .role(Role::Text)
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
                        buffer[(2 + u16::from(chosen), 0)].symbol(),
                        "p",
                        "the name follows the slot and the admitted opening bracket"
                    );
                    for x in 0..width {
                        let role = if x == 0 && attention.waiting > 0 {
                            Some(Role::Waiting)
                        } else if (x == 0 && attention.blocked > 0)
                            || (attention.waiting > 0
                                && attention.blocked > 0
                                && (12 + u16::from(chosen) * 2..15 + u16::from(chosen) * 2)
                                    .contains(&x))
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
    for (style, role) in [
        (
            tab(
                look,
                "product",
                true,
                false,
                Attention::default(),
                &TabColors::default(),
            )
            .style,
            Role::Text,
        ),
        (pane_tab(look, "detail", true).style, Role::Accent),
    ] {
        assert_eq!(style.fg, look.role(role).fg);
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
        members: false,
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
    // The cursor starts on the lead; the member is the next row.
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let screen = draw(&app, 100, 23);
    // Rows take 60 of 100 columns; detail sits over notes in the rest.
    let right = |line: &str| line.chars().skip(60).collect::<String>();
    assert!(screen[2].starts_with("─ Focus: rows"), "{screen:#?}");
    assert!(right(&screen[2]).starts_with("─ detail"), "{screen:#?}");
    let notes_top = screen
        .iter()
        .position(|line| right(line).starts_with("─ notes · sol"))
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
fn lead_detail_keeps_fields_separate_from_notebooks_and_replies() {
    for width in [160, 100, 80] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            for lifetime in ["saved", "temporary"] {
                let mut app =
                    board(json!([{"rows": [row("worker", "working", "ship", json!({}))]}]));
                let view = app.view.as_mut().unwrap();
                view.look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(
                        tmt_cli_style::theme::Base::parse(base).unwrap(),
                    ),
                    depth,
                };
                view.document["squad"]["lead"] = json!({
                    "id": "LEAD", "name": "sol", "lifetime": lifetime, "state": "review",
                    "pending": "Approve the rollout?",
                    "fields": {"task": "review the patch", "model": "sol", "cap": "high",
                        "pr_link": "https://example.com/1741", "link": ""},
                    "waitingOnYou": [{"preview": "request preview must stay out"}]
                });
                view.replies = vec![json!({"response": "Reply sentinel"})];
                app.notebooks
                    .borrow_mut()
                    .keep("LEAD".into(), Notes::Text("Notebook sentinel".into()));
                let buffer = detail_buffer(&app, width, 20);
                assert_eq!(
                    detail_text(&buffer),
                    [
                        "sol  lead",
                        "review · sol · high",
                        "task: review the patch",
                        "◆ waits on you: Approve the rollout?",
                        "links: pr_link https://example.com/1741",
                        "notes below · replies at right",
                    ]
                );
                assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
                assert_eq!(
                    crate::board::glyph_guard::glyph_error(&detail_text(&buffer)[3]),
                    None
                );
                assert_eq!(
                    buffer[(5, 0)].fg,
                    app.look().role(Role::Dim).fg.unwrap_or_default()
                );
                assert_eq!(
                    buffer[(0, 3)].fg,
                    app.look().role(Role::Waiting).fg.unwrap_or_default()
                );
                assert_eq!(app.notebook_identity(), None);
                app.view.as_mut().unwrap().document["squad"]["lead"]["pending"] = Value::Null;
                let text = detail_text(&detail_buffer(&app, width, 20)).join("\n");
                assert!(
                    !text.contains("waits on you"),
                    "an inbox preview is not lead pending text"
                );
                app.select(1);
                assert!(!app.selected_is_lead());
                assert!(
                    detail_text(&detail_buffer(&app, width, 20))
                        .join("\n")
                        .contains("─ notebook")
                );
            }
        }
    }
}

#[test]
fn lead_detail_omits_missing_fields_and_wraps_safe_text_through_the_shared_scroll() {
    let mut app = board(json!([]));
    app.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol", "id": "LEAD", "lifetime": "saved", "fields": {"task": "", "model": "", "cap": ""}});
    let buffer = detail_buffer(&app, 100, 20);
    assert_eq!(
        detail_text(&buffer),
        [
            "sol  lead",
            "no row fields set · tmt sq set sol task=…",
            "notes below · replies at right",
        ]
    );
    assert_eq!(
        buffer[(0, 1)].fg,
        app.look().role(Role::Dim).fg.unwrap_or_default()
    );
    app.view.as_mut().unwrap().document["squad"]["lead"]["fields"]["task"] =
        json!("one\ntwo\t\u{1b}[31m long task to wrap");
    let text = detail_text(&detail_buffer(&app, 12, 20)).join("");
    assert!(text.contains("task:"));
    assert!(!text.contains('\u{1b}') && !text.contains("no row fields set"));
    detail_buffer(&app, 12, 3);
    app.scrolls
        .scroll(Pane::Detail, super::super::scroll::Step::Bottom);
    assert!(
        detail_text(&detail_buffer(&app, 12, 3))
            .join("")
            .contains("right")
    );
    detail_buffer(&app, 0, 0);
    detail_buffer(&app, 1, 1);
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
            "r reply · ⏎ jump",
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
    let notes_at = screen[2].find("─ notes · sol").expect("notes block title");
    assert_eq!(screen[2][..notes_at].chars().count(), 60, "{screen:#?}");
    assert!(screen[2].starts_with("─ Focus: rows"));
    assert!(
        screen.iter().any(|line| line.contains("   Now")),
        "markdown heading"
    );
    assert!(screen.iter().any(|line| line.contains("◆ auth-fix")));

    let mut app = paned(
        split(
            Direction::TopBottom,
            vec![Pane::Rows, Pane::Detail],
            vec![50, 50],
        ),
        Notes::NotShown,
    );
    // The cursor starts on the lead; the member is the next row.
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let screen = draw(&app, 70, 23);
    let detail_row = screen
        .iter()
        .position(|line| line.starts_with("─ detail"))
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
        // Without the footer line: its `↑↓ move` is not a scroll mark.
        let screen = draw(&app, width, 16);
        let bottom = screen[..screen.len() - 1].join("\n");
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
    lead_sol(&mut app);
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
    assert_eq!(tabs.trim_end(), "◆ [product] 1 ✗ 1 ✗ reviews 2");
    let column = |name: &str| tabs[..tabs.find(name).unwrap()].chars().count() as u16;
    let product = &buffer[(column("product"), 0)];
    // Waiting wins over blocked; selection is bold without moving the tab.
    let fg = |role| app.look().role(role).fg.unwrap_or_default();
    assert_eq!(product.fg, fg(Role::Text), "a selected tab name is a word");
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
            false,
            Attention::default(),
            &TabColors::default()
        )
        .to_string(),
        "  product"
    );
    let colors = TabColors::default();
    let plain = tab(app.look(), "product", false, false, counts, &colors);
    let selected = tab(app.look(), "product", true, false, counts, &colors);
    assert_eq!(selected.to_string(), "◆ [product] 2 ✗ 1");
    assert_eq!(plain.to_string(), "◆ product 2 ✗ 1");
    assert_eq!(selected.width(), plain.width() + 2);
    assert_eq!(selected.style.fg, None);
    assert!(draw(&app, 60, 8)[0].contains("◆ [product] 2 ✗ 1"));
    app.switcher = Some(Switcher::new("product".into()));
    assert!(draw(&app, 60, 18).iter().any(|line| line.contains("[x]")
        && line.contains("product")
        && line.contains("◆")
        && line.contains("2")
        && line.contains("✗1")));
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
    assert_eq!(screen[0], "  product   [leads]");
    assert_eq!(screen[1], "2 squad leads");
    assert_eq!(screen[2], "  SQUAD          LEAD           STATE      TASK");
    assert_eq!(screen[3], " >product        sol            working    plan");
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
    assert_eq!(screen[0], "  reviews   leads   [product]");
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
            "[x]›  quiet (hidden)",
            "1–1 of 1",
            "Space pick/unpick · Enter opens · Esc closes"
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
    // The shown owner remains selected until the requested view is ready.
    assert_eq!(
        before[0], during[0],
        "pending requests do not move shown-tab brackets"
    );
    assert_eq!(before[0].trim_end(), "  [product]   reviews");
    assert!(
        during[1].contains("Opening reviews"),
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
fn initial_loading_uses_the_resolved_theme_before_any_snapshot() {
    for base in [tmt_cli_style::Base::Tmt, tmt_cli_style::Base::TmtLight] {
        for depth in [tmt_cli_style::Depth::TrueColor, tmt_cli_style::Depth::None] {
            let look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            };
            let mut app = App::new(Some("product".into()));
            app.initial_look = Some(look);
            app.loading_since = Some(std::time::Instant::now() - header::SPINNER_DELAY);
            let line = summary_line(&app);
            assert_eq!(line.spans[0].style, look.role(Role::Dim));
            assert_eq!(line.spans[1].style, look.role(Role::Muted));
            assert_eq!(line.spans[2].style, look.role(Role::Text));
            assert_eq!(
                super::tabs::paint(&app, Rect::new(0, 0, 80, 1)).style,
                look.role(Role::Accent).add_modifier(Modifier::BOLD)
            );
        }
    }
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
            assert!(expanded.iter().any(|line| line.contains("─ detail")));
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
                assert_eq!(title.chars().nth(hit.area.x as usize - 2), Some('─'));
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
        members: false,
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
    let mut app = paned(
        config
            .preview_view(
                &crate::view::ViewScope::Board,
                Some(crate::view::ViewName::Team),
                "team",
            )
            .unwrap()
            .board("team")
            .unwrap(),
        Notes::Missing,
    );
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
        // The footer's word has no state glyph; help keeps the state.
        let word = if targets.len() == 2 {
            "side panel".to_owned()
        } else {
            targets[0].title().to_owned()
        };
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
            // The toggle is help-only: the footer never lists it.
            let full = hints(&app, usize::MAX);
            assert!(!full.contains(&format!("d {word}")), "{full}");
            assert!(!full.contains('▾') && !full.contains('▸'), "{full}");
            let description = app.bindings()["d"].description();
            assert!(help_lines(&app).contains(&format!("d  {description} (▾ open, ▸ folded)")));
            assert_eq!(
                crate::board::view::toggle_label(&app, &app.bindings()["d"]),
                Some(format!("{label} {state}")),
                "help keeps the state"
            );
            for width in 0..160 {
                let shown = hints(&app, width);
                assert!(!shown.contains('…'));
                assert!(shown.width() <= width);
                assert!(shown.is_empty() || !shown.ends_with(" ·"));
            }
        }
        if targets.len() == 2 {
            fold(&mut app, Pane::Detail);
            assert!(!hints(&app, usize::MAX).contains("d side panel"));
            assert_eq!(
                crate::board::view::toggle_label(&app, &app.bindings()["d"]).as_deref(),
                Some("detail+replies ▾"),
                "mixed state uses the open mark in help"
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
    view.bindings = crate::action::with_action_keys(crate::action::preset(true, &view.board.panes));
    // Folding keys are in help only, whatever the panes.
    assert!(!hints(&app, usize::MAX).contains("d detail"));
    assert!(help_lines(&app).iter().any(|line| line.starts_with("d  ")));
    assert!(draw(&app, 120, 12)[11].contains("A ask lead"));
    app.view
        .as_mut()
        .unwrap()
        .bindings
        .insert("d".into(), crate::action::Action::parse("refresh").unwrap());
    // Refresh is help-only, on either key.
    let shown = hints(&app, usize::MAX);
    assert!(!shown.contains("refresh"), "{shown}");
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

#[test]
fn inline_middle_row_band_moves_rows_masks_panes_and_fits_every_theme() {
    for width in [160, 100, 80] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            for tab in ["product", crate::board::LEADS] {
                let rows = (0..5).map(|i| json!({"id":format!("id-{i}"), "name":format!("member-{i}"), "squad":"product",
                    "state":"working", "fields":{"task":"row remains visible"},
                    "waitingOnYou":[{"requestId":format!("q-{i}"), "preview":"A long waiting question that must truncate before the recipient or input disappears. ".repeat(8)}]})).collect::<Vec<_>>();
                let mut snapshot =
                    crate::board::app::tests::snapshot(tab, json!([{"title":null, "rows":rows}]));
                let view = snapshot.view.as_mut().unwrap();
                view.me = Some("Ben".into());
                if tab == "product" {
                    view.document["squad"]["lead"] = json!({"id": "LEAD", "name": "sol"});
                }
                view.look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(
                        tmt_cli_style::theme::Base::parse(base).unwrap(),
                    ),
                    depth,
                };
                view.board = split(
                    Direction::LeftRight,
                    vec![Pane::Rows, Pane::Notes],
                    vec![55, 45],
                );
                view.notes = Notes::Text("neighbor pane fragment\n".repeat(25));
                let mut app = App::new(Some(tab.into()));
                app.apply(snapshot);
                // The product tab's cursor row 0 is the lead.
                app.select(if tab == "product" { 3 } else { 2 });
                let before = draw(&app, width, 30);
                let old_after = before
                    .iter()
                    .position(|line| line.contains("member-3"))
                    .unwrap();
                app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
                app.input.as_mut().unwrap().text = "x".repeat(30);
                let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
                terminal.draw(|frame| render(frame, &app)).unwrap();
                let buffer = terminal.backend().buffer();
                let screen = draw(&app, width, 30);
                let band = app.input_band.get().expect("inline band");
                let header = &screen[usize::from(band.y + 1)];
                assert!(
                    header.contains("→ member-2 (product) · answer"),
                    "{base}/{width}/{tab}: {header}"
                );
                assert_eq!(band.height, 6);
                assert!(screen[usize::from(band.y + 3)].contains(&format!("{}▏", "x".repeat(30))));
                assert!(screen[usize::from(band.y + 2)].contains("◆ “A long waiting"));
                assert!(screen[usize::from(band.y + 2)].contains('…'));
                assert!(screen[usize::from(band.y + 4)].contains("Enter send · Esc cancel"));
                assert_eq!(screen[usize::from(band.y)], "─".repeat(usize::from(width)));
                assert_eq!(screen[usize::from(band.y)].width(), usize::from(width));
                for y in band.y..band.bottom() {
                    assert!(!screen[usize::from(y)].contains("neighbor pane fragment"));
                    assert!(!app.hits.borrow().iter().any(|hit| hit.y == y));
                    assert!(!app.note_hits.borrow().iter().any(|(area, _)| area.y == y));
                }
                assert_eq!(
                    screen
                        .iter()
                        .position(|line| line.contains("member-3"))
                        .unwrap(),
                    old_after + 6
                );
                let accent = app.look().role(Role::Accent);
                assert_eq!(buffer[(2, band.y + 1)].fg, accent.fg.unwrap_or_default());
                let waiting = app.look().role(Role::Waiting);
                assert_eq!(buffer[(2, band.y + 2)].fg, waiting.fg.unwrap_or_default());
                app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                let note = draw(&app, width, 30);
                assert!(note.iter().any(|line| line.contains(if tab == "product" {
                    "→ sol (product) · note · about member-2"
                } else {
                    "→ member-2 (product) · note"
                })));
                assert_eq!(app.input_band.get().unwrap().height, 5);
                app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                assert_eq!(
                    &draw(&app, width, 30)[..29],
                    &before[..29],
                    "cancel restores the body"
                );
            }
        }
    }
}

fn boxed_members(sections: Value) -> App {
    let mut app = board(sections);
    lead_sol(&mut app);
    let config = crate::config::Config::read(std::env::temp_dir().join(format!(
        "squad-members-default-{}.missing.toml",
        std::process::id()
    )))
    .unwrap();
    let view = app.view.as_mut().unwrap();
    view.board = config.board("product").unwrap();
    view.notes = Notes::Text("Squad coordination notes".into());
    view.me = Some("Ben".into());
    view.me_id = Some("USER".into());
    view.exchanges =
        crate::board::home_leads::members(&view.document, None, crate::status::now_ms());
    app
}

#[test]
fn boxed_members_share_home_scene_and_inline_band_at_all_widths_and_themes() {
    for width in [160, 100, 80] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            let now = crate::status::now_ms();
            let mut app = boxed_members(
                json!([{"title": null, "rows": [row("worker", "working", "implement issue", json!({"id": "WORKER", "state": "working", "pending": "Approve this change?", "fields": {"task": "implement issue", "model": "gpt-6.1-sol", "pr_link": "https://github.com/pj-tmt/tmt/pull/1766"}}))]}]),
            );
            let view = app.view.as_mut().unwrap();
            view.look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::theme::Base::parse(base).unwrap()),
                depth,
            };
            view.replies = vec![
                json!({"requestId": "reply", "recipientId": "WORKER", "to": "worker", "status": "retained", "submittedAtMs": now - 300000, "response": "Pushed the shared list.\u{001b}[31m"}),
            ];
            let collapsed = draw(&app, width, 32);
            let lead = collapsed
                .iter()
                .position(|line| line.contains("sol") && line.contains("lead"))
                .unwrap();
            let rule = collapsed
                .iter()
                .position(|line| line.contains("members · 1"))
                .unwrap();
            let worker = collapsed
                .iter()
                .position(|line| line.contains("worker") && line.contains("waits on you"))
                .unwrap();
            assert!(lead < rule && rule < worker);
            assert!(collapsed[worker + 1].contains("implement issue"));
            assert!(
                !app.hits
                    .borrow()
                    .iter()
                    .any(|hit| usize::from(hit.y) == rule)
            );
            assert_eq!(
                app.hits.borrow().iter().filter(|hit| hit.row == 1).count(),
                2
            );
            app.selected = 1;
            assert_eq!(
                app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)),
                Effect::None
            );
            let expanded = draw(&app, width, 32);
            let band = app.input_band.get().unwrap();
            assert_eq!(band.x, 1, "single list border, band inset: {base}/{width}");
            assert_eq!(band.width, width - 2);
            let body = crate::board::view::waiting::read_lines(&app, band.width - 4)
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                body.find("◆ waits on you").unwrap() < body.find("task  implement issue").unwrap()
            );
            assert!(body.find("links").unwrap() < body.find("latest reply · 5m").unwrap());
            assert!(body.contains("Pushed the shared list."));
            assert!(!body.contains('\u{1b}'));
            assert!(expanded.iter().any(|line| line.contains("a write")));
            assert!(
                !app.hits
                    .borrow()
                    .iter()
                    .any(|hit| (band.y..band.bottom()).contains(&hit.y))
            );
            assert_eq!(
                app.selected_read(),
                None,
                "reuse already acquired reply body"
            );
            assert_eq!(
                app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                Effect::None
            );
            assert!(app.input.is_none());
            assert_eq!(app.selected, 1);
            app.selected = 0;
            app.perform(&crate::action::Action::parse("home-message").unwrap());
            draw(&app, width, 32);
            assert!(matches!(
                app.input.as_ref().unwrap().compose,
                crate::board::app::Compose::ReadRow { .. }
            ));
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
            assert!(app.collapsed_panes().contains(&Pane::Notes));
            app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
            assert!(!app.collapsed_panes().contains(&Pane::Notes));
        }
    }
}

#[test]
fn boxed_member_read_without_user_is_read_only_and_replaced_rows_cannot_inherit_a_band() {
    let mut app = boxed_members(
        json!([{"title":null,"rows":[row("worker", "idle", "safe detail", json!({"id":"WORKER", "state":"idle"}))]}]),
    );
    let view = app.view.as_mut().unwrap();
    view.me = None;
    view.me_id = None;
    app.selected = 1;
    app.perform(&crate::action::Action::parse("home-message").unwrap());
    draw(&app, 80, 24);
    assert!(app.input_band.get().is_some());
    assert_eq!(app.selected_read(), None);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        Effect::None
    );
    assert!(app.input.is_none());
    app.perform(&crate::action::Action::parse("home-message").unwrap());
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["id"] = json!("REPLACEMENT");
    assert!(!app.message_valid());
}

#[test]
fn boxed_member_order_preserves_lead_and_authored_sections_and_uses_home_exchanges() {
    let mut app = boxed_members(json!([
        {"title":"First", "rows":[
            row("empty-z", "idle", "", json!({"id":"Z"})),
            row("reply", "working", "", json!({"id":"R"})),
            row("new-wait", "working", "", json!({"id":"N", "waitingOnYou":[{"preparedAtMs":20,"preview":"newer"}]})),
            row("old-wait", "working", "", json!({"id":"O", "waitingOnYou":[{"preparedAtMs":10,"preview":"older"}]})),
            row("empty-a", "idle", "", json!({"id":"A"}))]},
        {"title":"Second", "rows":[row("second-wait", "working", "", json!({"id":"S", "pending":"decision"}))]}
    ]));
    let view = app.view.as_mut().unwrap();
    let source = view.document.clone();
    view.exchanges = crate::board::home_leads::members(&view.document, None, 100);
    view.exchanges
        .iter_mut()
        .find(|row| row.id() == "R")
        .unwrap()
        .exchange = Some(crate::board::home_leads::LeadPreview {
        kind: crate::board::home_leads::Kind::Reply,
        request: Some("reply".into()),
        since_ms: Some(90),
        preview: "latest".into(),
        status: "retained".into(),
    });
    assert_eq!(
        app.rows()
            .iter()
            .map(|(_, row)| row["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "sol",
            "old-wait",
            "new-wait",
            "reply",
            "empty-a",
            "empty-z",
            "second-wait"
        ]
    );
    assert_eq!(
        app.view.as_ref().unwrap().document,
        source,
        "ordering is board-only"
    );
    let screen = draw(&app, 80, 40).join("\n");
    assert!(screen.find("First").unwrap() < screen.find("old-wait").unwrap());
    assert!(screen.find("Second").unwrap() > screen.find("empty-z").unwrap());
}

#[test]
fn boxed_member_band_uses_rebound_keys_and_scrolls_its_body_without_moving_the_cursor() {
    use crate::action::Action;
    let mut app = boxed_members(
        json!([{"title":null,"rows":[row("worker", "working", &"long task ".repeat(100), json!({"id":"WORKER"}))]}]),
    );
    let view = app.view.as_mut().unwrap();
    view.bindings.remove("e");
    view.bindings.remove("a");
    view.bindings
        .insert("E".into(), Action::parse("home-message").unwrap());
    view.bindings
        .insert("A".into(), Action::parse("annotate").unwrap());
    app.selected = 1;
    app.key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE));
    draw(&app, 80, 24);
    assert!(app.input_band.get().is_some());
    let screen = draw(&app, 80, 24).join("\n");
    assert!(screen.contains("E collapse · A write"));
    app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(app.selected, 1);
    assert!(
        matches!(app.input.as_ref().unwrap().compose, crate::board::app::Compose::ReadRow { offset, .. } if offset > 0)
    );
    app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert!(
        app.input.is_some(),
        "disabled collapse key remains disabled"
    );
    app.key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE));
    assert!(app.input.is_none());
    app.key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE));
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE)),
        Effect::None
    );
    assert!(
        matches!(&app.input.as_ref().unwrap().compose, crate::board::app::Compose::Annotate { to, row } if to == "sol" && row == "worker")
    );
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(
        matches!(&app.input.as_ref().unwrap().compose, crate::board::app::Compose::Talk { to } if to == "worker")
    );
}

#[test]
fn boxed_member_read_band_starts_answer_then_cycles_note_and_talk_without_losing_text() {
    use crate::board::app::Compose;
    let mut app = boxed_members(json!([{"title":null,"rows":[
        row("worker", "working", "decision", json!({"id":"WORKER", "waitingOnYou":[{"requestId":"decision-q","preview":"Ship this?"}]}))
    ]}]));
    app.selected = 1;
    app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert!(matches!(
        &app.input.as_ref().unwrap().compose,
        Compose::ReadRow { .. }
    ));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(
        matches!(&app.input.as_ref().unwrap().compose, Compose::Reply { request, from } if request == "decision-q" && from == "worker")
    );
    app.input.as_mut().unwrap().text = "Keep this draft".into();
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(
        matches!(&app.input.as_ref().unwrap().compose, Compose::Annotate { to, row } if to == "sol" && row == "worker")
    );
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(matches!(&app.input.as_ref().unwrap().compose, Compose::Talk { to } if to == "worker"));
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(matches!(
        app.input.as_ref().unwrap().compose,
        Compose::Status
    ));
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(
        matches!(&app.input.as_ref().unwrap().compose, Compose::Reply { request, .. } if request == "decision-q")
    );
    assert_eq!(app.input.as_ref().unwrap().text, "Keep this draft");
}

#[test]
fn boxed_member_band_follows_its_occurrence_through_exchange_reordering_without_a_user() {
    let mut app = boxed_members(json!([{"title":null,"rows":[
        row("worker", "working", "keep reading", json!({"id":"WORKER"})),
        row("alpha", "working", "other", json!({"id":"ALPHA"}))
    ]}]));
    app.view.as_mut().unwrap().me = None;
    app.view.as_mut().unwrap().me_id = None;
    app.selected = 2;
    assert_eq!(app.selected_row().unwrap()["id"], "WORKER");
    app.perform(&crate::action::Action::parse("home-message").unwrap());
    let mut view = app.view.take().unwrap();
    view.document["sections"][0]["rows"][0]["pending"] = json!("question moves this row first");
    view.exchanges =
        crate::board::home_leads::members(&view.document, None, crate::status::now_ms());
    let mut snapshot = crate::board::app::tests::snapshot("product", json!([]));
    snapshot.view = Ok(view);
    app.apply(snapshot);
    assert_eq!(app.selected, 1);
    assert_eq!(app.selected_row().unwrap()["id"], "WORKER");
    assert!(app.input.is_some());
    assert!(app.message_valid());
    draw(&app, 80, 24);
    assert!(app.input_band.get().is_some());
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["id"] = json!("REPLACED");
    assert!(!app.message_valid());
}

/// Read-only runtime evidence for the immutable #1829 first-slice packet.
/// The packet and output directory are explicit task-owned inputs; this never
/// updates repository parity or performs acquisition/actions.
#[test]
#[ignore = "explicit pinned selection/focus evidence capture"]
fn capture_selection_focus_packet() {
    let packet_path = std::env::var("TMT_SELECTION_PACKET").expect("pinned packet path");
    let output = std::env::var("TMT_SELECTION_OUTPUT").expect("task-owned output path");
    let packet: Value = serde_json::from_slice(&std::fs::read(packet_path).unwrap()).unwrap();
    crate::status::with_now_ms(packet["clock"]["fixedNowMs"].as_u64().unwrap(), || {
        let mut captures = Vec::new();
        for case in packet["cases"].as_array().unwrap() {
            for (base, depth) in [
                ("tmt", tmt_cli_style::Depth::TrueColor),
                ("tmt-light", tmt_cli_style::Depth::TrueColor),
                ("terminal", tmt_cli_style::Depth::Ansi16),
                ("tmt", tmt_cli_style::Depth::None),
            ] {
                let members = packet["members"].as_array().unwrap().iter().map(|member| {
                let waiting = if member["requestIds"].as_array().unwrap().is_empty() { json!([]) } else {
                    json!([{"requestId": packet["request"]["id"], "preview": packet["request"]["preview"],
                        "preparedAtMs": packet["clock"]["requestCreatedAtMs"]}])
                };
                row(member["name"].as_str().unwrap(), member["state"].as_str().unwrap(),
                    member["task"].as_str().unwrap(), json!({"id":member["uuid"], "squad":"ux-demo",
                        "waitingOnYou":waiting, "state":member["state"]}))
            }).collect::<Vec<_>>();
                let mut app = board(json!([{"title":"primary", "rows":members[1..]},
                {"title":"repeat", "rows":[members[1].clone()]}]));
                app.current = Some("ux-demo".into());
                app.tabs = vec!["ux-demo".into()];
                let width = case["viewport"]["columns"].as_u64().unwrap() as u16;
                let height = case["viewport"]["rows"].as_u64().unwrap() as u16;
                let view_name = case["view"].as_str().unwrap_or("single");
                let view = app.view.as_mut().unwrap();
                view.document["squad"]["name"] = json!("ux-demo");
                view.document["squad"]["lead"] = members[0].clone();
                view.me = Some("fixture-user".into());
                view.me_id = Some(packet["scope"]["actorUUID"].as_str().unwrap().into());
                view.rows = rows_from(
                    r#"[p.rows]
columns = [{name = "member", width = 18}, {name = "task", grow = 1, overflow = "wrap", max_lines = 8}]
"#,
                );
                let settings = if view_name == "single" || view_name == "HOME" {
                    None
                } else {
                    Some(crate::view::ViewName::parse(view_name).unwrap())
                };
                if let Some(settings) = settings {
                    let settings = settings.settings();
                    let split =
                        crate::split::read(settings.get("layout").unwrap(), "fixture.layout")
                            .unwrap();
                    view.board = crate::config::Board {
                        members: view_name == "members",
                        mode: BoardMode::Split,
                        panes: split.panes(),
                        split,
                        collapsed: settings
                            .get("collapsed")
                            .and_then(toml_edit::Item::as_array)
                            .into_iter()
                            .flatten()
                            .map(|v| Pane::parse(v.as_str().unwrap()).unwrap())
                            .collect(),
                        fold_below: settings
                            .get("fold_below")
                            .map(|v| crate::config::FoldBelow {
                                width: v["width"].as_integer().unwrap() as u16,
                                panes: v["panes"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|v| Pane::parse(v.as_str().unwrap()).unwrap())
                                    .collect(),
                            }),
                    };
                }
                view.notes = Notes::Text(if case["selectedLink"].is_object() {
                    format!(
                        "[{}]({})",
                        case["selectedLink"]["label"].as_str().unwrap(),
                        case["selectedLink"]["target"].as_str().unwrap()
                    )
                } else {
                    "Fixture coordination notes\nKeep the saved bindings.".into()
                });
                view.bindings = crate::action::preset(true, &view.board.panes);
                view.look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
                    depth,
                };
                if view_name == "HOME" || case["id"].as_str().unwrap().contains("HOME") {
                    use crate::board::{
                        home::{Home, MemberRow, MemberSection},
                        home_leads::{Kind, Lead, LeadPreview},
                    };
                    view.home = Some(Home {
                        summary: crate::board::home::Counts {
                            members: 4,
                            waiting: 1,
                            blocked: 1,
                            working: 2,
                            ..Default::default()
                        },
                        windows: crate::config::TokenWindow::DEFAULTS,
                        sections: vec![MemberSection {
                            key: "needs-you".into(),
                            rows: vec![MemberRow {
                                squad: "ux-demo".into(),
                                member: members[1].clone(),
                                lead: Some("ux-demo-lead".into()),
                                age: Some(crate::board::home::Age {
                                    source: crate::board::home::AgeSource::Request,
                                    since_ms: packet["clock"]["requestCreatedAtMs"]
                                        .as_u64()
                                        .unwrap(),
                                }),
                            }],
                        }],
                        squads: vec![crate::board::home::SquadLine {
                            squad: "ux-demo".into(),
                            lead: Some(members[0].clone()),
                            counts: crate::board::home::Counts {
                                members: 4,
                                waiting: 1,
                                blocked: 1,
                                working: 2,
                                ..Default::default()
                            },
                            members: crate::board::home::Counts {
                                members: 3,
                                waiting: 1,
                                blocked: 1,
                                working: 1,
                                ..Default::default()
                            },
                        }],
                        failures: Vec::new(),
                        incomplete: false,
                    });
                    view.exchanges = vec![Lead {
                        squad: "ux-demo".into(),
                        row: members[0].clone(),
                        failure: None,
                        exchange: case
                            .get("syntheticLeadExchange")
                            .map(|exchange| LeadPreview {
                                kind: Kind::Reply,
                                request: None,
                                since_ms: Some(exchange["sinceMs"].as_u64().unwrap()),
                                preview: exchange["preview"].as_str().unwrap().into(),
                                status: "retained".into(),
                            }),
                    }];
                    view.bindings = crate::action::all_preset();
                    app.current = Some(crate::tabs::ALL.into());
                    app.tabs = vec![crate::tabs::ALL.into()];
                } else {
                    view.exchanges = crate::board::home_leads::members(
                        &view.document,
                        None,
                        packet["clock"]["fixedNowMs"].as_u64().unwrap(),
                    );
                }
                let tab = app.current.clone().unwrap();
                let mut snapshot = crate::board::app::tests::snapshot(&tab, json!([]));
                snapshot.view = Ok(app.view.take().unwrap());
                snapshot.tabs = app.tabs.clone();
                app = App::new(Some(tab));
                app.apply(snapshot);
                if app.view.as_ref().unwrap().home.is_some() {
                    let leads = app.view.as_ref().unwrap().exchanges.clone();
                    app.apply_home_leads(crate::board::home_leads::Read::for_test(
                        packet["scope"]["actorUUID"].as_str().unwrap(),
                        leads,
                    ));
                }
                app.set_body_width(width);
                app.select(1);
                if case["receivingFocus"] == "notes" {
                    // Focus view's Notes is initially folded: use the existing focus
                    // transition so its configured body is expanded before painting.
                    if app.effective_board().unwrap().members {
                        app.focus = app
                            .effective_board()
                            .unwrap()
                            .panes
                            .iter()
                            .position(|p| *p == Pane::Notes)
                            .unwrap();
                    } else {
                        app.perform(&crate::action::Action::parse("notes").unwrap());
                    }
                }
                if let Some(composer) = case.get("composer").filter(|v| v.is_object()) {
                    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
                    app.input.as_mut().expect("answer composer").text =
                        composer["draft"].as_str().unwrap().into();
                }
                if case["id"] == "grid-decoration-exclusions-100x30" {
                    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["annotation"] = json!({"to":case["renderOnlyFeedback"]["annotation"]["to"], "text":case["renderOnlyFeedback"]["annotation"]["text"]});
                    app.sent = Some(crate::board::app::RowFeedback {
                        target: app.row_target(app.selected).unwrap(),
                        sent: true,
                        home: None,
                    });
                }
                if case["selectedLink"].is_object() {
                    board_buffer(&app, width, height);
                    let view = app.view.as_ref().unwrap();
                    let derived = view.derived.borrow();
                    let link = derived.notes.as_ref().unwrap().links.first().unwrap();
                    app.note_link = Some((link.target.clone(), link.offset));
                }
                let document = app.view.as_ref().unwrap().document.clone();
                let buffer = board_buffer(&app, width, height);
                let cells = buffer
                    .content
                    .iter()
                    .map(|cell| {
                        json!({"symbol":cell.symbol(),
                "fg":format!("{:?}",cell.fg), "bg":format!("{:?}",cell.bg),
                "modifier":format!("{:?}",cell.modifier)})
                    })
                    .collect::<Vec<_>>();
                captures.push(
                    json!({"case":case["id"], "base":base, "depth":format!("{depth:?}"),
                "width":width, "height":height, "cells":cells,
                "document":document, "hits":format!("{:?}",app.hits.borrow()),
                "starts":*app.row_starts.borrow(), "titles":format!("{:?}",app.title_hits.borrow()),
                "band":format!("{:?}",app.input_band.get()), "selected":app.selected,
                "focus":format!("{:?}",app.focused_pane()),
                "footer26":hints(&app,26), "footer27":hints(&app,27),
                "footer31":hints(&app,31), "footer32":hints(&app,32)}),
                );
            }
        }
        std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
    });
}

#[test]
fn occurrence_cues_follow_content_and_clip_without_changing_hits_or_feedback() {
    let member = row(
        "duplicate",
        "working",
        "alpha beta gamma delta epsilon",
        json!({
        "id":"same-id", "waitingOnYou":[{"requestId":"fixture", "preview":"Question?"}],
        "annotation":{"to":"fixture", "text":"decoration stays separate"}}),
    );
    let mut app = board(json!([{"title":"primary", "rows":[member.clone()]},
        {"title":"repeat", "rows":[member]}]));
    app.view.as_mut().unwrap().rows = rows_from(
        r#"[p.rows]
columns = [{name="member", width=12}, {name="task", width=12, overflow="wrap", max_lines=8}]
"#,
    );
    app.sent = Some(crate::board::app::RowFeedback {
        target: app.row_target(0).unwrap(),
        sent: true,
        home: None,
    });
    let document = app.view.as_ref().unwrap().document.clone();
    for (base, depth) in [
        (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
        (
            tmt_cli_style::Base::TmtLight,
            tmt_cli_style::Depth::TrueColor,
        ),
        (tmt_cli_style::Base::Terminal, tmt_cli_style::Depth::Ansi16),
        (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
    ] {
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(base),
            depth,
        };
        board_buffer(&app, 50, 24);
        let view = app.view.as_ref().unwrap();
        let derived = view.derived.borrow();
        let scene = &derived.grid.as_ref().unwrap().scene;
        let area = Rect::new(0, 0, 50, 24);
        let mut selected = ratatui::buffer::Buffer::empty(area);
        let hits = scene.paint(&mut selected, area, 0, 0, app.look());
        let mut other = ratatui::buffer::Buffer::empty(area);
        assert_eq!(
            format!("{hits:?}"),
            format!("{:?}", scene.paint(&mut other, area, 0, 1, app.look()))
        );
        let lines = detail_text(&selected);
        assert!(
            lines.iter().filter(|line| line.starts_with(" >")).count() >= 3,
            "wrapped content and automatic question each carry a cue"
        );
        assert!(
            lines[scene.starts[0]].starts_with("◆>"),
            "attention diamond stays at x0"
        );
        assert!(
            lines[scene.starts[1]].starts_with("◆ "),
            "duplicate occurrence remains unselected"
        );
        for (y, line) in lines.iter().enumerate() {
            if line.contains("✓ sent")
                || line.contains("decoration stays")
                || line.contains("PRIMARY")
            {
                assert_ne!(selected[(1, y as u16)].symbol(), ">");
                assert_eq!(selected[(1, y as u16)], other[(1, y as u16)]);
            }
        }
        for y in 0..24 {
            for x in 0..50 {
                if selected[(x, y)].symbol() != other[(x, y)].symbol() {
                    assert_eq!(x, 1, "only the existing prefix changes text");
                    assert!([" ", ">"].contains(&selected[(x, y)].symbol()));
                }
            }
        }
        // A one-cell clip admits the diamond but has no occurrence-cue cell.
        let clip = Rect::new(0, 0, 1, 24);
        let mut narrow = ratatui::buffer::Buffer::empty(clip);
        let narrow_hits = scene.paint(&mut narrow, clip, 0, 0, app.look());
        assert_eq!(narrow[(0, scene.starts[0] as u16)].symbol(), "◆");
        assert!(narrow.content.iter().all(|cell| cell.symbol() != ">"));
        assert!(narrow_hits.iter().all(|hit| hit.x == 0 && hit.width == 1));
        // Scrolling clips content, rather than painting the reveal envelope.
        let mut scrolled = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 50, 2));
        let visible = scene.paint(
            &mut scrolled,
            Rect::new(0, 0, 50, 2),
            scene.starts[0] + 1,
            0,
            app.look(),
        );
        assert_eq!(scrolled[(1, 0)].symbol(), ">");
        assert!(visible.iter().all(|hit| hit.y < 2 && hit.row == 0));
        assert_eq!(view.document, document);
    }
}

#[test]
fn receiving_focus_labels_keep_title_footer_and_input_ownership_separate() {
    let mut app =
        board(json!([{"title":null,"rows":[row("selected","working","task",json!({}))]}]));
    assert_eq!(hints(&app, 27), "Focus: rows  ? more  q quit");
    assert!(!hints(&app, 26).contains("Focus:"));
    assert!(hints(&app, 26).ends_with("? more  q quit"));
    app.view.as_mut().unwrap().board = split(
        Direction::LeftRight,
        vec![Pane::Rows, Pane::Notes],
        vec![60, 40],
    );
    app.view.as_mut().unwrap().notes = Notes::Text("coordination".into());
    let outlined = draw(&app, 80, 20);
    assert!(outlined.iter().any(|line| line.contains("Focus: rows")));
    assert!(!outlined.last().unwrap().contains("Focus:"));
    app.focus = 1;
    let unfocused = draw(&app, 80, 20);
    assert!(unfocused.iter().any(|line| line.contains("Focus: notes")));
    assert!(!unfocused.iter().any(|line| line.contains("Focus: rows")));
    assert!(
        unfocused.iter().any(|line| line.contains(" >selected")),
        "selection persists outside receiving pane"
    );
    app.focus = 0;
    app.help = true;
    assert!(!hints(&app, 80).contains("Focus:"));
    app.help = false;
    app.searching = true;
    assert!(
        !draw(&app, 80, 20)
            .iter()
            .any(|line| line.contains("Focus:"))
    );
    app.searching = false;
    app.notice = Some("Notice owns the footer".into());
    assert_eq!(
        draw(&app, 80, 20).last().unwrap().trim(),
        "Notice owns the footer"
    );
    app.notice = None;
    app.error = Some("Error owns the footer".into());
    assert_eq!(
        draw(&app, 80, 20).last().unwrap().trim(),
        "Error owns the footer"
    );
}

#[test]
fn flat_custom_split_retains_inner_hits_and_focus_title_without_dimming_it() {
    for width in [80, 100, 160, 180] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("terminal", tmt_cli_style::Depth::Ansi16),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            let mut app = paned(
                split(
                    Direction::LeftRight,
                    vec![Pane::Rows, Pane::Notes],
                    vec![60, 40],
                ),
                Notes::Text("coordination".into()),
            );
            app.view.as_mut().unwrap().look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
                depth,
            };
            app.set_body_width(width);
            let buffer = board_buffer(&app, width, 23);
            let titles = app.title_hits.borrow().clone();
            let rows = titles
                .iter()
                .find(|hit| hit.pane == Pane::Rows)
                .unwrap()
                .area;
            let notes = titles
                .iter()
                .find(|hit| hit.pane == Pane::Notes)
                .unwrap()
                .area;
            assert!(!super::panes::borderless_rows(&app));
            assert_eq!((rows.y, rows.height), (2, 1));
            assert_eq!(notes.x, width * 60 / 100);
            let dim = app.look().role(Role::Dim);
            for x in [rows.x, rows.right() - 1] {
                assert_eq!(buffer[(x, rows.y)].symbol(), "─");
                assert_eq!(buffer[(x, rows.y)].fg, dim.fg.unwrap_or_default());
                assert_eq!(buffer[(x, rows.y + 1)].symbol(), " ");
            }
            let title = &buffer[(rows.x + 2, rows.y)];
            assert_eq!(title.symbol(), "F");
            assert!(title.modifier.contains(Modifier::BOLD));
            assert!(!title.modifier.contains(Modifier::DIM), "{base}/{depth:?}");
            assert!(
                app.hits.borrow().iter().all(|hit| hit.x > rows.x
                    && hit.x + hit.width < rows.right()
                    && hit.y > rows.y)
            );
            assert_eq!(app.scrolls.pane_at(rows.x, rows.y + 1), None);
            assert_eq!(
                app.scrolls.pane_at(rows.x + 1, rows.y + 1),
                Some(Pane::Rows)
            );
            let original = draw(&app, width, 23);
            click_title(&mut app, Pane::Notes);
            draw(&app, width, 23);
            assert_ne!(
                app.scrolls.pane_at(notes.x + 1, notes.y + 1),
                Some(Pane::Notes)
            );
            click_title(&mut app, Pane::Notes);
            assert_eq!(
                draw(&app, width, 23),
                original,
                "unfold restores exact frame"
            );
        }
    }
}

#[test]
fn receiving_title_preserves_configured_accent_effects_and_semantic_notes_span() {
    for (base, depth) in [
        ("tmt", tmt_cli_style::Depth::TrueColor),
        ("tmt-light", tmt_cli_style::Depth::TrueColor),
        ("terminal", tmt_cli_style::Depth::Ansi16),
        ("tmt", tmt_cli_style::Depth::None),
    ] {
        for accent in [None, Some("dim")] {
            let mut app = paned(
                split(
                    Direction::LeftRight,
                    vec![Pane::Rows, Pane::Notes],
                    vec![50, 50],
                ),
                Notes::Text("coordination".into()),
            );
            let mut settings = vec![("base", base)];
            if let Some(accent) = accent {
                settings.push(("accent", accent));
            }
            app.view.as_mut().unwrap().look = crate::look::Look {
                theme: tmt_cli_style::Theme::parse("board.theme", settings).unwrap(),
                depth,
            };
            app.view.as_mut().unwrap().document["squad"]["notesStaleness"] =
                json!({"state": "stale", "ageMs": 2 * 3_600_000});
            app.focus = 1;
            assert_eq!(app.focused_pane(), Some(Pane::Notes));
            let buffer = board_buffer(&app, 160, 23);
            let title = app
                .title_hits
                .borrow()
                .iter()
                .find(|hit| hit.pane == Pane::Notes)
                .unwrap()
                .area;
            let receiving = app.look().role(Role::Accent).add_modifier(Modifier::BOLD);
            // The original receiving title inherited its receiving role, not
            // the incidental frame. Styled age spans then patch that role.
            let mut expected = ratatui::buffer::Cell::default();
            expected.set_style(receiving);
            let at = &buffer[(title.x + 2, title.y)];
            assert_eq!(at.symbol(), "F");
            assert_eq!(
                (at.fg, at.bg, at.modifier),
                (expected.fg, expected.bg, expected.modifier),
                "{base}/{depth:?}/{accent:?}"
            );
            let line: String = (title.x..title.right())
                .map(|x| buffer[(x, title.y)].symbol())
                .collect();
            let age_x = title.x + line[..line.find("stale 2h").unwrap()].chars().count() as u16;
            expected.set_style(app.look().role(Role::Waiting));
            let age = &buffer[(age_x, title.y)];
            assert_eq!(
                (age.fg, age.bg, age.modifier),
                (expected.fg, expected.bg, expected.modifier),
                "semantic span: {base}/{depth:?}/{accent:?}"
            );
            let border = &buffer[(title.x, title.y)];
            assert_eq!(border.symbol(), "─");
            assert_eq!(border.fg, app.look().role(Role::Dim).fg.unwrap_or_default());
        }
    }
}

#[test]
fn switcher_cursor_moves_without_changing_picks_attention_or_query() {
    use crate::board::picker_surface::evidence;
    for (variant, look) in evidence::looks().into_iter().enumerate() {
        for (width, height) in [(80, 30), (100, 30), (160, 30), (180, 30), (80, 8)] {
            let mut app = board(json!([]));
            app.tabs = vec!["alpha".into(), "beta".into()];
            app.hidden = vec!["gamma".into()];
            app.picks = crate::board::pick::Picks::parse(Some("alpha"), &app.switchable()).unwrap();
            app.attention.insert(
                "beta".into(),
                Attention {
                    waiting: 1,
                    blocked: 2,
                },
            );
            app.view.as_mut().unwrap().look = look;
            app.switcher = Some(Switcher::default());
            let picks = app.picks.clone();
            for selected in ["alpha", "beta"] {
                // Initial paint reconciles actual rows before selecting; subsequent
                // movement goes through the board's consumed overlay event path.
                if selected == "beta" {
                    assert_eq!(
                        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                        Effect::None
                    );
                }
                let mut screen = Terminal::new(TestBackend::new(width, height)).unwrap();
                screen
                    .draw(|frame| {
                        render_switcher(frame, &app, app.switcher.as_ref().unwrap(), frame.area())
                    })
                    .unwrap();
                let surface = app.switcher.as_ref().unwrap().surface.borrow();
                let buffer = screen.backend().buffer();
                assert_eq!(surface.picker.list.selected(), Some(selected));
                assert_eq!(surface.picker.query(), Some(""));
                for id in ["alpha", "beta"] {
                    let row = evidence::row(&surface, id);
                    if row.height == 0 {
                        continue;
                    }
                    assert_eq!(buffer[(row.x, row.y)].symbol(), "[");
                    assert_eq!(
                        buffer[(row.x + 1, row.y)].symbol(),
                        if id == "alpha" { "x" } else { " " }
                    );
                    assert_eq!(
                        buffer[(row.x + 3, row.y)].symbol(),
                        if id == selected { "›" } else { " " }
                    );
                    assert_eq!(
                        buffer[(row.x + 4, row.y)].symbol(),
                        if id == "beta" { "◆" } else { " " }
                    );
                    assert_eq!(
                        buffer[(row.x + 6, row.y)].symbol(),
                        if id == "alpha" { "a" } else { "b" }
                    );
                }
                evidence::capture(
                    &format!("switcher-{width}x{height}-{variant}-{selected}"),
                    buffer,
                    &surface,
                );
            }
            assert_eq!(app.picks, picks);
            assert_eq!(
                app.attention["beta"],
                Attention {
                    waiting: 1,
                    blocked: 2
                }
            );
        }
    }
}
