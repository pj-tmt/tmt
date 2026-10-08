use super::*;
use crate::checklist::test_support::{CHECKLIST, Fixture, ITEM, OTHER, ROOM, WORKER, id};

fn parsed(words: &[&str]) -> Result<ChecklistCommand, Error> {
    let matches = grammar()
        .try_get_matches_from(std::iter::once("checklist").chain(words.iter().copied()))
        .unwrap();
    parse(&matches)
}
fn args<'a>(action: &'a str, extra: &'a [&'a str]) -> Vec<&'a str> {
    let mut words = vec![
        action,
        "--room",
        ROOM,
        "--checklist",
        CHECKLIST,
        "--item",
        ITEM,
    ];
    words.extend_from_slice(extra);
    words
}
fn invoke(f: &Fixture, words: &[&str]) -> Outcome {
    run(&f.core, &f.config, parsed(words).unwrap())
}

#[test]
fn canonical_listing_alias_filters_and_completion_share_the_real_grammar() {
    for name in ["ls", "list"] {
        let input = parsed(&[
            name,
            "--room",
            ROOM,
            "--include-archived",
            "--completion",
            "complete",
            "--assignee",
            WORKER,
        ])
        .unwrap();
        assert_eq!(input.action, "list");
        assert_eq!(input.room_id, id(ROOM));
        let Operation::List(filter) = input.operation else {
            panic!("list")
        };
        assert!(filter.include_archived);
        assert_eq!(filter.completion, Some(Completion::Complete));
        assert_eq!(filter.assignee, Some(id(WORKER)));
    }
    let helping = crate::complete(&["help".into(), "squad".into(), "checklist".into(), "".into()]);
    assert!(helping.contains(&"ls".into()));
    assert!(!helping.contains(&"list".into()));
    assert_eq!(
        crate::complete(&[
            "squad".into(),
            "checklist".into(),
            "delete".into(),
            "--confirm".into()
        ]),
        vec!["--confirm-item", "--confirm-revision"]
    );
    assert!(
        tmt_cli_style::audit::list_spelling_report(&crate::grammar(), &["tmt", "ops"]).is_empty()
    );
}

