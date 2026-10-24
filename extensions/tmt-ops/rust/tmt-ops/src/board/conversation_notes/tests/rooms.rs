//! Room lists share count acquisition with conversations, not their draft/write owner.
use super::*;
use crate::board::{app::App, workspace::OpsTarget};

pub(in crate::board) const ROOM_SCENES: &[&str] = &[
    "room-todo-known",
    "room-todo-empty",
    "room-todo-pending",
    "room-todo-partial",
    "room-todo-unavailable",
    "room-todo-done",
    "room-todo-unread",
    "room-checklist-list",
];

pub(in crate::board) fn capture_case(scene: &str, look: crate::look::Look) -> App {
    let (mut app, room) = board_with_room_count();
    app.local_zone = Some(jiff::tz::TimeZone::get("Asia/Tokyo").unwrap());
    let view = app.view.as_mut().unwrap();
    view.look = look;
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    let now = Instant::now();
    let mut current = room_current(10);
    current.room.id = Id::parse(&room).unwrap();
    match scene {
        "room-todo-empty" => {
            current.items.clear();
            app.notes.observe_room(&current, now);
        }
        "room-todo-done" => {
            current.items[0].item.completion = Completion::Complete;
            current.items[0].item.revision += 1;
            current.inventory_revision = Some(11);
            app.notes.observe_room(&current, now);
        }
        "room-todo-pending" | "room-todo-partial" | "room-todo-unavailable" => {
            let keys = app
                .workspace_snapshot()
                .unwrap()
                .rows
                .keys()
                .filter_map(Key::list)
                .collect::<Vec<_>>();
            app.notes = State::reading();
            app.notes.reconcile(keys.clone().into_iter());
            if scene != "room-todo-pending" {
                for _ in 0..keys.len() {
                    let task = app.notes.offer_background(None, now).unwrap();
                    let result = if scene == "room-todo-unavailable"
                        || scene == "room-todo-partial"
                            && task.key.member().is_none()
                            && task.key.room != room
                    {
                        Err("Room checklist unavailable.".into())
                    } else if task.key == Key::room(&room) {
                        Ok(Counts::of(&current.items))
                    } else {
                        Ok(Counts::default())
                    };
                    app.notes.finished(
                        Completed {
                            key: task.key,
                            epoch: task.epoch,
                            outcome: Outcome::Counts(result),
                        },
                        now,
                    );
                }
            }
        }
        "room-todo-unread" => admit_unread(&mut app, &room),
        "room-checklist-list" => {
            use crate::board::{app::Effect, checklist::load};
            let Effect::Checklist(task) = app.open_checklist() else {
                panic!("exact room list")
            };
            let mut admitted = preview(Vec::new());
            current.room.name = Some("squad-01".into());
            admitted.actor_id =
                Id::parse(app.view.as_ref().unwrap().me_id.as_deref().unwrap()).unwrap();
            admitted.current = current;
            app.finished_checklist(load::Completed {
                key: task.key,
                outcome: load::Outcome::Read(Ok(admitted)),
            });
        }
        "room-todo-known" => {}
        _ => panic!("unknown room count scene"),
    }
    app
}

fn admit_unread(app: &mut App, room: &str) {
    let actor = app.view.as_ref().unwrap().me_id.clone().unwrap();
    let member = app
        .workspace_snapshot()
        .unwrap()
        .roster
        .squads
        .iter()
        .find(|s| s.room_id == room)
        .unwrap()
        .members[0]
        .clone();
    app.home_leads.test_results(&actor, serde_json::json!({"items":[{
        "requestId":"room-count-unread", "kind":"request", "sender":{"identityId":actor}, "recipientId":member,
        "final":{"status":"retained","submittedAtMs":100}, "responsePreview":"Exact answer", "recipientAcknowledged":true
    }],"nextBefore":null}));
    app.workspace_reconcile();
}

fn room_current(count: u8) -> crate::checklist::Current {
    let mut current = preview(
        (1..=count)
            .map(|n| view(n, Some("https://example.com/item"), Completion::Open, false))
            .collect(),
    )
    .current;
    current.member = None;
    current.checklist_id = Some(Id::parse("99999999-9999-4999-8999-999999999999").unwrap());
    current.inventory_revision = Some(u64::from(count.max(1)));
    current
}

