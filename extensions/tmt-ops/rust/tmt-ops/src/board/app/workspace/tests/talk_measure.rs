//! `t` switches the Agents tab's right column between Talk and Measure.
use super::*;
use crate::board::shell::{OpsTab, RightTab};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::time::Instant;

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn t_switches_between_talk_and_measure_and_measure_shows_the_lower_modules() {
    let mut app = board();
    app.shell.tab = OpsTab::Agents;
    assert_eq!(app.right_tab, RightTab::Talk);
    let talk = paint(&app, 160, 40).join("\n");
    assert!(!talk.contains("Digest by agent"), "{talk}");
    assert_eq!(press(&mut app, KeyCode::Char('t')), Effect::None);
    assert_eq!(app.right_tab, RightTab::Measure);
    let measure = paint(&app, 160, 40).join("\n");
    assert!(measure.contains("TOK 1h"), "{measure}");
    assert!(measure.contains("STATE"), "{measure}");
    assert!(measure.contains("Timers"), "{measure}");
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(app.right_tab, RightTab::Talk);
}

fn click(app: &mut App, tab: RightTab) {
    let area = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find_map(|(area, choice)| (*choice == Choice::RightTab(tab)).then_some(*area))
        .expect("painted Talk/Measure chip");
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        },
        Instant::now(),
    );
}

#[test]
fn a_chip_click_selects_its_pane_and_the_active_chip_stays() {
    let mut app = board();
    app.shell.tab = OpsTab::Agents;
    paint(&app, 160, 40);
    click(&mut app, RightTab::Talk);
    assert_eq!(app.right_tab, RightTab::Talk, "the active chip is a no-op");
    paint(&app, 160, 40);
    click(&mut app, RightTab::Measure);
    assert_eq!(app.right_tab, RightTab::Measure);
    paint(&app, 160, 40);
    click(&mut app, RightTab::Measure);
    assert_eq!(
        app.right_tab,
        RightTab::Measure,
        "the active chip is a no-op"
    );
    paint(&app, 160, 40);
    click(&mut app, RightTab::Talk);
    assert_eq!(app.right_tab, RightTab::Talk);
}
