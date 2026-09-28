use super::*;
use crate::test_support::TestDirectory;

fn insert_identity(storage: &OfficeStore, id: &str, name: &str) {
    storage.connection().unwrap().execute("INSERT INTO identities (id,name,canonical_name,lifetime,created_at,updated_at) VALUES (?,?,?,'temporary','now','now')",params![id,name,name]).unwrap();
}
fn post(storage: &mut OfficeStore, actor: Actor, operation_id: &str) -> CreateReceipt {
    storage
        .post(&PostRequest {
            category: Category::General,
            actor,
            title: "Hello".into(),
            body: "Body\tline\n".into(),
            operation_id: operation_id.into(),
        })
        .unwrap()
}

#[test]
fn room_categories_share_storage_without_leaking_threads_or_gating_on_membership() {
    for retire in [false, true] {
        check_retained_room_discussion(retire);
    }
}

fn check_retained_room_discussion(retire: bool) {
    let directory = TestDirectory::new();
    let database = directory.path.join("state.db");
    let mut storage = OfficeStore::open(&database).unwrap();
    let identity_id = "11111111-1111-4111-8111-111111111111";
    insert_identity(&storage, identity_id, "alice");
    let room_a = "22222222-2222-4222-8222-222222222222";
    let room_b = "33333333-3333-4333-8333-333333333333";
    for id in [room_a, room_b] {
        storage
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO office_meeting_rooms (room_id,name,revision) VALUES (?,'Review',1)",
                [id],
            )
            .unwrap();
    }
    let actor = Actor::Identity {
        identity_id: identity_id.into(),
        name: "alice".into(),
    };
    let categories = [
        Category::General,
        Category::Repository("example.com/team/project".into()),
        Category::Room(room_a.into()),
        Category::Room(room_b.into()),
    ];
    let mut requests = Vec::new();
    let mut receipts = Vec::new();
    for category in &categories {
        let request = PostRequest {
            category: category.clone(),
            actor: actor.clone(),
            title: "Review".into(),
            body: "Independent discussion".into(),
            operation_id: uuid::Uuid::new_v4().to_string(),
        };
        receipts.push(storage.post(&request).unwrap());
        requests.push(request);
    }
    let list = |category| ListRequest {
        category,
        view: ListView::Recent,
        author: None,
        since_ms: None,
        limit: 20,
        cursor: None,
    };
    for (index, category) in categories.iter().enumerate() {
        let threads = storage.list(&list(category.clone())).unwrap().threads;
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].entry.id, receipts[index].entry_id);
        assert_eq!(&threads[0].entry.category, category);
    }
    let reply = storage
        .reply(&ReplyRequest {
            thread_id: receipts[2].thread_id.clone(),
            actor: actor.clone(),
            body: "Reply".into(),
            operation_id: uuid::Uuid::new_v4().to_string(),
        })
        .unwrap();
    let detail = storage
        .show(&ShowRequest {
            thread_id: receipts[2].thread_id.clone(),
            reply_limit: 20,
            reply_cursor: None,
        })
        .unwrap();
    assert_eq!(detail.replies[0].id, reply.entry_id);
    assert_eq!(detail.replies[0].category, categories[2]);
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = storage
            .categories(&CategoryListRequest { limit: 1, cursor })
            .unwrap();
        seen.extend(page.categories);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, categories);
    let changed_scope = PostRequest {
        category: categories[3].clone(),
        ..requests[2].clone()
    };
    assert_eq!(
        storage.post(&changed_scope).unwrap_err().code,
        BoardErrorCode::IdempotencyConflict
    );

    // Missing or retired rooms keep discussion history and exact receipt replay.
    if retire {
        use tmt_core::room::RoomRepository;
        // Rooms are core-owned; retire through the core store on the same file.
        let mut core = tmt_adapters::storage::Storage::open(&database).unwrap();
        core.retire_meeting_room(room_a, 1).unwrap();
        core.close().unwrap();
    } else {
        storage
            .connection()
            .unwrap()
            .execute("DELETE FROM office_meeting_rooms WHERE room_id=?", [room_a])
            .unwrap();
    }
    assert_eq!(storage.post(&requests[2]).unwrap(), receipts[2]);
    let before: (i64, i64) = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT revision,next_sequence FROM office_board_state",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let absent = PostRequest {
        operation_id: uuid::Uuid::new_v4().to_string(),
        ..requests[2].clone()
    };
    assert_eq!(
        storage.post(&absent).unwrap_err().code,
        BoardErrorCode::Invalid
    );
    let after = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT revision,next_sequence FROM office_board_state",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, after);
    storage.close().unwrap();
    let reopened = OfficeStore::open(&database).unwrap();
    assert_eq!(
        reopened.list(&list(categories[2].clone())).unwrap().threads[0]
            .entry
            .id,
        receipts[2].entry_id
    );
    assert_eq!(
        reopened.list(&list(categories[3].clone())).unwrap().threads[0]
            .entry
            .id,
        receipts[3].entry_id
    );
}

