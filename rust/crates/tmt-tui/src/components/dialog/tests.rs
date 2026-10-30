use super::*;
use ratatui::crossterm::event::KeyEvent;
use tmt_cli_style::{
    Base, breakpoint,
    theme::screen::{self, ChipKind, ChipState},
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn confirm_buttons() -> [Button<'static>; 2] {
    [Button::secondary("Cancel"), Button::primary("Delete")]
}

#[test]
fn tab_is_never_a_dialog_key_in_any_kind() {
    let fields = [FieldKind::Text, FieldKind::Choice];
    let buttons = confirm_buttons();
    for kind in [
        DialogKind::Confirm,
        DialogKind::Form,
        DialogKind::List,
        DialogKind::ReadOnly,
    ] {
        let mut state = DialogState::new(&fields, &buttons);
        for code in [KeyCode::Tab, KeyCode::BackTab] {
            assert_eq!(
                state.key(kind, &key(code), &fields, &buttons),
                DialogInput::Ignored,
                "{kind:?}"
            );
            assert_eq!(state.focus, Focus::Field(0), "{kind:?} focus stays");
        }
    }
}

#[test]
fn form_moves_with_up_down_through_fields_then_the_button_row() {
    let fields = [FieldKind::Text, FieldKind::Choice];
    let buttons = confirm_buttons();
    let mut state = DialogState::new(&fields, &buttons);
    let down = |state: &mut DialogState| {
        state.key(DialogKind::Form, &key(KeyCode::Down), &fields, &buttons)
    };
    assert_eq!(down(&mut state), DialogInput::Moved);
    assert_eq!(state.focus, Focus::Field(1));
    assert_eq!(down(&mut state), DialogInput::Moved);
    assert_eq!(
        state.focus,
        Focus::Buttons(1),
        "the row is entered on the primary button"
    );
    assert_eq!(down(&mut state), DialogInput::Ignored, "↓ stops at the row");
    let up = state.key(DialogKind::Form, &key(KeyCode::Up), &fields, &buttons);
    assert_eq!((up, state.focus), (DialogInput::Moved, Focus::Field(1)));
}

#[test]
fn left_right_edit_a_field_or_move_between_enabled_buttons() {
    let fields = [FieldKind::Choice];
    let buttons = [
        Button::secondary("Back"),
        Button::secondary("Skip").enabled(false),
        Button::primary("Save"),
    ];
    let mut state = DialogState::new(&fields, &buttons);
    let right = key(KeyCode::Right);
    assert_eq!(
        state.key(DialogKind::Form, &right, &fields, &buttons),
        DialogInput::Field(0, right)
    );
    state.focus = Focus::Buttons(2);
    let left = key(KeyCode::Left);
    assert_eq!(
        state.key(DialogKind::Form, &left, &fields, &buttons),
        DialogInput::Moved
    );
    assert_eq!(
        state.focus,
        Focus::Buttons(0),
        "the disabled button is skipped"
    );
    assert_eq!(
        state.key(DialogKind::Form, &left, &fields, &buttons),
        DialogInput::Ignored
    );
}

#[test]
fn enter_chooses_the_focused_button_else_the_enabled_primary() {
    let fields = [FieldKind::Text];
    let buttons = confirm_buttons();
    let mut state = DialogState::new(&fields, &buttons);
    let enter = key(KeyCode::Enter);
    assert_eq!(
        state.key(DialogKind::Form, &enter, &fields, &buttons),
        DialogInput::Choose(1)
    );
    state.focus = Focus::Buttons(0);
    assert_eq!(
        state.key(DialogKind::Form, &enter, &fields, &buttons),
        DialogInput::Choose(0)
    );
    let disabled = [
        Button::secondary("Cancel"),
        Button::primary("Save").enabled(false),
    ];
    let mut state = DialogState::new(&fields, &disabled);
    assert_eq!(
        state.key(DialogKind::Form, &enter, &fields, &disabled),
        DialogInput::Ignored
    );
    let mut confirm = DialogState::new(&[], &buttons);
    assert_eq!(
        confirm.focus,
        Focus::Buttons(1),
        "a confirm opens on its primary button"
    );
    assert_eq!(
        confirm.key(DialogKind::Confirm, &enter, &[], &buttons),
        DialogInput::Choose(1)
    );
}

#[test]
fn esc_cancels_everywhere_and_q_cancels_outside_text_fields() {
    let fields = [FieldKind::Text, FieldKind::Choice];
    let buttons = confirm_buttons();
    for kind in [
        DialogKind::Confirm,
        DialogKind::Form,
        DialogKind::List,
        DialogKind::ReadOnly,
    ] {
        let mut state = DialogState::new(&fields, &buttons);
        assert_eq!(
            state.key(kind, &key(KeyCode::Esc), &fields, &buttons),
            DialogInput::Cancel,
            "{kind:?}"
        );
    }
    let mut state = DialogState::new(&fields, &buttons);
    let q = key(KeyCode::Char('q'));
    assert_eq!(
        state.key(DialogKind::Form, &q, &fields, &buttons),
        DialogInput::Field(0, q),
        "q is text in a text field"
    );
    state.focus = Focus::Field(1);
    assert_eq!(
        state.key(DialogKind::Form, &q, &fields, &buttons),
        DialogInput::Cancel
    );
    let mut list = DialogState::new(&[], &[]);
    assert_eq!(
        list.key(DialogKind::List, &q, &[], &[]),
        DialogInput::Cancel
    );
}

#[test]
fn lists_and_read_only_hand_rows_and_scroll_to_their_owner() {
    let mut state = DialogState::new(&[], &[]);
    for code in [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::Char('j'),
    ] {
        for kind in [DialogKind::List, DialogKind::ReadOnly] {
            assert_eq!(
                state.key(kind, &key(code), &[], &[]),
                DialogInput::Content(key(code)),
                "{kind:?} {code:?}"
            );
        }
    }
    let enter = key(KeyCode::Enter);
    assert_eq!(
        state.key(DialogKind::List, &enter, &[], &[]),
        DialogInput::Content(enter)
    );
    assert_eq!(
        state.key(DialogKind::ReadOnly, &enter, &[], &[]),
        DialogInput::Ignored
    );
}

#[test]
fn control_chords_and_releases_are_not_dialog_keys() {
    let buttons = confirm_buttons();
    let mut state = DialogState::new(&[], &buttons);
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(
        state.key(DialogKind::Confirm, &ctrl_c, &[], &buttons),
        DialogInput::Ignored,
        "app::route owns Ctrl-C"
    );
    let release = KeyEvent::new_with_kind(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Release);
    assert_eq!(
        state.key(DialogKind::Confirm, &release, &[], &buttons),
        DialogInput::Ignored
    );
}

#[test]
fn a_press_outside_cancels_inside_presses_choose_or_focus() {
    let buttons = confirm_buttons();
    let fields = [FieldKind::Text, FieldKind::Text];
    let hits = DialogHits {
        outer: Rect::new(10, 5, 40, 10),
        content: Rect::new(12, 6, 36, 6),
        buttons: vec![Some(Rect::new(30, 13, 8, 1)), Some(Rect::new(39, 13, 8, 1))],
        fields: vec![Rect::new(12, 6, 36, 1), Rect::new(12, 8, 36, 1)],
    };
    let mut state = DialogState::new(&fields, &buttons);
    assert_eq!(
        state.mouse(&press(0, 0), &hits, &buttons),
        DialogInput::Cancel
    );
    assert_eq!(
        state.mouse(&press(9, 7), &hits, &buttons),
        DialogInput::Cancel,
        "just left of the border"
    );
    assert_eq!(
        state.mouse(&press(20, 10), &hits, &buttons),
        DialogInput::Ignored,
        "inside, on nothing"
    );
    assert_eq!(
        state.mouse(&press(13, 8), &hits, &buttons),
        DialogInput::Moved
    );
    assert_eq!(state.focus, Focus::Field(1));
    assert_eq!(
        state.mouse(&press(31, 13), &hits, &buttons),
        DialogInput::Choose(0)
    );
    let disabled = [
        Button::secondary("Cancel").enabled(false),
        Button::primary("Delete"),
    ];
    assert_eq!(
        state.mouse(&press(31, 13), &hits, &disabled),
        DialogInput::Ignored
    );
    let moved = MouseEvent {
        kind: MouseEventKind::Moved,
        ..press(0, 0)
    };
    assert_eq!(
        state.mouse(&moved, &hits, &buttons),
        DialogInput::Ignored,
        "only a press closes"
    );
}

#[test]
fn size_follows_the_modal_rule_at_sm_md_and_lg() {
    let dialog = Dialog {
        title: "delete timer?".into(),
        kind: DialogKind::Confirm,
        placement: Placement::Center,
        buttons: confirm_buttons().into(),
    };
    let width = |cells: u16| {
        dialog
            .areas(Rect::new(0, 0, cells, 40), [60, 8], false)
            .outer
            .width
    };
    assert_eq!(
        width(breakpoint::SM.cells),
        breakpoint::SM.cells,
        "full width below md"
    );
    assert_eq!(width(breakpoint::MD.cells), 60, "as wide as its content");
    assert_eq!(width(breakpoint::LG.cells), 60);
    assert_eq!(
        dialog
            .areas(Rect::new(0, 0, 120, 40), [200, 80], false)
            .outer
            .height,
        32,
        "four fifths of the body"
    );
    let areas = dialog.areas(Rect::new(0, 0, 120, 40), [60, 8], false);
    assert_eq!(
        areas.footer.height, 1,
        "buttons reserve the last inside line"
    );
    let read_only = Dialog {
        kind: DialogKind::ReadOnly,
        placement: Placement::Body,
        buttons: Vec::new(),
        ..dialog
    };
    let areas = read_only.areas(Rect::new(0, 2, 120, 30), [10, 10], false);
    assert_eq!(
        (areas.outer, areas.footer.height),
        (Rect::new(0, 2, 120, 30), 0)
    );
}

#[test]
fn paint_frames_the_title_and_right_aligns_buttons_with_the_cursor_focused_at_every_depth() {
    let dialog = Dialog {
        title: "delete timer?".into(),
        kind: DialogKind::Confirm,
        placement: Placement::Center,
        buttons: confirm_buttons().into(),
    };
    for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
        let theme = Theme::new(Base::Tmt);
        let area = Rect::new(0, 0, 120, 30);
        let mut buffer = Buffer::empty(area);
        let areas = dialog.areas(area, [40, 7], false);
        let state = DialogState::new(&[], &dialog.buttons);
        let hits = dialog.paint(areas, &state, &mut buffer, &theme, depth, None);
        assert_eq!(hits.outer, areas.outer);
        let row: String = (areas.footer.x..areas.footer.right())
            .map(|x| buffer[(x, areas.footer.y)].symbol())
            .collect();
        assert!(row.ends_with(" Cancel   Delete "), "{depth:?}: {row:?}");
        let top: String = (areas.outer.x..areas.outer.right())
            .map(|x| buffer[(x, areas.outer.y)].symbol())
            .collect();
        assert!(top.contains(" delete timer? "), "{top:?}");
        let [Some(cancel), Some(delete)] = hits.buttons[..] else {
            panic!("both buttons fit: {:?}", hits.buttons)
        };
        assert_eq!(delete.right(), areas.footer.right());
        let hot = screen::chip(&theme, ChipKind::Secondary, ChipState::Hot, depth);
        let idle = screen::chip(&theme, ChipKind::Secondary, ChipState::Idle, depth);
        assert_eq!(
            buffer[(delete.x, delete.y)].modifier,
            hot.add_modifier,
            "{depth:?} focused"
        );
        assert_eq!(
            buffer[(cancel.x, cancel.y)].modifier,
            idle.add_modifier,
            "{depth:?} idle"
        );
        let hovered = dialog.paint(areas, &state, &mut buffer, &theme, depth, Some(cancel));
        assert_eq!(hovered.buttons, hits.buttons, "hover never moves a button");
        assert_eq!(
            buffer[(cancel.x, cancel.y)].modifier,
            hot.add_modifier,
            "{depth:?} hovered"
        );
    }
}
