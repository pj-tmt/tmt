//! An `unbound` key leaves every surface that reads the effective bindings: the footer, Help,
//! the Checklist chip's keycap and the key itself.
use super::*;
use ratatui::{Terminal, backend::TestBackend};

fn text(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..40)
        .map(|y| {
            (0..160)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn configured(app: &mut App, body: &str, name: &str) {
    let path = std::env::temp_dir().join(format!("ops-unbound-{name}-{}.toml", std::process::id()));
    std::fs::write(&path, body).unwrap();
    let config = crate::config::Config::read(path.clone()).unwrap();
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
}

#[test]
fn an_unbound_checklist_key_leaves_the_footer_help_and_the_chip() {
    let mut app = board();
    app.focus = 0;
    let before = text(&app);
    assert!(
        before.contains("C  Checklist"),
        "the chip leads with its key\n{before}"
    );
    assert!(app.bindings().contains_key("C"));
    configured(&mut app, "[bind]\nC = 'unbound'\n", "checklist");
    assert!(!app.bindings().contains_key("C"), "the key is free");
    let after = text(&app);
    assert!(
        after.contains("Checklist"),
        "the chip keeps its label\n{after}"
    );
    assert!(
        !after.contains("C  Checklist"),
        "and loses the keycap\n{after}"
    );
    assert!(
        !after.contains("C checklist"),
        "the footer drops the hint\n{after}"
    );
    let help = crate::board::help::model(&app);
    assert!(
        !help
            .sections
            .iter()
            .flat_map(|section| &section.entries)
            .any(|entry| entry.keys == "C"),
        "Help lists no C"
    );
    // Pressing the key now does nothing.
    app.key(ratatui::crossterm::event::KeyEvent::new(
        ratatui::crossterm::event::KeyCode::Char('C'),
        ratatui::crossterm::event::KeyModifiers::NONE,
    ));
    assert!(app.notes_overlay.is_none() && !app.checklist_shown());
}

#[test]
fn a_no_op_unbound_is_noticed_in_the_workspace_notices() {
    let mut app = board();
    configured(&mut app, "[bind]\nctrl-g = 'unbound'\n", "noop");
    let notices = &app
        .view
        .as_ref()
        .unwrap()
        .home
        .as_ref()
        .unwrap()
        .workspace
        .notices;
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("`bind.ctrl-g`") && notice.contains("no default binding")),
        "{notices:?}"
    );
}