#[test]
fn mutations_replay_without_activity_and_soft_delete_without_body_receipts() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    let identity_id = "11111111-1111-4111-8111-111111111111";
    insert_identity(&storage, identity_id, "alice");
    let actor = Actor::Identity {
        identity_id: identity_id.into(),
        name: "stale name ignored".into(),
    };
    let operation = "22222222-2222-4222-8222-222222222222";
    let created = post(&mut storage, actor.clone(), operation);
    assert_eq!(post(&mut storage, actor.clone(), operation), created);
    let revision: u64 = storage
        .connection()
        .unwrap()
        .query_row("SELECT revision FROM office_board_state", [], |r| {
            r.get::<_, i64>(0).map(|v| v as u64)
        })
        .unwrap();
    assert_eq!(revision, 1);
    let reply = storage
        .reply(&ReplyRequest {
            thread_id: created.thread_id.clone(),
            actor: actor.clone(),
            body: "reply".into(),
            operation_id: "33333333-3333-4333-8333-333333333333".into(),
        })
        .unwrap();
    let invalid = storage
        .edit(&EditRequest {
            entry_id: reply.entry_id.clone(),
            actor: actor.clone(),
            title: None,
            body: Some("x".repeat(tmt_office_model::office_board::REPLY_BODY_MAX_BYTES + 1)),
            expected_revision: 1,
            operation_id: "55555555-5555-4555-8555-555555555555".into(),
        })
        .unwrap_err();
    assert_eq!(invalid.code, BoardErrorCode::Invalid);
    let preserved: String = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT body FROM office_board_entries WHERE id=?",
            [&reply.entry_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(preserved, "reply");
    let listed = storage
        .list(&ListRequest {
            category: Category::General,
            view: ListView::Updated,
            author: None,
            since_ms: None,
            limit: 1,
            cursor: None,
        })
        .unwrap();
    assert_eq!(listed.threads[0].reply_count, 1);
    assert_eq!(
        listed.threads[0].entry.author,
        Actor::Identity {
            identity_id: identity_id.into(),
            name: "alice".into()
        }
    );
    assert_eq!(post(&mut storage, actor.clone(), operation), created);
    assert_eq!(
        storage
            .post(&PostRequest {
                category: Category::General,
                actor: actor.clone(),
                title: "Hello".into(),
                body: "different".into(),
                operation_id: operation.into()
            })
            .unwrap_err()
            .code,
        BoardErrorCode::IdempotencyConflict
    );
    let deleted = storage
        .delete(&DeleteRequest {
            entry_id: reply.entry_id.clone(),
            actor: actor.clone(),
            moderate: false,
            expected_revision: 1,
            operation_id: "44444444-4444-4444-8444-444444444444".into(),
        })
        .unwrap();
    assert!(deleted.changed);
    let stored: (Option<String>, Option<String>) = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT title,body FROM office_board_entries WHERE id=?",
            [reply.entry_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, (None, None));
    let receipt_columns: String = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT group_concat(name,',') FROM pragma_table_info('office_board_operations')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!receipt_columns.contains("body"));
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms=1 WHERE id=?",
            [identity_id],
        )
        .unwrap();
    assert_eq!(
        storage
            .post(&PostRequest {
                category: Category::General,
                actor: actor.clone(),
                title: "Hello".into(),
                body: "Body\tline\n".into(),
                operation_id: operation.into()
            })
            .unwrap_err()
            .code,
        BoardErrorCode::Forbidden
    );
    let replacement = "66666666-6666-4666-8666-666666666666";
    insert_identity(&storage, replacement, "alice");
    assert_eq!(
        storage
            .edit(&EditRequest {
                entry_id: created.entry_id,
                actor: Actor::Identity {
                    identity_id: replacement.into(),
                    name: "alice".into()
                },
                title: Some("stolen".into()),
                body: None,
                expected_revision: 1,
                operation_id: "77777777-7777-4777-8777-777777777777".into()
            })
            .unwrap_err()
            .code,
        BoardErrorCode::Forbidden
    );
    storage.close().unwrap();
}

