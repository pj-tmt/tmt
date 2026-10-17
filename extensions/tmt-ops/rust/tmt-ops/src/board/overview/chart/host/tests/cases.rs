//! Acquired-data cases shared by frozen-oracle candidates and visual evidence.
use super::*;

pub(in crate::board) const NAMES: [&str; 8] = [
    "usage-dropdowns",
    "usage-each-menu",
    "usage-auto",
    "usage-newest",
    "usage-middle",
    "usage-hover",
    "usage-pinned",
    "usage-disabled-each",
];

pub(in crate::board) fn board_case(name: &str, look: crate::look::Look, width: u16) -> App {
    let mut app = board();
    fixtures::mixed(&mut app);
    app.view.as_mut().unwrap().look = look;
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    assert_eq!(app.look().depth, look.depth);
    assert_eq!(
        app.look().role(tmt_cli_style::Role::Text),
        look.role(tmt_cli_style::Role::Text)
    );
    let range = if name == "usage-disabled-each" {
        43_200
    } else {
        10_080
    };
    app.usage_chart.borrow_mut().restore_chart_controls(
        range,
        if name == "usage-auto" {
            Each::Auto
        } else {
            Each::Fixed(60)
        },
    );
    paint(&mut app, width, 51); // resolve Auto before acquiring its exact grid
    let selection = app.usage_chart.borrow().selection;
    let read = fixtures::retained(&app, selection);
    assert!(app.apply_history(read, std::time::Instant::now()));
    paint(&mut app, width, 51);
    if name == "usage-middle" {
        app.usage_chart
            .borrow_mut()
            .viewport
            .scroll(selection.bars().unwrap(), -40);
    }
    if matches!(name, "usage-each-menu" | "usage-disabled-each") {
        crate::board::overview::chart::host::open_dropdown(&mut app, Focus::Each);
        if name == "usage-disabled-each" {
            app.usage_chart
                .borrow_mut()
                .dropdown
                .as_mut()
                .unwrap()
                .hint_for(1);
        }
    }
    if matches!(name, "usage-hover" | "usage-pinned") {
        let state = app.usage_chart.borrow();
        let window = state.viewport.window(selection.bars().unwrap());
        let index = (window.start + window.end) / 2;
        let hit = state
            .hits
            .iter()
            .find(|h| h.target == Target::Bar(index))
            .unwrap()
            .area;
        drop(state);
        app.move_pointer(MouseEvent {
            kind: MouseEventKind::Moved,
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        });
        if name == "usage-pinned" {
            click(&mut app, hit);
        }
    }
    app
}

#[test]
fn chart_visual_evidence() {
    let Some(path) = std::env::var_os("TMT_OPS_CHART_CAPTURE") else {
        return;
    };
    let _no_commands = crate::runner::forbid_commands();
    crate::status::with_now_ms(fixtures::NOW, || {
        let mut frames = Vec::new();
        for (theme, look) in crate::board::home::tests::oracle::looks() {
            for name in NAMES {
                for width in [60, 100, 103, 148, 200] {
                    let mut app = board_case(name, look, width);
                    let buffer = paint(&mut app, width, 51);
                    let state = app.usage_chart.borrow();
                    frames.push(serde_json::json!({"state":name,"look":theme,"width":width,"height":51,
                        "rangeMinutes":state.selection.range_minutes,"eachMinutes":state.selection.each_minutes,
                        "window":[state.viewport.window(state.selection.bars().unwrap()).start,state.viewport.window(state.selection.bars().unwrap()).end],
                        "lines":buffer.content.chunks(usize::from(width)).map(|row|row.iter().map(|c|c.symbol()).collect::<String>()).collect::<Vec<_>>(),
                        "cells":buffer.content.iter().map(|cell|serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>() }));
                }
            }
        }
        std::fs::write(path, serde_json::to_vec(&frames).unwrap()).unwrap();
    });
}

/// Opt-in fixture driver for a private terminal capture. Normal checks do no I/O.
#[test]
fn chart_private_terminal_evidence() {
    use std::io::{Read, Write};
    let Ok(socket) = std::env::var("TMT_OPS_CHART_SOCKET") else {
        return;
    };
    let tmux = std::env::var("TMUX").expect("private tmux is required");
    assert_eq!(tmux.split(',').next(), Some(socket.as_str()));
    assert!(socket.starts_with("/private/tmp/ux-revamp-3642-terminal-"));
    let name = std::env::var("TMT_OPS_CHART_SCENE").unwrap();
    assert!(NAMES.contains(&name.as_str()) || name == "usage-oldest" || name == "usage-help");
    let width: u16 = std::env::var("TMT_OPS_CHART_WIDTH")
        .unwrap()
        .parse()
        .unwrap();
    assert!([80, 100, 103, 148, 160, 200].contains(&width));
    let look_name = std::env::var("TMT_OPS_CHART_LOOK").unwrap();
    let look = crate::board::home::tests::oracle::looks()
        .into_iter()
        .find(|(label, _)| *label == look_name)
        .unwrap()
        .1;
    let ready = std::env::var("TMT_OPS_CHART_READY").unwrap();
    assert!(
        ready.starts_with("chart-") && ready.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    );
    let _no_commands = crate::runner::forbid_commands();
    crate::board::terminal::restore_before_panic_reports();
    let mut screen =
        crate::board::terminal::Guard::enter(crate::board::terminal::Crossterm::default()).unwrap();
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout())).unwrap();
    assert_eq!(
        terminal.size().unwrap(),
        ratatui::layout::Size::new(width, 51)
    );
    crate::status::with_now_ms(fixtures::NOW, || {
        let mut app = board_case(
            if name == "usage-oldest" || name == "usage-help" {
                "usage-newest"
            } else {
                &name
            },
            look,
            width,
        );
        if name == "usage-oldest" {
            app.usage_chart.borrow_mut().viewport.oldest();
        }
        if name == "usage-help" {
            app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        }
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
    });
    // tmux's window-renamed hook runs after parsing all preceding frame bytes.
    // The capture runner owns that hook and the unique private-server channel.
    write!(std::io::stdout(), "\x1bk{ready}\x1b\\").unwrap();
    std::io::stdout().flush().unwrap();
    let mut quit = [0];
    std::io::stdin().read_exact(&mut quit).unwrap();
    assert_eq!(quit, [b'q']);
    screen.restore().unwrap();
}