#[test]
fn all_requests_preserve_exact_targets_revisions_and_supplied_fields() {
    let created = parsed(&args(
        "create",
        &["Authored title", "--expect-inventory", "absent"],
    ))
    .unwrap();
    assert_eq!(created.action, "create");
    let Operation::Apply(created) = created.operation else {
        panic!("create")
    };
    assert_eq!(
        created,
        Request {
            room_id: id(ROOM),
            checklist_id: id(CHECKLIST),
            action: Action::Create {
                item_id: id(ITEM),
                inventory: InventoryExpectation::Absent,
                content: ItemContent {
                    title: "Authored title".into(),
                    body: None,
                    reference: None
                },
                assignee: None,
            }
        }
    );
    let explicit = parsed(&args(
        "create",
        &[
            "Title",
            "--expect-inventory",
            "2",
            "--body",
            "",
            "--reference",
            "https://example.org",
            "--assignee",
            WORKER,
        ],
    ))
    .unwrap();
    let Operation::Apply(Request {
        action:
            Action::Create {
                inventory,
                content,
                assignee,
                ..
            },
        ..
    }) = explicit.operation
    else {
        panic!("create")
    };
    assert_eq!(inventory, InventoryExpectation::Revision(2));
    assert_eq!(content.body, None);
    assert_eq!(content.reference.as_deref(), Some("https://example.org"));
    assert_eq!(assignee, Some(id(WORKER)));
    let edited = parsed(&args(
        "edit",
        &[
            "--expect-revision",
            "7",
            "--title",
            "Draft",
            "--clear-body",
            "--clear-reference",
        ],
    ))
    .unwrap();
    let Operation::Apply(edited) = edited.operation else {
        panic!("edit")
    };
    assert_eq!(
        edited.action,
        Action::Item {
            item_id: id(ITEM),
            revision: 7,
            mutation: Mutation::Edit(Edit {
                title: Some("Draft".into()),
                body: Some(None),
                reference: Some(None),
            })
        }
    );
    for (name, mutation) in [
        ("unassign", Mutation::Unassign),
        ("complete", Mutation::Complete),
        ("reopen", Mutation::Reopen),
        ("archive", Mutation::Archive),
        ("restore", Mutation::Restore),
    ] {
        let input = parsed(&args(name, &["--expect-revision", "7"])).unwrap();
        let Operation::Apply(request) = input.operation else {
            panic!("item")
        };
        assert_eq!(input.action, name);
        assert_eq!(
            request.action,
            Action::Item {
                item_id: id(ITEM),
                revision: 7,
                mutation
            }
        );
    }
    let input = parsed(&args(
        "assign",
        &["--expect-revision", "7", "--assignee", WORKER],
    ))
    .unwrap();
    let Operation::Apply(request) = input.operation else {
        panic!("assign")
    };
    assert_eq!(
        request.action,
        Action::Item {
            item_id: id(ITEM),
            revision: 7,
            mutation: Mutation::Assign(id(WORKER))
        }
    );
    let input = parsed(&args(
        "delete",
        &[
            "--expect-revision",
            "7",
            "--expect-inventory",
            "3",
            "--confirm-item",
            ITEM,
            "--confirm-revision",
            "7",
        ],
    ))
    .unwrap();
    let Operation::Apply(request) = input.operation else {
        panic!("delete")
    };
    assert_eq!(
        request.action,
        Action::Item {
            item_id: id(ITEM),
            revision: 7,
            mutation: Mutation::Delete {
                inventory_revision: 3,
                confirmation: Confirmation {
                    item_id: id(ITEM),
                    item_revision: 7
                }
            }
        }
    );
    let input = parsed(&[
        "reorder",
        "--room",
        ROOM,
        "--checklist",
        CHECKLIST,
        "--expect-inventory",
        "2",
        "--order",
        &format!("[\"{OTHER}\",\"{ITEM}\"]"),
    ])
    .unwrap();
    let Operation::Apply(request) = input.operation else {
        panic!("reorder")
    };
    assert_eq!(
        request.action,
        Action::Reorder {
            inventory_revision: 2,
            order: vec![id(OTHER), id(ITEM)]
        }
    );
    let show = parsed(&args("show", &[])).unwrap();
    assert!(
        matches!(show.operation, Operation::Show { checklist_id, item_id } if checklist_id==id(CHECKLIST) && item_id==id(ITEM))
    );
}

