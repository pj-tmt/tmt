use super::*;
use crate::checklist::test_support::{CHECKLIST, ITEM, OTHER, ROOM, create, id, mutate};
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

fn literal() -> serde_json::Value {
    json!({"version":1,"roomId":ROOM,"checklistId":CHECKLIST,"inventoryRevision":1,"items":[{"id":ITEM,"revision":1,"title":"literal","body":null,"reference":null,"assignee":null,"completion":"open","archived":false}],"deleted":[]})
}

#[test]
fn literal_schema_fails_closed_and_tombstones_have_only_identity() {
    let doc = Document::decode(&literal()).unwrap();
    assert_eq!(doc.items[0].content.title, "literal");
    assert_eq!(doc.encode(), literal());
    for edit in [
        |v: &mut serde_json::Value| v["version"] = json!(2),
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
    let result = apply(&mut document, &mutate(ITEM, 1, Mutation::Reopen), None).unwrap();
    assert!(!result.changed);
    assert_eq!(document.as_ref().unwrap().inventory_revision, 1);
    assert_eq!(
        apply(&mut document, &mutate(ITEM, 2, Mutation::Reopen), None)
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
            None
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
            None
        )
        .err()
        .unwrap()
        .code,
        Code::StorageError
    );
}
