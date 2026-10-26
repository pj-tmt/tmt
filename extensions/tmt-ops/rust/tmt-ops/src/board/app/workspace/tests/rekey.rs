//! A key changed through the settings write reaches every surface that reads the effective
//! bindings: the Checklist chip's keycap, the footer, Help and the dispatch table.
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

#[test]
fn a_rebound_checklist_key_shows_on_the_chip_footer_and_help() {
    let mut app = board();
    app.focus = 0;
    let before = text(&app);
    assert!(
        before.contains("C  Checklist") && before.contains("C checklist"),
        "{before}"
    );
    let path = std::env::temp_dir().join(format!("ops-rekey-{}.toml", std::process::id()));
    std::fs::write(&path, "# mine\n").unwrap();
    let mut config = crate::config::Config::read(path.clone()).unwrap();
    config
        .set_setting(None, "keys.member-checklist", "K")
        .unwrap();
    let saved = crate::config::Config::read(path.clone()).unwrap();
    std::fs::remove_file(path).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .configure(&saved)
        .unwrap();
    let shown = text(&app);
    assert!(
        shown.contains("K  Checklist"),
        "the chip leads with K\n{shown}"
    );
    assert!(
        shown.contains("K checklist"),
        "the footer names the new key\n{shown}"
    );
    assert!(
        !shown.contains("C  Checklist") && !shown.contains("C checklist"),
        "{shown}"
    );
    assert!(app.bindings().contains_key("K") && !app.bindings().contains_key("C"));
    let help = crate::board::help::model(&app);
    let keys: Vec<_> = help
        .sections
        .iter()
        .flat_map(|section| &section.entries)
        .map(|entry| entry.keys.as_str())
        .collect();
    assert!(keys.contains(&"K") && !keys.contains(&"C"), "{keys:?}");
    assert_eq!(app.bindings()["K"].text, "member-checklist");
}