#[test]
fn cursors_go_stale_and_category_pages_are_bounded() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state.db");
    let mut storage = OfficeStore::open(&database).unwrap();
    let owner = local_owner_actor(&mut storage).unwrap();
    for index in 0..55 {
        storage
            .post(&PostRequest {
                category: Category::Repository(format!("host/Repo{index:02}")),
                actor: owner.clone(),
                title: "t".into(),
                body: "b".into(),
                operation_id: uuid::Uuid::new_v4().to_string(),
            })
            .unwrap();
    }
    let first = storage
        .categories(&CategoryListRequest {
            limit: 50,
            cursor: None,
        })
        .unwrap();
    assert_eq!(first.categories.len(), 50);
    assert!(matches!(first.categories[0], Category::General));
    let second = storage
        .categories(&CategoryListRequest {
            limit: 50,
            cursor: first.next_cursor.clone(),
        })
        .unwrap();
    assert_eq!(second.categories.len(), 6);
    storage
        .post(&PostRequest {
            category: Category::General,
            actor: owner,
            title: "t".into(),
            body: "b".into(),
            operation_id: uuid::Uuid::new_v4().to_string(),
        })
        .unwrap();
    let stale = storage
        .categories(&CategoryListRequest {
            limit: 50,
            cursor: first.next_cursor,
        })
        .unwrap_err();
    assert_eq!(stale.code, BoardErrorCode::CursorStale);
    storage.close().unwrap();
    let mut reopened = OfficeStore::open(database).unwrap();
    let general = reopened
        .categories(&CategoryListRequest {
            limit: 1,
            cursor: None,
        })
        .unwrap();
    assert_eq!(general.categories, vec![Category::General]);
    let repository = reopened
        .categories(&CategoryListRequest {
            limit: 1,
            cursor: general.next_cursor,
        })
        .unwrap();
    assert_eq!(
        repository.categories,
        vec![Category::Repository("host/Repo00".into())]
    );
    let plan: String = reopened
        .connection()
        .unwrap()
        .query_row(
            &format!("EXPLAIN QUERY PLAN {CATEGORIES_SQL}"),
            params!["", "", 51],
            |row| row.get(3),
        )
        .unwrap();
    assert!(plan.contains("office_board_categories"), "{plan}");
    reopened.close().unwrap();
}

