use super::super::{ListRow, RowGeometry};
use super::*;
use ratatui::{crossterm::event::KeyEvent, layout::Rect};
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn frame(picker: &mut Picker) -> ListFrame {
    let rows = vec![
        ListRow {
            id: "first".into(),
            disabled: false,
        },
        ListRow {
            id: "second".into(),
            disabled: false,
        },
    ];
    picker.list.reconcile(rows.clone()).unwrap();
    let viewport = Rect::new(0, 0, 20, 5);
    picker.list.scroll.update(viewport, 2, None);
    ListFrame {
        rows,
        viewport,
        offset: 0,
        geometry: vec![
            RowGeometry {
                id: "first".into(),
                lines: 0..1,
                visible: Rect::new(0, 0, 20, 1),
            },
            RowGeometry {
                id: "second".into(),
                lines: 1..2,
                visible: Rect::new(0, 1, 20, 1),
            },
        ],
    }
}
#[test]
fn query_owns_printable_keys_but_arrow_selection_confirm_and_escape_are_neutral() {
    let mut picker = Picker::new(Some(String::new())).unwrap();
    let frame = frame(&mut picker);
    for ch in ['q', '?', 'j', 'k', 'g', 'G'] {
        assert!(matches!(
            picker.input(&key(KeyCode::Char(ch)), &frame),
            Some(PickerInput::Event(PickerEvent::QueryChanged(_)))
        ));
    }
    assert_eq!(picker.query(), Some("q?jkgG"));
    assert_eq!(picker.list.selected(), Some("first"));
    assert_eq!(
        picker.input(&key(KeyCode::Down), &frame),
        Some(PickerInput::Event(PickerEvent::Changed("second".into())))
    );
    assert_eq!(
        picker.input(&key(KeyCode::Enter), &frame),
        Some(PickerInput::Event(PickerEvent::Confirm("second".into())))
    );
    assert_eq!(
        picker.input(&key(KeyCode::Esc), &frame),
        Some(PickerInput::Event(PickerEvent::Cancel))
    );
    assert_eq!(picker.query(), Some("q?jkgG")); // caller owns rollback.
}
#[test]
fn grapheme_cursor_editing_and_bounded_paste_preserve_unicode() {
    let mut picker = Picker::new(Some("a👩‍💻e\u{301}".into())).unwrap();
    let frame = frame(&mut picker);
    picker.input(&key(KeyCode::Left), &frame);
    assert_eq!(picker.query_display().as_deref(), Some("a👩‍💻▏e\u{301}"));
    assert_eq!(
        picker.input(&key(KeyCode::Backspace), &frame),
        Some(PickerInput::Event(PickerEvent::QueryChanged(
            "ae\u{301}".into()
        )))
    );
    picker.input(&key(KeyCode::Delete), &frame);
    assert_eq!(picker.query(), Some("a"));
    picker.input(&Event::Paste("界".into()), &frame);
    assert_eq!(picker.query(), Some("a界"));
    assert_eq!(
        picker.input(&Event::Paste("x".repeat(MAX_QUERY_BYTES)), &frame),
        Some(PickerInput::Captured)
    );
    assert_eq!(picker.query(), Some("a界"));
    assert!(Picker::new(Some("x".repeat(MAX_QUERY_BYTES + 1))).is_err());
}
#[test]
fn selection_only_picker_q_cancels_and_empty_disabled_never_confirm() {
    let mut picker = Picker::new(None).unwrap();
    let frame = frame(&mut picker);
    assert_eq!(
        picker.input(&key(KeyCode::Char('j')), &frame),
        Some(PickerInput::Event(PickerEvent::Changed("second".into())))
    );
    assert_eq!(
        picker.input(&key(KeyCode::Char('q')), &frame),
        Some(PickerInput::Event(PickerEvent::Cancel))
    );
    assert_eq!(picker.input(&Event::Paste("text".into()), &frame), None);
    picker.list.reconcile(vec![]).unwrap();
    assert_eq!(
        picker.input(&key(KeyCode::Enter), &frame),
        Some(PickerInput::Captured)
    );
    let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(picker.input(&ctrl_c, &frame), None); // app::route is the quit owner.
}

