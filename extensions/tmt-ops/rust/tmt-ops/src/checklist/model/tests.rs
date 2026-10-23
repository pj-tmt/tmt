use super::*;
use crate::checklist::test_support::{
    CHECKLIST, ITEM, LEAD, OTHER, ROOM, WORKER, create, id, mutate,
};
use serde_json::json;

#[test]
fn field_bounds_and_uuid_aliases_are_explicit() {
    assert_eq!(
        Id::parse("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA").unwrap(),
        id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
    );
    for bad in [
        "../room",
        "",
        "44444444444444448444444444444444",
        "44444444-4444-4444-8444-44444444444z",
    ] {
        assert_eq!(Id::parse(bad).unwrap_err().code, Code::InputInvalid);
    }
    ItemContent::new(
        "x".repeat(1024),
        Some("x".repeat(65536)),
        Some(format!("https://example.org/{}", "x".repeat(4076))),
    )
    .unwrap();
    for title in [" ", "line\n", "\0", &"x".repeat(1025)] {
        assert_eq!(
            ItemContent::new(title.into(), None, None).unwrap_err().code,
            Code::InputInvalid
        );
    }
    assert!(ItemContent::new("title".into(), Some("x".repeat(65537)), None).is_err());
    assert!(ItemContent::new("title".into(), Some("\0".into()), None).is_err());
    assert_eq!(
        ItemContent::new("title".into(), Some("".into()), None)
            .unwrap()
            .body,
        None
    );
    for link in [
        "file:///private",
        "javascript:alert(1)",
        "https://",
        "https://host/white space",
    ] {
        assert!(ItemContent::new("title".into(), None, Some(link.into())).is_err());
    }
}

fn stamp() -> Stamp {
    Stamp {
        by: id(LEAD),
        at_ms: 1_000,
    }
}

/// The same document stored by the current release before anyone is known to have touched it.
fn literal_v2_unknown() -> serde_json::Value {
    json!({"version":2,"roomId":ROOM,"checklistId":CHECKLIST,"inventoryRevision":1,"items":[{"id":ITEM,"revision":1,"title":"literal","body":null,"reference":null,"assignee":null,"completion":"open","archived":false,"addedBy":null,"addedAtMs":null,"lastChangedBy":null,"lastChangedAtMs":null}],"deleted":[]})
}

fn literal_v2() -> serde_json::Value {
    let mut value = literal_v2_unknown();
    value["items"][0]["addedBy"] = json!(WORKER);
    value["items"][0]["addedAtMs"] = json!(500);
    value["items"][0]["lastChangedBy"] = json!(LEAD);
    value["items"][0]["lastChangedAtMs"] = json!(900);
    value
}

fn later() -> Stamp {
    Stamp {
        by: id(WORKER),
        at_ms: 2_000,
    }
}

/// A document as the previous release stored it: no provenance.
fn literal() -> serde_json::Value {
    json!({"version":1,"roomId":ROOM,"checklistId":CHECKLIST,"inventoryRevision":1,"items":[{"id":ITEM,"revision":1,"title":"literal","body":null,"reference":null,"assignee":null,"completion":"open","archived":false}],"deleted":[]})
}