#[test]
fn maximum_repository_and_filters_emit_reusable_compact_cursors() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    let identity_id = "11111111-1111-4111-8111-111111111111";
    insert_identity(&storage, identity_id, "alice");
    let actor = Actor::Identity {
        identity_id: identity_id.into(),
        name: "alice".into(),
    };
    let repository_id = format!("h/{}", "r".repeat(2046));
    assert_eq!(repository_id.len(), 2048);
    for index in 0..2 {
        storage
            .post(&PostRequest {
                category: Category::Repository(repository_id.clone()),
                actor: actor.clone(),
                title: format!("thread {index}"),
                body: "body".into(),
                operation_id: uuid::Uuid::new_v4().to_string(),
            })
            .unwrap();
    }
    let second_repository_id = format!("h/{}", "s".repeat(2046));
    storage
        .post(&PostRequest {
            category: Category::Repository(second_repository_id.clone()),
            actor: actor.clone(),
            title: "other category".into(),
            body: "body".into(),
            operation_id: uuid::Uuid::new_v4().to_string(),
        })
        .unwrap();
    let request = ListRequest {
        category: Category::Repository(repository_id),
        view: ListView::Updated,
        author: Some(AuthorFilter::Identity(identity_id.into())),
        since_ms: Some(0),
        limit: 1,
        cursor: None,
    };
    let first = storage.list(&request).unwrap();
    let cursor = first.next_cursor.unwrap();
    assert!(cursor.len() < tmt_office_model::office_board::CURSOR_MAX_BYTES);
    assert_eq!(
        storage
            .list(&ListRequest {
                cursor: Some(cursor.clone()),
                ..request.clone()
            })
            .unwrap()
            .threads
            .len(),
        1
    );
    let mut forged = cursor.split('.').map(str::to_owned).collect::<Vec<_>>();
    let replacement = if forged[4].starts_with('0') { "1" } else { "0" };
    forged[4].replace_range(..1, replacement);
    let forged = forged.join(".");
    assert_eq!(
        storage
            .list(&ListRequest {
                cursor: Some(forged),
                ..request
            })
            .unwrap_err()
            .code,
        BoardErrorCode::CursorInvalid
    );

    let general = storage
        .categories(&CategoryListRequest {
            limit: 1,
            cursor: None,
        })
        .unwrap();
    let first_repository = storage
        .categories(&CategoryListRequest {
            limit: 1,
            cursor: general.next_cursor,
        })
        .unwrap();
    let category_cursor = first_repository.next_cursor.unwrap();
    assert!(category_cursor.len() < tmt_office_model::office_board::CURSOR_MAX_BYTES);
    assert_eq!(
        storage
            .categories(&CategoryListRequest {
                limit: 1,
                cursor: Some(category_cursor),
            })
            .unwrap()
            .categories,
        vec![Category::Repository(second_repository_id)]
    );
    storage.close().unwrap();
}

#[test]
fn owner_moderation_is_explicit_and_never_grants_editing() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    insert_identity(&storage, id, "alice");
    let entry = post(
        &mut storage,
        Actor::Identity {
            identity_id: id.into(),
            name: "alice".into(),
        },
        "22222222-2222-4222-8222-222222222222",
    );
    let owner = local_owner_actor(&mut storage).unwrap();
    assert_eq!(
        storage
            .edit(&EditRequest {
                entry_id: entry.entry_id.clone(),
                actor: owner.clone(),
                title: Some("owner rewrite".into()),
                body: None,
                expected_revision: 1,
                operation_id: "33333333-3333-4333-8333-333333333333".into()
            })
            .unwrap_err()
            .code,
        BoardErrorCode::Forbidden
    );
    assert_eq!(
        storage
            .delete(&DeleteRequest {
                entry_id: entry.entry_id.clone(),
                actor: owner.clone(),
                moderate: false,
                expected_revision: 1,
                operation_id: "44444444-4444-4444-8444-444444444444".into()
            })
            .unwrap_err()
            .code,
        BoardErrorCode::Forbidden
    );
    let receipt = storage
        .delete(&DeleteRequest {
            entry_id: entry.entry_id,
            actor: owner,
            moderate: true,
            expected_revision: 1,
            operation_id: "55555555-5555-4555-8555-555555555555".into(),
        })
        .unwrap();
    assert!(receipt.moderated);
    storage.close().unwrap();
}

#[test]
fn reply_limit_is_exact_and_rejection_has_no_partial_write() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    let owner = local_owner_actor(&mut storage).unwrap();
    let root = post(
        &mut storage,
        owner.clone(),
        "11111111-1111-4111-8111-111111111111",
    );
    for _ in 0..REPLY_LIMIT {
        storage
            .reply(&ReplyRequest {
                thread_id: root.thread_id.clone(),
                actor: owner.clone(),
                body: "r".into(),
                operation_id: uuid::Uuid::new_v4().to_string(),
            })
            .unwrap();
    }
    let before:(i64,i64)=storage.connection().unwrap().query_row("SELECT (SELECT count(*) FROM office_board_entries WHERE is_root=0),revision FROM office_board_state",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        storage
            .reply(&ReplyRequest {
                thread_id: root.thread_id,
                actor: owner,
                body: "overflow".into(),
                operation_id: uuid::Uuid::new_v4().to_string()
            })
            .unwrap_err()
            .code,
        BoardErrorCode::Invalid
    );
    let after:(i64,i64)=storage.connection().unwrap().query_row("SELECT (SELECT count(*) FROM office_board_entries WHERE is_root=0),revision FROM office_board_state",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(before, after);
    assert_eq!(before.0, REPLY_LIMIT as i64);
    storage.close().unwrap();
}

