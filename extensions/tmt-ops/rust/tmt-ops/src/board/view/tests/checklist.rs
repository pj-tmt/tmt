use super::*;
#[test]
fn checklist_masks_body_and_owns_keys_without_claiming_base_receiving_focus() {
    let mut app = preset_board();
    let selected = app.selected;
    let pane = app.focused();
    let before = app.bindings();
    app.open_checklist();
    assert!(app.checklist_shown());
    assert!(app.menu.is_none());
    assert_eq!(super::super::panes::receiving_pane(&app), None);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Checklist"));
    assert!(text.contains("Loading"));
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        Effect::None
    );
    assert_eq!(app.focused(), pane);
    assert_eq!(app.selected, selected);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Effect::None
    );
    assert!(!app.checklist_shown());
    assert_eq!(app.bindings(), before);
}
