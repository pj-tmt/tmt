//! The `keys` group on the `,` page: a key is changed in the one-line editor, saved in one write
//! and restored with `r`.
use super::*;
use crate::board::app::Effect;

const ROW: &str = "keys.member-checklist";

fn overlay(f: &Fixture) -> &Overlay {
    f.app.settings.as_ref().unwrap()
}

fn row<'a>(f: &'a Fixture, key: &str) -> &'a crate::settings::BoardSetting {
    overlay(f)
        .settings
        .entries
        .iter()
        .find(|entry| entry.key == key)
        .unwrap()
}

fn text(f: &Fixture) -> String {
    let mut terminal = Terminal::new(TestBackend::new(110, 50)).unwrap();
    terminal
        .draw(|frame| render(frame, overlay(f), Look::default(), frame.area()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn the_group_lists_each_key_with_where_it_comes_from_and_whether_it_can_change() {
    let f = fixture("keys-list", "");
    let checklist = row(&f, ROW);
    assert_eq!(checklist.value, json!("C"));
    assert_eq!(checklist.source, "workspace default");
    assert!(checklist.editable);
    let scroll = row(&f, "keys.scroll-up");
    assert_eq!(scroll.value, json!("k, up"));
    assert!(!scroll.editable, "several keys are edited in ops.toml");
    assert!(!overlay(&f).rebound.contains(ROW) && !overlay(&f).overrides.contains(ROW));
}

#[test]
fn a_key_is_changed_saved_shown_and_restored_with_one_write_each() {
    let mut f = fixture("keys-round-trip", "");
    edit(&mut f.app, ROW, "K");
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
    assert_eq!(press(&mut f.app, KeyCode::Enter), Effect::SaveSetting);
    assert!(f.app.settings.as_mut().unwrap().save());
    let written = std::fs::read_to_string(&f.path).unwrap();
    assert!(written.starts_with(&f.original), "{written}");
    assert!(written.contains("K = \"member-checklist\""), "{written}");
    assert!(written.contains("C = \"unbound\""), "{written}");
    assert_eq!(row(&f, ROW).value, json!("K"));
    assert_eq!(row(&f, ROW).source, "bind.K");
    assert!(
        !overlay(&f).overrides.contains(ROW) && overlay(&f).rebound.contains(ROW),
        "a key is not a squad override"
    );
    let shown = text(&f);
    assert!(shown.contains("bind.K · r reset"), "{shown}");
    f.app
        .settings
        .as_mut()
        .unwrap()
        .surface
        .borrow_mut()
        .select(ROW);
    assert_eq!(press(&mut f.app, KeyCode::Char('r')), Effect::ResetSetting);
    assert!(f.app.settings.as_mut().unwrap().reset());
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
    assert_eq!(row(&f, ROW).value, json!("C"));
    assert_eq!(row(&f, ROW).source, "workspace default");
}

#[test]
fn a_taken_key_is_noticed_while_typing_and_nothing_is_written() {
    let mut f = fixture("keys-conflict", "");
    edit(&mut f.app, ROW, "q");
    let overlay = f.app.settings.as_ref().unwrap();
    assert!(overlay.draft.is_none(), "no draft to save");
    let notice = overlay.notice.as_ref().expect("a notice").message.clone();
    assert!(
        notice.contains("already runs") && notice.contains("quit"),
        "{notice}"
    );
    assert_ne!(
        press(&mut f.app, KeyCode::Enter),
        Effect::SaveSetting,
        "Enter cannot save a refused key"
    );
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
}

#[test]
fn esc_leaves_the_editor_with_nothing_written() {
    let mut f = fixture("keys-esc", "");
    edit(&mut f.app, ROW, "K");
    assert!(f.app.settings.as_ref().unwrap().draft.is_some());
    press(&mut f.app, KeyCode::Esc);
    let overlay = f.app.settings.as_ref().unwrap();
    assert!(!overlay.editing() && overlay.draft.is_none());
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
    assert_eq!(row(&f, ROW).value, json!("C"));
}

#[test]
fn a_row_with_several_keys_says_it_is_read_only() {
    let mut f = fixture("keys-readonly", "");
    f.app
        .settings
        .as_mut()
        .unwrap()
        .surface
        .borrow_mut()
        .select("keys.scroll-up");
    assert_eq!(press(&mut f.app, KeyCode::Enter), Effect::None);
    let overlay = f.app.settings.as_ref().unwrap();
    assert!(!overlay.editing());
    assert!(
        overlay
            .notice
            .as_ref()
            .is_some_and(|notice| notice.message.contains("read-only"))
    );
}

#[test]
fn a_file_changed_underneath_refuses_the_save_and_stays_intact() {
    let mut f = fixture("keys-cas", "");
    edit(&mut f.app, ROW, "K");
    assert_eq!(press(&mut f.app, KeyCode::Enter), Effect::SaveSetting);
    let elsewhere = format!("# edited elsewhere\n{}", f.original);
    std::fs::write(&f.path, &elsewhere).unwrap();
    assert!(!f.app.settings.as_mut().unwrap().save());
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), elsewhere);
    let overlay = f.app.settings.as_ref().unwrap();
    assert!(overlay.editing(), "the typed key is kept to retry");
    assert!(
        overlay
            .notice
            .as_ref()
            .is_some_and(|notice| notice.message.contains("changed"))
    );
}

#[test]
fn saving_the_unchanged_key_writes_nothing() {
    let mut f = fixture("keys-unchanged", "");
    edit(&mut f.app, ROW, "C");
    assert!(f.app.settings.as_ref().unwrap().draft.is_some(), "valid");
    let before = std::fs::read(&f.path).unwrap();
    assert_eq!(press(&mut f.app, KeyCode::Enter), Effect::SaveSetting);
    assert!(f.app.settings.as_mut().unwrap().save());
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    assert_eq!(row(&f, ROW).source, "workspace default");
}
