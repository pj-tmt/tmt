use super::*;

#[test]
fn a_conversation_list_is_created_on_its_first_add_under_a_derived_checklist_id() {
    let f = Fixture::new();
    let service = f.conversation(WORKER);
    let listed = service.list(&Filter::default()).unwrap().current;
    assert!(listed.checklist_id.is_none());
    let member = listed.member.expect("a conversation names its member");
    assert_eq!(
        (member.id, member.name.as_deref(), member.retired),
        (id(WORKER), Some("Worker"), false)
    );
    assert!(!f.root.join("ops").exists(), "reading creates nothing");

    let derived = service.conversation_checklist_id().unwrap();
    assert_eq!(derived, Id::conversation_checklist(&id(ROOM), &id(WORKER)));
    let applied = service
        .apply(&conversation_create(
            &service,
            ITEM,
            InventoryExpectation::Absent,
        ))
        .unwrap();
    assert_eq!(applied.current.checklist_id, Some(derived.clone()));
    assert_eq!(f.keys(), [format!("{ROOM}/{WORKER}")]);
    let stored = f.conversation_literal(WORKER);
    assert_eq!(stored["memberId"], WORKER);
    assert_eq!(stored["roomId"], ROOM);
    assert_eq!(stored["checklistId"], derived.as_str());
    let again = service.list(&Filter::default()).unwrap();
    assert_eq!(
        (again.total_count, again.current.checklist_id),
        (1, Some(derived))
    );
}

#[test]
fn lists_are_isolated_between_members_and_from_the_rooms_own() {
    let f = Fixture::new();
    f.service()
        .apply(&create(ITEM, InventoryExpectation::Absent, None))
        .unwrap();
    let worker = f.conversation(WORKER);
    worker
        .apply(&conversation_create(
            &worker,
            ITEM,
            InventoryExpectation::Absent,
        ))
        .unwrap();
    let manager = f.manager();
    drop(manager);
    let lead = f.conversation(LEAD);
    lead.apply(&conversation_create(
        &lead,
        OTHER,
        InventoryExpectation::Absent,
    ))
    .unwrap();

    let mut keys = vec![
        ROOM.to_owned(),
        format!("{ROOM}/{LEAD}"),
        format!("{ROOM}/{WORKER}"),
    ];
    keys.sort();
    assert_eq!(f.keys(), keys);
    let ids = |service: &Service| {
        service
            .list(&Filter::default())
            .unwrap()
            .current
            .items
            .iter()
            .map(|view| view.item.id.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&worker), [ITEM]);
    assert_eq!(ids(&lead), [OTHER]);
    assert_eq!(ids(&f.service()), [ITEM]);
    assert_ne!(
        worker.conversation_checklist_id(),
        lead.conversation_checklist_id()
    );
    // The room list is byte for byte what it was: no member field.
    assert!(f.literal().get("memberId").is_none());
}

#[test]
fn a_conversation_list_takes_only_its_derived_checklist_id() {
    let f = Fixture::new();
    let service = f.conversation(WORKER);
    let error = service
        .apply(&create(ITEM, InventoryExpectation::Absent, None))
        .unwrap_err();
    assert_eq!(error.code, Code::InputInvalid);
    assert!(f.keys().is_empty());
}

