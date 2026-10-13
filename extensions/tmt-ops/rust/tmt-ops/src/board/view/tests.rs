use super::{
    footer::hints,
    header::{SPINNER_TICK, meter_region, summary_line},
};
use crate::{attention::Attention, config::NotesRender, rows::Rows};
use ratatui::widgets::Paragraph;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
};
use serde_json::Value;
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

mod checklist;
mod cron;
mod help;
mod menu;
mod meter;
mod parity;
mod rules;
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

fn workspace_board() -> App {
    crate::board::app::workspace::tests::board()
}

fn workspace_keys(app: &mut App, text: &str) {
    let path =
        std::env::temp_dir().join(format!("ops-footer-{}.toml", crate::id::new_v4().unwrap()));
    std::fs::write(&path, text).unwrap();
    let config = crate::config::Config::read(path.clone()).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .configure(&config)
        .unwrap();
    std::fs::remove_file(path).unwrap();
}

fn workspace_row(app: &mut App) -> &mut Value {
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
        .unwrap()
}

/// Exercise the shared typed field renderer, not the retired notebook/detail pane.
fn entity_detail_buffer(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let value = app
        .row_target(app.selected)
        .and_then(|target| app.detail(&crate::board::row_detail::Target::Row(target)))
        .map_or(Value::Null, |detail| detail.value());
    let lines = crate::board::row_detail::render(&value, width, 0, app.look(), false);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| frame.render_widget(Paragraph::new(lines.clone()), frame.area()))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn buffer_text(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
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
fn narrow_annotation_header_truncates_source_before_separator_and_removal_hint() {
    let mut app = board(json!([]));
    app.ask(
        String::new(),
        crate::board::app::Compose::Talk { to: "agent".into() },
        "product".into(),
    );
    let request = "req_1a2b3c4d-1234-4567-89ab-0123456789ab";
    app.input
        .as_mut()
        .unwrap()
        .response_quotes
        .push(crate::board::response::Quote {
            key: crate::board::home_leads::MessageKey {
                sender: "owner".into(),
                lead: "agent".into(),
                squad: "product".into(),
                request: request.into(),
                kind: crate::board::home_leads::Kind::Reply,
            },
            range: 0..6,
            text: "source".into(),
        });
    app.input_band.set(Some(Rect::new(0, 0, 40, 6)));
    let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
    terminal
        .draw(|frame| waiting::inline_prompt(frame, &app, frame.area()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let line = (0..8)
        .map(|y| (0..40).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .find(|line| line.contains("Alt-Backspace removes"))
        .expect("whole removal hint at 40 cells");
    assert!(line.contains("… · Alt-Backspace removes"), "{line}");
    assert!(line.contains("1 quote(s)"), "{line}");
    assert!(
        !line.contains(request),
        "source display must be the truncated part"
    );
}

#[test]
fn home_pending_history_has_a_status_row_before_any_usage_is_available() {
    for width in [80, 100, 160] {
        let mut app = workspace_board();
        app.shell.tab = crate::board::shell::OpsTab::Overview;
        let summary = draw(&app, width, 24)[2].clone();
        app.view.as_mut().unwrap().history_pending = true;
        let pending = draw(&app, width, 24);
        assert!(
            pending.iter().any(|line| line.contains("Updating usage…")),
            "{pending:?}"
        );
        assert_eq!(
            pending[2], summary,
            "usage loading cannot rewrite the observed aggregate"
        );
        app.view.as_mut().unwrap().history_pending = false;
        assert!(
            draw(&app, width, 24)
                .iter()
                .all(|line| !line.contains("Updating usage"))
        );
        app.view.as_mut().unwrap().history_pending = true;
        app.shell.tab = crate::board::shell::OpsTab::Agents;
        assert!(
            draw(&app, width, 24)
                .iter()
                .all(|line| !line.contains("Updating usage")),
            "hidden Overview status cannot appear in Agents"
        );
    }
}

#[test]
fn footer_keys_are_bold_accent_and_labels_muted_in_every_board_mode() {
    for (base, depth) in [
        (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
        (
            tmt_cli_style::Base::TmtLight,
            tmt_cli_style::Depth::TrueColor,
        ),
        (tmt_cli_style::Base::Terminal, tmt_cli_style::Depth::Ansi16),
        (tmt_cli_style::Base::Mono, tmt_cli_style::Depth::None),
    ] {
        let mut app = workspace_board();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .squad_looks
            .clear();
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(base),
            depth,
        };
        let text = hints(&app, 160);
        assert!(
            !text.contains("a write") && !text.contains("A ask lead") && !text.contains("Focus:")
        );
        let help = help_lines(&app);
        assert!(help.iter().any(|line| line.starts_with("t  ")));
        assert!(help.iter().any(|line| line.starts_with("j  ")));
        for list in [text] {
            let line = super::footer::hint_line(&list, app.look());
            let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 160, 1));
            tmt_tui::components::strip::paint_left(&mut buffer, Rect::new(0, 0, 160, 1), line);
            let mut x = 0u16;
            for part in list.split("  ") {
                let (key, label) = part.split_once(' ').unwrap();
                if key != "◆" {
                    for offset in 0..key.width() as u16 {
                        let cell = &buffer[(x + offset, 0)];
                        assert_eq!(
                            cell.fg,
                            app.look().role(Role::Accent).fg.unwrap_or_default()
                        );
                        assert!(cell.modifier.contains(Modifier::BOLD));
                    }
                    let label_x = x + key.width() as u16 + 1;
                    assert_eq!(
                        buffer[(label_x, 0)].fg,
                        app.look().role(Role::Muted).fg.unwrap_or_default()
                    );
                    assert_eq!(
                        buffer[(label_x, 0)].modifier,
                        app.look().role(Role::Muted).add_modifier
                    );
                    assert!(!label.is_empty());
                }
                x += part.width() as u16 + 2;
            }
        }
    }
}

#[test]
fn a_due_reload_is_named_beside_the_open_prompt_and_only_then() {
    let mut app = workspace_board();
    let footer = |app: &App| draw(app, 100, 24).last().unwrap().trim_end().to_owned();
    app.searching = true;
    assert_eq!(footer(&app), "/▏");
    app.reload = crate::board::reload::Watch::due();
    assert_eq!(
        footer(&app),
        "/▏  reload waiting",
        "a search being typed holds it back"
    );
    app.searching = false;
    assert!(
        !footer(&app).contains("reload waiting"),
        "nothing is open: the reload runs, so there is nothing to wait on"
    );
}

#[test]
fn footer_omits_whole_hints_instead_of_clipping_words() {
    let mut app = workspace_board();
    let full = hints(&app, usize::MAX);
    assert!(
        full.contains("t talk") && full.ends_with("? help  q quit"),
        "{full}"
    );
    for width in [0, 1, 5, 6, 13, 14, 20, 40, 80, 100, 160] {
        let shown = hints(&app, width);
        assert!(shown.width() <= width, "{width}: {shown}");
        if width >= 6 {
            assert!(shown.contains("? help"), "{width}: {shown}");
        }
        assert!(
            shown
                .split("  ")
                .filter(|part| !part.is_empty())
                .all(|part| full.split("  ").any(|whole| whole == part)),
            "partial hint at {width}: {shown}"
        );
    }
    workspace_keys(
        &mut app,
        "[bind]\n'?' = \"copy {name}\"\nh = \"help\"\nq = \"copy {name}\"\n",
    );
    assert_eq!(hints(&app, 6), "h help");
    let configured = hints(&app, usize::MAX);
    assert!(configured.ends_with("h help  Ctrl-C quit"), "{configured}");
    assert!(!configured.contains("? help") && !configured.contains("q quit"));
}

/// The shown hints without the reserved tail.
fn shown_hints(text: &str) -> Vec<&str> {
    text.split("  ")
        .filter(|hint| !hint.is_empty() && !["q quit", "? help"].contains(hint))
        .collect()
}

#[test]
fn footer_orders_hints_by_priority_and_always_keeps_quit_and_more() {
    let mut app = workspace_board();
    workspace_row(&mut app)["fields"] = json!({"link":"https://example.test/pr"});
    let full = hints(&app, usize::MAX);
    let order = ["t talk", "j host", "o open", "down next", "up previous"];
    let at = |hint: &str| full.find(hint).unwrap_or_else(|| panic!("{hint}: {full}"));
    assert!(
        order.windows(2).all(|pair| at(pair[0]) < at(pair[1])),
        "{full}"
    );
    let whole = shown_hints(&full);
    for width in 0..=full.width() {
        let shown = hints(&app, width);
        assert!(shown.width() <= width);
        match width {
            0..=5 => assert!(shown.is_empty()),
            6..=13 => assert_eq!(shown, "? help"),
            _ => assert!(shown.ends_with("? help  q quit"), "{width}: {shown}"),
        }
        let kept = shown_hints(&shown);
        assert_eq!(kept, whole[..kept.len()], "{width}: {shown}");
    }
}

#[test]
fn footer_shows_only_the_row_actions_the_selected_row_allows() {
    let mut app = workspace_board();
    workspace_keys(&mut app, "[bind]\nr = \"reply\"\n");
    let plain = hints(&app, usize::MAX);
    assert!(plain.contains("t talk") && plain.contains("j host"));
    assert!(!plain.contains("r reply") && !plain.contains("o open"));
    workspace_row(&mut app)["waitingOnYou"] = json!([{"requestId":"request","preview":"decision"}]);
    workspace_row(&mut app)["fields"] = json!({"link":"https://example.test/pr"});
    let ready = hints(&app, usize::MAX);
    assert!(
        ready.contains("r reply") && ready.contains("o open"),
        "{ready}"
    );
    workspace_row(&mut app)["waitingOnYou"] = Value::Null;
    assert!(!hints(&app, usize::MAX).contains("r reply"));
    app.select(0);
    let pool = hints(&app, usize::MAX);
    assert!(
        !pool.contains("t talk") && !pool.contains("j host") && !pool.contains("o open"),
        "{pool}"
    );
    assert!(pool.contains("/ search") && pool.ends_with("? help  q quit"));
}

#[test]
fn footer_labels_use_only_registered_spaced_marks_and_structural_typography() {
    use crate::board::glyph_guard::glyph_error;
    for tab in [
        crate::board::shell::OpsTab::Overview,
        crate::board::shell::OpsTab::Agents,
    ] {
        for reader in [false, true] {
            let mut app = workspace_board();
            app.shell.tab = tab;
            app.focus = usize::from(reader);
            for width in [usize::MAX, 160, 100, 80, 48, 13, 6] {
                let shown = hints(&app, width);
                assert_eq!(glyph_error(&shown), None, "{width}: {shown}");
            }
        }
    }
}

#[test]
fn footer_hints_follow_rebound_keys_and_show_one_hint_per_action() {
    let mut app = workspace_board();
    workspace_keys(
        &mut app,
        "[bind]\nt = \"copy {name}\"\nz = \"talk\"\nZ = \"talk\"\n",
    );
    let text = hints(&app, usize::MAX);
    assert!(!text.contains("t talk"));
    assert_eq!(
        text.split("  ")
            .filter(|hint| hint.ends_with(" talk"))
            .count(),
        1,
        "{text}"
    );
    assert!(
        text.contains("Z talk"),
        "stable first effective key represents duplicate action"
    );
    assert!(text.contains("t copy"));
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)),
        Effect::None
    );
    assert!(
        app.input.is_some(),
        "advertised compatible override is executable"
    );
}

