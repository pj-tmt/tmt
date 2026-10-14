use super::*;
use ratatui::{Terminal, backend::TestBackend};

mod routing;

fn app(depth: Depth) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    let view = app.view.as_mut().unwrap();
    view.look.depth = depth;
    for look in view
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .values_mut()
    {
        look.depth = depth;
    }
    assert_eq!(app.look().depth, depth);
    app
}

#[test]
fn focused_rule_title_and_marker_share_existing_focus_with_one_header_row() {
    for depth in [Depth::TrueColor, Depth::None] {
        for ascii in [false, true] {
            let mut app = app(depth);
            app.cron_glyphs = GlyphSet::from_ascii(ascii);
            let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
            let mut inside = Rect::default();
            terminal
                .draw(|frame| {
                    inside = paint(frame, &app, Rect::new(2, 1, 50, 8), Pane::Rows, "Agents");
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(inside, Rect::new(3, 2, 48, 7));
            assert_eq!(buffer[(2, 1)].symbol(), "━");
            let title = (2..52).map(|x| buffer[(x, 1)].symbol()).collect::<String>();
            assert!(title.contains("Agents · focused"), "{title}");
            if depth == Depth::None {
                assert!(title.contains(if ascii { "> Agents" } else { "▶ Agents" }));
            }
            let x = (2..52).find(|x| buffer[(*x, 1)].symbol() == "A").unwrap();
            assert!(buffer[(x, 1)].modifier.contains(Modifier::BOLD));
            assert_eq!(
                buffer[(x, 1)].fg,
                app.look().role(Role::Accent).fg.unwrap_or_default()
            );
            app.focus = 1;
            terminal
                .draw(|frame| {
                    paint(frame, &app, Rect::new(2, 1, 50, 8), Pane::Rows, "Agents");
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(2, 1)].symbol(), "─");
            let title = (2..52).map(|x| buffer[(x, 1)].symbol()).collect::<String>();
            assert!(!title.contains("focused") && !title.contains('▶'));
            let x = (2..52).find(|x| buffer[(*x, 1)].symbol() == "A").unwrap();
            assert!(!buffer[(x, 1)].modifier.contains(Modifier::BOLD));
            terminal
                .draw(|frame| {
                    paint(frame, &app, Rect::new(2, 1, 50, 8), Pane::Replies, "Squad");
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let title = (2..52).map(|x| buffer[(x, 1)].symbol()).collect::<String>();
            assert!(title.contains("Squad · focused · Esc back"));
        }
    }
}

#[test]
fn tiny_frame_keeps_navigation_cells_and_unselected_state_styles() {
    let app = app(Depth::None);
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    for area in [Rect::new(2, 1, 1, 8), Rect::new(2, 1, 50, 2)] {
        terminal
            .draw(|frame| assert_eq!(paint(frame, &app, area, Pane::Rows, "Agents"), area))
            .unwrap();
    }
    let working = app.look().role(Role::Working);
    assert_eq!(row_style(&app, false, working), working);
}

#[test]
fn long_focused_titles_keep_the_back_hint_and_do_not_paint_a_bottom_frame() {
    let mut app = app(Depth::None);
    app.focus = 1;
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal
        .draw(|frame| {
            frame.buffer_mut().set_string(3, 8, "kept", Style::new());
            let body = paint(
                frame,
                &app,
                Rect::new(2, 1, 50, 8),
                Pane::Replies,
                &"long".repeat(80),
            );
            assert_eq!(body, Rect::new(3, 2, 48, 7));
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let title = (2..52).map(|x| buffer[(x, 1)].symbol()).collect::<String>();
    assert!(title.contains("focused · Esc back"), "{title}");
    assert_eq!(buffer[(3, 8)].symbol(), "k");
}

#[test]
fn real_tree_retains_selected_uuid_and_cursor_but_dims_the_row_when_content_receives_focus() {
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
            let cursor = terminal.backend().buffer()[(hit.x, hit.y)].clone();
            assert_eq!(cursor.symbol(), "›");
            assert!(
                cursor.bg == app.look().selection().bg.unwrap_or_default()
                    || cursor.modifier.contains(Modifier::REVERSED)
            );
            app.focus = 1;
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let cell = &terminal.backend().buffer()[(hit.x, hit.y)];
            assert_eq!(cell.symbol(), "›");
            assert!(cell.modifier.contains(Modifier::DIM));
            assert!(!cell.modifier.contains(Modifier::REVERSED));
            assert_eq!(app.workspace.selected, selected);
        }
    }
}
