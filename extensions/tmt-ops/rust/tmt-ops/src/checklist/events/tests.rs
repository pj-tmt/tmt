use super::*;
use crate::checklist::{
    Error,
    model::{self, Confirmation, Edit, InventoryExpectation, Mutation, Request, Stamp},
    store::{Stage, Store, with_fault},
    test_support::*,
};

fn store(f: &Fixture) -> Store {
    Store::new(&f.root, id(ROOM), None, "ops").unwrap()
}
/// One change by LEAD at `at_ms`, logged the way the service logs it.
fn change(store: &Store, request: &Request, at_ms: u64) -> Result<(), Error> {
    store.update(
        |document| {
            let stamp = Stamp {
                by: id(LEAD),
                at_ms,
            };
            model::apply(document, request, None, &stamp).map(|outcome| ((), outcome.event))
        },
        || Ok(()),
    )
}
fn item(n: u8) -> String {
    format!("66666666-6666-4666-8666-66666666666{n}")
}
fn kinds(page: &Page) -> Vec<(&str, u64)> {
    page.events
        .iter()
        .map(|event| (event.action.as_str(), event.revision))
        .collect()
}
fn count(f: &Fixture) -> u64 {
    f.database()
        .query_row("SELECT COUNT(*) FROM checklist_events", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap() as u64
}

#[test]
fn every_applied_change_logs_one_event_and_a_no_op_logs_none() {
    let f = Fixture::new();
    let store = store(&f);
    let (first, second) = (item(1), item(2));
    change(
        &store,
        &create(&first, InventoryExpectation::Absent, None),
        1_000,
    )
    .unwrap();
    change(&store, &mutate(&first, 1, Mutation::Complete), 2_000).unwrap();
    // Already complete: the revision does not move, so there is nothing to say.
    change(&store, &mutate(&first, 2, Mutation::Complete), 2_500).unwrap();
    let edit = Mutation::Edit(Edit {
        title: Some("Renamed".into()),
        body: None,
        reference: None,
    });
    change(&store, &mutate(&first, 2, edit), 3_000).unwrap();
    change(&store, &mutate(&first, 3, Mutation::Archive), 3_500).unwrap();
    change(&store, &mutate(&first, 4, Mutation::Restore), 3_600).unwrap();
    change(&store, &mutate(&first, 5, Mutation::Reopen), 3_700).unwrap();
    change(
        &store,
        &create(&second, InventoryExpectation::Revision(1), None),
        4_000,
    )
    .unwrap();
    let reorder = |order: [&str; 2]| Request {
        room_id: id(ROOM),
        checklist_id: id(CHECKLIST),
        action: model::Action::Reorder {
            inventory_revision: 2,
            order: order.map(id).to_vec(),
        },
    };
    // The same order is a no-op; the swap is one event carrying the new inventory revision.
    change(&store, &reorder([&first, &second]), 4_500).unwrap();
    change(&store, &reorder([&second, &first]), 5_000).unwrap();
    let delete = Mutation::Delete {
        inventory_revision: 3,
        confirmation: Confirmation {
            item_id: id(&first),
            item_revision: 6,
        },
    };
    change(&store, &mutate(&first, 6, delete), 6_000).unwrap();

    let page = store.events(0, 100).unwrap();
    assert_eq!(
        kinds(&page),
        [
            ("add", 1),
            ("complete", 2),
            ("edit", 3),
            ("archive", 4),
            ("restore", 5),
            ("reopen", 6),
            ("add", 1),
            ("reorder", 3),
            ("delete", 7),
        ]
    );
    assert_eq!(page.events[0].actor_id, LEAD);
    assert_eq!(page.events[0].at_ms, 1_000);
    assert_eq!(page.events[0].room_id, ROOM);
    assert_eq!(page.events[0].checklist_id, CHECKLIST);
    assert_eq!(page.events[0].member_id, None);
    assert_eq!(page.events[0].item_id.as_deref(), Some(first.as_str()));
    // A reorder moves the whole list, so it names no item.
    assert_eq!(page.events[7].item_id, None);
    assert!(page.gap.is_none() && !page.more);
    assert_eq!(page.next_cursor, page.events[8].id);
    // The log holds no title and no body.
    let columns: Vec<String> = f
        .database()
        .prepare("SELECT name FROM pragma_table_info('checklist_events')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        !columns
            .iter()
            .any(|name| name.contains("title") || name.contains("body"))
    );
}

#[test]
fn an_event_commits_with_its_document_or_not_at_all() {
    let f = Fixture::new();
    let store = store(&f);
    let first = item(1);
    let make = || {
        change(
            &store,
            &create(&first, InventoryExpectation::Absent, None),
            1_000,
        )
    };
    // A refused publication discards the candidate document and its row alike.
    let refused = with_fault(
        Box::new(|stage| match stage {
            Stage::BeforePublish => Err(std::io::Error::other("refused")),
            Stage::AfterCommit => Ok(()),
        }),
        make,
    );
    assert!(refused.is_err());
    assert!(store.read().unwrap().is_none());
    assert_eq!(store.events(0, 100).unwrap().events.len(), 0);

    // A commit whose acknowledgement never arrived is committed with its row.
    let unknown = with_fault(
        Box::new(|stage| match stage {
            Stage::AfterCommit => Err(std::io::Error::other("lost")),
            Stage::BeforePublish => Ok(()),
        }),
        make,
    );
    assert!(unknown.is_err());
    assert!(store.read().unwrap().is_some());
    assert_eq!(kinds(&store.events(0, 100).unwrap()), [("add", 1)]);
}

#[test]
fn a_read_pages_by_cursor_and_creates_nothing() {
    let f = Fixture::new();
    let store = store(&f);
    let empty = store.events(0, 10).unwrap();
    assert!(empty.events.is_empty() && !empty.more && empty.gap.is_none());
    assert_eq!(empty.next_cursor, 0);
    assert!(!f.root.join("ops").exists());

    for n in 1..=3u8 {
        let inventory = if n == 1 {
            InventoryExpectation::Absent
        } else {
            InventoryExpectation::Revision(u64::from(n) - 1)
        };
        change(
            &store,
            &create(&item(n), inventory, None),
            1_000 + u64::from(n),
        )
        .unwrap();
    }
    let first = store.events(0, 2).unwrap();
    assert_eq!(first.events.len(), 2);
    assert!(first.more);
    assert_eq!(first.next_cursor, first.events[1].id);
    let rest = store.events(first.next_cursor, 2).unwrap();
    assert_eq!(rest.events.len(), 1);
    assert!(!rest.more);
    assert_eq!(rest.next_cursor, rest.events[0].id);
    // Asking again after the end returns nothing and keeps the cursor.
    let end = store.events(rest.next_cursor, 2).unwrap();
    assert!(end.events.is_empty() && end.next_cursor == rest.next_cursor);
    // The cap bounds one page whatever is asked.
    assert_eq!(store.events(0, 1_000_000).unwrap().events.len(), 3);
}

#[test]
fn a_conversation_list_logs_under_its_own_key_with_its_member() {
    let f = Fixture::new();
    let room = store(&f);
    let member = Store::new(&f.root, id(ROOM), Some(id(WORKER)), "ops").unwrap();
    change(
        &room,
        &create(&item(1), InventoryExpectation::Absent, None),
        1_000,
    )
    .unwrap();
    member
        .update(
            |document| {
                let stamp = Stamp {
                    by: id(WORKER),
                    at_ms: 2_000,
                };
                let outcome = model::apply(
                    document,
                    &create(&item(2), InventoryExpectation::Absent, None),
                    None,
                    &stamp,
                )?;
                document.as_mut().unwrap().member_id = Some(id(WORKER));
                Ok(((), outcome.event))
            },
            || Ok(()),
        )
        .unwrap();
    let own = member.events(0, 100).unwrap();
    assert_eq!(own.events.len(), 1);
    assert_eq!(own.events[0].member_id.as_deref(), Some(WORKER));
    assert_eq!(own.events[0].actor_id, WORKER);
    assert_eq!(room.events(0, 100).unwrap().events.len(), 1);
}

#[test]
fn retention_keeps_the_shorter_of_thirty_days_and_a_thousand_events_per_room() {
    let f = Fixture::new();
    let store = store(&f);
    let first = item(1);
    change(
        &store,
        &create(&first, InventoryExpectation::Absent, None),
        1_000,
    )
    .unwrap();
    // Within thirty days of each other, both stay.
    change(&store, &mutate(&first, 1, Mutation::Complete), RETAIN_MS).unwrap();
    assert_eq!(store.events(0, 100).unwrap().events.len(), 2);
    // The next one is more than thirty days after the first: only that one is lost.
    let now = RETAIN_MS + 2_000;
    change(&store, &mutate(&first, 2, Mutation::Reopen), now).unwrap();
    let page = store.events(0, 100).unwrap();
    assert_eq!(kinds(&page), [("complete", 2), ("reopen", 3)]);
    assert_eq!(page.gap, Some(Gap::Pruned));
    // A reader that is up to date is not told of a gap it did not miss.
    assert_eq!(store.events(page.next_cursor, 100).unwrap().gap, None);

    // The count bound: a room that already holds more than a thousand keeps the newest thousand.
    f.database()
        .execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1200)
             INSERT INTO checklist_events(checklist_key, room_id, checklist_id, actor_id, at_ms, action, revision)
             SELECT '{ROOM}', '{ROOM}', '{CHECKLIST}', '{LEAD}', {now}, 'edit', i FROM n;"
        ))
        .unwrap();
    change(&store, &mutate(&first, 3, Mutation::Complete), now + 1).unwrap();
    assert_eq!(count(&f), RETAIN_PER_ROOM);
    assert_eq!(store.events(0, MAX_PAGE).unwrap().gap, Some(Gap::Pruned));
    // The newest event is the one just made.
    let last = f
        .database()
        .query_row(
            "SELECT action, revision FROM checklist_events ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!(last, ("complete".to_owned(), 4i64));
}

#[test]
fn a_list_changed_before_the_log_existed_reads_legacy_from_the_beginning() {
    let f = Fixture::new();
    let store = store(&f);
    let first = item(1);
    change(
        &store,
        &create(&first, InventoryExpectation::Absent, None),
        1_000,
    )
    .unwrap();
    // Stand in for a database upgraded under this list: the document is there, the log never saw it.
    f.database()
        .execute_batch("DELETE FROM checklist_events; DELETE FROM checklist_event_floor;")
        .unwrap();
    assert_eq!(store.events(0, 100).unwrap().gap, None);
    change(&store, &mutate(&first, 1, Mutation::Complete), 2_000).unwrap();

    let page = store.events(0, 100).unwrap();
    assert_eq!(kinds(&page), [("complete", 2)]);
    assert_eq!(page.gap, Some(Gap::Legacy));
    // A reader that has already seen an event is past the unknown history.
    assert_eq!(store.events(page.next_cursor, 100).unwrap().gap, None);
}
