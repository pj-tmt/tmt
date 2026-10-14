use super::*;
use crate::config::OverviewPanel;
use ratatui::crossterm::event::KeyEvent;
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn opening_and_preview_never_mutate_expected_and_cancel_is_disposable() {
    let expected = OpsPreferences::default();
    let mut picker = Picker::open(expected.clone(), false);
    assert!(matches!(
        picker.input(&key(KeyCode::Enter)),
        Some(Input::Preview)
    ));
    assert!(
        !picker
            .draft
            .overview()
            .unwrap()
            .shown(OverviewPanel::Summary)
    );
    assert_eq!(picker.expected, expected);
    assert!(picker.next().overview_panels.is_some());
    assert!(matches!(
        picker.input(&key(KeyCode::Esc)),
        Some(Input::Cancel)
    ));
    assert_eq!(
        Picker::open(expected.clone(), false)
            .draft
            .overview()
            .unwrap(),
        OverviewPanels::default()
    );
    assert_eq!(picker.expected, expected);
}
#[test]
fn plain_keys_and_visible_move_actions_change_the_same_selected_panel() {
    let mut keyboard = Picker::open(OpsPreferences::default(), false);
    keyboard.input(&key(KeyCode::Down));
    keyboard.input(&key(KeyCode::Char('u')));
    let mut control = Picker::open(OpsPreferences::default(), false);
    control.input(&key(KeyCode::Down));
    control.surface.borrow_mut().select("up");
    control.input(&key(KeyCode::Enter));
    assert_eq!(keyboard.draft, control.draft);
    assert_eq!(
        keyboard.draft.overview().unwrap().order()[0],
        OverviewPanel::Digest
    );
}
#[test]
fn reset_removes_only_the_active_policy_and_save_of_default_is_explicit() {
    let expected = OpsPreferences {
        digest_panels: Some(PanelSetting::configured(DigestPanels::default())),
        ..Default::default()
    };
    let mut picker = Picker::open(expected.clone(), false);
    assert!(picker.next().overview_panels.is_some());
    picker.surface.borrow_mut().select("reset");
    picker.input(&key(KeyCode::Enter));
    let next = picker.next();
    assert!(next.overview_panels.is_none());
    assert_eq!(next.digest_panels, expected.digest_panels);
    assert_eq!(picker.draft.overview().unwrap(), OverviewPanels::default());
    picker.input(&key(KeyCode::Char('d')));
    assert!(picker.next().overview_panels.is_some());
}
#[test]
fn last_surface_refusal_and_pending_save_keep_the_draft() {
    let mut picker = Picker::open(OpsPreferences::default(), false);
    picker.surface.borrow_mut().select("monitor");
    picker.input(&key(KeyCode::Char(' ')));
    picker.surface.borrow_mut().select("side");
    let before = picker.draft.clone();
    picker.input(&key(KeyCode::Enter));
    assert_eq!(picker.draft, before);
    assert!(picker.notice.as_deref().unwrap().contains("monitor"));
    picker.start_save();
    assert!(picker.input(&key(KeyCode::Enter)).is_none());
    assert_eq!(picker.draft, before);
    assert!(matches!(
        picker.input(&key(KeyCode::Esc)),
        Some(Input::Cancel)
    ));
}