#[test]
fn operation_inputs_are_typed_errors_and_grammar_errors_stay_usage_errors() {
    let cases = [
        vec!["ls", "--room", "product"],
        vec!["ls", "--room", ROOM, "--assignee", "Worker"],
        args("complete", &["--expect-revision", "0"]),
        args("complete", &["--expect-revision", "-1"]),
        args("complete", &["--expect-revision", "+1"]),
        args("complete", &["--expect-revision", "1.0"]),
        args("complete", &["--expect-revision", "18446744073709551616"]),
        args("create", &[" ", "--expect-inventory", "absent"]),
        args(
            "create",
            &["Title", "--expect-inventory", "absent", "--reference", ""],
        ),
        args(
            "create",
            &[
                "Title",
                "--expect-inventory",
                "absent",
                "--reference",
                "file:///tmp/secret",
            ],
        ),
        args("edit", &["--expect-revision", "1", "--body", "\u{1b}"]),
        args(
            "delete",
            &[
                "--expect-revision",
                "1",
                "--expect-inventory",
                "1",
                "--confirm-item",
                OTHER,
                "--confirm-revision",
                "1",
            ],
        ),
        args(
            "delete",
            &[
                "--expect-revision",
                "1",
                "--expect-inventory",
                "1",
                "--confirm-item",
                ITEM,
                "--confirm-revision",
                "2",
            ],
        ),
        vec![
            "reorder",
            "--room",
            ROOM,
            "--checklist",
            CHECKLIST,
            "--expect-inventory",
            "1",
            "--order",
            "not JSON",
        ],
        vec![
            "reorder",
            "--room",
            ROOM,
            "--checklist",
            CHECKLIST,
            "--expect-inventory",
            "1",
            "--order",
            "[1]",
        ],
    ];
    for words in cases {
        let error = match parsed(&words) {
            Err(error) => error,
            Ok(_) => panic!("accepted {words:?}"),
        };
        assert_eq!(error.code, Code::InputInvalid, "{words:?}");
        assert!(error.current.is_none());
    }
    for words in [
        vec!["ls"],
        vec!["unknown"],
        vec!["ls", "--room", ROOM, "--identity", WORKER],
        vec!["ls", "--room", ROOM, "--completion", "closed"],
        args("edit", &["--expect-revision", "1"]),
        args(
            "edit",
            &["--expect-revision", "1", "--body", "x", "--clear-body"],
        ),
        args(
            "edit",
            &[
                "--expect-revision",
                "1",
                "--reference",
                "https://example.org",
                "--clear-reference",
            ],
        ),
    ] {
        let error = grammar()
            .try_get_matches_from(std::iter::once("checklist").chain(words))
            .unwrap_err();
        assert!(error.use_stderr());
        assert!(!matches!(
            error.kind(),
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
        ));
    }
}

#[test]
fn admitted_service_result_has_literal_nulls_current_labels_and_distinct_empty_states() {
    let f = Fixture::new();
    let absent = invoke(&f, &["ls", "--room", ROOM]);
    assert!(absent.complete);
    assert_eq!(
        absent.document,
        json!({"action":"list","totalCount":0,"matchedCount":0,"current":{"room":{"id":ROOM,"name":"product","available":true,"manager":false},"checklistId":null,"inventoryRevision":null,"items":[],"deletion":null}})
    );
    assert!(!f.directory().exists());
    let assigned = invoke(
        &f,
        &args(
            "create",
            &[
                "Title",
                "--expect-inventory",
                "absent",
                "--assignee",
                WORKER,
            ],
        ),
    );
    assert!(!assigned.complete);
    assert_eq!(assigned.document["error"]["code"], "CHECKLIST_FORBIDDEN");
    assert!(assigned.document["error"].get("current").is_none());
    assert!(!f.directory().exists());
    let created = invoke(
        &f,
        &args("create", &["Title", "--expect-inventory", "absent"]),
    );
    assert!(created.complete);
    assert_eq!(
        created.document,
        json!({"action":"create","changed":true,"itemId":ITEM,"itemRevision":1,"current":{"room":{"id":ROOM,"name":"product","available":true,"manager":false},"checklistId":CHECKLIST,"inventoryRevision":1,"items":[{"id":ITEM,"revision":1,"title":"Title","body":null,"reference":null,"assignee":null,"completion":"open","archived":false}],"deletion":null}})
    );
    let filtered = invoke(&f, &["list", "--room", ROOM, "--completion", "complete"]);
    assert_eq!(filtered.document["totalCount"], 1);
    assert_eq!(filtered.document["matchedCount"], 0);
    assert!(text_output(&filtered.document, Terminal::PLAIN).contains("not empty"));
    assert!(text_output(&absent.document, Terminal::PLAIN).contains("No checklist exists"));
    f.manager();
    let assigned = invoke(
        &f,
        &args("assign", &["--expect-revision", "1", "--assignee", WORKER]),
    );
    assert_eq!(
        assigned.document["current"]["items"][0]["assignee"],
        json!({"id":WORKER,"label":"Worker","available":true})
    );
    f.change(|m| m["retired"] = json!([WORKER]));
    let listed = invoke(&f, &["ls", "--room", ROOM]);
    assert_eq!(
        listed.document["current"]["items"][0]["assignee"],
        json!({"id":WORKER,"label":"Worker","available":false})
    );
}