#[test]
fn ten_linked_room_items_and_an_empty_conversation_count_once_in_the_header() {
    let room = Key::room(ROOM);
    let conversation = key();
    let targets = [
        OpsTarget::Squad(ROOM.into()),
        OpsTarget::Squad(ROOM.into()),
        OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        },
    ];
    let mut state = State::reading();
    state.observe_roster(targets.iter());
    let now = Instant::now();
    let first = state.offer_background(None, now).unwrap();
    assert_eq!(first.key, room);
    assert!(state.offer(Some(&conversation), now).is_none());
    let counts = Counts::of(&room_current(10).items);
    assert_eq!(
        counts,
        Counts {
            links: 10,
            todo: 10,
            done: 0
        }
    );
    state.finished(
        Completed {
            key: first.key,
            epoch: first.epoch,
            outcome: Outcome::Counts(Ok(counts)),
        },
        now,
    );
    assert_eq!(state.total_label(), "10 todo · partial");
    let next = state.offer_background(None, now).unwrap();
    assert_eq!(next.key, conversation);
    state.finished(
        Completed {
            key: next.key,
            epoch: next.epoch,
            outcome: Outcome::Counts(Ok(Counts::default())),
        },
        now,
    );
    assert_eq!(state.total_label(), "10 todo");
    assert_eq!(state.row_label(&targets[0]).as_deref(), Some("○ 10 todo"));
    assert_eq!(state.room_label(ROOM), "Checklist · 10 todo");
    assert!(state.preview(&room).is_none(), "room counts retain no body");
    let selected = state.offer(Some(&conversation), now).unwrap();
    state.finished(
        reading(
            &selected,
            Ok(Counts {
                links: 1,
                todo: 2,
                done: 0,
            }),
        ),
        now,
    );
    assert_eq!(
        state.total_label(),
        "12 todo",
        "room and conversation are independent lists"
    );
    assert!(
        state
            .write(
                &room,
                Write::Toggle {
                    item: view(1, None, Completion::Open, false).item.id,
                    revision: 1,
                    complete: true
                }
            )
            .is_none()
    );
}

#[test]
fn a_selected_room_refreshes_counts_without_a_preview_and_late_reads_cannot_undo_a_modal_change() {
    let room = Key::room(ROOM);
    let mut state = State::reading();
    state.reconcile([room.clone()].into_iter());
    let now = Instant::now();
    let task = state.offer(Some(&room), now).unwrap();
    assert_eq!(task.work, Work::Counts);
    state.finished(
        Completed {
            key: task.key,
            epoch: task.epoch,
            outcome: Outcome::Counts(Ok(Counts::default())),
        },
        now,
    );
    assert!(
        state
            .offer(Some(&room), now + FRESH - Duration::from_millis(1))
            .is_none()
    );
    let late = state.offer(Some(&room), now + FRESH).unwrap();
    state.observe_room(&room_current(10), now + FRESH);
    state.finished(
        Completed {
            key: late.key,
            epoch: late.epoch,
            outcome: Outcome::Counts(Ok(Counts::default())),
        },
        now + FRESH,
    );
    assert_eq!(state.total_label(), "10 todo");
    assert!(state.read.is_none());
    state.reconcile(std::iter::empty());
    state.observe_room(&room_current(10), now + FRESH);
    assert_eq!(
        state.total_label(),
        "0 todo",
        "removed rooms never reenter the total through a modal"
    );
}

#[test]
fn room_counts_use_the_admitted_room_service_and_do_not_create_a_conversation_list() {
    use crate::checklist::{
        Service,
        model::{Action, InventoryExpectation, Request},
        test_support::{CHECKLIST, Fixture, ROOM as SERVICE_ROOM, WORKER},
    };
    let fixture = Fixture::new();
    let service = fixture.service();
    for ordinal in 0..10 {
        service
            .apply(&Request {
                room_id: Id::parse(SERVICE_ROOM).unwrap(),
                checklist_id: Id::parse(CHECKLIST).unwrap(),
                action: Action::Create {
                    item_id: Id::parse(&format!("88888888-8888-4888-8888-8888888888{ordinal:02}"))
                        .unwrap(),
                    inventory: if ordinal == 0 {
                        InventoryExpectation::Absent
                    } else {
                        InventoryExpectation::Revision(ordinal)
                    },
                    content: ItemContent::new(
                        format!("Room item {ordinal}"),
                        None,
                        Some("https://example.com/item".into()),
                    )
                    .unwrap(),
                    assignee: None,
                },
            })
            .unwrap();
    }
    let task = Task {
        key: Key::room(SERVICE_ROOM),
        epoch: 0,
        work: Work::Counts,
    };
    let completed = work::complete(&fixture.core, task);
    assert!(matches!(
        completed.outcome,
        Outcome::Counts(Ok(Counts {
            links: 10,
            todo: 10,
            done: 0
        }))
    ));
    let conversation = Service::for_conversation(
        &fixture.core,
        &fixture.config,
        Id::parse(SERVICE_ROOM).unwrap(),
        Id::parse(WORKER).unwrap(),
    )
    .unwrap();
    assert!(conversation.preview().unwrap().current.items.is_empty());
    assert_eq!(fixture.keys(), [SERVICE_ROOM]);
}

