use super::*;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

#[test]
fn tab_and_both_backtab_encodings_toggle_existing_panes_without_changing_selection() {
    for width in [60, 100, 160] {
        let mut app = app(Depth::None);
        app.set_body_width(width);
        let selected = app.workspace.selected.clone();
        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focused(), Pane::Replies);
        assert!(
            app.hits.borrow().is_empty(),
            "a focus layout change revokes old row hits"
        );
        app.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.focused(), Pane::Rows);
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT));
        assert_eq!(app.focused(), Pane::Replies);
        assert_eq!(app.workspace.selected, selected);
    }
}

#[test]
fn compact_overview_tab_never_enters_a_hidden_pane_or_changes_destination() {
    use crate::board::shell::OpsTab;
    for width in [60, 100] {
        let mut app = app(Depth::None);
        app.shell.tab = OpsTab::Overview;
        app.set_body_width(width);
        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        for code in [KeyCode::Tab, KeyCode::BackTab] {
            app.key(KeyEvent::new(code, KeyModifiers::NONE));
            assert_eq!(app.focused(), Pane::Rows);
            assert_eq!(app.shell.tab, OpsTab::Overview);
        }
    }
}

#[test]
fn raw_backtab_remains_owned_by_typing_search_and_overlay_before_base_bindings() {
    let mut app = app(Depth::None);
    app.set_body_width(160);
    for table in app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .keys
        .values_mut()
    {
        table.insert(
            "backtab".into(),
            crate::action::Action::parse("quit").unwrap(),
        );
    }
    let key = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
    app.searching = true;
    assert_eq!(app.key(key), crate::board::app::Effect::None);
    assert!(app.searching);
    app.searching = false;
    app.help = true;
    assert_eq!(app.key(key), crate::board::app::Effect::None);
    assert!(app.help);
    app.help = false;
    app.perform(&crate::action::Action::parse("talk").unwrap());
    app.input.as_mut().unwrap().text = "draft".into();
    let focus = app.focus;
    assert_eq!(app.key(key), crate::board::app::Effect::None);
    assert_eq!(app.input.as_ref().unwrap().text, "draft");
    assert_eq!(app.focus, focus);
    app.input = None;
    assert_eq!(
        app.key(key),
        crate::board::app::Effect::Quit,
        "the same authored binding is active outside text owners"
    );
}

#[test]
fn title_click_and_content_click_focus_the_painted_pane_and_stale_titles_are_refused() {
    for width in [100, 160] {
        let mut app = app(Depth::None);
        app.set_body_width(width);
        let selected = app.workspace.selected.clone();
        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let area = app.conversation.borrow().area;
        let content = app.conversation.borrow().pane.unwrap();
        let click = |x, y| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(click(content.x + 1, content.y), std::time::Instant::now());
        assert_eq!(app.focused(), Pane::Replies);
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        app.mouse(click(area.x + 1, area.y), std::time::Instant::now());
        assert_eq!(app.focused(), Pane::Rows);
        assert_eq!(app.workspace.selected, selected);
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.mouse(click(area.x + 1, area.y), std::time::Instant::now());
        assert_eq!(
            app.focused(),
            Pane::Replies,
            "no paint has republished title authority"
        );
    }
}