#[test]
fn literal_schema_fails_closed_and_tombstones_have_only_identity() {
    let doc = Document::decode(&literal()).unwrap();
    assert_eq!(doc.items[0].content.title, "literal");
    // Storing it again writes the current version, every stamp still unknown.
    let stored = doc.encode();
    assert_eq!(stored, literal_v2_unknown());
    assert_eq!(Document::decode(&stored).unwrap(), doc);
    for edit in [
        |v: &mut serde_json::Value| v["version"] = json!(3),
        |v: &mut serde_json::Value| v["inventoryRevision"] = json!(0),
        |v: &mut serde_json::Value| v["unexpected"] = json!(true),
        |v: &mut serde_json::Value| v["items"][0]["completion"] = json!("done"),
        |v: &mut serde_json::Value| v["items"][0]["body"] = json!(""),
        |v: &mut serde_json::Value| {
            v["items"][0]
                .as_object_mut()
                .unwrap()
                .remove("archived")
                .map(|_| ())
                .unwrap()
        },
    ] {
        let mut value = literal();
        edit(&mut value);
        assert_eq!(
            Document::decode(&value).unwrap_err().code,
            Code::StorageError
        );
    }
    let mut document = Some(doc);
    apply(
        &mut document,
        &mutate(
            ITEM,
            1,
            Mutation::Delete {
                inventory_revision: 1,
                confirmation: Confirmation {
                    item_id: id(ITEM),
                    item_revision: 1,
                },
            },
        ),
        None,
        &stamp(),
    )
    .unwrap();
    let v = document.unwrap().encode();
    assert_eq!(v["items"], json!([]));
    assert_eq!(
        v["deleted"],
        json!([{"roomId":ROOM,"checklistId":CHECKLIST,"itemId":ITEM,"deletionRevision":2}])
    );
}

#[test]
fn noops_still_check_revisions_and_overflow_never_wraps() {
    let mut document = Some(Document::decode(&literal()).unwrap());
    let result = apply(
        &mut document,
        &mutate(ITEM, 1, Mutation::Reopen),
        None,
        &stamp(),
    )
    .unwrap();
    assert!(!result.changed);
    assert_eq!(document.as_ref().unwrap().inventory_revision, 1);
    assert_eq!(
        apply(
            &mut document,
            &mutate(ITEM, 2, Mutation::Reopen),
            None,
            &stamp()
        )
        .err()
        .unwrap()
        .code,
        Code::Conflict
    );
    document.as_mut().unwrap().items[0].revision = u64::MAX;
    assert_eq!(
        apply(
            &mut document,
            &mutate(ITEM, u64::MAX, Mutation::Complete),
            None,
            &stamp()
        )
        .err()
        .unwrap()
        .code,
        Code::StorageError
    );
    document.as_mut().unwrap().inventory_revision = u64::MAX;
    assert_eq!(
        apply(
            &mut document,
            &create(OTHER, InventoryExpectation::Revision(u64::MAX), None),
            None,
            &stamp()
        )
        .err()
        .unwrap()
        .code,
        Code::StorageError
    );
}

#[test]
fn a_version_2_document_keeps_who_added_and_last_changed_each_item() {
    let doc = Document::decode(&literal_v2()).unwrap();
    assert_eq!(
        doc.items[0].added,
        Some(Stamp {
            by: id(WORKER),
            at_ms: 500
        })
    );
    assert_eq!(
        doc.items[0].last_changed,
        Some(Stamp {
            by: id(LEAD),
            at_ms: 900
        })
    );
    assert_eq!(doc.encode(), literal_v2());
    for (label, edit) in [
        (
            "a missing key",
            (|v: &mut serde_json::Value| {
                v["items"][0].as_object_mut().unwrap().remove("addedAtMs");
            }) as fn(&mut serde_json::Value),
        ),
        ("an author without a time", |v| {
            v["items"][0]["addedAtMs"] = json!(null)
        }),
        ("a time without an author", |v| {
            v["items"][0]["lastChangedBy"] = json!(null)
        }),
        ("a non-canonical author", |v| {
            v["items"][0]["addedBy"] = json!("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA")
        }),
        ("a time JavaScript cannot hold", |v| {
            v["items"][0]["addedAtMs"] = json!(1u64 << 53)
        }),
        ("a time that is not a number", |v| {
            v["items"][0]["lastChangedAtMs"] = json!("900")
        }),
    ] {
        let mut value = literal_v2();
        edit(&mut value);
        assert_eq!(
            Document::decode(&value).unwrap_err().code,
            Code::StorageError,
            "{label}"
        );
    }
    // Version 1 has no such keys, so a version 1 document that carries them is not one.
    let mut value = literal();
    value["items"][0]["addedBy"] = json!(WORKER);
    value["items"][0]["addedAtMs"] = json!(500);
    value["items"][0]["lastChangedBy"] = json!(WORKER);
    value["items"][0]["lastChangedAtMs"] = json!(500);
    assert_eq!(
        Document::decode(&value).unwrap_err().code,
        Code::StorageError
    );
}