fn render(app: &App) -> String {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    crate::status::with_now_ms(1_791_600_000_000, || {
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
    });
    (0..40)
        .map(|y| {
            (0..160)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn board_with_room_count() -> (App, String) {
    let mut app = crate::board::app::workspace::tests::populated_board(2, 2);
    admit_empty(&mut app);
    let index = app
        .workspace_rows()
        .iter()
        .rposition(|row| matches!(row.target, OpsTarget::Squad(_)))
        .unwrap();
    let OpsTarget::Squad(room) = app.workspace_rows()[index].target.clone() else {
        unreachable!()
    };
    let mut current = room_current(10);
    current.room.id = Id::parse(&room).unwrap();
    app.notes.observe_room(&current, Instant::now());
    app.select(index);
    (app, room)
}

fn click(area: ratatui::layout::Rect) -> ratatui::crossterm::event::MouseEvent {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn the_squad_row_and_visible_detail_entry_open_the_exact_room_list() {
    use crate::board::app::{Choice, Effect};
    let (mut app, room) = board_with_room_count();
    let text = render(&app);
    assert!(
        text.lines()
            .any(|line| line.contains("squad-01") && line.contains("○ 10 todo")),
        "{text}"
    );
    assert!(text.contains("Checklist · 10 todo"), "{text}");
    let member = app
        .workspace_rows()
        .iter()
        .position(
            |row| matches!(&row.target, OpsTarget::Member { room_id: Some(id), .. } if id != &room),
        )
        .unwrap();
    app.select(member);
    render(&app);
    let (area, _) = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find(|(_, choice)| matches!(choice, Choice::RoomChecklist(_)))
        .cloned()
        .unwrap();
    assert!(area.width > 0);
    let Effect::Checklist(task) = app.mouse(click(area), Instant::now()) else {
        panic!("the count must open Checklist")
    };
    assert_eq!(task.key.room, Some(Id::parse(&room).unwrap()));
    assert_eq!(app.workspace.selected, Some(OpsTarget::Squad(room)));
}

#[test]
fn loading_overlays_and_stale_count_hits_never_open_a_room() {
    use crate::board::app::Choice;
    for guard in ["loading", "overlay", "stale", "removed"] {
        let (mut app, room) = board_with_room_count();
        render(&app);
        let (area, choice) = app
            .workspace_strip_hits
            .borrow()
            .iter()
            .find(|(_, c)| matches!(c, Choice::RoomChecklist(_)))
            .cloned()
            .unwrap();
        let before = app.workspace.selected.clone();
        match guard {
            "loading" => app.current = Some("another view".into()),
            "overlay" => app.help = true,
            "stale" => app.invalidate_frame_geometry(),
            "removed" => {
                app.view
                    .as_mut()
                    .unwrap()
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .roster
                    .squads
                    .retain(|s| s.room_id != room);
                app.workspace_reconcile();
                app.choose(choice);
                assert!(
                    app.checklist.is_none(),
                    "removed UUID cannot redirect an action"
                );
                continue;
            }
            _ => unreachable!(),
        }
        app.mouse(click(area), Instant::now());
        assert!(app.checklist.is_none(), "{guard}");
        assert_eq!(app.workspace.selected, before, "{guard}");
    }
}

#[test]
fn the_squad_todo_suffix_yields_to_the_complete_unread_state() {
    let (mut app, room) = board_with_room_count();
    admit_unread(&mut app, &room);
    let text = render(&app);
    let row = text
        .lines()
        .find(|line| line.contains("squad-01") && line.contains("1 unread"))
        .expect(&text);
    assert!(
        !row.contains("○ 10 todo"),
        "the suffix yields before truncating unread: {row}"
    );
    assert!(
        text.contains("Checklist · 10 todo"),
        "the count remains discoverable: {text}"
    );
}

#[test]
fn an_unavailable_room_is_not_zero_and_its_first_retry_waits() {
    let key = Key::room(ROOM);
    let mut state = State::reading();
    state.reconcile([key.clone()].into_iter());
    let now = Instant::now();
    let task = state.offer(Some(&key), now).unwrap();
    state.finished(
        Completed {
            key: task.key,
            epoch: task.epoch,
            outcome: Outcome::Counts(Err("unavailable".into())),
        },
        now,
    );
    assert_eq!(state.total_label(), "— todo");
    assert_eq!(state.room_label(ROOM), "Checklist");
    assert!(
        state
            .offer(Some(&key), now + RETRY - Duration::from_millis(1))
            .is_none()
    );
    let retry = state.offer(Some(&key), now + RETRY).unwrap();
    state.finished(
        Completed {
            key: retry.key,
            epoch: retry.epoch,
            outcome: Outcome::Counts(Ok(Counts::default())),
        },
        now + RETRY,
    );
    assert_eq!(state.total_label(), "0 todo");
    assert_eq!(state.room_label(ROOM), "Checklist · 0 todo");
}