#[test]
fn corrupted_create_edit_and_delete_receipts_fail_closed_without_panic() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    let owner = local_owner_actor(&mut storage).unwrap();
    let op = "11111111-1111-4111-8111-111111111111";
    let created = post(&mut storage, owner.clone(), op);
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE office_board_operations SET thread_id=NULL WHERE operation_id=?",
            [op],
        )
        .unwrap();
    let error = storage
        .post(&PostRequest {
            category: Category::General,
            actor: owner.clone(),
            title: "Hello".into(),
            body: "Body\tline\n".into(),
            operation_id: op.into(),
        })
        .unwrap_err();
    assert_eq!(error.code, BoardErrorCode::Storage);
    assert_eq!(error.source.unwrap().code, StorageErrorCode::Corrupt);
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE office_board_operations SET thread_id=?,created=1 WHERE operation_id=?",
            params![created.thread_id, op],
        )
        .unwrap();
    let edit_request = EditRequest {
        entry_id: created.entry_id.clone(),
        actor: owner.clone(),
        title: Some("edit".into()),
        body: None,
        expected_revision: 1,
        operation_id: "22222222-2222-4222-8222-222222222222".into(),
    };
    let edit = storage.edit(&edit_request).unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE office_board_operations SET changed=NULL WHERE operation_id=?",
            [&edit.operation_id],
        )
        .unwrap();
    let error = storage.edit(&edit_request).unwrap_err();
    assert_eq!(error.code, BoardErrorCode::Storage);
    assert_eq!(error.source.unwrap().code, StorageErrorCode::Corrupt);
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE office_board_operations SET changed=1 WHERE operation_id=?",
            [&edit.operation_id],
        )
        .unwrap();
    let delete_request = DeleteRequest {
        entry_id: created.entry_id,
        actor: owner,
        moderate: false,
        expected_revision: edit.revision,
        operation_id: "33333333-3333-4333-8333-333333333333".into(),
    };
    let deleted = storage.delete(&delete_request).unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE office_board_operations SET deleted=NULL WHERE operation_id=?",
            [&deleted.operation_id],
        )
        .unwrap();
    let error = storage.delete(&delete_request).unwrap_err();
    assert_eq!(error.code, BoardErrorCode::Storage);
    assert_eq!(error.source.unwrap().code, StorageErrorCode::Corrupt);
    storage.close().unwrap();
}

#[test]
fn list_and_reply_cursors_continue_then_stale_only_after_changed_mutation() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    let owner = local_owner_actor(&mut storage).unwrap();
    let mut roots = Vec::new();
    for _ in 0..3 {
        roots.push(post(
            &mut storage,
            owner.clone(),
            &uuid::Uuid::new_v4().to_string(),
        ));
    }
    let request = ListRequest {
        category: Category::General,
        view: ListView::Recent,
        author: None,
        since_ms: None,
        limit: 2,
        cursor: None,
    };
    let first = storage.list(&request).unwrap();
    assert_eq!(first.threads.len(), 2);
    let second = storage
        .list(&ListRequest {
            cursor: first.next_cursor.clone(),
            ..request.clone()
        })
        .unwrap();
    assert_eq!(second.threads.len(), 1);
    assert_ne!(first.threads[0].entry.id, second.threads[0].entry.id);
    let root = &roots[0];
    let mut replay_request = None;
    for _ in 0..3 {
        let request = ReplyRequest {
            thread_id: root.thread_id.clone(),
            actor: owner.clone(),
            body: "r".into(),
            operation_id: uuid::Uuid::new_v4().to_string(),
        };
        storage.reply(&request).unwrap();
        replay_request = Some(request);
    }
    let show = ShowRequest {
        thread_id: root.thread_id.clone(),
        reply_limit: 2,
        reply_cursor: None,
    };
    let replies = storage.show(&show).unwrap();
    assert_eq!(replies.replies.len(), 2);
    assert_eq!(
        storage
            .show(&ShowRequest {
                reply_cursor: replies.next_cursor.clone(),
                ..show.clone()
            })
            .unwrap()
            .replies
            .len(),
        1
    );
    let stable_list_cursor = storage.list(&request).unwrap().next_cursor;
    let before_replay_revision = replies.board_revision;
    storage.reply(&replay_request.unwrap()).unwrap();
    assert_eq!(
        storage.show(&show).unwrap().board_revision,
        before_replay_revision
    );
    assert!(
        storage
            .list(&ListRequest {
                cursor: stable_list_cursor.clone(),
                ..request.clone()
            })
            .is_ok()
    );
    let noop = storage
        .edit(&EditRequest {
            entry_id: root.entry_id.clone(),
            actor: owner.clone(),
            title: Some("Hello".into()),
            body: None,
            expected_revision: 1,
            operation_id: uuid::Uuid::new_v4().to_string(),
        })
        .unwrap();
    assert!(!noop.changed);
    assert!(
        storage
            .list(&ListRequest {
                cursor: stable_list_cursor.clone(),
                ..request.clone()
            })
            .is_ok()
    );
    storage
        .edit(&EditRequest {
            entry_id: root.entry_id.clone(),
            actor: owner,
            title: Some("changed".into()),
            body: None,
            expected_revision: 1,
            operation_id: uuid::Uuid::new_v4().to_string(),
        })
        .unwrap();
    assert_eq!(
        storage
            .list(&ListRequest {
                cursor: stable_list_cursor,
                ..request
            })
            .unwrap_err()
            .code,
        BoardErrorCode::CursorStale
    );
    assert_eq!(
        storage
            .show(&ShowRequest {
                reply_cursor: replies.next_cursor,
                ..show
            })
            .unwrap_err()
            .code,
        BoardErrorCode::CursorStale
    );
    storage.close().unwrap();
}

