use super::super::tests::{key, preview};
use super::*;

fn event(id: u64, at_ms: u64) -> Logged {
    Logged {
        id,
        at_ms,
        room_id: key().room,
        member_id: key().member().map(str::to_owned),
        checklist_id: "internal list".into(),
        item_id: None,
        actor_id: "agent".into(),
        action: "add".into(),
        revision: id,
    }
}
fn page(ids: &[u64], more: bool) -> Page {
    Page {
        events: ids.iter().map(|id| event(*id, 10000 - id)).collect(),
        next_cursor: ids.last().copied().unwrap_or(0),
        more,
        gap: None,
    }
}
fn read(pages: Result<Vec<Page>, String>) -> Read {
    Read {
        observed: true,
        user: Some("user".into()),
        pages,
    }
}

#[test]
fn two_pages_are_bounded_and_more_survives_for_the_next_refresh() {
    let mut calls = Vec::new();
    let pages = pages(0, |after| {
        calls.push(after);
        Ok(page(&[after + 1], true))
    })
    .unwrap();
    assert_eq!(calls, [0, 1]);
    let mut held = Held::default();
    held.adopt(read(Ok(pages)));
    assert_eq!(held.cursor(), 2);
    assert!(held.more);
    held.adopt(read(Ok(vec![page(&[2, 3], false)])));
    assert_eq!(
        held.events.iter().map(|event| event.id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        held.events
            .iter()
            .map(|event| event.at_ms)
            .collect::<Vec<_>>(),
        [9999, 9998, 9997]
    );
    assert!(!held.more);
}

#[test]
fn a_failed_second_page_and_nonprogress_leave_the_held_cursor_and_events() {
    let mut held = Held::default();
    held.adopt(read(Ok(vec![page(&[1], false)])));
    let mut calls = 0;
    let failed = pages(1, |_| {
        calls += 1;
        if calls == 1 {
            Ok(page(&[2], true))
        } else {
            Err("failed".into())
        }
    });
    held.adopt(read(failed));
    assert_eq!(held.cursor(), 1);
    assert_eq!(held.events.len(), 1);
    assert!(held.failed);
    assert!(pages(1, |_| Ok(page(&[1], true))).is_err());
}

#[test]
fn retained_window_gap_and_current_titles_are_explicit_and_unchanged_facts_reuse_snapshot() {
    let mut held = Held::default();
    held.adopt(read(Ok(vec![page(&(1..=1001).collect::<Vec<_>>(), false)])));
    held.present(Some(&preview(vec![])));
    assert_eq!(held.events.len(), 1000);
    assert_eq!(held.events[0].id, 2);
    assert_eq!(held.gap, Some(Gap::Pruned));
    let first = held.snapshot.clone().unwrap();
    held.present(Some(&preview(vec![])));
    assert!(Arc::ptr_eq(&first, held.snapshot.as_ref().unwrap()));
}

#[test]
fn stale_read_cannot_advance_events_and_a_preview_failure_keeps_independent_events() {
    use super::super::{Completed, Outcome, State};
    use std::time::Instant;
    let now = Instant::now();
    let mut state = State::reading();
    let key = key();
    let task = state.offer(Some(&key), now).unwrap();
    state.lists.get_mut(&key).unwrap().epoch += 1;
    state.finished(
        Completed {
            key: key.clone(),
            epoch: task.epoch,
            outcome: Outcome::Read {
                preview: Ok(preview(vec![])),
                activity: read(Ok(vec![page(&[1], false)])),
            },
        },
        now,
    );
    assert_eq!(state.lists[&key].activity.cursor(), 0);
    let entry = state.lists.get_mut(&key).unwrap();
    entry.preview_at = None;
    let task = state.offer(Some(&key), now).unwrap();
    state.finished(
        Completed {
            key: key.clone(),
            epoch: task.epoch,
            outcome: Outcome::Read {
                preview: Err("preview failed".into()),
                activity: read(Ok(vec![page(&[2], false)])),
            },
        },
        now,
    );
    assert_eq!(state.lists[&key].activity.cursor(), 2);
    assert_eq!(state.activity(&key).unwrap().events[0].id, 2);
    assert!(state.preview(&key).is_none());
}

#[test]
fn real_service_changes_between_reads_are_all_retained_without_snapshot_inference() {
    use crate::checklist::{
        model::{
            Action, Confirmation, Edit, Id, InventoryExpectation, ItemContent, Mutation, Request,
        },
        test_support::{Fixture, ITEM, OTHER, ROOM, WORKER, id},
    };
    let f = Fixture::new();
    drop(f.manager()); // A room lead can exercise every admitted action.
    let service = f.conversation(WORKER);
    let checklist = service.conversation_checklist_id().unwrap();
    let request = |action| Request {
        room_id: id(ROOM),
        checklist_id: checklist.clone(),
        action,
    };
    let create = |item, inventory| {
        request(Action::Create {
            item_id: id(item),
            inventory,
            content: ItemContent::new("Current title".into(), None, None).unwrap(),
            assignee: None,
        })
    };
    let mutate = |revision, mutation| {
        request(Action::Item {
            item_id: id(ITEM),
            revision,
            mutation,
        })
    };
    let mut held = Held::default();
    held.adopt(super::read(&f.core, Some(&service), 0));
    assert_eq!(held.cursor(), 0);
    service
        .apply(&create(ITEM, InventoryExpectation::Absent))
        .unwrap();
    service.apply(&mutate(1, Mutation::Complete)).unwrap();
    service.apply(&mutate(2, Mutation::Complete)).unwrap(); // no-op
    service.apply(&mutate(2, Mutation::Reopen)).unwrap();
    service
        .apply(&mutate(
            3,
            Mutation::Edit(Edit {
                title: Some("Renamed".into()),
                ..Default::default()
            }),
        ))
        .unwrap();
    service
        .apply(&mutate(4, Mutation::Assign(id(WORKER))))
        .unwrap();
    service.apply(&mutate(5, Mutation::Unassign)).unwrap();
    service.apply(&mutate(6, Mutation::Archive)).unwrap();
    service.apply(&mutate(7, Mutation::Restore)).unwrap();
    service
        .apply(&create(OTHER, InventoryExpectation::Revision(1)))
        .unwrap();
    service
        .apply(&request(Action::Reorder {
            inventory_revision: 2,
            order: vec![id(OTHER), id(ITEM)],
        }))
        .unwrap();
    assert!(service.apply(&mutate(1, Mutation::Complete)).is_err()); // refused CAS
    service
        .apply(&mutate(
            8,
            Mutation::Delete {
                inventory_revision: 3,
                confirmation: Confirmation {
                    item_id: Id::parse(ITEM).unwrap(),
                    item_revision: 8,
                },
            },
        ))
        .unwrap();
    held.adopt(super::read(&f.core, Some(&service), held.cursor()));
    let preview = service.preview().unwrap();
    held.present(Some(&preview));
    assert_eq!(
        held.events
            .iter()
            .map(|event| event.action.as_str())
            .collect::<Vec<_>>(),
        [
            "add", "complete", "reopen", "edit", "assign", "unassign", "archive", "restore", "add",
            "reorder", "delete"
        ]
    );
    assert_eq!(held.user.as_deref(), f.model()["user"].as_str());
    assert_ne!(
        held.user.as_deref(),
        Some(preview.actor_id.as_str()),
        "the lead caller is not the Core user"
    );
    assert!(
        !held.snapshot.as_ref().unwrap().titles.contains_key(ITEM),
        "deletion does not invent an old title"
    );
    let cursor = held.cursor();
    held.adopt(super::read(&f.core, Some(&service), cursor));
    assert_eq!(held.cursor(), cursor);
    assert_eq!(held.events.len(), 11);
}

#[test]
fn same_member_in_different_rooms_and_removed_membership_never_share_a_cursor() {
    use super::super::{Completed, Outcome, State};
    use std::time::Instant;
    let now = Instant::now();
    let first = key();
    let mut second = first.clone();
    second.room = "99999999-9999-4999-8999-999999999999".into();
    let mut state = State::reading();
    state.reconcile([first.clone(), second.clone()].into_iter());
    let task = state.offer(Some(&first), now).unwrap();
    state.finished(
        Completed {
            key: first.clone(),
            epoch: task.epoch,
            outcome: Outcome::Read {
                preview: Ok(preview(vec![])),
                activity: read(Ok(vec![page(&[7], false)])),
            },
        },
        now,
    );
    let task = state.offer(Some(&second), now).unwrap();
    assert!(matches!(task.work, super::super::Work::Read { after: 0 }));
    state.reconcile([first.clone()].into_iter());
    state.finished(
        Completed {
            key: second.clone(),
            epoch: task.epoch,
            outcome: Outcome::Read {
                preview: Ok(preview(vec![])),
                activity: read(Ok(vec![page(&[8], false)])),
            },
        },
        now,
    );
    assert!(state.activity(&second).is_none());
    assert_eq!(state.lists[&first].activity.cursor(), 7);
}

#[test]
fn successful_and_unknown_local_writes_do_not_invent_activity() {
    use super::super::{Completed, Outcome, State, Work, Write};
    use crate::checklist::{Applied, Code, Error};
    use std::time::Instant;
    let now = Instant::now();
    let key = key();
    let mut state = State::reading();
    let task = state.offer(Some(&key), now).unwrap();
    state.finished(
        Completed {
            key: key.clone(),
            epoch: task.epoch,
            outcome: Outcome::Read {
                preview: Ok(preview(vec![])),
                activity: read(Ok(vec![page(&[1], false)])),
            },
        },
        now,
    );
    for success in [true, false] {
        let task = state
            .write(
                &key,
                Write::Add {
                    title: "local".into(),
                    body: None,
                    reference: None,
                    inventory: None,
                },
            )
            .unwrap();
        assert!(matches!(task.work, Work::Write(_)));
        let result = if success {
            Ok(Applied {
                current: preview(vec![]).current,
                item_id: None,
                item_revision: None,
                changed: true,
            })
        } else {
            Err(Error {
                code: Code::OutcomeUnknown,
                message: "uncertain".into(),
                current: None,
            })
        };
        state.finished(
            Completed {
                key: key.clone(),
                epoch: task.epoch,
                outcome: Outcome::Write(result),
            },
            now,
        );
        assert_eq!(state.activity(&key).unwrap().events.len(), 1);
        assert_eq!(state.lists[&key].activity.cursor(), 1);
    }
}

#[test]
fn labels_are_limited_to_event_references_and_unused_edits_keep_the_snapshot() {
    use super::super::tests::view;
    use crate::checklist::model::Completion;
    let mut preview = preview(vec![
        view(1, None, Completion::Open, false),
        view(2, None, Completion::Open, false),
    ]);
    let mut held = Held::default();
    held.adopt(read(Ok(vec![page(&[], false)])));
    held.present(Some(&preview));
    let empty = held.snapshot.as_ref().unwrap();
    assert!(
        empty.titles.is_empty() && empty.names.is_empty(),
        "no events need no copied titles or roster labels"
    );

    let mut event = event(1, 1000);
    event.item_id = Some(preview.current.items[0].item.id.as_str().into());
    event.actor_id = preview.members[0].0.as_str().into();
    held.adopt(read(Ok(vec![Page {
        events: vec![event],
        next_cursor: 1,
        more: false,
        gap: None,
    }])));
    held.present(Some(&preview));
    let used = held.snapshot.clone().unwrap();
    assert_eq!(used.titles.len(), 1);
    assert_eq!(used.names.len(), 1);
    preview.current.items[1].item.content.title = "An unrelated edit".into();
    preview.members[1].1 = "An unrelated rename".into();
    held.present(Some(&preview));
    assert!(
        Arc::ptr_eq(&used, held.snapshot.as_ref().unwrap()),
        "unused labels do not invalidate the transcript"
    );
    preview.current.items[0].item.content.title = "Current used title".into();
    held.present(Some(&preview));
    let changed = held.snapshot.as_ref().unwrap();
    assert!(!Arc::ptr_eq(&used, changed));
    assert_eq!(
        changed.titles.values().next().unwrap(),
        "Current used title"
    );
    assert_eq!(changed.events.len(), 1);
    assert_eq!(held.cursor(), 1);
}

#[test]
fn deletion_discards_a_title_even_when_a_failed_preview_keeps_the_old_item() {
    use super::super::tests::view;
    use super::super::{Completed, FRESH, Outcome, State};
    use crate::checklist::model::Completion;
    use std::time::Instant;
    let now = Instant::now();
    let key = key();
    let preview = preview(vec![view(1, None, Completion::Open, false)]);
    let item = preview.current.items[0].item.id.as_str().to_owned();
    let mut added = event(1, 1000);
    added.item_id = Some(item.clone());
    let mut state = State::reading();
    let task = state.offer(Some(&key), now).unwrap();
    state.finished(
        Completed {
            key: key.clone(),
            epoch: task.epoch,
            outcome: Outcome::Read {
                preview: Ok(preview),
                activity: read(Ok(vec![Page {
                    events: vec![added],
                    next_cursor: 1,
                    more: false,
                    gap: None,
                }])),
            },
        },
        now,
    );
    assert!(state.activity(&key).unwrap().titles.contains_key(&item));
    let later = now + FRESH;
    let task = state.offer(Some(&key), later).unwrap();
    let mut deleted = event(2, 2000);
    deleted.item_id = Some(item.clone());
    deleted.action = "delete".into();
    state.finished(
        Completed {
            key: key.clone(),
            epoch: task.epoch,
            outcome: Outcome::Read {
                preview: Err("preview unavailable".into()),
                activity: read(Ok(vec![Page {
                    events: vec![deleted],
                    next_cursor: 2,
                    more: false,
                    gap: None,
                }])),
            },
        },
        later,
    );
    assert_eq!(
        state.preview(&key).unwrap().current.items.len(),
        1,
        "the independent preview remains held"
    );
    let facts = state.activity(&key).unwrap();
    assert_eq!(facts.events.len(), 2);
    assert_eq!(state.lists[&key].activity.cursor(), 2);
    assert!(
        !facts.titles.contains_key(&item),
        "deleted items never borrow a stale held title"
    );
}