/// Rows read from a squad config snippet, as `ops.toml` would give them.
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
        references: Default::default(),
        timing: None,
        squad_keys: Vec::new(),
        tabs: vec!["product".into(), "reviews".into()],
        hidden: Vec::new(),
        pinned: 0,
        attention: Default::default(),
        squad: Some("product".into()),
        view: Ok(View {
            history_pending: false,
            ask_lead: crate::config::DEFAULT_ASK_LEAD.into(),
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
            opener: None,
            clipboard: None,

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
fn observed_presence_names_rows_without_state_and_idle_has_its_own_mark() {
    let mut app = board(json!([{"title": null, "rows": [
        row("alpha", "", "", json!({"presence": "active"})),
        row("bravo", "", "", json!({"presence": "offline"})),
        row("carol", "", "", json!({"presence": "unknown"})),
        row("delta", "working", "", json!({"presence": "offline", "state": "working"})),
        row("echo", "idle", "", json!({"presence": "active", "state": "idle"})),
    ]}]));
    app.view.as_mut().unwrap().board.members = true;
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let screen = draw(&app, 100, 20);
    let line = |name: &str| {
        screen
            .iter()
            .position(|line| line.contains(name))
            .unwrap_or_else(|| panic!("{name} in {screen:#?}"))
    };
    let text = |name: &str| &screen[line(name)];
    assert!(text("alpha").contains("online") && !text("alpha").contains("offline"));
    assert!(text("bravo").contains("offline") && !text("bravo").contains("online"));
    assert!(!text("carol").contains("line") && text("carol").contains('–'));
    assert!(
        text("delta").contains("working · offline"),
        "{}",
        text("delta")
    );
    assert!(
        text("delta").contains('●'),
        "the reported state keeps its mark"
    );
    assert!(text("echo").contains("◌ ") && text("echo").contains("idle"));
    assert!(
        !screen.iter().any(|line| line.contains('○')),
        "○ means offline only and no row draws it: {screen:#?}"
    );
    // The word carries the difference; the name also dims while the member is offline.
    let name = |name: &str| {
        let y = line(name);
        buffer[(screen[y].find(name).unwrap() as u16, y as u16)].fg
    };
    let role = |role: Role| app.look().role(role).fg.unwrap_or_default();
    assert_eq!(name("alpha"), role(Role::Text));
    assert_eq!(name("echo"), role(Role::Text));
    assert_eq!(name("bravo"), role(Role::Dim));
    assert_eq!(name("delta"), role(Role::Dim));
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
                "◆ auth-fix       blocked    rotate… https://git…",
                "    approve",
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
                "    approve",
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
                "    approve",
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
                "    approve",
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
        .map(|hit| (hit.y, hit.row().unwrap()))
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
    assert!(
        app.hits
            .borrow()
            .iter()
            .filter(|hit| hit.row().unwrap() == 3)
            .count()
            == 2
    );
    for width in [8, 12, 18] {
        let screen = draw(&app, width, 7);
        assert!(screen.iter().all(|line| line.width() <= usize::from(width)));
        assert!(app.hits.borrow().iter().all(|hit| hit.row().unwrap() < 4));
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
    let mut app = board(json!([
        {"title": "Needs me", "rows": [row("auth-fix", "blocked", "rotate session tokens", json!({"pending": "approve", "note": "needs a call"}))]},
        {"title": "Everyone", "rows": [row("文件-sweep", "working", "整理安装指南", json!({}))]}
    ]));
    lead_sol(&mut app);
    let screen = draw(&app, 48, 12);
    // The tab line holds only the tabs; the summary has its own line.
    assert_eq!(screen[0], "  product    reviews");
    assert_eq!(screen[1], "lead sol · 2 members");
    assert_eq!(screen[2], "  MEMBER     STATE    TASK");
    // The lead is the first row; the rule names what follows.
    assert_eq!(screen[3], "  sol  lead  working  coordinate");
    assert_eq!(screen[4], format!("── members · 2 {}", "─".repeat(33)));
    assert_eq!(screen[5], "NEEDS ME");
    assert_eq!(screen[6], "◆ auth-fix   blocked  rotate session tokens");
    assert_eq!(screen[7], "    approve");
    assert_eq!(screen[8], "EVERYONE");
    assert_eq!(screen[9], "  文件-sweep working  整理安装指南");
    assert!(screen.iter().all(|line| !line.contains("needs a call")));
    assert!(screen[11].starts_with("◆ 1 waiting"));
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
    assert_eq!(screen[3], "  sol  lead  working");
    assert_eq!(screen[5], "  docs       working");
    // Narrow cell: the tag shrinks (ellipsis) before the name does, then goes.
    assert_eq!(draw_lead(8)[3], "  sol  le… working");
    assert_eq!(draw_lead(7)[3], "  sol  l… working");
    assert_eq!(draw_lead(6)[3], "  sol    working");
    assert_eq!(draw_lead(3)[3], "  sol working");
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
    let lines: Vec<(u16, usize)> = app
        .hits
        .borrow()
        .iter()
        .map(|h| (h.y, h.row().unwrap()))
        .collect();
    assert_eq!(
        lines,
        [(4, 0), (6, 1)],
        "a retired note adds no clickable line"
    );
    // Scrolled: only visible lines are clickable, at their screen rows.
    app.selected = 1;
    draw(&app, 48, 6);
    let lines: Vec<(u16, usize)> = app
        .hits
        .borrow()
        .iter()
        .map(|h| (h.y, h.row().unwrap()))
        .collect();
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
fn fit_truncates_by_display_width_and_selected_workspace_targets_scroll_into_view() {
    // Wide Unicode fixture verifies cell width rather than byte or scalar count.
    assert_eq!(fit("整理安装指南", 7), "整理安…");
    assert_eq!(fit("abc", 5), "abc  ");
    assert_eq!(fit("abcdef", 4), "abc…");
    let mut app = crate::board::app::workspace::tests::populated_board(0, 20);
    let target = crate::board::workspace::OpsTarget::Member {
        room_id: None,
        identity_id: "66666666-6666-4666-8666-000000000015".into(),
    };
    let index = app
        .workspace_rows()
        .iter()
        .position(|row| row.target == target)
        .unwrap();
    app.select(index);
    let screen = draw(&app, 80, 12);
    assert!(
        screen.iter().any(|line| line.contains("agent-00015")),
        "{screen:?}"
    );
    assert!(!screen.iter().any(|line| line.contains("agent-00000")));
    assert!(app.scrolls.offset(Pane::Rows) > 0);
    assert!(app.hits.borrow().iter().any(|hit| matches!(hit.target, crate::board::app::HitTarget::Row(row) if app.row_target(row) == Some(crate::board::app::RowTarget::Workspace(target.clone())))));
    app.select(0);
    let screen = draw(&app, 80, 12);
    assert!(screen.iter().any(|line| line.contains("Common pool")));
    assert_eq!(app.scrolls.offset(Pane::Rows), 0);
}

#[test]
fn refresh_hint_and_help_render_exact_lowercase_ctrl_r() {
    let mut app = workspace_board();
    let screen = draw(&app, 160, 12);
    assert!(hints(&app, usize::MAX).contains("ctrl-r refresh"));
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
fn workspace_search_help_and_errors_use_the_footer_and_overlay() {
    let mut app = workspace_board();
    let selected = app.workspace.selected.clone();
    app.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    for character in "zz".chars() {
        app.key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    let screen = draw(&app, 80, 12);
    assert!(
        screen
            .iter()
            .any(|line| line.contains("No matches in this view")),
        "{screen:?}"
    );
    assert!(screen.iter().any(|line| line.contains("Common pool")));
    assert_eq!(screen[11], "/zz▏");
    assert_eq!(
        app.workspace.selected, selected,
        "filtering cannot replace identity"
    );
    assert!(
        app.hits.borrow().iter().all(
            |hit| !matches!(hit.target, crate::board::app::HitTarget::Row(index) if index > 0)
        ),
        "the empty message has no click target"
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.searching);
    assert!(app.search.is_empty());
    app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    assert!(
        draw(&app, 80, 12)
            .iter()
            .any(|line| line.contains("↑↓ scroll"))
    );
    app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    app.error = Some("tmt did not finish in time".into());
    assert_eq!(draw(&app, 80, 12)[11], "tmt did not finish in time");
}

fn paned(board: crate::config::Board, notes: Notes) -> App {
    let bindings = crate::action::with_action_keys(crate::action::preset(true, &board.panes));
    let mut app = App::new(Some("product".into()));
    app.apply(Snapshot {
            references: Default::default(),
            timing: None,
            squad_keys: Vec::new(),
            tabs: vec!["product".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some("product".into()),
            view: Ok(View {
                history_pending: false,
                ask_lead: crate::config::DEFAULT_ASK_LEAD.into(),
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


                opener: None,
                clipboard: None,

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
"                               approve rollout                                  "
[(80, "Rgb(216, 216, 216)/Rgb(74, 74, 74)/Reset/NONE/None")]"#;
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
        for (x, y) in [(2, 2), (15, 0)] {
            assert_eq!(buffer[(x, y)].fg, fg(Role::Muted), "chrome {x},{y}");
        }
        assert_eq!(buffer[(1, 6)].fg, fg(Role::Accent));
        assert!(buffer[(1, 6)].modifier.contains(Modifier::BOLD));
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
    assert!(screen[2].starts_with("── Focus: rows"), "{screen:#?}");
    assert!(right(&screen[2]).starts_with("── detail"), "{screen:#?}");
    let notes_top = screen
        .iter()
        .position(|line| right(line).starts_with("── notes · sol"))
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
fn workspace_field_preview_omits_empty_values_and_clamps_safe_wrapped_text() {
    let mut app = workspace_board();
    workspace_row(&mut app)["fields"] = json!({"task":"", "model":"", "link":""});
    let target = app.row_target(app.selected).unwrap();
    let detail = app.row_detail(&target, &[]).unwrap();
    assert!(detail.fields.is_empty());
    let text = buffer_text(&entity_detail_buffer(&app, 100, 20));
    assert!(text.iter().any(|line| line.contains("no reply yet")));
    assert!(
        !text
            .iter()
            .any(|line| line.contains("task") || line.contains("model") || line.contains("links"))
    );
    workspace_row(&mut app)["fields"]["task"] =
        json!("one\ntwo\t\u{1b}[31m long task that wraps over several preview lines");
    let text = buffer_text(&entity_detail_buffer(&app, 18, 20));
    assert!(text.iter().any(|line| line.contains("task")));
    assert!(text.iter().any(|line| line.contains('…')));
    assert!(
        text.iter()
            .all(|line| line.width() <= 18 && !line.contains('\u{1b}'))
    );
    assert!(
        text.len() <= 4,
        "three task preview lines and the reply placeholder: {text:?}"
    );
    entity_detail_buffer(&app, 0, 0);
    entity_detail_buffer(&app, 1, 1);
}

#[test]
fn the_footer_shows_no_board_key_hints_while_an_overlay_owns_input() {
    let mut app = workspace_board();
    let footer = |app: &App| draw(app, 100, 20).pop().unwrap();
    assert!(footer(&app).contains("q quit"), "{}", footer(&app));
    app.help = true;
    assert!(app.modal_open());
    assert!(footer(&app).trim().is_empty(), "{}", footer(&app));
    app.notice = Some("saved".into());
    assert!(
        footer(&app).contains("saved"),
        "notices stay: {}",
        footer(&app)
    );
    app.help = false;
    app.notice = None;
    assert!(footer(&app).contains("q quit"));
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
    let notes_at = screen[2].find("── notes · sol").expect("notes block title");
    assert_eq!(screen[2][..notes_at].chars().count(), 60, "{screen:#?}");
    assert!(screen[2].starts_with("── Focus: rows"));
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
        .position(|line| line.starts_with("── detail"))
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
        references: Default::default(),
        timing: None,
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
        "pending requests retain shown-tab names and positions"
    );
    assert_eq!(before[0].trim_end(), "  product    reviews");
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
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["id"] = json!("auth-fix");
    app.selected = 1;
    let screen = draw(&app, 70, 11);
    assert!(
        screen[2].starts_with("rows  replies   notes"),
        "{screen:#?}"
    );
    assert!(screen.iter().any(|line| line.contains("auth-fix")));
    let tab = |app: &mut App| {
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    };
    tab(&mut app);
    let screen = draw(&app, 70, 11);
    // The selected pane loses only its brackets; other pane padding stays.
    assert!(
        screen[2].starts_with(" rows  replies  notes"),
        "{screen:#?}"
    );
    assert!(
        screen
            .iter()
            .any(|line| line.contains("Conversation · auth-fix"))
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
    assert!(app.hits.borrow().iter().all(|hit| hit.row().unwrap() >= 8));
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
            let mut app = App::new(Some(crate::board::ALL.into()));
            app.initial_look = Some(look);
            app.loading_since = Some(std::time::Instant::now() - header::SPINNER_DELAY);
            let line = summary_line(&app);
            assert_eq!(line.spans[0].style, look.role(Role::Dim));
            assert_eq!(line.spans[1].style, look.role(Role::Muted));
            assert_eq!(line.spans[2].style, look.role(Role::Text));
            assert_eq!(
                super::tabs::paint(&app, Rect::new(0, 0, 80, 1)).spans[1].style,
                look.role(Role::Accent)
                    .patch(look.selection())
                    .add_modifier(Modifier::BOLD)
            );
            let mut terminal = Terminal::new(TestBackend::new(80, 8)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            assert!(buffer[(10, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(
                buffer[(0, 1)].fg,
                look.role(Role::Dim).fg.unwrap_or_default()
            );
            assert_eq!(
                buffer[(3, 1)].fg,
                look.role(Role::Muted).fg.unwrap_or_default()
            );
        }
    }
}

#[test]
fn startup_spinner_is_delayed_and_animated_without_squad_switch_state() {
    let mut app = App::new(Some(crate::board::ALL.into()));
    let started = std::time::Instant::now();
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
    let screen = draw(&app, 80, 8);
    assert!(screen[0].contains("1 Overview") && screen[0].contains("2 Agents"));
    assert!(screen.iter().any(|line| line.contains("Opening")));
    let settled = workspace_board();
    assert!(!settled.loading());
    assert_eq!(spinner_frame(&settled, std::time::Instant::now()), None);
    assert!(
        !draw(&settled, 80, 8)
            .iter()
            .any(|line| line.contains("Opening"))
    );
}

#[test]
fn only_visible_label_text_invalidates_the_clock() {
    crate::status::with_now_ms(1_000, || {
        let mut app = workspace_board();
        app.view.as_mut().unwrap().replies = vec![json!({
            "recipientId": crate::board::app::workspace::tests::MEMBER,
            "requestId":"response", "submittedAtMs":1_000, "response":"quiet response"
        })];
        draw(&app, 160, 24);
        assert!(
            time_marks(&app, 1_000).is_empty(),
            "a timestamp without displayed age cannot invalidate"
        );
        workspace_row(&mut app)["digest"] =
            json!({"active":true, "digestUntilMs":121_000, "heldCount":2});
        draw(&app, 160, 24);
        let first = time_marks(&app, 1_000);
        assert!(!first.is_empty());
        assert_eq!(first, time_marks(&app, 1_200));
        assert_ne!(first, time_marks(&app, 61_000));
        assert_ne!(
            first,
            time_marks(&app, 121_000),
            "expiration removes the displayed label"
        );
        app.invalidate_frame_geometry();
        assert!(
            time_marks(&app, 61_000).is_empty(),
            "old geometry has no clock authority"
        );
        app.workspace.expanded.clear();
        let room = crate::board::workspace::OpsTarget::Squad(
            crate::board::app::workspace::tests::ROOM.into(),
        );
        let index = app
            .workspace_rows()
            .iter()
            .position(|row| row.target == room)
            .unwrap();
        app.select(index);
        draw(&app, 80, 24);
        assert!(
            time_marks(&app, 61_000).is_empty(),
            "collapsed members have no visible labels"
        );
    });
}

#[test]
fn clipped_label_suffix_does_not_invalidate_until_the_visible_word_expires() {
    crate::status::with_now_ms(1_000, || {
        let mut app = workspace_board();
        workspace_row(&mut app)["digest"] =
            json!({"active":true, "digestUntilMs":121_000, "heldCount":2});
        let mut terminal = Terminal::new(TestBackend::new(6, 24)).unwrap();
        terminal
            .draw(|frame| crate::board::conversation::render(frame, &app, frame.area()))
            .unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|cell| cell.symbol() == "d")
        );
        let first = time_marks(&app, 1_000);
        assert_eq!(first, vec!["digest"]);
        assert_eq!(first, time_marks(&app, 61_000));
        assert_ne!(first, time_marks(&app, 121_000));
    });
}

#[test]
fn overview_quota_clock_tracks_its_visible_projection_and_stays_out_of_agents() {
    const NOW: u64 = 1_791_600_000_000;
    crate::status::with_now_ms(NOW, || {
        let mut app = crate::board::home::tests::oracle::metered(crate::look::Look::default());
        draw(&app, 160, 36);
        let initial = time_marks(&app, NOW);
        assert!(!initial.is_empty());
        let later = time_marks(&app, NOW + 3_600_000);
        assert_ne!(
            initial, later,
            "reset/observation text advances without a new source read"
        );
        app.shell.tab = crate::board::shell::OpsTab::Agents;
        draw(&app, 160, 36);
        assert!(
            time_marks(&app, NOW).is_empty(),
            "quota clock belongs to Overview"
        );
    });
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
    assert!(!draw(&app, 120, 12)[11].contains("A ask lead"));
    assert!(help_lines(&app).iter().any(|line| line.starts_with("A  ")));
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

fn help_lines(app: &App) -> Vec<String> {
    crate::board::help::model(app)
        .sections
        .into_iter()
        .flat_map(|section| section.entries)
        .map(|entry| format!("{}  {}", entry.keys, entry.description))
        .collect()
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
fn receiving_focus_labels_keep_title_footer_and_input_ownership_separate() {
    let mut app =
        board(json!([{"title":null,"rows":[row("selected","working","task",json!({}))]}]));
    assert_eq!(hints(&app, 27), "↑↓ move  ? more  q quit");
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
    let y = unfocused
        .iter()
        .position(|line| line.contains("  selected"))
        .unwrap() as u16;
    let buffer = board_buffer(&app, 80, 20);
    assert_eq!(buffer[(1, y)].symbol(), " ");
    assert_eq!(
        buffer[(2, y)].bg,
        app.look().selection().bg.unwrap_or_default()
    );
    assert_eq!(
        buffer[(2, y)].modifier.contains(Modifier::REVERSED),
        app.look().selection().bg.is_none()
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

fn digest_app(members: bool, _home: bool, held: u64, until: u64, look: crate::look::Look) -> App {
    let focused = row(
        "worker",
        "working",
        "implementation",
        json!({"id":"worker", "state":"working", "digest":{"active":true,"digestUntilMs":until,"remainingMs":until-1000,"heldCount":held}}),
    );
    let mut app = board(json!([{"title":null,"rows":[focused]}]));
    app.view.as_mut().unwrap().board.members = members;
    app.view.as_mut().unwrap().look = look;
    app
}

/// Where `needle` starts on the screen, by cell.
fn find_cells(buffer: &ratatui::buffer::Buffer, needle: &str) -> Vec<(u16, u16)> {
    let width = usize::from(buffer.area.width);
    (0..buffer.area.height)
        .flat_map(|y| {
            let line: Vec<&str> = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            (0..width)
                .filter(|from| line[*from..].concat().starts_with(needle))
                .map(|from| (from as u16, y))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn supplied_chips(ids: &[&str]) -> crate::labels::Supplied {
    use crate::labels::Label;
    use tmt_cli_style::Role;
    let labels = vec![
        Label {
            text: "Auto".into(),
            role: Role::Text,
            action: None,
        },
        Label {
            text: "3 held".into(),
            role: Role::Muted,
            action: None,
        },
        Label {
            text: "Due now".into(),
            role: Role::Waiting,
            action: None,
        },
    ];
    crate::labels::Supplied::from_rows(vec![(
        "digest".into(),
        ids.iter()
            .map(|id| ((*id).to_owned(), labels.clone()))
            .collect(),
    )])
}

#[test]
fn supplied_labels_replace_the_policy_chip_on_every_surface_with_their_roles() {
    use tmt_cli_style::Role;
    let look = crate::look::Look {
        theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse("tmt").unwrap()),
        depth: tmt_cli_style::Depth::TrueColor,
    };
    for (members, home) in [(false, false), (true, false)] {
        for width in [100, 160] {
            crate::status::with_now_ms(1_000, || {
                let mut app = digest_app(members, home, 2, 1_801_000, look);
                if !home {
                    // A second row, so one is unselected and keeps its roles.
                    let view = app.view.as_mut().unwrap();
                    let mut other = view.document["sections"][0]["rows"][0].clone();
                    other["id"] = json!("other");
                    other["name"] = json!("other");
                    view.document["sections"][0]["rows"]
                        .as_array_mut()
                        .unwrap()
                        .push(other);
                }
                app.labels = supplied_chips(&["worker", "lead", "other"]);
                let buffer = board_buffer(&app, width, 20);
                let screen = buffer_text(&buffer).join("\n");
                let narrow_monitor = home && width == 160;
                let shown = screen
                    .matches(if narrow_monitor {
                        "Auto · 3 held"
                    } else {
                        "Auto · 3 held · Due now"
                    })
                    .count();
                assert_eq!(shown, 2, "{members} {home} {width}: {screen}");
                assert!(
                    !screen.contains("digest 30m"),
                    "both chips stacked: {screen}"
                );
                let color = |needle: &str| {
                    find_cells(&buffer, needle)
                        .into_iter()
                        .map(|(x, y)| buffer[(x, y)].fg)
                        .collect::<Vec<_>>()
                };
                let (waiting, muted) = (color("Due now"), color("3 held"));
                assert!(
                    narrow_monitor && waiting.is_empty()
                        || waiting.contains(&look.role(Role::Waiting).fg.unwrap()),
                    "{members} {home} {width}: {waiting:?}"
                );
                assert!(muted.contains(&look.role(Role::Muted).fg.unwrap()));
                assert_ne!(look.role(Role::Waiting).fg, look.role(Role::Muted).fg);
                // Supplied text moves only when its source answers, never with the clock.
                assert!(
                    !super::header::time_marks(&app, 1_000)
                        .iter()
                        .any(|label| label.contains("digest"))
                );
            });
        }
    }
}

/// The choice chip is the only place a click opens the digest dropdown, on every
/// surface that paints member rows; the rest of the row keeps its click.
#[test]
fn clicking_the_choice_chip_asks_for_the_dropdown_on_every_surface() {
    use crate::{
        board::app::Request,
        labels::{Choose, Label, Supplied},
    };
    let choose = Choose::parse(
        "digest",
        &json!({"kind":"choose","options":[{"label":"Auto","value":"default"}],
            "current":"default","argv":["digest","m","{value}"]}),
    )
    .unwrap();
    let supplied = |ids: &[&str]| {
        Supplied::from_rows(vec![(
            "digest".into(),
            ids.iter()
                .map(|id| {
                    (
                        (*id).to_owned(),
                        vec![
                            Label {
                                text: "Auto".into(),
                                role: Role::Text,
                                action: Some(choose.clone()),
                            },
                            Label {
                                text: "3 held".into(),
                                role: Role::Muted,
                                action: None,
                            },
                        ],
                    )
                })
                .collect(),
        )])
    };
    let settle = |app: &mut App| {
        if let Some(tab) = app.current.clone() {
            app.set_tab_focus_for_test(&tab);
        }
    };
    let click = |app: &mut App, (column, row): (u16, u16)| {
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
    for (members, home) in [(false, false), (true, false)] {
        for width in [100, 160] {
            crate::status::with_now_ms(1_000, || {
                let mut app = digest_app(members, home, 2, 1_801_000, Default::default());
                app.labels = supplied(&["worker", "lead"]);
                settle(&mut app);
                let buffer = board_buffer(&app, width, 20);
                let chips = find_cells(&buffer, "Auto ▾");
                assert_eq!(
                    chips.len(),
                    if home { 2 } else { 1 },
                    "{members} {home} {width}: {}",
                    buffer_text(&buffer).join("\n")
                );
                for (index, at) in chips.into_iter().enumerate() {
                    let expected = if index == 1 { "lead" } else { "worker" };
                    // Both ends of the chip, and the glyph itself, are the chip.
                    for dx in [0, 4, 5] {
                        let mut app = digest_app(members, home, 2, 1_801_000, Default::default());
                        app.labels = supplied(&["worker", "lead"]);
                        settle(&mut app);
                        board_buffer(&app, width, 20);
                        let effect = click(&mut app, (at.0 + dx, at.1));
                        let Effect::Act(Request::DigestOpen(ask)) = effect else {
                            panic!(
                                "{members} {home} {width} +{dx}: the chip did not open: {effect:?} {:?}",
                                app.notice
                            );
                        };
                        assert_eq!(ask.squad, "product");
                        assert_eq!(ask.member, expected, "{members} {home} {width}");
                        assert_eq!(app.selected, usize::from(expected == "lead"));
                    }
                    // Left of the chip is the row, not the dropdown.
                    let mut app = digest_app(members, home, 2, 1_801_000, Default::default());
                    app.labels = supplied(&["worker", "lead"]);
                    settle(&mut app);
                    board_buffer(&app, width, 20);
                    assert!(!matches!(
                        click(&mut app, (at.0 - 2, at.1)),
                        Effect::Act(Request::DigestOpen(_))
                    ));
                }
            });
        }
    }
}

#[test]
fn narrow_headings_keep_leading_labels_whole_and_detail_carries_all_of_them() {
    crate::status::with_now_ms(1_000, || {
        for (members, home) in [(false, false), (true, false)] {
            let mut app = digest_app(members, home, 2, 1_801_000, Default::default());
            app.labels = supplied_chips(&["worker", "lead"]);
            let index = 0;
            let target = app.row_target(index).unwrap();
            app.row_details
                .toggle(crate::board::row_detail::Target::Row(target));
            let narrow = buffer_text(&board_buffer(&app, 22, 30)).join("\n");
            assert!(
                !narrow.contains("Due now ·"),
                "a label was cut mid-way: {narrow}"
            );
            // The detail wraps its value; the label column is as narrow as the pane.
            let flat: String = narrow
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '│')
                .collect();
            assert!(
                flat.contains("Auto·3held·Duenow"),
                "{members} {home}: {narrow}"
            );
        }
    });
}

#[test]
fn digest_rows_tick_expire_and_keep_words_and_expanded_details() {
    for (members, home) in [(false, false), (true, false)] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            for width in [80, 160] {
                crate::status::with_now_ms(1_000, || {
                    let app = digest_app(
                        members,
                        home,
                        2,
                        1_801_000,
                        crate::look::Look {
                            theme: tmt_cli_style::Theme::new(
                                tmt_cli_style::Base::parse(base).unwrap(),
                            ),
                            depth,
                        },
                    );
                    let buffer = board_buffer(&app, width, 20);
                    let lines = buffer_text(&buffer);
                    for line in lines.iter().filter(|line| line.contains("digest 30m")) {
                        assert!(
                            line.contains("worker") || line.contains("lead"),
                            "digest must stay on the heading: {line}"
                        );
                    }
                    let screen = lines.join("\n");
                    assert_eq!(
                        screen.matches("digest 30m · 2 held").count(),
                        if home { 2 } else { 1 }
                    );
                    assert!(
                        screen.contains("digest 30m · 2 held"),
                        "{members} {home} {base} {width}: {screen}"
                    );
                    assert!(
                        super::header::time_marks(&app, 1_000)
                            .iter()
                            .any(|label| label == "digest 30m · 2 held")
                    );
                    assert!(
                        !super::header::time_marks(&app, 1_801_000)
                            .iter()
                            .any(|label| label.contains("digest"))
                    );
                    crate::status::with_now_ms(1_801_000, || {
                        assert!(
                            !buffer_text(&board_buffer(&app, width, 20))
                                .join("\n")
                                .contains("digest")
                        )
                    });
                });
            }
        }
    }
    for (members, home) in [(false, false), (true, false)] {
        crate::status::with_now_ms(1_000, || {
            let mut app = digest_app(members, home, 999999, 4_801_000, Default::default());
            let index = 0;
            let target = app.row_target(index).unwrap();
            app.row_details
                .toggle(crate::board::row_detail::Target::Row(target));
            let narrow = buffer_text(&board_buffer(&app, 20, 30)).join("\n");
            // The state keeps the heading; the expanded detail carries the rest.
            assert!(narrow.contains("dige"), "{narrow}");
            assert!(narrow.contains("1h20m"), "{narrow}");
            assert!(narrow.contains("left"), "{narrow}");
        });
    }
}

#[test]
fn digest_heading_keeps_row_height_hits_and_selected_background() {
    crate::status::with_now_ms(1_000, || {
        for (members, home) in [(false, false), (true, false)] {
            for width in [20, 80, 160] {
                let focused = digest_app(members, home, 2, 4_801_000, Default::default());
                let plain = digest_app(members, home, 2, 1_000, Default::default());
                board_buffer(&plain, width, 30);
                let hits = |app: &App| {
                    app.hits
                        .borrow()
                        .iter()
                        .filter_map(|hit| hit.row().map(|row| (row, hit.x, hit.y, hit.width)))
                        .collect::<Vec<_>>()
                };
                let before = hits(&plain);
                let buffer = board_buffer(&focused, width, 30);
                assert_eq!(
                    hits(&focused),
                    before,
                    "digest adds no row lines or hit regions"
                );
                // Below this width the state keeps the whole heading.
                if let Some(hit) = focused
                    .hits
                    .borrow()
                    .iter()
                    .find(|hit| hit.row() == Some(0))
                    .filter(|_| width >= 80)
                {
                    let line = (0..width)
                        .map(|x| buffer[(x, hit.y)].symbol())
                        .collect::<String>();
                    assert!(line.contains("digest"), "{line}");
                    let from = (0..width.saturating_sub("digest".len() as u16 - 1))
                        .find(|x| {
                            (*x..*x + "digest".len() as u16)
                                .map(|at| buffer[(at, hit.y)].symbol())
                                .collect::<String>()
                                == "digest"
                        })
                        .unwrap();
                    for x in from..from + "digest".len() as u16 {
                        assert_eq!(
                            buffer[(x, hit.y)].bg,
                            focused.look().selection().bg.unwrap()
                        );
                    }
                }
            }
        }
        // Wrapped first-line cells retain their original continuation geometry.
        let mut focused = digest_app(false, false, 2, 4_801_000, Default::default());
        let mut plain = digest_app(false, false, 2, 1_000, Default::default());
        for app in [&mut focused, &mut plain] {
            let view = app.view.as_mut().unwrap();
            view.rows = rows_from(
                r#"[p.rows]
columns = [{name = "member", width = "30%"}, {name = "task", width = "70%", overflow = "wrap", max_lines = 4}]
"#,
            );
            view.document["sections"][0]["rows"][0]["fields"]["task"] =
                json!("alpha beta gamma delta epsilon zeta eta theta");
        }
        board_buffer(&plain, 40, 30);
        let starts = plain.row_starts.borrow().clone();
        board_buffer(&focused, 40, 30);
        assert_eq!(*focused.row_starts.borrow(), starts);
        assert_eq!(
            focused
                .hits
                .borrow()
                .iter()
                .map(|hit| hit.y)
                .collect::<Vec<_>>(),
            plain
                .hits
                .borrow()
                .iter()
                .map(|hit| hit.y)
                .collect::<Vec<_>>()
        );
    });
}

#[test]
#[ignore = "explicit digest-window renderer evidence capture"]
fn capture_digest_window_rows() {
    let output = std::env::var("TMT_DIGEST_OUTPUT").expect("task-owned output path");
    let mut cases = Vec::new();
    crate::status::with_now_ms(1_000, || {
        for (members, home) in [(false, false), (true, false)] {
            for (base, depth) in [
                ("tmt", tmt_cli_style::Depth::TrueColor),
                ("tmt-light", tmt_cli_style::Depth::TrueColor),
                ("NO_COLOR", tmt_cli_style::Depth::None),
            ] {
                for width in [20, 80, 160] {
                    for held in [0, 2] {
                        for expanded in [false, true] {
                            for selected in [false, true] {
                                let mut app = digest_app(
                                    members,
                                    home,
                                    held,
                                    4_801_000,
                                    crate::look::Look {
                                        theme: tmt_cli_style::Theme::new(
                                            tmt_cli_style::Base::parse(if base == "NO_COLOR" {
                                                "tmt"
                                            } else {
                                                base
                                            })
                                            .unwrap(),
                                        ),
                                        depth,
                                    },
                                );
                                if !selected {
                                    app.selected = 1;
                                }
                                if expanded {
                                    let target = app.row_target(0).unwrap();
                                    app.row_details
                                        .toggle(crate::board::row_detail::Target::Row(target));
                                }
                                let buffer = board_buffer(&app, width, 30);
                                cases.push(json!({"selected":selected,"members":members,"home":home,"expanded":expanded,"theme":base,"width":width,"held":held,"text":buffer_text(&buffer).join("\n"),"cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>() }));
                            }
                        }
                    }
                }
            }
        }
    });
    std::fs::write(output, serde_json::to_vec_pretty(&cases).unwrap()).unwrap();
}

#[test]
fn migration_notice_survives_keys_and_refresh_until_promotion_in_all_themes() {
    let paused = "Ops migration pending: quit old Squad boards (q); Ops then retries.";
    let holder = "Ops migration pending: quit the old board in pane %41 (q); Ops then retries.";
    for width in [80, 100, 160] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            let mut app = board(json!([]));
            app.view.as_mut().unwrap().look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
                depth,
            };
            for notice in [paused, holder] {
                app.migration_notice = Some(notice.into());
                app.notice = Some("Unrelated action".into());
                app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
                assert_eq!(draw(&app, width, 24).last().unwrap(), notice);
                app.apply(crate::board::app::tests::snapshot("product", json!([])));
                assert_eq!(draw(&app, width, 24).last().unwrap(), notice);
            }
            app.migration_notice = None;
            assert!(
                !draw(&app, width, 24)
                    .last()
                    .unwrap()
                    .contains("migration pending")
            );
        }
    }
}
#[test]
fn supplied_labels_yield_to_the_name_and_the_whole_state_dropping_trailing_labels_first() {
    use crate::labels::Label;
    use tmt_cli_style::Role;
    let sets: [&[(&str, Role)]; 3] = [
        &[("Auto", Role::Text)],
        &[
            ("Every 20s", Role::Text),
            ("3 held", Role::Muted),
            ("In 20s", Role::Muted),
        ],
        &[
            ("Every 5m", Role::Text),
            ("4 held", Role::Muted),
            ("Due now", Role::Waiting),
        ],
    ];
    let source = crate::labels::Supplied::from_rows(vec![(
        "digest".into(),
        sets.iter()
            .enumerate()
            .map(|(n, set)| {
                let labels = set
                    .iter()
                    .map(|(text, role)| Label {
                        text: (*text).into(),
                        role: *role,
                        action: None,
                    })
                    .collect();
                (format!("member-m{}", n + 1), labels)
            })
            .collect(),
    )]);
    for members in [false, true] {
        for width in [100u16, 60, 36] {
            let rows: Vec<Value> = (1..=3)
                .map(|n| {
                    let id = format!("member-m{n}");
                    row(
                        &id,
                        "working",
                        "implementation",
                        json!({"id": id, "state": "working"}),
                    )
                })
                .collect();
            let mut app = board(json!([{"title": null, "rows": rows}]));
            app.view.as_mut().unwrap().board.members = members;
            app.labels = source.clone();
            let lines = buffer_text(&board_buffer(&app, width, 24));
            let screen = lines.join("\n");
            let context = format!("{members} {width}:\n{screen}");
            assert_eq!(
                lines.iter().filter(|line| line.contains("working")).count(),
                3,
                "the state stays whole: {context}"
            );
            for line in lines.iter().filter(|line| line.contains("digest")) {
                assert!(
                    !line.contains('…'),
                    "a label is shown whole or not at all: {context}"
                );
                assert!(
                    !line.trim_end().ends_with("digest"),
                    "the source name never stands alone: {context}"
                );
            }
            if width == 100 {
                for text in [
                    "digest Auto",
                    "digest Every 20s · 3 held · In 20s",
                    "digest Every 5m · 4 held · Due now",
                ] {
                    assert!(screen.contains(text), "{text}: {context}");
                }
            }
            if width == 36 {
                assert!(
                    !screen.contains("Due now") && !screen.contains("In 20s"),
                    "{context}"
                );
            }
        }
    }
}

#[test]
fn supplied_digest_zero_held_and_mode_changes_replace_old_labels_and_keep_the_choice_hit() {
    use crate::labels::{Choose, Label, Supplied};
    for (members, home) in [(false, false), (true, false)] {
        for width in [80, 100, 160] {
            crate::status::with_now_ms(1000, || {
                let mut app = digest_app(members, home, 0, 1_801_000, Default::default());
                if let Some(tab) = app.current.clone() {
                    app.set_tab_focus_for_test(&tab);
                }
                for (mode, text, held) in [
                    ("auto", "Auto", false),
                    ("auto", "Auto", true),
                    ("5m", "Every 5m", false),
                    ("off", "Off", false),
                ] {
                    let choice = Choose::parse(
                        "digest",
                        &json!({"kind":"choose","options":[
                            {"label":"Auto","value":"auto"},
                            {"label":"Every 5m","value":"5m"},
                            {"label":"Off","value":"off"}],
                            "current":mode,"argv":["digest","worker","{value}"]}),
                    )
                    .unwrap();
                    let mut labels = vec![Label {
                        text: text.into(),
                        role: if mode == "off" { Role::Dim } else { Role::Text },
                        action: Some(choice),
                    }];
                    if held {
                        labels.push(Label {
                            text: "3 held".into(),
                            role: Role::Muted,
                            action: None,
                        });
                        labels.push(Label {
                            text: "When idle".into(),
                            role: Role::Muted,
                            action: None,
                        });
                    }
                    app.labels = Supplied::from_rows(vec![(
                        "digest".into(),
                        ["worker", "lead"]
                            .into_iter()
                            .map(|id| (id.into(), labels.clone()))
                            .collect(),
                    )]);
                    let buffer = board_buffer(&app, width, 20);
                    let screen = buffer_text(&buffer).join("\n");
                    let context = format!("{members} {home} {width} {mode} {held}:\n{screen}");
                    assert!(!screen.contains("No held messages"), "{context}");
                    assert_eq!(screen.contains("3 held"), held, "{context}");
                    if width == 160 || !held {
                        assert_eq!(screen.contains("When idle"), held, "{context}");
                    }
                    for old in ["Auto ▾", "Every 5m ▾", "Off ▾"] {
                        assert_eq!(
                            screen.contains(old),
                            old == format!("{text} ▾"),
                            "{context}"
                        );
                    }
                    let at = find_cells(&buffer, &format!("{text} ▾"))[0];
                    let effect = app.mouse(
                        MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column: at.0,
                            row: at.1,
                            modifiers: KeyModifiers::NONE,
                        },
                        std::time::Instant::now(),
                    );
                    let Effect::Act(crate::board::app::Request::DigestOpen(ask)) = effect else {
                        panic!("the mode remains actionable: {context}: {effect:?}");
                    };
                    assert_eq!(ask.member, "worker");
                }
            });
        }
    }
}

#[test]
fn squad_name_columns_follow_display_cells_and_leave_room_for_interval_labels() {
    use crate::labels::Label;
    // CJK fixture glyphs distinguish terminal cells from bytes and character counts.
    for (second, state_columns) in [
        ("bravo", [18, 18, 18]),
        ("documentation", [19, 19, 19]),
        ("界界界界界界界", [20, 20, 20]),
        ("documentation-with-a-very-long-name", [32, 29, 21]),
    ] {
        for (width, state_column) in [100, 60, 36].into_iter().zip(state_columns) {
            for (base, depth) in [
                ("tmt", tmt_cli_style::Depth::TrueColor),
                ("tmt-light", tmt_cli_style::Depth::TrueColor),
                ("tmt", tmt_cli_style::Depth::None),
            ] {
                let mut app = board(json!([{"title": null, "rows": [
                    row("alpha", "working", "", json!({"id":"first", "state":"working"})),
                    row(second, "working", "", json!({"id":"second", "state":"working"})),
                ]}]));
                let view = app.view.as_mut().unwrap();
                view.board.members = true;
                view.look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(
                        tmt_cli_style::theme::Base::parse(base).unwrap(),
                    ),
                    depth,
                };
                let without = board_buffer(&app, width, 24);
                let hits = format!("{:?}", app.hits.borrow());
                let states = find_cells(&without, "working");
                assert_eq!(states.len(), 2, "{second}/{width}/{base}: whole states");
                assert!(states.iter().all(|(x, _)| *x == state_column), "{states:?}");
                app.labels = crate::labels::Supplied::from_rows(vec![(
                    "digest".into(),
                    [("first", "Every 20s"), ("second", "Every 5m")]
                        .into_iter()
                        .map(|(id, text)| {
                            (
                                id.into(),
                                vec![Label {
                                    text: text.into(),
                                    role: Role::Text,
                                    action: None,
                                }],
                            )
                        })
                        .collect(),
                )]);
                let with = board_buffer(&app, width, 24);
                assert_eq!(find_cells(&with, "working"), states, "labels move no state");
                assert_eq!(
                    format!("{:?}", app.hits.borrow()),
                    hits,
                    "labels move no row hit"
                );
                if second == "bravo" && width >= 60 {
                    assert_eq!(find_cells(&with, "digest Every 20s").len(), 1);
                    assert_eq!(find_cells(&with, "digest Every 5m").len(), 1);
                }
            }
        }
    }
}

#[test]
fn squad_search_remeasures_the_name_column_without_retaining_the_old_scene() {
    let mut app = board(json!([{"title": null, "rows": [
        row("alpha", "working", "", json!({"id":"first", "state":"working"})),
        row("documentation", "working", "", json!({"id":"second", "state":"working"})),
    ]}]));
    app.view.as_mut().unwrap().board.members = true;
    let before = board_buffer(&app, 100, 24);
    assert!(find_cells(&before, "working").iter().all(|(x, _)| *x == 19));
    app.search = "alpha".into();
    let filtered = board_buffer(&app, 100, 24);
    assert_eq!(find_cells(&filtered, "working").len(), 1);
    assert_eq!(find_cells(&filtered, "working")[0].0, 18);
    assert!(find_cells(&filtered, "documentation").is_empty());
    app.search.clear();
    assert_eq!(
        board_buffer(&app, 100, 24),
        before,
        "restored cells and styles"
    );
}

#[test]
#[ignore = "read-only name-column review packet; requires SQUAD_NAMES_OUT"]
fn capture_squad_name_column_frames() {
    let mut frames = Vec::new();
    for (case, name) in [
        ("short", "bravo"),
        ("long", "documentation-with-a-very-long-name"),
        ("wide", "界界界界界界界"),
    ] {
        for width in [100, 60, 36] {
            for depth in [tmt_cli_style::Depth::TrueColor, tmt_cli_style::Depth::None] {
                let mut app = board(json!([{"title": null, "rows": [
                    row("alpha", "working", "", json!({"id":"first", "state":"working"})),
                    row(name, "working", "", json!({"id":"second", "state":"working"})),
                ]}]));
                let view = app.view.as_mut().unwrap();
                view.board.members = true;
                view.look.depth = depth;
                app.labels = crate::labels::Supplied::from_rows(vec![(
                    "digest".into(),
                    [("first", "Every 20s"), ("second", "Every 5m")]
                        .into_iter()
                        .map(|(id, text)| {
                            (
                                id.into(),
                                vec![crate::labels::Label {
                                    text: text.into(),
                                    role: Role::Text,
                                    action: None,
                                }],
                            )
                        })
                        .collect(),
                )]);
                frames.push(json!({"case":case,"width":width,"noColor":depth == tmt_cli_style::Depth::None,"lines":draw(&app,width,12)}));
            }
        }
    }
    std::fs::write(
        std::env::var_os("SQUAD_NAMES_OUT").expect("SQUAD_NAMES_OUT"),
        serde_json::to_string_pretty(&frames).unwrap() + "\n",
    )
    .unwrap();
}
