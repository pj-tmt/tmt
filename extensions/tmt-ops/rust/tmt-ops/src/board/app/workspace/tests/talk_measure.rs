//! `t` switches the Agents tab's right column between Talk and Measure.
use super::*;
use crate::board::shell::{OpsTab, RightTab};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn t_switches_between_talk_and_measure_and_measure_shows_the_lower_modules() {
    let mut app = board();
    app.shell.tab = OpsTab::Agents;
    assert_eq!(app.right_tab, RightTab::Talk);
    let talk = paint(&app, 160, 40).join("\n");
    assert!(!talk.contains("Who talks to whom"), "{talk}");
    assert_eq!(press(&mut app, KeyCode::Char('t')), Effect::None);
    assert_eq!(app.right_tab, RightTab::Measure);
    let measure = paint(&app, 160, 40).join("\n");
    assert!(measure.contains("TOK 1h"), "{measure}");
    assert!(measure.contains("STATE"), "{measure}");
    assert!(measure.contains("Timers"), "{measure}");
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(app.right_tab, RightTab::Talk);
}
