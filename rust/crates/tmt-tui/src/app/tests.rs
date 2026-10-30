use super::*;
use ratatui::crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};

fn id(name: &str) -> ComponentId {
    vec![name.into()]
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn modal_captures_unknown_keys_and_outside_mouse_without_calling_base() {
    let mut stack = FocusStack::new(vec![id("base")]);
    stack.open(id("help"));
    let events = [
        key(KeyCode::Left),
        key(KeyCode::Enter),
        key(KeyCode::Char('/')),
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 300,
            row: 200,
            modifiers: KeyModifiers::NONE,
        }),
    ];
    for event in events {
        let mut called = Vec::new();
        assert_eq!(
            route::<()>(&mut stack, &event, |target, _| {
                called.push(target.clone());
                None
            }),
            Routed::Captured
        );
        assert_eq!(called, vec![id("help")]);
    }
}

#[test]
fn close_and_replace_restore_original_base_without_replaying_close() {
    let mut stack = FocusStack::new(vec![id("first"), id("second")]);
    assert!(stack.focus_base(&id("second")));
    stack.open(id("help"));
    stack.open(id("settings"));
    let mut called = Vec::new();
    assert_eq!(
        route(&mut stack, &key(KeyCode::Esc), |target, _| {
            called.push(target.clone());
            Some("close")
        }),
        Routed::Handled("close")
    );
    assert_eq!(called, vec![id("settings")]);
    assert_eq!(stack.close(), Some(id("settings")));
    assert_eq!(stack.focused(), Some(&id("second")));
    assert_eq!(
        route::<()>(&mut stack, &key(KeyCode::Enter), |_, _| None),
        Routed::Unhandled
    );
}

#[test]
fn the_modal_gets_every_key_and_tab_moves_only_between_base_panes() {
    let mut stack = FocusStack::new(vec![id("left"), id("right")]);
    stack.open(id("picker"));
    for ch in ['q', '?', 'j', 'k', 'g', 'G'] {
        assert_eq!(
            route(&mut stack, &key(KeyCode::Char(ch)), |target, _| (target
                == &id("picker"))
                .then_some("text")),
            Routed::Handled("text")
        );
    }
    for tab in [KeyCode::Tab, KeyCode::BackTab] {
        assert_eq!(
            route::<()>(&mut stack, &key(tab), |_, _| None),
            Routed::Captured,
            "an unhandled Tab stays in the modal"
        );
    }
    assert_eq!(stack.focused(), Some(&id("picker")));
    assert_eq!(
        stack.base_focus(),
        Some(&id("left")),
        "Tab never moved the base under it"
    );
    stack.close();
    assert_eq!(
        route::<()>(&mut stack, &key(KeyCode::Tab), |_, _| None),
        Routed::Captured
    );
    assert_eq!(
        stack.focused(),
        Some(&id("right")),
        "Tab moves between base panes"
    );
}

#[test]
fn ctrl_c_always_quits_but_release_does_not_trigger_actions() {
    let mut stack = FocusStack::new(vec![id("base")]);
    stack.open(id("help"));
    let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(
        route::<()>(&mut stack, &ctrl_c, |_, _| panic!(
            "quit must bypass handlers"
        )),
        Routed::Quit
    );
    let release = Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
    ));
    assert_eq!(
        route::<()>(&mut stack, &release, |_, _| panic!(
            "release must not dispatch"
        )),
        Routed::Captured
    );
}

#[test]
fn refresh_keeps_base_identity_or_nearest_surviving_position_under_modal() {
    let mut stack = FocusStack::new(vec![id("a"), id("b"), id("c")]);
    stack.focus_base(&id("b"));
    stack.open(id("help"));
    stack.reconcile_base(vec![id("c"), id("a"), id("b")]);
    assert_eq!(stack.base_focus(), Some(&id("b")));
    stack.reconcile_base(vec![id("c"), id("a")]);
    stack.close();
    assert_eq!(stack.focused(), Some(&id("a")));
    stack.reconcile_base(vec![]);
    assert_eq!(stack.focused(), None);
}
