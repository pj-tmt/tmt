use super::*;
use ratatui::style::Style;
use ratatui::{Terminal, backend::TestBackend};
use tmt_cli_style::Depth;

mod routing;

#[test]
fn digest_panes_share_focus_chrome_without_stacked_rules_or_a_false_back_hint() {
    use crate::board::{app::Effect, shell::OpsTab};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    for depth in [Depth::TrueColor, Depth::None] {
        for (width, height) in [(80, 36), (100, 35), (100, 36), (100, 37), (160, 36)] {
            let mut app = app(depth);
            app.key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
            assert_eq!(app.shell.tab, OpsTab::Digest);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut draw = |app: &mut App| {
                terminal
                    .draw(|frame| {
                        crate::board::view::prepare(app, frame.area());
                        crate::board::view::render(frame, app);
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let lines: Vec<_> = (0..height)
                    .map(|y| {
                        (0..width)
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>()
                    })
                    .collect();
                assert_eq!(crate::test_support::stacked_rules(&lines), None);
                let focused: Vec<_> = lines
                    .into_iter()
                    .filter(|line| line.contains(" ▸ "))
                    .collect();
                assert_eq!(focused.len(), 1, "{focused:?}");
                focused.into_iter().next().unwrap()
            };
            let title = draw(&mut app);
            assert!(title.contains("▸ Agents by TOK"), "{title}");
            assert!(!title.contains("Esc back"));
            let selected = app.workspace.selected.clone();
            assert_eq!(
                app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
                Effect::None
            );
            assert_eq!(app.focused_pane(), Some(Pane::Replies));
            let title = draw(&mut app);
            assert!(
                title.contains(if width >= 145 {
                    "▸ Details · Esc back"
                } else {
                    "▸ Exchanges · talk + reply · Esc back"
                }),
                "{title}"
            );
            assert_eq!(
                app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                Effect::None
            );
            assert_eq!(app.focused_pane(), Some(Pane::Rows));
            assert_eq!(app.shell.tab, OpsTab::Digest);
            assert_eq!(app.workspace.selected, selected);
            assert!(draw(&mut app).contains("▸ Agents by TOK"));
        }
    }
}

fn app(depth: Depth) -> App {
    let app = crate::board::app::workspace::tests::capture_case(
        "agents",
        crate::look::Look {
            theme: tmt_cli_style::Theme::default(),
            depth,
        },
    );
    assert_eq!(app.look().depth, depth);
    app
}

#[test]
fn the_focused_pane_has_a_heavy_bright_frame_and_the_others_light_dim_lines() {
    let area = Rect::new(2, 1, 50, 8);
    for depth in [Depth::TrueColor, Depth::None] {
        for ascii in [false, true] {
            let mut app = app(depth);
            app.cron_glyphs = GlyphSet::from_ascii(ascii);
            let (heavy, light, vertical_heavy, vertical_light, corner) = if ascii {
                ("=", "-", "#", "|", "+")
            } else {
                ("━", "─", "┃", "│", "")
            };
            let corners = |focused: bool| {
                if ascii {
                    ["+"; 4]
                } else if focused {
                    ["┏", "┓", "┗", "┛"]
                } else {
                    ["┌", "┐", "└", "┘"]
                }
            };
            let _ = corner;
            let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
            let mut inside = Rect::default();
            terminal
                .draw(|frame| {
                    inside = paint(frame, &app, area, Pane::Rows, "Agents");
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(inside, Rect::new(4, 2, 46, 6));
            let [tl, tr, bl, br] = corners(true);
            assert_eq!(buffer[(2, 1)].symbol(), tl);
            assert_eq!(buffer[(51, 1)].symbol(), tr);
            assert_eq!(buffer[(2, 8)].symbol(), bl);
            assert_eq!(buffer[(51, 8)].symbol(), br);
            assert_eq!(buffer[(3, 1)].symbol(), heavy);
            assert_eq!(buffer[(30, 8)].symbol(), heavy);
            for y in 2..8 {
                assert_eq!(buffer[(2, y)].symbol(), vertical_heavy);
                assert_eq!(buffer[(51, y)].symbol(), vertical_heavy);
            }
            let title = (2..52).map(|x| buffer[(x, 1)].symbol()).collect::<String>();
            assert!(
                title.contains(if ascii { "> Agents" } else { "▸ Agents" }),
                "{title}"
            );
            let x = (2..52).find(|x| buffer[(*x, 1)].symbol() == "A").unwrap();
            assert!(buffer[(x, 1)].modifier.contains(Modifier::BOLD));
            assert_eq!(
                buffer[(x, 1)].fg,
                app.look().role(Role::Text).fg.unwrap_or_default()
            );
            assert_eq!(
                buffer[(2, 1)].fg,
                app.look().role(Role::Text).fg.unwrap_or_default()
            );
            app.focus = 1;
            terminal
                .draw(|frame| {
                    paint(frame, &app, area, Pane::Rows, "Agents");
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let [tl, tr, bl, br] = corners(false);
            assert_eq!(buffer[(2, 1)].symbol(), tl);
            assert_eq!(buffer[(51, 1)].symbol(), tr);
            assert_eq!(buffer[(2, 8)].symbol(), bl);
            assert_eq!(buffer[(51, 8)].symbol(), br);
            assert_eq!(buffer[(3, 1)].symbol(), light);
            assert_eq!(buffer[(2, 4)].symbol(), vertical_light);
            let title = (2..52).map(|x| buffer[(x, 1)].symbol()).collect::<String>();
            assert!(!title.contains('▸') && !title.contains('>'), "{title}");
            let x = (2..52).find(|x| buffer[(*x, 1)].symbol() == "A").unwrap();
            assert!(!buffer[(x, 1)].modifier.contains(Modifier::BOLD));
            assert_eq!(
                buffer[(x, 1)].fg,
                app.look().role(Role::Muted).fg.unwrap_or_default()
            );
            assert_eq!(
                buffer[(2, 4)].fg,
                app.look().role(Role::Dim).fg.unwrap_or_default()
            );
            terminal
                .draw(|frame| {
                    paint(frame, &app, area, Pane::Replies, "Squad");
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let title = (2..52).map(|x| buffer[(x, 1)].symbol()).collect::<String>();
            assert!(title.contains("Squad · Esc back"), "{title}");
        }
    }
}

#[test]
fn tiny_frame_keeps_navigation_cells_and_unselected_state_styles() {
    let app = app(Depth::None);
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    let working = app.look().role(Role::Working);
    for area in [Rect::new(2, 1, 1, 8), Rect::new(2, 1, 50, 2)] {
        terminal
            .draw(|frame| {
                frame.buffer_mut()[(area.x, area.y)].set_style(working);
                assert_eq!(paint(frame, &app, area, Pane::Rows, "Agents"), area);
                assert_eq!(
                    frame.buffer_mut()[(area.x, area.y)].fg,
                    working.fg.unwrap_or_default()
                );
            })
            .unwrap();
    }
}

#[test]
fn long_focused_titles_keep_the_back_hint_and_short_panes_keep_only_the_top_rule() {
    let mut app = app(Depth::None);
    app.focus = 1;
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal
        .draw(|frame| {
            let boxed = paint(
                frame,
                &app,
                Rect::new(2, 1, 50, 8),
                Pane::Replies,
                &"long".repeat(80),
            );
            assert_eq!(boxed, Rect::new(4, 2, 46, 6));
            // A pane too short to afford the sides keeps the cells below its rule.
            frame
                .buffer_mut()
                .set_string(3, 12 - 1, "kept", Style::new());
            let short = paint(
                frame,
                &app,
                Rect::new(2, 9, 50, 3),
                Pane::Replies,
                &"long".repeat(80),
            );
            assert_eq!(short, Rect::new(3, 10, 48, 2));
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let title = (2..52).map(|x| buffer[(x, 1)].symbol()).collect::<String>();
    assert!(title.contains("· Esc back"), "{title}");
    assert_eq!(buffer[(2, 1)].symbol(), "┏");
    let short = (2..52).map(|x| buffer[(x, 9)].symbol()).collect::<String>();
    assert!(short.starts_with("━━ ▸ "), "{short}");
    assert_eq!(buffer[(2, 11)].symbol(), " ");
    assert_eq!(buffer[(3, 11)].symbol(), "k");
}

#[test]
fn real_tree_retains_selected_uuid_and_cursor_but_tints_the_row_when_content_receives_focus() {
    use crate::board::app::HitTarget;
    for width in [100, 160] {
        for depth in [Depth::TrueColor, Depth::None] {
            let mut app = app(depth);
            app.set_body_width(width);
            let selected = app.workspace.selected.clone();
            let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let hit = app
                .hits
                .borrow()
                .iter()
                .find(|hit| hit.target == HitTarget::Row(app.selected))
                .copied()
                .unwrap();
            let cursor = terminal.backend().buffer()[(hit.x + 3, hit.y)].clone();
            assert_eq!(cursor.symbol(), "└");
            assert!(
                cursor.bg == app.look().selection().bg.unwrap_or_default()
                    || cursor.modifier.contains(Modifier::REVERSED)
            );
            app.focus = 1;
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let cell = &terminal.backend().buffer()[(hit.x + 3, hit.y)];
            assert_eq!(cell.symbol(), "└");
            // Content focus moves the row to the conversation tint (reverse video where there is
            // no background colour); only the agent's name is bold.
            match app.look().role(tmt_cli_style::Role::Conversation).bg {
                Some(tint) => {
                    assert_eq!(cell.bg, tint);
                    assert_ne!(cell.bg, cursor.bg);
                }
                None => assert!(cell.modifier.contains(Modifier::REVERSED)),
            }
            assert!(!cell.modifier.contains(Modifier::BOLD));
            assert_eq!(app.workspace.selected, selected);
        }
    }
}
