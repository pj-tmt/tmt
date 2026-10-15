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
            .shown(OverviewPanel::Digest)
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
    let mut keyboard = Picker::open(OpsPreferences::default(), true);
    keyboard.input(&key(KeyCode::Down));
    keyboard.input(&key(KeyCode::Char('u')));
    let mut control = Picker::open(OpsPreferences::default(), true);
    control.input(&key(KeyCode::Down));
    control.surface.borrow_mut().select("up");
    control.input(&key(KeyCode::Enter));
    assert_eq!(keyboard.draft, control.draft);
    assert_eq!(
        keyboard.draft.digest().unwrap().order()[0],
        crate::config::DigestPanel::Usage
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
    picker.surface.borrow_mut().select("usage");
    picker.input(&key(KeyCode::Enter));
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

#[test]
fn overview_picker_only_exposes_live_fixed_sections_and_keeps_durable_policy() {
    let mut draft = Draft::Overview(Some(OverviewPanels::default()));
    let keys: Vec<_> = draft.rows().into_iter().map(|row| row.0).collect();
    assert_eq!(
        keys,
        [
            "digest",
            "subscriptions",
            "usage",
            "monitor",
            "side",
            "actions-divider",
            "reset",
            "save",
            "cancel"
        ]
    );
    for key in ["usage", "subscriptions", "monitor"] {
        let before = draft.clone();
        assert!(
            draft
                .edit(Edit::Down, key)
                .unwrap_err()
                .contains("fixed positions")
        );
        assert_eq!(draft, before);
    }
    draft.edit(Edit::Toggle, "subscriptions").unwrap();
    assert!(
        !draft
            .overview()
            .unwrap()
            .shown(OverviewPanel::Subscriptions)
    );
    assert!(draft.overview().unwrap().shown(OverviewPanel::Summary));
    assert!(draft.overview().unwrap().shown(OverviewPanel::Exchanges));
    let mut digest = Draft::Digest(Some(DigestPanels::default()));
    let before = digest.clone();
    digest.edit(Edit::Down, "subscriptions").unwrap();
    assert_ne!(digest, before);
}

#[test]
fn picker_has_one_title_and_one_noninteractive_divider_between_toggles_and_actions() {
    use ratatui::crossterm::event::MouseEvent;
    use ratatui::{Terminal, backend::TestBackend};
    for (variant, look) in picker_surface::evidence::looks().into_iter().enumerate() {
        for digest in [false, true] {
            for (width, height) in [(160, 36), (100, 30), (60, 24)] {
                let mut picker = Picker::open(OpsPreferences::default(), digest);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| render(frame, &picker, look, frame.area()))
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
                assert_eq!(text.matches("Panels").count(), 1);
                assert!(!text.contains("Overview panels") && !text.contains("Digest panels"));
                let geometry = {
                    let surface = picker.surface.borrow();
                    let list = surface.frame.as_ref().unwrap().list.as_ref().unwrap();
                    let divider = list
                        .geometry
                        .iter()
                        .find(|row| row.id == "actions-divider")
                        .unwrap()
                        .visible;
                    let side = list
                        .geometry
                        .iter()
                        .find(|row| row.id == "side")
                        .unwrap()
                        .visible;
                    let first_action = if digest { "up" } else { "reset" };
                    let action = list
                        .geometry
                        .iter()
                        .find(|row| row.id == first_action)
                        .unwrap()
                        .visible;
                    assert!(
                        list.rows
                            .iter()
                            .find(|row| row.id == "actions-divider")
                            .unwrap()
                            .disabled
                    );
                    assert_eq!(divider.y, side.bottom());
                    assert_eq!(action.y, divider.bottom());
                    assert_eq!(divider.height, 1);
                    divider
                };
                for x in geometry.x..geometry.right() {
                    assert_eq!(buffer[(x, geometry.y)].symbol(), "─");
                    assert_eq!(
                        buffer[(x, geometry.y)].fg,
                        look.role(tmt_cli_style::Role::Muted).fg.unwrap_or_default()
                    );
                }
                picker_surface::evidence::capture(
                    &format!("panels-{digest}-{variant}-{width}x{height}"),
                    buffer,
                    &picker.surface.borrow(),
                );
                let before = picker.draft.clone();
                assert!(
                    picker
                        .input(&Event::Mouse(MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column: geometry.x,
                            row: geometry.y,
                            modifiers: KeyModifiers::NONE,
                        }))
                        .is_none()
                );
                assert_eq!(picker.draft, before);
                picker.surface.borrow_mut().select("side");
                picker.input(&key(KeyCode::Down));
                assert_eq!(
                    picker.surface.borrow().picker.list.selected(),
                    Some(if digest { "up" } else { "reset" })
                );
            }
        }
    }
}
