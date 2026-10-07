use super::*;
use crate::board::app::{App, Choice, Effect};
use crate::checklist::{
    Code,
    model::{Action, InventoryExpectation, ItemContent, Mutation},
    test_support::{CHECKLIST, Fixture, ITEM, OTHER, ROOM, WORKER, id},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::json;
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn pick(c: &mut Controller, id: &str) -> Effect {
    if matches!(c.screen, Screen::Details(_)) && c.focus == 0 {
        c.input(&key(KeyCode::Tab), None);
    }
    let rows = c.rows();
    let state = if c.screen == Screen::List {
        &c.list
    } else {
        &c.panel
    };
    state.borrow_mut().reconcile(
        rows.into_iter()
            .map(|r| tmt_tui::components::ListRow {
                id: r.id,
                disabled: r.disabled,
            })
            .collect(),
    );
    state.borrow_mut().picker.list.select(id);
    c.input(&key(KeyCode::Enter), None)
}
fn finish(c: &mut Controller, lane: &mut load::Lane, f: &Fixture, effect: Effect) {
    let Effect::Checklist(task) = effect else {
        panic!("expected worker job: {effect:?}")
    };
    c.loaded(lane.complete(&f.core, task));
}
fn controller(f: &Fixture) -> Controller {
    let mut c = Controller::new(1, Some(id(ROOM)));
    c.preview = Some(f.manager().preview().unwrap());
    c
}
fn render(c: &Controller, width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| c.render(frame, picker_surface::evidence::looks()[0], frame.area()))
        .unwrap();
}
fn seed(f: &Fixture, item: &str, inventory: InventoryExpectation) {
    f.manager()
        .apply(&Request {
            room_id: id(ROOM),
            checklist_id: id(CHECKLIST),
            action: Action::Create {
                item_id: id(item),
                inventory,
                content: ItemContent::new(
                    format!("Item {item}"),
                    Some("Body\nsecond line".into()),
                    Some("https://example.org".into()),
                )
                .unwrap(),
                assignee: None,
            },
        })
        .unwrap();
}
fn mutate(f: &Fixture, item: &str, revision: u64, mutation: Mutation) {
    f.manager()
        .apply(&Request {
            room_id: id(ROOM),
            checklist_id: id(CHECKLIST),
            action: Action::Item {
                item_id: id(item),
                revision,
                mutation,
            },
        })
        .unwrap();
}
#[test]
fn context_menu_is_shared_and_does_not_manufacture_empty_selection() {
    for tmux in [true, false] {
        let mut app = App::new(Some("product".into()));
        let mut snapshot = crate::board::app::tests::snapshot("product", json!([]));
        snapshot.view.as_mut().unwrap().bindings =
            crate::action::preset(tmux, &[crate::config::Pane::Rows]);
        app.apply(snapshot);
        app.context_menu();
        let comma: Vec<_> = app.menu.as_ref().unwrap().entries.clone();
        assert!(comma.iter().any(|e| e.choice == Choice::Checklist));
        assert!(comma.iter().all(|e| match &e.choice {
            Choice::Action(a) => !a.verb.acts_on_member(),
            _ => true,
        }));
        app.menu = None;
        app.perform(&crate::action::Action::parse("menu").unwrap());
        assert_eq!(app.menu.as_ref().unwrap().entries, comma);
        app.choose(Choice::Checklist);
        assert!(app.menu.is_none());
        assert!(app.checklist_shown());
        app.checklist_event(&key(KeyCode::Esc));
        assert!(!app.checklist_shown());
        assert_eq!(app.selected, 0);
    }
}
#[test]
fn full_typed_board_actions_persist_without_dispatch_or_metadata_changes() {
    let f = Fixture::new();
    let before = [
        "pending",
        "agent-state",
        "requests",
        "config.json",
        "squad.toml",
    ]
    .map(|name| (name, std::fs::read(f.root.join(name)).unwrap()));
    let mut c = controller(&f);
    let mut lane = load::Lane::seeded(1, id(ROOM), f.manager());
    c.screen = Screen::Actions;
    let effect = pick(&mut c, "create");
    finish(&mut c, &mut lane, &f, effect);
    assert_eq!(c.screen, Screen::Form);
    let draft = c.draft.as_mut().unwrap();
    draft.title = "Authored from board".into();
    draft.body = "Retained\nliteral $(true)".into();
    draft.reference = "https://example.org/board".into();
    pick(&mut c, "apply");
    assert_eq!(c.panel.borrow().picker.list.selected(), Some("cancel"));
    render(&c, 100, 30);
    assert!(c.readable.get());
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    let item = c.current().unwrap().items[0].item.id.clone();
    assert_eq!(f.literal()["items"][0]["title"], "Authored from board");
    assert!(f.literal()["items"][0]["assignee"].is_null());
    for (action, expected) in [("complete", "complete"), ("reopen", "open")] {
        c.screen = Screen::Details(item.clone());
        pick(&mut c, action);
        render(&c, 100, 30);
        let effect = pick(&mut c, "confirm");
        finish(&mut c, &mut lane, &f, effect);
        assert_eq!(f.literal()["items"][0]["completion"], expected);
    }
    c.screen = Screen::Details(item.clone());
    pick(&mut c, "assign");
    assert_eq!(c.screen, Screen::Members(Some(item.clone())));
    pick(&mut c, WORKER);
    render(&c, 100, 30);
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    assert_eq!(f.literal()["items"][0]["assignee"]["id"], WORKER);
    for action in ["unassign", "archive"] {
        c.screen = Screen::Details(item.clone());
        pick(&mut c, action);
        render(&c, 100, 30);
        let effect = pick(&mut c, "confirm");
        finish(&mut c, &mut lane, &f, effect);
    }
    assert!(f.literal()["items"][0]["archived"].as_bool().unwrap());
    assert!(c.notice.as_deref().unwrap().contains("hidden by filters"));
    c.archived = true;
    c.screen = Screen::Details(item.clone());
    assert!(!c.rows().iter().any(|r| r.id == "edit"));
    pick(&mut c, "restore");
    render(&c, 100, 30);
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    c.screen = Screen::Details(item.clone());
    pick(&mut c, "edit");
    c.draft.as_mut().unwrap().title = "Edited".into();
    pick(&mut c, "apply");
    render(&c, 100, 30);
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    assert_eq!(f.literal()["items"][0]["title"], "Edited");
    c.screen = Screen::Details(item.clone());
    pick(&mut c, "delete");
    render(&c, 80, 8);
    assert!(!c.readable.get());
    assert_eq!(pick(&mut c, "confirm"), Effect::None);
    assert_eq!(f.literal()["items"].as_array().unwrap().len(), 1);
    render(&c, 100, 30);
    assert!(c.readable.get());
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    let literal = f.literal();
    assert!(literal["items"].as_array().unwrap().is_empty());
    assert_eq!(literal["deleted"][0]["itemId"], item.as_str());
    assert!(literal["deleted"][0].get("title").is_none());
    for (name, bytes) in before {
        assert_eq!(std::fs::read(f.root.join(name)).unwrap(), bytes, "{name}");
    }
    let m = f.model();
    assert_eq!(m["pending"], "waiting");
    assert_eq!(m["agentState"], "busy");
    assert_eq!(m["requests"], json!(["unchanged request"]));
}
#[test]
fn conflict_preserves_fields_and_requires_explicit_refresh_review() {
    let f = Fixture::new();
    seed(&f, ITEM, InventoryExpectation::Absent);
    let mut c = controller(&f);
    let mut lane = load::Lane::seeded(1, id(ROOM), f.manager());
    c.screen = Screen::Details(id(ITEM));
    pick(&mut c, "edit");
    c.draft.as_mut().unwrap().title = "Do not overwrite".into();
    pick(&mut c, "apply");
    let original = c.submit.clone();
    mutate(&f, ITEM, 1, Mutation::Complete);
    render(&c, 100, 30);
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    assert_eq!(c.failure.as_ref().unwrap().code, Code::Conflict);
    assert_eq!(c.draft.as_ref().unwrap().title, "Do not overwrite");
    assert_eq!(c.submit, original);
    let effect = pick(&mut c, "refresh");
    finish(&mut c, &mut lane, &f, effect);
    render(&c, 100, 30);
    assert_eq!(pick(&mut c, "confirm"), Effect::None);
    pick(&mut c, "review");
    assert_ne!(c.submit, original);
    render(&c, 100, 30);
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    assert_eq!(f.literal()["items"][0]["title"], "Do not overwrite");
    assert_eq!(f.literal()["items"][0]["completion"], "complete");
}
#[test]
fn original_unknown_survives_reads_and_later_acknowledgements_with_late_fence() {
    let f = Fixture::new();
    seed(&f, ITEM, InventoryExpectation::Absent);
    let mut c = controller(&f);
    let mut lane = load::Lane::seeded(1, id(ROOM), f.manager());
    c.screen = Screen::Details(id(ITEM));
    pick(&mut c, "complete");
    render(&c, 100, 30);
    let Effect::Checklist(task) = pick(&mut c, "confirm") else {
        panic!()
    };
    let original = c.target_text();
    let mut late = task.key.clone();
    late.serial += 1;
    c.loaded(load::Completed {
        key: late,
        outcome: load::Outcome::Apply(Err(Error {
            code: Code::Forbidden,
            message: "late".into(),
            current: None,
        })),
    });
    assert_eq!(c.pending.as_ref(), Some(&task.key));
    c.loaded(load::Completed {
        key: task.key,
        outcome: load::Outcome::Apply(Err(Error {
            code: Code::OutcomeUnknown,
            message: "publication uncertain".into(),
            current: None,
        })),
    });
    assert_eq!(c.unknown, vec![original]);
    let effect = pick(&mut c, "refresh");
    finish(&mut c, &mut lane, &f, effect);
    assert_eq!(c.unknown.len(), 1);
    pick(&mut c, "review");
    render(&c, 100, 30);
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    assert_eq!(c.unknown.len(), 1);
    assert!(c.notice.as_deref().unwrap().contains("remains unknown"));
}
#[test]
fn filters_assignment_and_full_archived_reorder_keep_independent_identity() {
    let f = Fixture::new();
    seed(&f, ITEM, InventoryExpectation::Absent);
    seed(&f, OTHER, InventoryExpectation::Revision(1));
    mutate(&f, OTHER, 1, Mutation::Archive);
    let mut c = controller(&f);
    let mut lane = load::Lane::seeded(1, id(ROOM), f.manager());
    c.screen = Screen::Filters;
    pick(&mut c, "unassigned");
    assert_eq!(c.assignment, Assignment::Unassigned);
    assert_eq!(c.rows().iter().filter(|r| !r.disabled).count(), 1);
    c.screen = Screen::Actions;
    pick(&mut c, "reorder");
    let Some(Request {
        action: Action::Reorder { order, .. },
        ..
    }) = &c.submit
    else {
        panic!()
    };
    assert_eq!(order, &vec![id(ITEM), id(OTHER)]);
    pick(&mut c, OTHER);
    pick(&mut c, "up");
    pick(&mut c, "apply");
    render(&c, 160, 30);
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    assert_eq!(f.literal()["items"][0]["id"], OTHER);
    assert_eq!(c.assignment, Assignment::Unassigned);
    assert!(!c.archived);
}
#[test]
fn viewport_cues_and_stale_hits_are_bounded_in_all_looks() {
    let f = Fixture::new();
    seed(&f, ITEM, InventoryExpectation::Absent);
    let c = controller(&f);
    for (variant, look) in picker_surface::evidence::looks().into_iter().enumerate() {
        for (width, height) in [(80, 30), (100, 30), (160, 30), (180, 30), (80, 8)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| c.render(frame, look, frame.area()))
                .unwrap();
            picker_surface::evidence::capture(
                &format!("checklist-{width}x{height}-{variant}"),
                terminal.backend().buffer(),
                &c.list.borrow(),
            );
            let map = c.list.borrow();
            let map = map.frame.as_ref().unwrap();
            assert!(map.areas.outer.width <= width);
            assert!(map.areas.outer.height <= height);
        }
    }
    c.invalidate();
    assert!(c.controls.get().iter().all(|r| r.width == 0));
    assert!(c.list.borrow().frame.is_none());
}

