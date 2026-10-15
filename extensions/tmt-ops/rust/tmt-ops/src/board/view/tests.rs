use super::{
    footer::hints,
    header::{SPINNER_TICK, meter_region, summary_line},
};
use crate::rows::Rows;
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
use crate::board::app::{Effect, Snapshot, View};
use crate::config::Pane;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
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
fn narrow_annotation_caption_keeps_excerpt_and_whole_removal_control_and_hint() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = workspace_board();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let request = "req_1a2b3c4d-1234-4567-89ab-0123456789ab";
    let key = app
        .input
        .as_ref()
        .unwrap()
        .draft_key
        .as_ref()
        .unwrap()
        .clone();
    app.input
        .as_mut()
        .unwrap()
        .response_quotes
        .push(crate::board::response::Quote {
            key: crate::board::home_leads::MessageKey {
                sender: key.0,
                lead: key.1,
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
    let lines = (0..8)
        .map(|y| (0..40).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Quote · worker: \"source\"")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Enter send · Esc list · Alt-⌫ remove")),
        "{lines:?}"
    );
    assert!(!lines.iter().any(|line| line.contains(request)));
    let hit = app
        .composer
        .quote_hit
        .borrow()
        .clone()
        .expect("whole painted removal control");
    assert_eq!(hit.area.width, 3);
    let control: String = (hit.area.x..hit.area.right())
        .map(|x| buffer[(x, hit.area.y)].symbol())
        .collect();
    assert_eq!(control, " ✕ ");
    assert_eq!(hit.source.request, request);
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
        assert!(help.iter().any(|line| line.starts_with("enter  ")));
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
        full.contains("⏎ chat") && full.ends_with("? help  q quit"),
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
    let order = ["⏎ chat", "j host", "o open", "down next", "up previous"];
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
    assert!(plain.contains("⏎ chat") && plain.contains("j host"));
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
        !pool.contains("⏎ chat") && !pool.contains("j host") && !pool.contains("o open"),
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
    assert!(!text.contains("⏎ chat"));
    assert_eq!(
        text.split("  ")
            .filter(|hint| hint.ends_with(" chat"))
            .count(),
        1,
        "{text}"
    );
    assert!(
        text.contains("Z chat"),
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
fn a_squad_of_one_has_one_member() {
    let mut app =
        board(json!([{"title": null, "rows": [row("docs", "working", "guide", json!({}))]}]));
    lead_sol(&mut app);
    assert_eq!(draw(&app, 48, 5)[1], "lead sol · 1 member");
}

#[test]
fn help_lists_effective_theme_keys_and_reports_retired_implicit_lead_and_layout_actions() {
    let mut app = workspace_board();
    workspace_keys(
        &mut app,
        r#"[bind]
T = "theme"
l = "view"
L = "jump lead"
"#,
    );
    let help = help_lines(&app);
    assert!(help.iter().any(|line| line == "T  pick a theme"));
    assert!(help.iter().any(|line| line == "j  go to the member's pane"));
    assert!(!app.bindings().contains_key("L"));
    assert!(!app.bindings().contains_key("l"));
    assert!(
        !help
            .iter()
            .any(|line| line == "L  go to the squad lead's pane")
    );
    let notices = help.join("\n");
    assert!(notices.contains("bind.L") && notices.contains("implicit lead targeting is retired"));
    assert!(
        notices.contains("bind.l") && notices.contains("retired squad-tab/HOME/notebook layout")
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
    use crate::board::app::workspace::tests::{MEMBER, ROOM};
    use crate::board::workspace::OpsTarget;
    for base in [tmt_cli_style::Base::Tmt, tmt_cli_style::Base::TmtLight] {
        for depth in [
            tmt_cli_style::Depth::TrueColor,
            tmt_cli_style::Depth::Ansi16,
            tmt_cli_style::Depth::None,
        ] {
            let look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            };
            for state in [
                "blocked",
                "blocked-on-ci-with-a-long-tail-that-exceeds-every-admitted-state-track",
                "waiting",
            ] {
                for selected in [false, true] {
                    let mut app = workspace_board();
                    app.shell.tab = crate::board::shell::OpsTab::Agents;
                    app.view.as_mut().unwrap().look = look;
                    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                    snapshot.squad_looks.clear();
                    snapshot.roster.members.get_mut(MEMBER).unwrap().state = Some(state.into());
                    snapshot.roster.squads[0]
                        .member_states
                        .get_mut(MEMBER)
                        .unwrap()
                        .state = Some(state.into());
                    let target = OpsTarget::Member {
                        room_id: Some(ROOM.into()),
                        identity_id: MEMBER.into(),
                    };
                    snapshot.rows.get_mut(&target).unwrap()["colors"] = json!({"state":"review"});
                    let index = app
                        .workspace_rows()
                        .iter()
                        .position(|row| row.target == target)
                        .unwrap();
                    if selected {
                        app.select(index);
                    } else {
                        // Clear UUID selection to inspect the unselected member style.
                        app.workspace.selected = None;
                    }
                    let buffer = board_buffer(&app, 160, 24);
                    let hit = *app
                        .hits
                        .borrow()
                        .iter()
                        .find(|hit| hit.row() == Some(index))
                        .unwrap();
                    let cells: Vec<_> = (hit.x..hit.x + hit.width)
                        .map(|x| buffer[(x, hit.y)].symbol())
                        .collect();
                    let shown = if state.starts_with("blocked-on-ci") {
                        let row_text = cells.concat();
                        assert!(
                            !row_text.contains(state),
                            "the long state must fit its track"
                        );
                        let tail = &row_text[row_text.find("blocked").expect("state prefix")..];
                        let end = tail.find('…').expect("clipped State keeps an ellipsis");
                        tail[..end + '…'.len_utf8()].to_owned()
                    } else {
                        state.to_owned()
                    };
                    let at = (0..cells.len())
                        .find(|start| cells[*start..].concat().starts_with(&shown))
                        .unwrap_or_else(|| {
                            panic!(
                                "the State column keeps a visible ellipsis for {state:?}: {}",
                                cells.concat()
                            )
                        });
                    let token = if state == "waiting" {
                        look.role(Role::Waiting)
                    } else {
                        look.named("review")
                    };
                    let expected = if selected {
                        look.selection().patch(look.row_span(true, token, false))
                    } else {
                        token
                    };
                    for x in hit.x + at as u16
                        ..hit.x
                            + at as u16
                            + unicode_width::UnicodeWidthStr::width(shown.as_str()) as u16
                    {
                        assert_eq!(
                            painted_at(&buffer, x, hit.y),
                            painted(expected),
                            "{base:?}/{depth:?}/{state}/selected={selected}"
                        );
                    }
                }
            }
        }
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
            let tab = app.tab_hits.borrow()[0];
            assert!(buffer[(tab.x, tab.y)].modifier.contains(Modifier::BOLD));
            let loading_y = app.conversation.borrow().area.y;
            assert_eq!(
                buffer[(0, loading_y)].fg,
                look.role(Role::Dim).fg.unwrap_or_default()
            );
            assert_eq!(
                buffer[(3, loading_y)].fg,
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
    let tab_y = usize::from(app.tab_hits.borrow()[0].y);
    assert!(screen[tab_y].contains("1 Overview") && screen[tab_y].contains("2 Agents"));
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
        let mut terminal = Terminal::new(TestBackend::new(8, 24)).unwrap();
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

fn help_lines(app: &App) -> Vec<String> {
    crate::board::help::model(app)
        .sections
        .into_iter()
        .flat_map(|section| section.entries)
        .map(|entry| format!("{}  {}", entry.keys, entry.description))
        .collect()
}

#[test]
fn policy_chips_keep_actual_workspace_row_height_hits_and_selection_at_each_width() {
    use crate::board::{app::RowTarget, shell::OpsTab};
    crate::status::with_now_ms(1_000, || {
        for tab in [OpsTab::Overview, OpsTab::Agents, OpsTab::Digest] {
            for width in [40, 80, 160] {
                for (base, depth) in [
                    (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
                    (
                        tmt_cli_style::Base::TmtLight,
                        tmt_cli_style::Depth::TrueColor,
                    ),
                    (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::Ansi16),
                    (tmt_cli_style::Base::Mono, tmt_cli_style::Depth::None),
                ] {
                    let look = crate::look::Look {
                        theme: tmt_cli_style::Theme::new(base),
                        depth,
                    };
                    let mut plain = workspace_board();
                    crate::board::app::workspace::tests::observe_waiting_member(&mut plain);
                    plain.shell.tab = tab;
                    plain.view.as_mut().unwrap().look = look;
                    plain
                        .view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .squad_looks
                        .insert(crate::board::app::workspace::tests::ROOM.into(), look);
                    assert_eq!(plain.look(), look);
                    let target = RowTarget::Workspace(plain.workspace.selected.clone().unwrap());
                    let row_hits = |app: &App| {
                        app.hits
                            .borrow()
                            .iter()
                            .filter_map(|hit| {
                                hit.row()
                                    .map(|index| (app.row_target(index), hit.x, hit.y, hit.width))
                            })
                            .collect::<Vec<_>>()
                    };
                    board_buffer(&plain, width, 30);
                    let before = row_hits(&plain);
                    assert!(
                        before.iter().any(|(row, ..)| row.as_ref() == Some(&target)),
                        "the selected actual member must be painted: {tab:?}/{width}/{depth:?}"
                    );
                    let starts = plain.row_starts.borrow().clone();
                    let mut focused = workspace_board();
                    crate::board::app::workspace::tests::observe_waiting_member(&mut focused);
                    focused.shell.tab = tab;
                    focused.view.as_mut().unwrap().look = look;
                    focused
                        .view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .squad_looks
                        .insert(crate::board::app::workspace::tests::ROOM.into(), look);
                    assert_eq!(focused.look(), look);
                    workspace_row(&mut focused)["digest"] = json!({"active":true,
                        "digestUntilMs":4_801_000,"remainingMs":4_800_000,"heldCount":2});
                    let buffer = board_buffer(&focused, width, 30);
                    assert_eq!(
                        row_hits(&focused),
                        before,
                        "chips add no row or hit geometry"
                    );
                    assert_eq!(*focused.row_starts.borrow(), starts);
                    let (_, x, y, cells) = before
                        .iter()
                        .find(|(row, ..)| row.as_ref() == Some(&target))
                        .unwrap();
                    for column in *x..*x + *cells {
                        assert_eq!(
                            buffer[(column, *y)].bg,
                            look.selection().bg.unwrap_or_default(),
                            "selected row {tab:?}/{width}/{depth:?} at ({column},{y})"
                        );
                        if look.selection().bg.is_none() {
                            assert!(buffer[(column, *y)].modifier.contains(Modifier::REVERSED));
                        }
                    }
                    if width >= 80 {
                        let line = (*x..*x + *cells)
                            .map(|column| buffer[(column, *y)].symbol())
                            .collect::<String>();
                        let label = match tab {
                            OpsTab::Digest => "unavailable",
                            OpsTab::Overview => "waiting",
                            OpsTab::Agents => "—",
                        };
                        assert!(line.contains(label), "{tab:?}/{width}/{depth:?}: {line}");
                        if tab == OpsTab::Overview {
                            assert!(
                                !line.contains("Auto ▾"),
                                "the removed MODE cell grants no choice authority"
                            );
                        }
                    }
                    assert_eq!(focused.workspace.selected, plain.workspace.selected);
                }
            }
        }
    });
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
fn supplied_digest_zero_held_and_mode_changes_replace_old_labels_and_keep_the_choice_hit() {
    use crate::board::app::workspace::tests::{MEMBER, ROOM};
    use crate::board::shell::OpsTab;
    use crate::labels::{Choose, Label, Supplied};
    for tab in [OpsTab::Agents, OpsTab::Digest] {
        for width in [80, 100, 160] {
            crate::status::with_now_ms(1000, || {
                let mut app = workspace_board();
                app.shell.tab = tab;
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
                        labels.extend([
                            Label {
                                text: "3 held".into(),
                                role: Role::Muted,
                                action: None,
                            },
                            Label {
                                text: "When idle".into(),
                                role: Role::Muted,
                                action: None,
                            },
                        ]);
                    }
                    app.labels = Supplied::from_rows(vec![(
                        "digest".into(),
                        [(MEMBER.into(), labels)].into(),
                    )]);
                    let buffer = board_buffer(&app, width, 36);
                    let screen = buffer_text(&buffer).join("\n");
                    let context = format!("{tab:?}/{width}/{mode}/{held}:\n{screen}");
                    assert!(!screen.contains("No held messages"), "{context}");
                    if tab == crate::board::shell::OpsTab::Overview {
                        let rows = app.workspace_rows();
                        let values = crate::board::overview::monitor::values(
                            &app,
                            app.workspace_snapshot().unwrap(),
                            &rows[0],
                        );
                        assert_eq!(
                            values
                                .digest_labels
                                .iter()
                                .any(|label| label.text == "3 held"),
                            held,
                            "{context}"
                        );
                        assert!(
                            !screen.contains("3 hel "),
                            "trailing labels never partially clip: {context}"
                        );
                    } else {
                        assert_eq!(screen.contains("3 held"), held, "{context}");
                    }
                    // Complete trailing labels may be omitted at compact widths.
                    if width == 160 && tab != crate::board::shell::OpsTab::Overview {
                        assert_eq!(screen.contains("When idle"), held, "{context}");
                    }
                    for old in ["Auto ▾", "Every 5m ▾", "Off ▾"] {
                        assert_eq!(
                            screen.contains(old),
                            old == format!("{text} ▾"),
                            "{context}"
                        );
                    }
                    let hits = app.chip_hits.borrow().clone();
                    assert!(
                        !hits.is_empty(),
                        "the visible mode has an exact hit: {context}"
                    );
                    for (area, _) in hits {
                        // Every independent activation follows a fresh frame;
                        // selecting the prior chip retired its old hit map.
                        board_buffer(&app, width, 36);
                        let effect = app.mouse(
                            MouseEvent {
                                kind: MouseEventKind::Down(MouseButton::Left),
                                column: area.x,
                                row: area.y,
                                modifiers: KeyModifiers::NONE,
                            },
                            std::time::Instant::now(),
                        );
                        let Effect::Act(crate::board::app::Request::DigestOpen(ask)) = effect
                        else {
                            panic!("the mode remains actionable: {context}: {effect:?}");
                        };
                        assert_eq!(ask.member, "worker");
                        assert_eq!(ask.squad, "product");
                        assert_eq!(ask.choose.current, mode);
                        assert_eq!(
                            app.workspace.selected,
                            Some(crate::board::workspace::OpsTarget::Member {
                                room_id: Some(ROOM.into()),
                                identity_id: MEMBER.into(),
                            })
                        );
                    }
                }
            });
        }
    }
}
