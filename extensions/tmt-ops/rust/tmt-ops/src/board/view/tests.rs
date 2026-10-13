use super::{
    footer::hints,
    header::{SPINNER_TICK, meter_region, summary_line},
    overlays::render_switcher,
};
use crate::board::app::Switcher;
use crate::{
    attention::Attention,
    config::{NotesRender, TabColors},
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
        let mut app = board(json!([]));
        let view = app.view.as_mut().unwrap();
        view.home = Some(crate::board::home::Home {
            workspace: Default::default(),
            summary: Default::default(),
            windows: crate::config::TokenRate::default().windows,
            sections: Vec::new(),
            squads: Vec::new(),
            failures: Vec::new(),
            incomplete: false,
        });
        view.history_pending = true;
        let pending = draw(&app, width, 24);
        assert!(pending.iter().any(|line| line.contains("Updating usage…")));
        assert!(
            pending
                .iter()
                .any(|line| line.contains("Nothing needs you"))
        );
        app.view.as_mut().unwrap().history_pending = false;
        assert!(
            draw(&app, width, 24)
                .iter()
                .all(|line| !line.contains("Updating usage"))
        );
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
    assert!(!hints(&app, 120).contains("z ask lead"));
    assert!(help_lines(&app).iter().any(|line| line.starts_with("z  ")));
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
        let mut app =
            board(json!([{"title":null,"rows":[row("worker","working","task",json!({}))]}]));
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(base),
            depth,
        };
        let text = hints(&app, 160);
        assert!(
            !text.contains("a write") && !text.contains("A ask lead") && !text.contains("Focus:")
        );
        let help = help_lines(&app);
        assert!(help.iter().any(|line| line.starts_with("a  ")));
        assert!(help.iter().any(|line| line.starts_with("A  ")));
        for list in [text, crate::board::cronboard::jobs_hints(160, true)] {
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
    let mut app = board(json!([{"title": null, "rows": [row("a", "working", "", json!({}))]}]));
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
    let mut app = App::new(Some("product".into()));
    app.apply(crate::board::app::tests::snapshot("product", json!([])));
    let full = hints(&app, usize::MAX);
    assert!(full.starts_with("↑↓ move  / search"), "{full}");
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
    let order = ["↑↓ move", "⏎ open", "t talk", "r reply", "/ search"];
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
    // Mandatory hints remain while optional actions fit whole.
    let narrow = hints(&app, 100);
    assert!(narrow.contains("⏎ open  t talk  r reply"), "{narrow}");
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
    for hint in ["⏎ open", "t talk"] {
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
            composition: Default::default(),
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
            host_bindings: Default::default(),
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
            composition: Default::default(),
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
                host_bindings: Default::default(),
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
fn sent_feedback_sits_under_its_row_before_the_annotation_and_stays_clickable() {
    let mut app = board(json!([{"title": null, "rows": [
        row("alpha", "working", "one", json!({"id": "u1", "annotation": {"to": "sol", "text": "noted"}})),
        row("bravo", "working", "two", json!({"id": "u2"})),
    ]}]));
    app.view.as_mut().unwrap().look = Default::default();
    app.sent = Some(crate::board::app::RowFeedback {
        mark: Some("sent"),
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
    assert_eq!(hit.map(|hit| hit.row().unwrap()), Some(0));
    let buffer = board_buffer(&app, 60, 12);
    let look = app.look();
    assert_eq!(
        painted_at(&buffer, 4, (alpha + 1) as u16),
        painted(look.role(Role::Working))
    );
    // A queued request says so instead of claiming it was sent.
    app.sent.as_mut().unwrap().mark = Some("queued · offline");
    assert_eq!(draw(&app, 60, 12)[alpha + 1], "    ✓ queued · offline");
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
fn lead_detail_omits_missing_fields_and_wraps_safe_text_through_the_shared_scroll() {
    let mut app = board(json!([]));
    app.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol", "id": "LEAD", "lifetime": "saved", "fields": {"task": "", "model": "", "cap": ""}});
    let buffer = entity_detail_buffer(&app, 100, 20);
    assert_eq!(
        buffer_text(&buffer),
        ["sol  lead", "no row fields set · tmt ops sq set sol task=…",]
    );
    assert_eq!(
        buffer[(0, 1)].fg,
        app.look().role(Role::Dim).fg.unwrap_or_default()
    );
    app.view.as_mut().unwrap().document["squad"]["lead"]["fields"]["task"] =
        json!("one\ntwo\t\u{1b}[31m long task to wrap");
    let text = buffer_text(&entity_detail_buffer(&app, 12, 20)).join("");
    assert!(text.contains("task:"));
    assert!(!text.contains('\u{1b}') && !text.contains("no row fields set"));
    entity_detail_buffer(&app, 12, 3);
    app.scrolls
        .scroll(Pane::Detail, super::super::scroll::Step::Bottom);
    assert!(
        buffer_text(&entity_detail_buffer(&app, 12, 3))
            .join("")
            .contains("wrap")
    );
    entity_detail_buffer(&app, 0, 0);
    entity_detail_buffer(&app, 1, 1);
}

#[test]
fn the_footer_shows_no_board_key_hints_while_an_overlay_owns_input() {
    let mut app = board(
        json!([{"rows":[row("worker", "working", "ship", json!({"id":"W", "lifetime":"saved"}))]}]),
    );
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
fn a_pane_above_another_ends_on_the_next_heading_rule() {
    for (height, panes) in [
        (24, vec![Pane::Rows, Pane::Notes]),
        (40, vec![Pane::Rows, Pane::Notes]),
        (40, vec![Pane::Notes, Pane::Rows]),
    ] {
        let app = paned(
            split(Direction::TopBottom, panes, vec![50, 50]),
            Notes::Text("ship it".into()),
        );
        let screen = draw(&app, 100, height);
        let mut headings = app
            .title_hits
            .borrow()
            .iter()
            .map(|hit| usize::from(hit.area.y))
            .collect::<Vec<_>>();
        headings.sort_unstable();
        assert_eq!(headings.len(), 2, "{screen:#?}");
        let lower = headings[1];
        assert!(
            !screen[lower - 1].starts_with('─'),
            "a frame edge stacks above the heading rule: {screen:#?}"
        );
    }
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
    assert_eq!(tabs.trim_end(), "◆ product 1 ✗ 1  ✗ reviews 2");
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
            assert_eq!((rows.y, rows.height), (2, 1));
            assert_eq!(notes.x, width * 60 / 100);
            let dim = app.look().role(Role::Dim);
            for x in [rows.x, rows.right() - 1] {
                assert_eq!(buffer[(x, rows.y)].symbol(), "─");
                assert_eq!(buffer[(x, rows.y)].fg, dim.fg.unwrap_or_default());
                assert_eq!(buffer[(x, rows.y + 1)].symbol(), " ");
            }
            let title = &buffer[(rows.x + 3, rows.y)];
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
            let at = &buffer[(title.x + 3, title.y)];
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

fn digest_app(members: bool, home: bool, held: u64, until: u64, look: crate::look::Look) -> App {
    let focused = row(
        "worker",
        "working",
        "implementation",
        json!({"id":"worker", "state":"working", "digest":{"active":true,"digestUntilMs":until,"remainingMs":until-1000,"heldCount":held}}),
    );
    let mut app = board(json!([{"title":null,"rows":[focused]}]));
    app.view.as_mut().unwrap().board.members = members;
    app.view.as_mut().unwrap().look = look;
    if home {
        let member = app.view.as_ref().unwrap().document["sections"][0]["rows"][0].clone();
        let mut lead = member.clone();
        lead["id"] = json!("lead");
        lead["name"] = json!("lead");
        app.current = Some(crate::tabs::ALL.into());
        app.view.as_mut().unwrap().home = Some(crate::board::home::Home {
            workspace: Default::default(),
            summary: Default::default(),
            windows: crate::config::TokenWindow::DEFAULTS,
            sections: vec![crate::board::home::MemberSection {
                key: "blocked".into(),
                rows: vec![crate::board::home::MemberRow {
                    squad: "product".into(),
                    member,
                    lead: Some("lead".into()),
                    age: None,
                }],
            }],
            squads: vec![crate::board::home::SquadLine {
                squad: "product".into(),
                lead: Some(lead.clone()),
                counts: Default::default(),
                members: Default::default(),
            }],
            failures: Vec::new(),
            incomplete: false,
        });
        app.home_leads.reconcile(
            app.view.as_ref().unwrap().home.as_ref().unwrap(),
            Some("owner"),
        );
    }
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
    for (members, home) in [(false, false), (true, false), (false, true)] {
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
    for (members, home) in [(false, false), (true, false), (false, true)] {
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
                        if home {
                            assert_eq!(
                                app.home_entries()[app.selected].target.member.as_deref(),
                                Some(expected)
                            );
                        } else {
                            assert_eq!(app.selected, usize::from(expected == "lead"));
                        }
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
        for (members, home) in [(false, false), (true, false), (false, true)] {
            let mut app = digest_app(members, home, 2, 1_801_000, Default::default());
            app.labels = supplied_chips(&["worker", "lead"]);
            let index = if home {
                app.home_entries()
                    .iter()
                    .position(|entry| entry.target.member.as_deref() == Some("worker"))
                    .unwrap()
            } else {
                0
            };
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
    for (members, home) in [(false, false), (true, false), (false, true)] {
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
    for (members, home) in [(false, false), (true, false), (false, true)] {
        crate::status::with_now_ms(1_000, || {
            let mut app = digest_app(members, home, 999999, 4_801_000, Default::default());
            let index = if home {
                app.home_entries()
                    .iter()
                    .position(|entry| entry.target.member.is_some())
                    .unwrap()
            } else {
                0
            };
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
        for (members, home) in [(false, false), (true, false), (false, true)] {
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
        for (members, home) in [(false, false), (true, false), (false, true)] {
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
    for (members, home) in [(false, false), (true, false), (false, true)] {
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
