use super::*;
use std::time::Instant;

fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn help_captures_board_actions_and_restores_selection_focus_and_scroll() {
    let mut app = workspace_board();
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
    assert!(app.input.is_none() && app.settings.is_none());
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
        let mut app = workspace_board();
        key(&mut app, KeyCode::Char('?'));
        let top = draw(&app, width, 30);
        let first = top
            .iter()
            .position(|line| line.starts_with('┌') && line.contains(" help "))
            .expect("painted help header");
        let last = top
            .iter()
            .rposition(|line| line.starts_with('└'))
            .expect("painted help bottom");
        for line in &top[first + 1..last] {
            assert!(line.starts_with("│ ") && line.ends_with('│'), "{line}");
        }
        assert!(top[last - 1].contains(super::super::super::help::FOOTER));
        key(&mut app, KeyCode::End);
        let end = draw(&app, width, 30);
        assert_ne!(end, top);
        assert!(end.iter().any(|line| line.contains("Alt-Backspace")));
        assert_eq!(end[last - 1], top[last - 1]);
        key(&mut app, KeyCode::Home);
        assert_eq!(draw(&app, width, 30), top);
    }
}

#[test]
fn help_mouse_scrolls_its_content_and_an_outside_press_closes_it() {
    let mut app = workspace_board();
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
    // A press inside the sheet never reaches the base rows under it.
    event.kind = MouseEventKind::Down(MouseButton::Left);
    event.row = 5;
    app.mouse(event, Instant::now());
    assert_eq!(app.selected, selected);
    assert!(app.help);
    // A press outside the sheet, on the header, closes it and goes on to the base.
    event.row = 0;
    app.mouse(event, Instant::now());
    assert!(!app.help);
}

#[test]
fn help_pages_by_the_painted_viewport_after_resizing_an_editor() {
    let mut app = workspace_board();
    let selected = app.workspace.selected.clone();
    key(&mut app, KeyCode::Enter);
    assert!(app.input.is_some());
    app.paste("kept draft 界 e\u{301}");
    for (width, height) in [(160, 36), (100, 36), (80, 18), (60, 8)] {
        app.invalidate_frame_geometry();
        app.set_body_width(width);
        draw(&app, width, height);
    }
    key(&mut app, KeyCode::Esc);
    assert!(app.input.is_none());
    for (width, height) in [(100, 36), (80, 18), (60, 8)] {
        app.invalidate_frame_geometry();
        app.set_body_width(width);
        draw(&app, width, height);
        key(&mut app, KeyCode::Char('?'));
        draw(&app, width, height);
        let (page, limit) = {
            let help = app.help_state.borrow();
            let viewport = help.scroll.viewport();
            if height == 8 {
                assert_eq!(
                    viewport.height, 0,
                    "tiny help must not retain a stale large viewport"
                );
            } else {
                assert!(
                    viewport.height > 0,
                    "readable help must publish its current viewport"
                );
            }
            if width == 100 {
                assert!(viewport.height > 20, "large help must publish a full page");
            }
            (
                usize::from(viewport.height).saturating_sub(1).max(1),
                help.scroll.limit(),
            )
        };
        assert!(limit > 0);
        assert_eq!(app.help_state.borrow().scroll.offset(), 0);
        if height == 8 {
            key(&mut app, KeyCode::Esc);
            assert!(!app.help);
            assert_eq!(app.workspace.selected, selected);
            continue;
        }
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.help_state.borrow().scroll.offset(), page.min(limit));
        draw(&app, width, height);
        key(&mut app, KeyCode::PageUp);
        assert_eq!(app.help_state.borrow().scroll.offset(), 0);
        key(&mut app, KeyCode::Esc);
        assert!(!app.help);
        assert_eq!(app.workspace.selected, selected);
    }
}
