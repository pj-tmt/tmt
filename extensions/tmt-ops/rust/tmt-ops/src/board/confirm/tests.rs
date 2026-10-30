use super::*;
use crate::board::app::workspace::tests::board;
use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

fn opened() -> App {
    let mut app = board();
    app.confirm(
        "open panels?".into(),
        vec!["The panels picker opens.".into()],
        "Open".into(),
        Choice::Panels,
    );
    app
}

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn paint(app: &App) {
    ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30))
        .unwrap()
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
}

#[test]
fn the_confirm_follows_the_close_rule() {
    crate::board::home::tests::dialog_dismiss::assert_close_rule(
        &mut opened(),
        &mut opened(),
        Overlay::Confirm,
    );
}

#[test]
fn enter_starts_on_cancel_and_the_action_needs_a_move_to_it() {
    let mut app = opened();
    paint(&app);
    press(&mut app, KeyCode::Enter);
    assert!(app.confirm.is_none(), "Enter on the starting Cancel closes");
    assert!(app.panels_picker.is_none(), "and applies nothing");

    let mut app = opened();
    paint(&app);
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Enter);
    assert!(app.confirm.is_none());
    assert!(app.panels_picker.is_some(), "the action applies its choice");
}

#[test]
fn n_keeps_and_y_applies_as_the_menu_it_replaced() {
    let mut app = opened();
    paint(&app);
    press(&mut app, KeyCode::Char('n'));
    assert!(app.confirm.is_none() && app.panels_picker.is_none());

    let mut app = opened();
    paint(&app);
    press(&mut app, KeyCode::Char('y'));
    assert!(app.confirm.is_none() && app.panels_picker.is_some());
}

#[test]
fn an_unpainted_action_never_applies() {
    let mut app = opened();
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Enter);
    assert!(app.confirm.is_some(), "nothing was painted to choose from");
    assert!(app.panels_picker.is_none());

    let mut app = opened();
    press(&mut app, KeyCode::Char('y'));
    assert!(app.confirm.is_some(), "y needs the painted action too");
    assert!(app.panels_picker.is_none());
}