#[test]
fn an_unknown_member_is_refused_and_a_retired_one_keeps_a_read_only_list() {
    let f = Fixture::new();
    let stranger = f.conversation(OTHER);
    assert_eq!(
        stranger.list(&Filter::default()).unwrap_err().code,
        Code::MemberUnavailable
    );
    assert_eq!(
        stranger
            .apply(&conversation_create(
                &stranger,
                ITEM,
                InventoryExpectation::Absent
            ))
            .unwrap_err()
            .code,
        Code::MemberUnavailable
    );
    assert!(f.keys().is_empty());

    f.change(|m| m["caller"] = json!(LEAD));
    let service = f.conversation(WORKER);
    service
        .apply(&conversation_create(
            &service,
            ITEM,
            InventoryExpectation::Absent,
        ))
        .unwrap();
    f.change(|m| m["retired"] = json!([WORKER]));
    let kept = service.list(&Filter::default()).unwrap();
    assert_eq!(kept.total_count, 1);
    assert!(kept.current.member.unwrap().retired);
    let refused = service
        .apply(&conversation_create(
            &service,
            OTHER,
            InventoryExpectation::Revision(1),
        ))
        .unwrap_err();
    assert_eq!(refused.code, Code::MemberUnavailable);
    assert!(refused.message.contains("retired"), "{}", refused.message);
    assert_eq!(
        f.conversation_literal(WORKER)["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_replacement_with_the_same_name_is_another_uuid_with_an_empty_list() {
    let f = Fixture::new();
    f.change(|m| m["caller"] = json!(LEAD));
    let old = f.conversation(WORKER);
    old.apply(&conversation_create(
        &old,
        ITEM,
        InventoryExpectation::Absent,
    ))
    .unwrap();
    f.change(|m| {
        m["retired"] = json!([WORKER]);
        m["members"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":OTHER,"name":"Worker","lifetime":"saved"}));
    });
    // The name now resolves to the replacement; the retired UUID still resolves, to its list.
    let replacement = resolve_member(&f.core, "Worker").unwrap();
    assert_eq!(replacement, id(OTHER));
    assert_eq!(resolve_member(&f.core, WORKER).unwrap(), id(WORKER));
    let fresh = f.conversation(OTHER);
    assert!(
        fresh
            .list(&Filter::default())
            .unwrap()
            .current
            .checklist_id
            .is_none()
    );
    assert_eq!(old.list(&Filter::default()).unwrap().total_count, 1);
}

#[test]
fn a_member_resolves_once_by_uuid_or_active_name_and_a_refusal_names_what_it_was_given() {
    let f = Fixture::new();
    assert_eq!(resolve_member(&f.core, "Worker").unwrap(), id(WORKER));
    assert_eq!(
        resolve_member(&f.core, &WORKER.to_uppercase()).unwrap(),
        id(WORKER)
    );
    f.change(|m| m["retired"] = json!([WORKER]));
    let retired_name = resolve_member(&f.core, "Worker").unwrap_err();
    assert_eq!(retired_name.code, Code::MemberUnavailable);
    assert!(
        retired_name.message.contains("\"Worker\""),
        "{}",
        retired_name.message
    );
    let unknown = resolve_member(&f.core, "nobody").unwrap_err();
    assert!(
        unknown.message.contains("\"nobody\""),
        "{}",
        unknown.message
    );
    let missing = resolve_member(&f.core, OTHER).unwrap_err();
    assert_eq!(missing.code, Code::MemberUnavailable);
    assert!(missing.message.contains(OTHER), "{}", missing.message);
}

#[test]
fn a_document_under_another_conversations_key_fails_closed() {
    let f = Fixture::new();
    let worker = f.conversation(WORKER);
    worker
        .apply(&conversation_create(
            &worker,
            ITEM,
            InventoryExpectation::Absent,
        ))
        .unwrap();
    let bytes: Vec<u8> = f
        .database()
        .query_row(
            "SELECT body FROM documents WHERE kind = 'checklist' AND key = ?1",
            [format!("{ROOM}/{WORKER}")],
            |row| row.get(0),
        )
        .unwrap();
    // Another member's key, and the room's own, each holding the worker's document.
    for key in [format!("{ROOM}/{LEAD}"), ROOM.to_owned()] {
        f.database()
            .execute(
                "INSERT INTO documents(kind, key, body) VALUES ('checklist', ?1, ?2)",
                rusqlite::params![key, bytes],
            )
            .unwrap();
    }
    f.change(|m| m["caller"] = json!(LEAD));
    for service in [f.conversation(LEAD), f.service()] {
        let error = service.list(&Filter::default()).unwrap_err();
        assert_eq!(error.code, Code::StorageError);
        assert!(
            error.message.contains("another conversation"),
            "{}",
            error.message
        );
    }
}

#[test]
fn the_derived_checklist_id_is_stable_canonical_and_unique_per_pair() {
    let (room, other_room) = (id(ROOM), id(CHECKLIST));
    let (worker, lead) = (id(WORKER), id(LEAD));
    let derived = Id::conversation_checklist(&room, &worker);
    assert_eq!(derived, Id::conversation_checklist(&room, &worker));
    assert_eq!(Id::parse(derived.as_str()).unwrap(), derived);
    assert_eq!(derived.as_str(), derived.as_str().to_ascii_lowercase());
    for other in [
        Id::conversation_checklist(&room, &lead),
        Id::conversation_checklist(&other_room, &worker),
        Id::conversation_checklist(&worker, &room),
    ] {
        assert_ne!(derived, other);
    }
}