#[test]
fn manager_loss_refuses_exact_delete_and_retains_target_and_bytes() {
    let f = Fixture::new();
    seed(&f, ITEM, InventoryExpectation::Absent);
    let mut c = controller(&f);
    let mut lane = load::Lane::seeded(1, id(ROOM), f.manager());
    c.screen = Screen::Details(id(ITEM));
    pick(&mut c, "delete");
    let target = c.submit.clone();
    let before = f.bytes();
    f.change(|m| m["lead"] = json!(WORKER));
    render(&c, 100, 30);
    let effect = pick(&mut c, "confirm");
    finish(&mut c, &mut lane, &f, effect);
    assert_eq!(c.failure.as_ref().unwrap().code, Code::Forbidden);
    assert!(c.failure.as_ref().unwrap().current.is_none());
    assert_eq!(c.submit, target);
    assert_eq!(f.bytes(), before);
    let effect = pick(&mut c, "refresh");
    finish(&mut c, &mut lane, &f, effect);
    assert!(!c.manager());
    render(&c, 100, 30);
    assert_eq!(pick(&mut c, "confirm"), Effect::None);
    assert_eq!(f.bytes(), before);
}

#[test]
fn long_details_have_keyboard_reading_focus_before_actions() {
    let f = Fixture::new();
    seed(&f, ITEM, InventoryExpectation::Absent);
    let mut c = controller(&f);
    c.preview.as_mut().unwrap().current.items[0]
        .item
        .content
        .body = Some(
        (0..80)
            .map(|i| format!("Body line {i:02}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    c.screen = Screen::Details(id(ITEM));
    render(&c, 80, 12);
    assert_eq!(c.reading.borrow().picker.list.scroll.offset(), 0);
    c.input(&key(KeyCode::PageDown), None);
    assert!(c.reading.borrow().picker.list.scroll.offset() > 0);
    c.input(&key(KeyCode::End), None);
    render(&c, 80, 12);
    let end = c.reading.borrow().picker.list.scroll.offset();
    assert!(end > 50);
    c.input(&key(KeyCode::Home), None);
    assert_eq!(c.reading.borrow().picker.list.scroll.offset(), 0);
    c.input(&key(KeyCode::Tab), None);
    assert_eq!(c.focus, 1);
    render(&c, 80, 12);
    assert_eq!(c.panel.borrow().picker.list.selected(), Some("reference"));
    c.input(&key(KeyCode::Down), None);
    assert_eq!(c.panel.borrow().picker.list.selected(), Some("edit"));
    c.input(&key(KeyCode::Enter), None);
    assert_eq!(c.screen, Screen::Form);
}
