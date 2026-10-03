use super::*;
use std::time::Instant;

fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn help_captures_board_actions_and_restores_selection_focus_and_scroll() {
    let mut app = preset_board();
    draw(&app, 100, 30);
    let original = (
        app.current.clone(),
        app.selected,
        app.focus,
        app.search.clone(),
    );
    let offsets =
        [Pane::Rows, Pane::Notes, Pane::Detail, Pane::Replies].map(|pane| app.scrolls.offset(pane));
    key(&mut app, KeyCode::Char('?'));
    draw(&app, 100, 30);
    for code in [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Tab,
        KeyCode::Enter,
        KeyCode::Char('/'),
        KeyCode::Char('s'),
        KeyCode::Char('d'),
        KeyCode::Char('T'),
        KeyCode::Char(','),
        KeyCode::Char('t'),
        KeyCode::Char('r'),
        KeyCode::Char('a'),
    ] {
        assert_eq!(key(&mut app, code), Effect::None);
        assert!(app.help);
    }
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        Effect::None
    );
    for code in [
        KeyCode::Down,
        KeyCode::Char('j'),
        KeyCode::PageDown,
        KeyCode::End,
    ] {
        assert_eq!(key(&mut app, code), Effect::None);
    }
    assert!(app.help_state.borrow().scroll.offset() > 0);
    assert_eq!(
        (
            app.current.clone(),
            app.selected,
            app.focus,
            app.search.clone()
        ),
        original
    );
    assert_eq!(
        [Pane::Rows, Pane::Notes, Pane::Detail, Pane::Replies].map(|pane| app.scrolls.offset(pane)),
        offsets
    );
    assert!(app.input.is_none() && app.settings.is_none() && app.switcher.is_none());
    for close in [KeyCode::Esc, KeyCode::Char('?'), KeyCode::Char('q')] {
        assert_eq!(key(&mut app, close), Effect::None);
        assert!(!app.help);
        key(&mut app, KeyCode::Char('?'));
        assert!(app.help);
        draw(&app, 100, 30);
        assert_eq!(app.help_state.borrow().scroll.offset(), 0);
    }
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Effect::Quit
    );
}

#[test]
fn help_chrome_is_opaque_and_end_keeps_its_footer_at_every_capture_width() {
    for width in [160, 100, 80] {
        let mut app = preset_board();
        key(&mut app, KeyCode::Char('?'));
        let top = draw(&app, width, 30);
        assert!(top[2].starts_with('┌') && top[2].contains(" help "));
        assert!(top[28].starts_with('└'));
        for line in &top[3..28] {
            assert!(line.starts_with("│ ") && line.ends_with('│'), "{line}");
        }
        assert!(top[27].contains(super::super::super::help::FOOTER));
        key(&mut app, KeyCode::End);
        let end = draw(&app, width, 30);
        assert_ne!(end, top);
        assert!(
            end.iter()
                .any(|line| line.contains("copy from the selected row"))
        );
        assert_eq!(end[27], top[27]);
        key(&mut app, KeyCode::Home);
        assert_eq!(draw(&app, width, 30), top);
    }
}

#[test]
fn help_mouse_only_scrolls_its_content_and_never_activates_base_hits() {
    let mut app = preset_board();
    key(&mut app, KeyCode::Char('?'));
    draw(&app, 100, 30);
    let selected = app.selected;
    let mut event = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 5,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    assert_eq!(app.mouse(event, Instant::now()), Effect::None);
    assert_eq!(app.help_state.borrow().scroll.offset(), 3);
    event.row = 0;
    app.mouse(event, Instant::now());
    assert_eq!(app.help_state.borrow().scroll.offset(), 3);
    event.kind = MouseEventKind::Down(MouseButton::Left);
    app.mouse(event, Instant::now());
    assert_eq!(app.selected, selected);
    assert!(app.help);
}