#[test]
fn stale_revision_and_lost_response_replays_preserve_state_and_side_effect_tables() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    let owner = local_owner_actor(&mut storage).unwrap();
    let op = "11111111-1111-4111-8111-111111111111";
    let original = post(&mut storage, owner.clone(), op);
    let edit = storage
        .edit(&EditRequest {
            entry_id: original.entry_id.clone(),
            actor: owner.clone(),
            title: Some("v2".into()),
            body: None,
            expected_revision: 1,
            operation_id: "22222222-2222-4222-8222-222222222222".into(),
        })
        .unwrap();
    let stale = storage
        .edit(&EditRequest {
            entry_id: original.entry_id.clone(),
            actor: owner.clone(),
            title: Some("lost".into()),
            body: None,
            expected_revision: 1,
            operation_id: "33333333-3333-4333-8333-333333333333".into(),
        })
        .unwrap_err();
    assert_eq!(stale.code, BoardErrorCode::RevisionConflict);
    let shown = storage
        .show(&ShowRequest {
            thread_id: original.thread_id.clone(),
            reply_limit: 20,
            reply_cursor: None,
        })
        .unwrap();
    assert_eq!(shown.thread.title.as_deref(), Some("v2"));
    assert_eq!(shown.thread.revision, edit.revision);
    let edit_replay = EditRequest {
        entry_id: original.entry_id.clone(),
        actor: owner.clone(),
        title: Some("v2".into()),
        body: None,
        expected_revision: 1,
        operation_id: "22222222-2222-4222-8222-222222222222".into(),
    };
    let delete_request = DeleteRequest {
        entry_id: original.entry_id.clone(),
        actor: owner.clone(),
        moderate: false,
        expected_revision: edit.revision,
        operation_id: "44444444-4444-4444-8444-444444444444".into(),
    };
    let deleted = storage.delete(&delete_request).unwrap();
    assert_eq!(storage.edit(&edit_replay).unwrap(), edit);
    assert_eq!(storage.delete(&delete_request).unwrap(), deleted);
    assert_eq!(post(&mut storage, owner, op), original);
    for table in [
        "request_attempts",
        "request_responses",
        "request_recipient_attention_identities",
        "identity_hooks",
    ] {
        let sql = format!("SELECT count(*) FROM {table}");
        let count: i64 = storage
            .connection()
            .unwrap()
            .query_row(&sql, [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "unexpected side effect in {table}");
    }
    storage.close().unwrap();
}

