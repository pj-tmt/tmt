use super::*;
use model::{Action, Confirmation, Edit, InventoryExpectation, Mutation};
use std::fs;
use test_support::*;

#[test]
fn collaborator_create_and_manager_assignment_are_distinct_and_inert() {
    let f = Fixture::new();
    let service = f.service();
    let untouched = [
        "pending",
        "agent-state",
        "requests",
        "config.json",
        "squad.toml",
    ]
    .map(|name| (name, fs::read(f.root.join(name)).unwrap()));
    assert!(
        service
            .list(&Filter::default())
            .unwrap()
            .current
            .checklist_id
            .is_none()
    );
    assert!(!f.root.join("squad").exists());
    for assignee in [WORKER, LEAD] {
        let error = service
            .apply(&create(ITEM, InventoryExpectation::Absent, Some(assignee)))
            .unwrap_err();
        assert_eq!(error.code, Code::Forbidden);
        assert!(error.current.is_none());
        assert!(!f.root.join("squad").exists());
    }
    let applied = service
        .apply(&create(ITEM, InventoryExpectation::Absent, None))
        .unwrap();
    assert_eq!(applied.item_revision, Some(1));
    assert_eq!(applied.current.inventory_revision, Some(1));
    assert!(applied.current.items[0].item.assignee.is_none());
    assert_eq!(applied.current.items[0].item.completion, Completion::Open);
    let manager = f.manager();
    manager
        .apply(&create(
            OTHER,
            InventoryExpectation::Revision(1),
            Some(WORKER),
        ))
        .unwrap();
    assert_eq!(
        f.literal()["items"][1]["assignee"],
        json!({"id":WORKER,"label":"Worker"})
    );
    for (name, bytes) in untouched {
        assert_eq!(fs::read(f.root.join(name)).unwrap(), bytes, "{name}");
    }
    let m = f.model();
    assert_eq!(m["pending"], "waiting");
    assert_eq!(m["agentState"], "busy");
    assert_eq!(m["requests"], json!(["unchanged request"]));
    assert!(
        m["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["operation"] == "rooms.roster" && c["locked"] == true)
    );
}

#[test]
fn independent_item_edits_stale_guards_and_full_archived_order_survive_restart() {
    let f = Fixture::new();
    let service = f.manager();
    service
        .apply(&create(ITEM, InventoryExpectation::Absent, None))
        .unwrap();
    service
        .apply(&create(OTHER, InventoryExpectation::Revision(1), None))
        .unwrap();
    let draft = mutate(
        ITEM,
        1,
        Mutation::Edit(Edit {
            title: Some("retained draft".into()),
            ..Edit::default()
        }),
    );
    service.apply(&mutate(ITEM, 1, Mutation::Complete)).unwrap();
    let error = service.apply(&draft).unwrap_err();
    assert_eq!(error.code, Code::Conflict);
    assert_eq!(error.current.unwrap().items[0].item.revision, 2);
    assert_eq!(
        draft,
        mutate(
            ITEM,
            1,
            Mutation::Edit(Edit {
                title: Some("retained draft".into()),
                ..Edit::default()
            })
        )
    );
    service
        .apply(&mutate(
            OTHER,
            1,
            Mutation::Edit(Edit {
                title: Some("independent".into()),
                ..Edit::default()
            }),
        ))
        .unwrap();
    service.apply(&mutate(ITEM, 2, Mutation::Archive)).unwrap();
    assert_eq!(service.list(&Filter::default()).unwrap().total_count, 2);
    assert_eq!(
        service
            .list(&Filter::default())
            .unwrap()
            .current
            .items
            .len(),
        1
    );
    let reorder = |revision, order| Request {
        room_id: id(ROOM),
        checklist_id: id(CHECKLIST),
        action: Action::Reorder {
            inventory_revision: revision,
            order,
        },
    };
    let before = f.bytes();
    assert_eq!(
        service
            .apply(&reorder(1, vec![id(OTHER), id(ITEM)]))
            .unwrap_err()
            .code,
        Code::Conflict
    );
    assert_eq!(
        service
            .apply(&reorder(2, vec![id(OTHER)]))
            .unwrap_err()
            .code,
        Code::InputInvalid
    );
    assert_eq!(f.bytes(), before);
    service
        .apply(&reorder(2, vec![id(OTHER), id(ITEM)]))
        .unwrap();
    service.apply(&mutate(ITEM, 3, Mutation::Restore)).unwrap();
    let restarted = f.service();
    let all = restarted
        .list(&Filter {
            include_archived: true,
            ..Filter::default()
        })
        .unwrap();
    assert_eq!(all.current.inventory_revision, Some(3));
    assert_eq!(all.current.items[0].item.id, id(OTHER));
    assert_eq!(all.current.items[0].item.content.title, "independent");
    assert_eq!(all.current.items[1].item.completion, Completion::Complete);
    assert!(!all.current.items[1].item.archived);
    let noop = restarted
        .apply(&mutate(ITEM, 4, Mutation::Complete))
        .unwrap();
    assert!(!noop.changed);
    assert_eq!(noop.item_revision, Some(4));
    assert_eq!(
        restarted
            .apply(&mutate(ITEM, 3, Mutation::Complete))
            .unwrap_err()
            .code,
        Code::Conflict
    );
}

#[test]
fn archived_state_unavailable_assignment_minimal_delete_and_nonreuse() {
    let f = Fixture::new();
    let manager = f.manager();
    manager
        .apply(&create(ITEM, InventoryExpectation::Absent, Some(WORKER)))
        .unwrap();
    manager.apply(&mutate(ITEM, 1, Mutation::Complete)).unwrap();
    manager.apply(&mutate(ITEM, 2, Mutation::Archive)).unwrap();
    let before = f.bytes();
    for mutation in [
        Mutation::Complete,
        Mutation::Reopen,
        Mutation::Unassign,
        Mutation::Assign(id(USER)),
        Mutation::Edit(Edit {
            body: Some(None),
            ..Edit::default()
        }),
    ] {
        assert_eq!(
            manager.apply(&mutate(ITEM, 3, mutation)).unwrap_err().code,
            Code::StateInvalid
        );
    }
    assert_eq!(f.bytes(), before);
    f.change(|m| m["retired"] = json!([WORKER]));
    let view = manager.show(&id(CHECKLIST), &id(ITEM)).unwrap();
    assert!(!view.items[0].assignee_available);
    assert_eq!(view.items[0].assignee_label.as_deref(), Some("Worker"));
    assert_eq!(view.items[0].item.assignee.as_ref().unwrap().id, id(WORKER));
    manager.apply(&mutate(ITEM, 3, Mutation::Restore)).unwrap();
    assert_eq!(
        manager
            .apply(&mutate(ITEM, 4, Mutation::Assign(id(WORKER))))
            .unwrap_err()
            .code,
        Code::AssigneeUnavailable
    );
    manager.apply(&mutate(ITEM, 4, Mutation::Unassign)).unwrap();
    let delete = mutate(
        ITEM,
        5,
        Mutation::Delete {
            inventory_revision: 1,
            confirmation: Confirmation {
                item_id: id(ITEM),
                item_revision: 5,
            },
        },
    );
    let result = manager.apply(&delete).unwrap();
    assert_eq!(result.item_revision, Some(6));
    assert_eq!(result.current.inventory_revision, Some(2));
    let value = f.literal();
    assert_eq!(value["items"], json!([]));
    assert_eq!(
        value["deleted"],
        json!([{"roomId":ROOM,"checklistId":CHECKLIST,"itemId":ITEM,"deletionRevision":6}])
    );
    let error = manager.show(&id(CHECKLIST), &id(ITEM)).unwrap_err();
    assert_eq!(error.code, Code::Deleted);
    assert_eq!(
        error.current.unwrap().deletion.unwrap().deletion_revision,
        6
    );
    assert_eq!(
        manager
            .apply(&create(ITEM, InventoryExpectation::Revision(2), None))
            .unwrap_err()
            .code,
        Code::Deleted
    );
    let list = manager.list(&Filter::default()).unwrap();
    assert_eq!(list.total_count, 0);
    assert_eq!(list.current.checklist_id, Some(id(CHECKLIST)));
    assert_eq!(
        manager.show(&id(CHECKLIST), &id(OTHER)).unwrap_err().code,
        Code::NotFound
    );
}

#[test]
fn exact_room_rename_and_orphan_access_never_follow_recreated_name() {
    let f = Fixture::new();
    let manager = f.manager();
    manager
        .apply(&create(ITEM, InventoryExpectation::Absent, None))
        .unwrap();
    let before = f.bytes();
    f.change(|m| m["roomName"] = json!("squad-renamed"));
    assert_eq!(
        manager
            .list(&Filter::default())
            .unwrap()
            .current
            .room
            .name
            .as_deref(),
        Some("renamed")
    );
    assert_eq!(f.bytes(), before);
    f.change(|m| m["roomRetired"] = json!(true));
    let error = manager.list(&Filter::default()).unwrap_err();
    assert_eq!(error.code, Code::Forbidden);
    assert!(error.current.is_none());
    assert_eq!(
        manager
            .apply(&mutate(ITEM, 1, Mutation::Complete))
            .unwrap_err()
            .code,
        Code::RoomUnavailable
    );
    f.change(|m| m["caller"] = json!(USER));
    let user = f.service();
    let orphan = user.list(&Filter::default()).unwrap();
    assert!(!orphan.current.room.available);
    assert_eq!(orphan.current.items.len(), 1);
    f.change(|m| {
        m["roomId"] = json!(OTHER);
        m["roomName"] = json!("squad-product");
        m["roomRetired"] = json!(false);
    });
    assert!(
        !user
            .list(&Filter::default())
            .unwrap()
            .current
            .room
            .available
    );
    let successor = Service::new(&f.core, &f.config, id(OTHER)).unwrap();
    assert!(
        successor
            .list(&Filter::default())
            .unwrap()
            .current
            .checklist_id
            .is_none()
    );
    assert_eq!(f.bytes(), before);
}

#[test]
fn ambiguous_caller_retired_actor_and_retired_recorded_uuid_refuse_without_fallback() {
    let f = Fixture::new();
    f.change(|m| m["caller"] = json!("ambiguous"));
    assert_eq!(
        Service::new(&f.core, &f.config, id(ROOM))
            .err()
            .unwrap()
            .code,
        Code::Forbidden
    );
    f.change(|m| m["caller"] = json!(WORKER));
    let worker = f.service();
    f.change(|m| m["retired"] = json!([WORKER]));
    let error = worker
        .apply(&create(ITEM, InventoryExpectation::Absent, None))
        .unwrap_err();
    assert_eq!(error.code, Code::Forbidden);
    assert!(error.current.is_none());
    f.change(|m| {
        m["caller"] = serde_json::Value::Null;
        m["retired"] = json!([USER]);
        m["members"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":OTHER,"name":"Ben","lifetime":"saved"}));
    });
    assert_eq!(
        Service::new(&f.core, &f.config, id(ROOM))
            .err()
            .unwrap()
            .code,
        Code::Forbidden
    );
    assert!(!f.root.join("squad").exists());
}

#[test]
fn locked_publication_rechecks_room_actor_privilege_and_assignment() {
    for change in [
        json!({"roomRetired":true}),
        json!({"retired":[LEAD]}),
        json!({"lead":USER}),
        json!({"removed":[WORKER]}),
        json!({"retired":[WORKER]}),
    ] {
        let f = Fixture::new();
        let manager = f.manager();
        manager
            .apply(&create(ITEM, InventoryExpectation::Absent, None))
            .unwrap();
        let before = f.bytes();
        f.change(|m| {
            m["lockedReads"] = json!(0);
            m["changeAtLockedRead"] = json!(3);
            m["change"] = change.clone();
        });
        let error = manager
            .apply(&mutate(ITEM, 1, Mutation::Assign(id(WORKER))))
            .unwrap_err();
        assert!(
            matches!(
                error.code,
                Code::RoomUnavailable | Code::Forbidden | Code::AssigneeUnavailable
            ),
            "{error}"
        );
        assert!(error.current.is_none());
        assert_eq!(f.bytes(), before);
        assert!(!f.directory().join("items.tmp").exists());
        f.change(|m| {
            m["roomRetired"] = json!(false);
            m["retired"] = json!([]);
            m["removed"] = json!([]);
            m["lead"] = json!(LEAD);
        });
        manager
            .apply(&mutate(ITEM, 1, Mutation::Assign(id(WORKER))))
            .unwrap();
        assert_eq!(f.literal()["items"][0]["revision"], 2);
    }
}

#[test]
fn unknown_readback_after_another_edit_and_delete_never_settles_original_operation() {
    let f = Fixture::new();
    let manager = f.manager();
    manager
        .apply(&create(ITEM, InventoryExpectation::Absent, None))
        .unwrap();
    let error = store::with_fault(
        Box::new(|stage, _| {
            if stage == store::Stage::DirectorySync {
                Err(std::io::Error::other("directory durability failed"))
            } else {
                Ok(())
            }
        }),
        || manager.apply(&mutate(ITEM, 1, Mutation::Complete)),
    )
    .unwrap_err();
    assert_eq!(error.code, Code::OutcomeUnknown);
    assert!(error.current.is_none());
    assert_eq!(f.literal()["items"][0]["revision"], 2);
    f.change(|m| m["caller"] = json!(USER));
    let other_actor = f.service();
    other_actor
        .apply(&mutate(
            ITEM,
            2,
            Mutation::Edit(Edit {
                title: Some("another actor edit".into()),
                ..Edit::default()
            }),
        ))
        .unwrap();
    let observed = other_actor.show(&id(CHECKLIST), &id(ITEM)).unwrap();
    assert_eq!(observed.items[0].item.revision, 3);
    other_actor
        .apply(&mutate(
            ITEM,
            3,
            Mutation::Delete {
                inventory_revision: 1,
                confirmation: Confirmation {
                    item_id: id(ITEM),
                    item_revision: 3,
                },
            },
        ))
        .unwrap();
    assert_eq!(
        other_actor
            .show(&id(CHECKLIST), &id(ITEM))
            .unwrap_err()
            .code,
        Code::Deleted
    );
    assert_eq!(error.code, Code::OutcomeUnknown);
    assert!(
        error
            .message
            .contains("original operation outcome remains unknown")
    );
}

#[test]
fn revision_seven_completion_filtered_empty_and_overflow_refuse_without_lost_bytes() {
    let f = Fixture::new();
    let manager = f.manager();
    manager
        .apply(&create(ITEM, InventoryExpectation::Absent, None))
        .unwrap();
    let mut literal = f.literal();
    literal["items"][0]["revision"] = json!(7);
    fs::write(f.directory().join("items.json"), literal.to_string()).unwrap();
    let result = manager.apply(&mutate(ITEM, 7, Mutation::Complete)).unwrap();
    assert_eq!(result.item_revision, Some(8));
    let filtered = manager
        .list(&Filter {
            completion: Some(Completion::Open),
            ..Filter::default()
        })
        .unwrap();
    assert!(filtered.current.items.is_empty());
    assert_eq!(filtered.total_count, 1);
    assert_eq!(filtered.current.checklist_id, Some(id(CHECKLIST)));
    assert_eq!(
        manager
            .apply(&mutate(
                ITEM,
                7,
                Mutation::Edit(Edit {
                    title: Some("draft".into()),
                    ..Edit::default()
                })
            ))
            .unwrap_err()
            .code,
        Code::Conflict
    );
    let mut literal = f.literal();
    literal["items"][0]["revision"] = json!(u64::MAX);
    literal["inventoryRevision"] = json!(u64::MAX);
    fs::write(f.directory().join("items.json"), literal.to_string()).unwrap();
    let before = f.bytes();
    assert_eq!(
        manager
            .apply(&mutate(ITEM, u64::MAX, Mutation::Reopen))
            .unwrap_err()
            .code,
        Code::StorageError
    );
    assert_eq!(
        manager
            .apply(&create(
                OTHER,
                InventoryExpectation::Revision(u64::MAX),
                None
            ))
            .unwrap_err()
            .code,
        Code::StorageError
    );
    assert_eq!(f.bytes(), before);
    assert!(!f.directory().join("items.tmp").exists());
}

#[test]
fn no_op_assignment_refreshes_only_projection_and_user_privilege_reload_refuses() {
    let f = Fixture::new();
    let manager = f.manager();
    manager
        .apply(&create(ITEM, InventoryExpectation::Absent, Some(WORKER)))
        .unwrap();
    let before = f.bytes();
    f.change(|m| m["members"][2]["name"] = json!("Renamed worker"));
    let result = manager
        .apply(&mutate(ITEM, 1, Mutation::Assign(id(WORKER))))
        .unwrap();
    assert!(!result.changed);
    assert_eq!(result.item_revision, Some(1));
    assert_eq!(f.bytes(), before);
    assert_eq!(
        result.current.items[0].assignee_label.as_deref(),
        Some("Renamed worker")
    );
    f.change(|m| m["caller"] = json!(USER));
    let user = f.service();
    let config_path = f.root.join("squad.toml");
    let error = store::with_fault(
        Box::new(move |stage, _| {
            if stage == store::Stage::FileSync {
                fs::write(&config_path, "")?;
            }
            Ok(())
        }),
        || user.apply(&mutate(ITEM, 1, Mutation::Archive)),
    )
    .unwrap_err();
    assert_eq!(error.code, Code::Forbidden);
    assert!(error.current.is_none());
    assert_eq!(f.bytes(), before);
    assert!(!f.directory().join("items.tmp").exists());
}

#[test]
fn recorded_uuid_case_alias_does_not_create_another_actor_or_namespace() {
    let f = Fixture::new();
    let user = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    f.change(|m| m["members"][0]["id"] = json!(user));
    fs::write(
        f.root.join("squad.toml"),
        format!("me='Ben'\nme_id='{}'\n", user.to_uppercase()),
    )
    .unwrap();
    let service = f.service();
    service
        .apply(&create(ITEM, InventoryExpectation::Absent, None))
        .unwrap();
    f.change(|m| m["caller"] = json!(user));
    let current = f.service().list(&Filter::default()).unwrap();
    assert!(current.current.room.manager);
    assert_eq!(current.current.room.id, id(ROOM));
    f.change(|m| {
        m["retired"] = json!([user]);
        m["members"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":OTHER,"name":"Ben","lifetime":"saved"}));
        m["caller"] = serde_json::Value::Null;
    });
    let config = Config::read(f.config.path().into()).unwrap();
    assert_eq!(
        Service::new(&f.core, &config, id(ROOM)).err().unwrap().code,
        Code::Forbidden
    );
}

#[test]
fn title_only_create_normalizes_empty_optional_body_before_publication() {
    let f = Fixture::new();
    let service = f.service();
    let request = Request {
        room_id: id(ROOM),
        checklist_id: id(CHECKLIST),
        action: Action::Create {
            item_id: id(ITEM),
            inventory: InventoryExpectation::Absent,
            content: model::ItemContent {
                title: "title only".into(),
                body: Some(String::new()),
                reference: None,
            },
            assignee: None,
        },
    };
    let result = service.apply(&request).unwrap();
    assert_eq!(result.current.items[0].item.content.body, None);
    assert_eq!(result.current.items[0].item.content.reference, None);
    assert_eq!(f.literal()["items"][0]["body"], serde_json::Value::Null);
    assert_eq!(
        f.literal()["items"][0]["reference"],
        serde_json::Value::Null
    );
}
