//! `M` switches the Agents tab's right column between Talk and Measure.
use super::*;
use crate::board::shell::RightTab;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::time::Instant;

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn measure_key_switches_both_scopes_and_t_stays_inert() {
    for target in [
        OpsTarget::Squad(ROOM.into()),
        OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        },
    ] {
        let mut app = board();
        app.workspace.selected = Some(target);
        app.workspace_reconcile();
        assert_eq!(app.right_tab, RightTab::Talk);
        let talk = paint(&app, 160, 40).join("\n");
        assert!(!talk.contains("Digest by agent"), "{talk}");
        assert_eq!(press(&mut app, KeyCode::Char('t')), Effect::None);
        assert_eq!(app.right_tab, RightTab::Talk);
        assert!(app.input.is_none(), "unbound t never opens a composer");
        assert_eq!(press(&mut app, KeyCode::Char('M')), Effect::None);
        assert_eq!(app.right_tab, RightTab::Measure);
        let measure = paint(&app, 160, 40).join("\n");
        assert!(measure.contains("TOK 1h"), "{measure}");
        assert!(measure.contains("STATE"), "{measure}");
        assert!(measure.contains("Timers"), "{measure}");
        app.key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT));
        assert_eq!(app.right_tab, RightTab::Talk);
    }
}

#[test]
fn measure_hints_and_help_follow_defaults_and_authored_keys() {
    for (bindings, key) in [("", "M"), ("[bind]\nM='copy'\nt='talk-measure'\n", "t")] {
        let mut app = board();
        let path = std::env::temp_dir().join(format!(
            "ops-measure-key-{}.toml",
            crate::id::new_v4().unwrap()
        ));
        std::fs::write(&path, bindings).unwrap();
        let config = Config::read(path.clone()).unwrap();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .configure(&config)
            .unwrap();
        std::fs::remove_file(path).unwrap();
        let text = paint(&app, 160, 40).join("\n");
        assert!(text.contains(&format!("{key}  Talk   Measure")), "{text}");
        let hints = crate::board::view::footer::agents_hints(&app, 500).unwrap();
        assert!(hints.contains(&format!("{key} talk/measure")), "{hints}");
        let help = crate::board::help::model(&app).value();
        let entry = help["sections"][0]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == format!("binding-{key}"))
            .unwrap();
        assert_eq!(entry["keys"], key);
        assert_eq!(
            entry["description"],
            "switch the right column between Talk and Measure"
        );
        press(&mut app, KeyCode::Char(key.chars().next().unwrap()));
        assert_eq!(app.right_tab, RightTab::Measure);
    }
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