#[test]
fn no_op_conflict_denial_tombstone_and_unknown_preserve_scoped_failure_projection() {
    let f = Fixture::new();
    f.manager();
    assert!(
        invoke(
            &f,
            &args("create", &["Title", "--expect-inventory", "absent"])
        )
        .complete
    );
    let unchanged = invoke(&f, &args("reopen", &["--expect-revision", "1"]));
    assert_eq!(unchanged.document["changed"], false);
    assert_eq!(unchanged.document["itemRevision"], 1);
    let complete = invoke(&f, &args("complete", &["--expect-revision", "1"]));
    assert_eq!(complete.document["itemRevision"], 2);
    let conflict = invoke(&f, &args("complete", &["--expect-revision", "1"]));
    assert!(!conflict.complete);
    assert_eq!(conflict.document["error"]["code"], "CHECKLIST_CONFLICT");
    assert_eq!(
        conflict.document["error"]["current"]["items"][0]["revision"],
        2
    );
    assert!(text_output(&conflict.document, Terminal::PLAIN).is_empty());
    assert!(
        failure(&conflict.document)[0]
            .0
            .contains(&format!("Item {ITEM} revision 2"))
    );
    f.change(|m| m["caller"] = json!(WORKER));
    let forbidden = invoke(&f, &args("archive", &["--expect-revision", "2"]));
    assert_eq!(forbidden.document["error"]["code"], "CHECKLIST_FORBIDDEN");
    assert!(forbidden.document["error"].get("current").is_none());
    assert!(!failure(&forbidden.document)[0].0.contains(CHECKLIST));
    f.manager();
    // The adapter's original typed Unknown is never rewritten by later observations.
    // Delivered service/store fault tests own actual uncertain-publication proof.
    let unknown = finish(Err(Error {
        code: Code::OutcomeUnknown,
        message: "Original update outcome remains unknown.".into(),
        current: None,
    }));
    let original = unknown.document.clone();
    assert!(text_output(&unknown.document, Terminal::PLAIN).is_empty());
    assert!(
        failure(&unknown.document)[0]
            .1
            .as_ref()
            .unwrap()
            .contains("remains unknown")
    );
    let deleted = invoke(
        &f,
        &args(
            "delete",
            &[
                "--expect-revision",
                "2",
                "--expect-inventory",
                "1",
                "--confirm-item",
                ITEM,
                "--confirm-revision",
                "2",
            ],
        ),
    );
    assert!(deleted.complete);
    let shown = invoke(&f, &args("show", &[]));
    assert_eq!(shown.document["error"]["code"], "CHECKLIST_DELETED");
    assert_eq!(
        shown.document["error"]["current"]["deletion"],
        json!({"itemId":ITEM,"deletionRevision":3})
    );
    assert_eq!(unknown.document, original);
    assert!(!unknown.complete);
    let empty = invoke(&f, &["ls", "--room", ROOM]);
    assert_eq!(empty.document["totalCount"], 0);
    assert!(!empty.document["current"]["checklistId"].is_null());
    assert!(text_output(&empty.document, Terminal::PLAIN).contains("checklist is empty"));
    for code in [
        Code::InputInvalid,
        Code::Forbidden,
        Code::RoomUnavailable,
        Code::AssigneeUnavailable,
        Code::NotFound,
        Code::Deleted,
        Code::StateInvalid,
        Code::Conflict,
        Code::StorageError,
        Code::OutcomeUnknown,
    ] {
        let failed = finish(Err(Error {
            code,
            message: "Literal failure".into(),
            current: None,
        }));
        assert!(!failed.complete);
        assert_eq!(
            failed.document,
            json!({"error":{"code":code.as_str(),"message":"Literal failure"}})
        );
        assert!(text_output(&failed.document, Terminal::PLAIN).is_empty());
    }
}
