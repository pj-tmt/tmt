//! The action menu is a `tmt-modal` list surface: the shared overlay chrome, with
//! its keys on the last inside line and the selected entry as a whole row.
use super::*;
use crate::board::app::{Choice, Menu, MenuEntry};

fn open(title: &str, keys: &[(&str, &str)]) -> App {
    let mut app =
        board(json!([{"title": null, "rows": [row("auth-fix", "blocked", "rotate", json!({}))]}]));
    app.menu = Some(Menu {
        row_send: None,
        link: None,
        prefill: String::new(),
        title: title.into(),
        entries: keys
            .iter()
            .map(|(key, label)| MenuEntry {
                key: (*key).into(),
                label: (*label).into(),
                choice: Choice::Dismiss,
            })
            .collect(),
        selected: 0,
        surface: Default::default(),
    });
    app
}

fn at(app: &mut App, width: u16, height: u16) -> (Vec<String>, ratatui::buffer::Buffer) {
    app.set_body_width(width);
    (draw(app, width, height), board_buffer(app, width, height))
}

#[test]
fn the_menu_is_shared_overlay_chrome_with_its_keys_on_the_last_inside_line() {
    let mut app = open("auth-fix", &[("y", "copy"), ("backspace", "back")]);
    let (screen, _) = at(&mut app, 120, 14);
    let top = screen
        .iter()
        .position(|line| line.contains("┌ auth-fix "))
        .expect("title");
    assert!(screen[top].contains('┐'), "{screen:#?}");
    assert!(screen[top + 1].contains("│ y        ›copy"), "{screen:#?}");
    assert!(screen[top + 2].contains("│ backspace back"), "{screen:#?}");
    assert!(screen[top + 3].contains("1–2 of 2"), "{screen:#?}");
    assert!(
        screen[top + 4].contains("Enter runs · Esc closes"),
        "{screen:#?}"
    );
    assert!(screen[top + 5].contains('└'), "{screen:#?}");
    assert!(
        !screen
            .iter()
            .any(|line| line.contains("Enter runs, Esc closes")),
        "the old title hint is gone"
    );
}

#[test]
fn selection_is_the_whole_row_and_follows_the_menu_cursor_in_each_depth() {
    for depth in [tmt_cli_style::Depth::TrueColor, tmt_cli_style::Depth::None] {
        let mut app = open("auth-fix", &[("y", "copy"), ("o", "open")]);
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::default(),
            depth,
        };
        let look = app.look();
        let (screen, before) = at(&mut app, 120, 14);
        let top = screen
            .iter()
            .position(|line| line.contains("┌ auth-fix "))
            .unwrap() as u16;
        let left = screen[usize::from(top)].find('┌').unwrap() as u16;
        // Content is inset one cell inside the border; selection fills it.
        let right = screen[usize::from(top)].chars().count() as u16 - 1;
        let selected = |buffer: &ratatui::buffer::Buffer, row: u16, x: u16| {
            let cell = &buffer[(x, top + row)];
            match look.selection().bg {
                Some(bg) => cell.bg == bg,
                None => cell.modifier.contains(ratatui::style::Modifier::REVERSED),
            }
        };
        for x in [left + 2, right - 2] {
            assert!(
                selected(&before, 1, x) && !selected(&before, 2, x),
                "{depth:?} {x}"
            );
        }
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        let (_, after) = at(&mut app, 120, 14);
        for x in [left + 2, right - 2] {
            assert!(
                !selected(&after, 1, x) && selected(&after, 2, x),
                "{depth:?} {x}"
            );
        }
    }
}

#[test]
fn a_narrow_view_gives_the_menu_the_whole_body_width_and_a_tall_menu_scrolls() {
    let keys: Vec<(String, String)> = (0..30)
        .map(|n| (format!("k{n}"), format!("label {n}")))
        .collect();
    let pairs: Vec<(&str, &str)> = keys.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
    let mut app = open("menu", &pairs);
    let (screen, _) = at(&mut app, 80, 20);
    assert!(
        screen
            .iter()
            .any(|line| line.starts_with('┌') && line.ends_with('┐')),
        "{screen:#?}"
    );
    assert!(
        screen
            .iter()
            .any(|line| line.contains("1–") && line.contains("of 30")),
        "{screen:#?}"
    );
    for _ in 0..25 {
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    }
    let (screen, _) = at(&mut app, 80, 20);
    assert!(
        screen.iter().any(|line| line.contains("k25›")),
        "the selected entry stays visible: {screen:#?}"
    );
}

#[test]
fn hostile_or_empty_titles_cannot_break_the_surface() {
    for title in ["", "  ", "a\u{1b}[31m\"<b>&amp;", "x\ny"] {
        let mut app = open(title, &[("y", "copy")]);
        let (screen, _) = at(&mut app, 100, 12);
        assert!(
            screen.iter().any(|line| line.contains("│ y")),
            "{title:?}: {screen:#?}"
        );
        assert!(
            !screen.iter().any(|line| line.contains('\u{1b}')),
            "{title:?}"
        );
    }
    let mut app = open("", &[("y", "copy")]);
    let (screen, _) = at(&mut app, 100, 12);
    assert!(
        screen.iter().any(|line| line.contains("┌ actions ")),
        "{screen:#?}"
    );
}
