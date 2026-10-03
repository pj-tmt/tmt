use super::*;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
fn rows(ids: &[&str]) -> Vec<ListRow> {
    ids.iter()
        .map(|id| ListRow {
            id: (*id).into(),
            disabled: false,
        })
        .collect()
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn frame(state: &mut ListState) -> ListFrame {
    let viewport = Rect::new(3, 4, 20, 4);
    let geometry = state
        .rows()
        .iter()
        .enumerate()
        .map(|(index, row)| RowGeometry {
            id: row.id.clone(),
            lines: index * 3..index * 3 + 3,
            visible: if index == 0 {
                viewport
            } else {
                Rect::default()
            },
        })
        .collect();
    state.scroll.update(viewport, state.rows().len() * 3, None);
    ListFrame {
        rows: state.rows().to_vec(),
        geometry,
        viewport,
        offset: 0,
    }
}
#[test]
fn identity_reorder_and_removal_choose_nearest_old_survivor() {
    let mut state = ListState::default();
    state.reconcile(rows(&["a", "b", "c", "d"])).unwrap();
    state.select("c");
    state.reconcile(rows(&["d", "a", "c", "b"])).unwrap();
    assert_eq!(state.selected(), Some("c"));
    state.reconcile(rows(&["b", "new", "a"])).unwrap();
    assert_eq!(state.selected(), Some("a")); // nearest old neighbor, not new index.
    let previous = state.rows().to_vec();
    assert!(state.reconcile(rows(&["duplicate", "duplicate"])).is_err());
    assert_eq!(state.rows(), previous);
    assert!(state.reconcile(rows(&["42"])).is_err());
}
#[test]
fn disabled_empty_and_release_never_activate() {
    let mut state = ListState::default();
    let mut model = rows(&["a", "disabled", "c"]);
    model[1].disabled = true;
    state.reconcile(model).unwrap();
    let frame = frame(&mut state);
    assert_eq!(
        state.input(&key(KeyCode::Down), &frame),
        Some(ListEvent::Changed("c".into()))
    );
    assert_eq!(state.select("disabled"), None);
    assert_eq!(
        state.input(&key(KeyCode::Enter), &frame),
        Some(ListEvent::Confirm("c".into()))
    );
    let release = Event::Key(KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ));
    assert_eq!(state.input(&release, &frame), None);
    state.reconcile(vec![]).unwrap();
    let empty = self::frame(&mut state);
    assert_eq!(state.confirm(), None);
    assert_eq!(state.input(&key(KeyCode::Down), &empty), None);
    assert_eq!(state.input(&key(KeyCode::Enter), &frame), None); // stale geometry.
}
#[test]
fn pages_use_wrapped_visual_ranges_and_mouse_uses_visible_rows_only() {
    let mut state = ListState::default();
    state.reconcile(rows(&["a", "b", "c", "d"])).unwrap();
    let frame = frame(&mut state);
    assert_eq!(
        state.input(&key(KeyCode::PageDown), &frame),
        Some(ListEvent::Changed("b".into()))
    );
    assert_eq!(state.scroll.offset(), 2); // all three wrapped lines revealed.
    state.select("c");
    let mouse = |x, y| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        })
    };
    assert_eq!(state.input(&mouse(2, 4), &frame), None);
    assert_eq!(state.input(&mouse(3, 8), &frame), None);
    assert_eq!(state.input(&mouse(3, 4), &frame), None); // old paint offset.
    state.scroll.step(Step::Top);
    assert_eq!(
        state.input(&mouse(3, 4), &frame),
        Some(ListEvent::Changed("a".into()))
    );
    assert_eq!(
        state.input(&key(KeyCode::End), &frame),
        Some(ListEvent::Changed("d".into()))
    );
    assert_eq!(state.scroll.offset(), 8);
}