#[test]
fn a_version_1_item_gets_its_change_stamped_and_never_a_guessed_author() {
    let mut document = Some(Document::decode(&literal()).unwrap());
    // A no-op moves nothing.
    let result = apply(
        &mut document,
        &mutate(ITEM, 1, Mutation::Reopen),
        None,
        &later(),
    )
    .unwrap();
    assert!(!result.changed);
    assert_eq!(document.as_ref().unwrap().items[0].last_changed, None);
    apply(
        &mut document,
        &mutate(ITEM, 1, Mutation::Complete),
        None,
        &later(),
    )
    .unwrap();
    let item = &document.as_ref().unwrap().items[0];
    assert_eq!(item.added, None, "who added it stays unknown");
    assert_eq!(item.last_changed, Some(later()));
    let stored = document.unwrap().encode();
    assert_eq!(stored["version"], 2);
    assert_eq!(stored["items"][0]["addedBy"], json!(null));
    assert_eq!(stored["items"][0]["lastChangedBy"], json!(WORKER));
    assert_eq!(stored["items"][0]["lastChangedAtMs"], json!(2_000));
}

#[test]
fn every_change_stamps_the_item_and_only_a_change_does() {
    let mut document = None;
    apply(
        &mut document,
        &create(ITEM, InventoryExpectation::Absent, None),
        None,
        &stamp(),
    )
    .unwrap();
    apply(
        &mut document,
        &create(OTHER, InventoryExpectation::Revision(1), None),
        None,
        &stamp(),
    )
    .unwrap();
    let created = Some(stamp());
    let d = document.as_ref().unwrap();
    assert!(
        d.items
            .iter()
            .all(|i| i.added == created && i.last_changed == created)
    );

    let edit = Mutation::Edit(Edit {
        title: Some("Renamed".into()),
        ..Edit::default()
    });
    let worker = Assignee {
        id: id(WORKER),
        label: "worker".into(),
    };
    let mut revision = 1;
    for (label, mutation, assignee) in [
        ("edit", edit, None),
        ("assign", Mutation::Assign(id(WORKER)), Some(worker)),
        ("unassign", Mutation::Unassign, None),
        ("complete", Mutation::Complete, None),
        ("reopen", Mutation::Reopen, None),
        ("archive", Mutation::Archive, None),
        ("restore", Mutation::Restore, None),
    ] {
        let by = Stamp {
            by: id(WORKER),
            at_ms: 3_000 + revision,
        };
        let result = apply(
            &mut document,
            &mutate(ITEM, revision, mutation),
            assignee,
            &by,
        )
        .unwrap();
        assert!(result.changed, "{label}");
        revision = result.item_revision.unwrap();
        let item = &document.as_ref().unwrap().items[0];
        assert_eq!(item.last_changed.as_ref(), Some(&by), "{label}");
        assert_eq!(item.added, created, "{label} leaves who added it");
    }
    // The other item was never touched, and neither reorder nor delete stamps an item.
    let before = document.as_ref().unwrap().items[1].clone();
    assert_eq!(before.last_changed, created);
    apply(
        &mut document,
        &Request {
            room_id: id(ROOM),
            checklist_id: id(CHECKLIST),
            action: Action::Reorder {
                inventory_revision: 2,
                order: vec![id(OTHER), id(ITEM)],
            },
        },
        None,
        &later(),
    )
    .unwrap();
    let d = document.as_ref().unwrap();
    assert_eq!(d.items[0].last_changed, created);
    assert_eq!(d.items[1].last_changed.as_ref().unwrap().at_ms, 3_000 + 7);
}