#[test]
fn board_query_plans_use_recent_updated_and_reply_indexes() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    for (view, index) in [
        (ListView::Recent, "office_board_recent"),
        (ListView::Updated, "office_board_updated"),
    ] {
        let sql = format!("EXPLAIN QUERY PLAN {}", list_sql(view));
        let plan = storage
            .connection()
            .unwrap()
            .prepare(&sql)
            .unwrap()
            .query_map(
                params![
                    "general",
                    Option::<String>::None,
                    Option::<String>::None,
                    Option::<String>::None,
                    Option::<String>::None,
                    Option::<String>::None,
                    0_i64,
                    0_i64,
                    i64::MAX,
                    i64::MAX,
                    "~",
                    21_i64
                ],
                |r| r.get::<_, String>(3),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join(" ");
        assert!(plan.contains(index), "{plan}");
        assert!(!plan.contains("SCAN office_board_entries"), "{plan}");
    }
    let sql = format!("EXPLAIN QUERY PLAN {REPLIES_SQL}");
    let plan = storage
        .connection()
        .unwrap()
        .prepare(&sql)
        .unwrap()
        .query_map(
            params!["11111111-1111-4111-8111-111111111111", 0_i64, 21_i64],
            |r| r.get::<_, String>(3),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join(" ");
    assert!(plan.contains("office_board_replies"), "{plan}");
    assert!(!plan.contains("SCAN office_board_entries"), "{plan}");
    storage.close().unwrap();
}

#[test]
fn independently_committed_replies_have_exact_count_and_monotonic_revision() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state.db");
    let mut first = OfficeStore::open(&database).unwrap();
    let owner = local_owner_actor(&mut first).unwrap();
    let root = post(
        &mut first,
        owner.clone(),
        "11111111-1111-4111-8111-111111111111",
    );
    first.close().unwrap();
    let mut second = OfficeStore::open(&database).unwrap();
    second
        .reply(&ReplyRequest {
            thread_id: root.thread_id.clone(),
            actor: owner.clone(),
            body: "one".into(),
            operation_id: "22222222-2222-4222-8222-222222222222".into(),
        })
        .unwrap();
    let first_revision: i64 = second
        .connection()
        .unwrap()
        .query_row("SELECT revision FROM office_board_state", [], |r| r.get(0))
        .unwrap();
    second.close().unwrap();
    let mut third = OfficeStore::open(&database).unwrap();
    third
        .reply(&ReplyRequest {
            thread_id: root.thread_id,
            actor: owner,
            body: "two".into(),
            operation_id: "33333333-3333-4333-8333-333333333333".into(),
        })
        .unwrap();
    let second_revision: i64 = third
        .connection()
        .unwrap()
        .query_row("SELECT revision FROM office_board_state", [], |r| r.get(0))
        .unwrap();
    assert!(second_revision > first_revision);
    let listed = third
        .list(&ListRequest {
            category: Category::General,
            view: ListView::Updated,
            author: None,
            since_ms: None,
            limit: 20,
            cursor: None,
        })
        .unwrap();
    assert_eq!(listed.threads[0].reply_count, 2);
    third.close().unwrap();
}