#[test]
fn focus_routing_keeps_edit_keys_in_query_and_list_keys_in_list() {
    let mut picker = Picker::new(Some("long query with 界 cells".into())).unwrap();
    let frame = frame(&mut picker);
    assert_eq!(picker.query_visible(1).as_deref(), Some("▏"));
    assert_eq!(picker.query_visible(0).as_deref(), Some(""));
    assert!(picker.query_visible(8).unwrap().ends_with('▏'));
    assert!(picker.query_visible(8).unwrap().width() <= 8);
    assert_eq!(
        picker.input_field(&key(KeyCode::Char('j')), &frame, PickerField::List),
        Some(PickerInput::Event(PickerEvent::Changed("second".into())))
    );
    assert_eq!(
        picker.input_field(&key(KeyCode::Char('q')), &frame, PickerField::List),
        Some(PickerInput::Event(PickerEvent::Cancel))
    );
    assert!(matches!(
        picker.input_field(&key(KeyCode::Char('q')), &frame, PickerField::Query),
        Some(PickerInput::Event(PickerEvent::QueryChanged(_)))
    ));
    let mut stack = crate::app::FocusStack::new(vec![vec!["base".into()]]);
    stack.open(
        vec!["picker".into()],
        vec![vec!["query".into()], vec!["list".into()]],
    );
    let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(
        crate::app::route::<PickerInput>(&mut stack, &ctrl_c, |_, _| panic!(
            "global quit bypasses picker"
        )),
        crate::app::Routed::Quit
    );
    assert_eq!(
        crate::app::route(&mut stack, &key(KeyCode::Esc), |_, event| picker
            .input(event, &frame)),
        crate::app::Routed::Handled(PickerInput::Event(PickerEvent::Cancel))
    );
    stack.close();
    assert_eq!(stack.focused(), Some(&vec!["base".into()]));
}

#[test]
fn query_filter_refresh_reports_new_identity_without_application_effects() {
    let mut picker = Picker::new(Some(String::new())).unwrap();
    let first = vec![ListRow {
        id: "one".into(),
        disabled: false,
    }];
    assert_eq!(
        picker.reconcile(first.clone()).unwrap(),
        Some(PickerEvent::Changed("one".into()))
    );
    assert_eq!(picker.reconcile(first).unwrap(), None);
    assert_eq!(
        picker
            .reconcile(vec![ListRow {
                id: "two".into(),
                disabled: false
            }])
            .unwrap(),
        Some(PickerEvent::Changed("two".into()))
    );
    assert_eq!(
        picker
            .reconcile(vec![ListRow {
                id: "two".into(),
                disabled: true
            }])
            .unwrap(),
        None
    );
    assert_eq!(picker.list.confirm(), None);
}

#[test]
fn cursor_motion_and_rejected_edits_are_handled_before_the_modal_handler() {
    let mut picker = Picker::new(Some("x".repeat(MAX_QUERY_BYTES))).unwrap();
    let frame = frame(&mut picker);
    let query = vec!["query".into()];
    let mut stack = crate::app::FocusStack::new(vec![vec!["base".into()]]);
    stack.open(vec!["picker".into()], vec![query.clone()]);
    for code in [KeyCode::Left, KeyCode::Home, KeyCode::Char('j')] {
        let routed = crate::app::route(&mut stack, &key(code), |id, event| {
            assert_eq!(id, &query, "accepted query input must not reach modal");
            picker.input_field(event, &frame, PickerField::Query)
        });
        assert_eq!(routed, crate::app::Routed::Handled(PickerInput::Captured));
    }
    assert_eq!(picker.list.selected(), Some("first"));
}
