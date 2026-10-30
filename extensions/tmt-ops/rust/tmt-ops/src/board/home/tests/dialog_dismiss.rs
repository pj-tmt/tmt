//! The close rule (`design/cli-style.md#dialogs`) for every overlay. Each overlay's own tests open
//! it twice the same way and run [`assert_close_rule`]; [`close_rule_test`] names that test for
//! every `Overlay`, so a new overlay cannot be added without one.
use crate::board::app::{App, Effect, Overlay};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};
use std::time::Instant;

fn paint(app: &App) {
    Terminal::new(TestBackend::new(160, 40))
        .unwrap()
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
}

fn click(app: &mut App, column: u16, row: u16) {
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        Instant::now(),
    );
}

/// A painted board row that the sheet does not cover, if there is one.
fn row_outside(app: &App, sheet: Rect) -> Option<(u16, u16)> {
    app.hits.borrow().iter().find_map(|hit| {
        let at = (hit.x + 2, hit.y);
        (hit.row().is_some() && !sheet.contains(at.into())).then_some(at)
    })
}

/// The text a close could lose: the composer's, and which editors still hold their state.
fn kept(app: &App) -> (Option<String>, [bool; 5], usize) {
    (
        app.input.as_ref().map(|input| input.text.to_string()),
        [
            app.workspace_form.is_some(),
            app.new_member.is_some(),
            app.switch_tool.is_some(),
            app.idle_alert_editor.is_some(),
            app.checklist.is_some(),
        ],
        app.notes.draft_count(),
    )
}

fn painted_sheet(app: &App, overlay: Overlay) -> Rect {
    assert_eq!(app.overlay(), Some(overlay), "opened");
    paint(app);
    app.peek_areas
        .area(overlay)
        .unwrap_or_else(|| panic!("{overlay:?} registers its painted sheet"))
}

/// `esc` and `outside` have `overlay` open the same way, on top. Checks the close rule:
/// - a press inside the sheet keeps it open, and Ctrl-C quits from it;
/// - Esc closes it;
/// - a press outside closes it, is taken unless the overlay peeks, and keeps every text Esc keeps.
pub(in crate::board) fn assert_close_rule(esc: &mut App, outside: &mut App, overlay: Overlay) {
    let sheet = painted_sheet(esc, overlay);
    // Every cell of the chart dropdown is an entry, where a press chooses.
    if overlay != Overlay::UsageControls {
        click(esc, sheet.x, sheet.y);
        assert_eq!(
            esc.overlay(),
            Some(overlay),
            "{overlay:?}: a press on its border keeps it"
        );
    }
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(esc.key(ctrl_c), Effect::Quit, "{overlay:?}: Ctrl-C quits");
    esc.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_ne!(esc.overlay(), Some(overlay), "{overlay:?}: Esc closes");

    assert_outside_press_closes(outside, overlay);
    assert_eq!(
        kept(outside),
        kept(esc),
        "{overlay:?}: an outside press keeps what Esc keeps"
    );
}

/// The outside half of [`assert_close_rule`], for a state inside an overlay that Esc leaves by
/// stepping back rather than closing (an open edit, a second screen).
pub(in crate::board) fn assert_outside_press_closes(app: &mut App, overlay: Overlay) {
    let sheet = painted_sheet(app, overlay);
    let selected = app.selected;
    let (column, row) = row_outside(app, sheet).unwrap_or_else(|| {
        let corner = (159, 39);
        assert!(
            !sheet.contains(corner.into()),
            "{overlay:?} leaves a corner"
        );
        corner
    });
    click(app, column, row);
    assert_ne!(
        app.overlay(),
        Some(overlay),
        "{overlay:?}: an outside press closes"
    );
    if !overlay.peeks() {
        assert_eq!(app.selected, selected, "{overlay:?}: the press is taken");
    }
}

/// Where each overlay runs [`assert_close_rule`].
fn close_rule_test(overlay: Overlay) -> &'static str {
    match overlay {
        Overlay::Workspace => {
            "app::workspace::tests::member_rename::the_workspace_form_follows_the_close_rule"
        }
        Overlay::Recovery => "recovery::tests::the_recovery_window_follows_the_close_rule",
        Overlay::Help => "home::tests::light_dismiss::every_peek_follows_the_close_rule",
        Overlay::Settings => "settings::tests::the_settings_sheet_follows_the_close_rule",
        Overlay::Theme => "theme_picker::tests::the_theme_picker_follows_the_close_rule",
        Overlay::Panels => "app::panels::tests::the_panels_picker_follows_the_close_rule",
        Overlay::CronList => "home::tests::light_dismiss::every_peek_follows_the_close_rule",
        Overlay::CronReview => "cronboard::preview::tests::the_cron_review_follows_the_close_rule",
        Overlay::MonitorDetails => "home::tests::light_dismiss::every_peek_follows_the_close_rule",
        Overlay::ReplyReader => "conversation::tests::the_reply_reader_follows_the_close_rule",
        Overlay::Checklist => "checklist::tests::the_squad_checklist_follows_the_close_rule",
        Overlay::QuotaDetails => "home::tests::light_dismiss::every_peek_follows_the_close_rule",
        Overlay::EstimateBasis => "home::tests::light_dismiss::every_peek_follows_the_close_rule",
        Overlay::Unread => "unread_overlay::tests::the_unread_window_follows_the_close_rule",
        Overlay::Discovery => "discovery::tests::the_discovery_window_follows_the_close_rule",
        Overlay::MemberSession => {
            "member_session::tests::the_session_window_follows_the_close_rule"
        }
        Overlay::Notes => {
            "conversation_checklist::tests::overlay::the_member_checklist_follows_the_close_rule"
        }
        Overlay::NewMember => "new_member::tests::the_new_member_form_follows_the_close_rule",
        Overlay::SwitchTool => "switch_tool::tests::the_switch_tool_form_follows_the_close_rule",
        Overlay::IdleAlert => {
            "idle_alert_editor::tests::the_idle_alert_form_follows_the_close_rule"
        }
        Overlay::UsageControls => {
            "overview::chart::host::tests::navigation::the_chart_dropdown_follows_the_close_rule"
        }
        Overlay::Confirm => "confirm::tests::the_confirm_follows_the_close_rule",
        Overlay::PastePreview => "home::tests::light_dismiss::every_peek_follows_the_close_rule",
    }
}

#[test]
fn every_overlay_names_its_close_rule_test() {
    for overlay in Overlay::ALL {
        assert!(close_rule_test(overlay).ends_with("follows_the_close_rule"));
    }
}