#[test]
fn concurrent_reply_limit_and_stale_edit_are_serialized_without_lost_updates() {
    use std::sync::{Arc, Barrier};
    let directory = TestDirectory::new();
    let database = directory.path.join("state.db");
    let mut setup = OfficeStore::open(&database).unwrap();
    let owner = local_owner_actor(&mut setup).unwrap();
    let root = post(
        &mut setup,
        owner.clone(),
        "11111111-1111-4111-8111-111111111111",
    );
    for _ in 0..(REPLY_LIMIT - 1) {
        setup
            .reply(&ReplyRequest {
                thread_id: root.thread_id.clone(),
                actor: owner.clone(),
                body: "r".into(),
                operation_id: uuid::Uuid::new_v4().to_string(),
            })
            .unwrap();
    }
    let before: i64 = setup
        .connection()
        .unwrap()
        .query_row("SELECT revision FROM office_board_state", [], |r| r.get(0))
        .unwrap();
    setup.close().unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for body in ["a", "b"] {
        let db = database.clone();
        let gate = barrier.clone();
        let actor = owner.clone();
        let thread = root.thread_id.clone();
        handles.push(std::thread::spawn(move || {
            let mut storage = OfficeStore::open(db).unwrap();
            gate.wait();
            storage
                .reply(&ReplyRequest {
                    thread_id: thread,
                    actor,
                    body: body.into(),
                    operation_id: uuid::Uuid::new_v4().to_string(),
                })
                .map(|_| None)
                .unwrap_or_else(|e| Some(e.code))
        }));
    }
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|v| v.is_none()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|v| **v == Some(BoardErrorCode::Invalid))
            .count(),
        1
    );
    let mut storage = OfficeStore::open(&database).unwrap();
    let state:(i64,i64)=storage.connection().unwrap().query_row("SELECT (SELECT count(*) FROM office_board_entries WHERE is_root=0),revision FROM office_board_state",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, (REPLY_LIMIT as i64, before + 1));
    let edit_root = post(
        &mut storage,
        owner.clone(),
        "22222222-2222-4222-8222-222222222222",
    );
    storage.close().unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for title in ["winner-a", "winner-b"] {
        let db = database.clone();
        let gate = barrier.clone();
        let actor = owner.clone();
        let entry = edit_root.entry_id.clone();
        handles.push(std::thread::spawn(move || {
            let mut storage = OfficeStore::open(db).unwrap();
            gate.wait();
            storage
                .edit(&EditRequest {
                    entry_id: entry,
                    actor,
                    title: Some(title.into()),
                    body: None,
                    expected_revision: 1,
                    operation_id: uuid::Uuid::new_v4().to_string(),
                })
                .map(|_| None)
                .unwrap_or_else(|e| Some(e.code))
        }));
    }
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|v| v.is_none()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|v| **v == Some(BoardErrorCode::RevisionConflict))
            .count(),
        1
    );
    let mut storage = OfficeStore::open(database).unwrap();
    let shown = storage
        .show(&ShowRequest {
            thread_id: edit_root.thread_id,
            reply_limit: 20,
            reply_cursor: None,
        })
        .unwrap();
    assert_eq!(shown.thread.revision, 2);
    assert!(matches!(
        shown.thread.title.as_deref(),
        Some("winner-a" | "winner-b")
    ));
    storage.close().unwrap();
}

#[test]
fn references_retired_inside_the_preflight_window_commit_like_post_then_retire() {
    let directory = TestDirectory::new();
    let path = directory.path.join("race.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    let identity_id = "33333333-1111-4111-8111-111111111111";
    insert_identity(&storage, identity_id, "alice");
    let actor = Actor::Identity {
        identity_id: identity_id.into(),
        name: "alice".into(),
    };
    crate::test_support::at_next_preflight_execute(
        &path,
        "UPDATE identities SET retired_at_ms=5 WHERE id=?",
        identity_id,
    );
    let receipt = post(
        &mut storage,
        actor.clone(),
        "44444444-1111-4111-8111-111111111111",
    );
    let author: String = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT author_name FROM office_board_entries WHERE id=?",
            [&receipt.entry_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(author, "alice");
    // Already retired at preflight: the same policy error and no write.
    let rejected = storage
        .post(&PostRequest {
            category: Category::General,
            actor,
            title: "Later".into(),
            body: "Body".into(),
            operation_id: "55555555-1111-4111-8111-111111111111".into(),
        })
        .unwrap_err();
    assert_eq!(rejected.code, BoardErrorCode::Forbidden);

    let room = "66666666-1111-4111-8111-111111111111";
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO office_meeting_rooms (room_id,name,revision) VALUES (?,'Review',1)",
            [room],
        )
        .unwrap();
    let owner = local_owner_actor(&mut storage).unwrap();
    let request = PostRequest {
        category: Category::Room(room.into()),
        actor: owner,
        title: "Room".into(),
        body: "Body".into(),
        operation_id: "77777777-1111-4111-8111-111111111111".into(),
    };
    crate::test_support::at_next_preflight_execute(
        &path,
        "UPDATE office_meeting_rooms SET retired=1,revision=revision+1 WHERE room_id=?",
        room,
    );
    let created = storage.post(&request).unwrap();
    // The retained thread replays exactly; a new thread in the retired room is rejected.
    assert_eq!(storage.post(&request).unwrap(), created);
    let mut later = request.clone();
    later.operation_id = "88888888-1111-4111-8111-111111111111".into();
    assert_eq!(
        storage.post(&later).unwrap_err().code,
        BoardErrorCode::Invalid
    );
}
