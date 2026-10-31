#[path = "support/core_fixture.rs"]
mod core_fixture;
mod support;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use std::{
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use tmt_colab::decoder::{
    BaselineInput, DecodeFault, Decoder, MemoryLimit, Namespace, Role, STREAM_BYTES, UpdateBatch,
};
use tmt_invoke::{Cleanup, EnvironmentPolicy, FailureKind, LaunchOptions, Request};
use yrs::{
    Array, Doc, GetString, Map, ReadTxn, StateVector, Text, Transact, Update,
    updates::decoder::Decode,
};
fn program() -> PathBuf {
    env!("CARGO_BIN_EXE_tmt-colab").into()
}
fn owner() -> Decoder {
    Decoder::with_config(support::decoder_config(program())).unwrap()
}
fn gone(pid: u32) {
    assert_eq!(
        kill(Pid::from_raw(pid as i32), None),
        Err(Errno::ESRCH),
        "decoder leaked"
    );
}
fn source() -> (Vec<u8>, Vec<Vec<u8>>) {
    let doc = Doc::with_client_id(123);
    let html = doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    html.insert(&mut doc.transact_mut(), 0, "hello");
    meta.insert(&mut doc.transact_mut(), "title", "test");
    let baseline = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let mut updates = Vec::new();
    for text in [" world", "!"] {
        let mut txn = doc.transact_mut();
        html.insert(&mut txn, 5, text);
        updates.push(txn.encode_update_v1());
    }
    let mut txn = doc.transact_mut();
    html.remove_range(&mut txn, 0, 1);
    updates.push(txn.encode_update_v1());
    (baseline, updates)
}
#[test]
fn shell_shared_values_are_checked_by_the_actual_isolated_decoder() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/shell-v1.json")).unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        if case.get("action").is_some() {
            continue;
        }
        let doc = Doc::new();
        doc.get_or_insert_text("html");
        let meta = doc.get_or_insert_map("meta");
        let mut tx = doc.transact_mut();
        meta.insert(&mut tx, "title", "Shell");
        meta.insert(&mut tx, "pageType", "shell");
        meta.insert(
            &mut tx,
            "shell",
            yrs::Any::from_json(&case["document"].to_string()).unwrap(),
        );
        let bytes = tx.encode_state_as_update_v1(&StateVector::default());
        drop(tx);
        let result = owner().decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &bytes,
                updates: &[],
            },
            Role::Editor,
            None,
        );
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Ok(value) = result {
            assert_eq!(value.projection["meta"]["shell"], case["document"]);
            gone(value.child_pid);
        }
    }
}

#[test]
fn shell_baseline_preserves_structured_content_and_checks_its_commitment() {
    let shell = serde_json::json!({"version":1,"items":{},"order":[],"summary":{
        "line1":"Release plan","line2":"Read the latest plan",
        "attribution":{"writerId":"10000000-0000-4000-8000-000000000001","timeMs":1000}
    }});
    let view = || BaselineInput {
        page_state: None,
        creator: None,
        questions: None,
        template: None,
        from_template: None,
        work: None,
        layout: None,
        original_author: Some("Agent 1"),
        attachments: None,
        source: b"",
        title: "Shell",
        publisher_agent: Some("Agent 1"),
        creation_recipient: None,
        source_digest: tmt_colab_model::crypto::digest(b""),
    };
    let mut decoder = owner();
    for page in [true, false] {
        let baseline = decoder.produce_shell(view(), &shell, page, None).unwrap();
        gone(baseline.child_pid);
        assert_eq!(baseline.chunks.len(), usize::from(page));
        let verified = decoder
            .verify_baseline(view(), &baseline.update, baseline.commitment, None)
            .unwrap();
        gone(verified.child_pid);
        let decoded = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &verified.update,
                    updates: &[],
                },
                Role::Editor,
                None,
            )
            .unwrap();
        assert_eq!(decoded.projection["meta"]["shell"], shell);
        assert_eq!(decoded.projection["meta"]["pageType"], "shell");
        assert_eq!(decoded.projection["html"], "");
        gone(decoded.child_pid);
        let mut changed = baseline.commitment;
        changed[0] ^= 1;
        assert!(
            decoder
                .verify_baseline(view(), &baseline.update, changed, None)
                .is_err()
        );
    }
}

#[test]
fn shell_actions_prepare_replayable_updates_without_changing_the_admitted_base() {
    use tmt_colab::decoder::{ContentBatch, ContentEdit, StructuredEdit, StructuredKind};
    let view = || BaselineInput {
        page_state: None,
        creator: None,
        questions: None,
        template: None,
        from_template: None,
        work: None,
        layout: None,
        original_author: None,
        attachments: None,
        source: b"",
        title: "Shell",
        publisher_agent: None,
        creation_recipient: None,
        source_digest: tmt_colab_model::crypto::digest(b""),
    };
    let mut decoder = owner();
    let baseline = decoder
        .produce_shell(view(), &tmt_colab::shell::empty(), true, None)
        .unwrap();
    let batch = || UpdateBatch {
        namespace: Namespace::Content,
        baseline: baseline.update.as_slice(),
        updates: &[],
    };
    let before = decoder.decode(batch(), Role::Editor, None).unwrap();
    let edit = StructuredEdit {
        kind: StructuredKind::Shell,
        action: "summary.set".into(),
        body: serde_json::json!({"line1":"Plan","line2":"Check first"}),
        attribution: serde_json::json!({"writerId":"10000000-0000-4000-8000-000000000001","timeMs":1000}),
    };
    let prepared = decoder
        .prepare_structured_batch(batch(), &before.projection, &edit, None)
        .unwrap();
    let ContentBatch::Updates(updates) = prepared.batch else {
        panic!("expected update")
    };
    assert_eq!(updates.len(), 1);
    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let after = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline.update,
                updates: &refs,
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(after.projection, prepared.projection);
    assert_eq!(
        after.projection["meta"]["shell"]["summary"]["line1"],
        "Plan"
    );
    assert!(before.projection["meta"]["shell"].get("summary").is_none());
    assert!(
        decoder
            .prepare_content_batch(
                batch(),
                &before.projection,
                ContentEdit {
                    layout: None,
                    source: "erase",
                    publisher_agent: None,
                    attachments: None,
                },
                None
            )
            .is_err()
    );
    let mut bad = edit.clone();
    bad.body["author"] = serde_json::json!("caller");
    assert!(
        decoder
            .prepare_structured_batch(batch(), &before.projection, &bad, None)
            .is_err()
    );
}
const WRITER: &str = "10000000-0000-4000-8000-000000000001";
#[test]
fn registry_metadata_survives_baselines_edits_and_preserves_content_authority() {
    use tmt_colab::decoder::{ContentBatch, ContentEdit, StructuredEdit, StructuredKind};
    let source = b"<p>page</p>";
    let attribution = serde_json::json!({"writerId":WRITER,"timeMs":1000});
    let creator = serde_json::json!({"agent":{"id":"50000000-0000-1000-8000-000000000001","name":"agent"},"attribution":attribution});
    let questions = tmt_colab::question_registry::apply(
        &serde_json::json!([]),
        "question.add",
        &serde_json::json!({"id":"00000000-0000-0000-0000-000000000001","agent":creator["agent"],"title":"Choose"}),
        &attribution,
    )
    .unwrap();
    let mut decoder = owner();
    let baseline = decoder
        .produce_page(
            BaselineInput {
                page_state: None,
                creator: Some(&creator),
                questions: Some(&questions),
                ..work_view(source, None)
            },
            None,
        )
        .unwrap();
    gone(baseline.child_pid);
    let batch = || UpdateBatch {
        namespace: Namespace::Content,
        baseline: &baseline.update,
        updates: &[],
    };
    let before = decoder.decode(batch(), Role::Editor, None).unwrap();
    gone(before.child_pid);
    assert_eq!(before.projection["meta"]["creator"], creator);
    assert_eq!(before.projection["meta"]["questions"], questions);
    assert!(decoder.decode(batch(), Role::Commenter, None).is_err());
    let prepared = decoder
        .prepare_structured_batch(
            batch(),
            &before.projection,
            &StructuredEdit {
                kind: StructuredKind::Questions,
                action: "question.handle".into(),
                body: serde_json::json!({"id":questions[0]["id"]}),
                attribution: attribution.clone(),
            },
            None,
        )
        .unwrap();
    gone(prepared.child_pid);
    let ContentBatch::Updates(updates) = prepared.batch else {
        panic!("expected registry update")
    };
    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let rewritten = decoder
        .prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline.update,
                updates: &refs,
            },
            &prepared.projection,
            ContentEdit {
                source: "<p>changed</p>",
                publisher_agent: None,
                layout: None,
                attachments: None,
            },
            None,
        )
        .unwrap();
    gone(rewritten.child_pid);
    assert_eq!(rewritten.projection["meta"]["creator"], creator);
    assert_eq!(
        rewritten.projection["meta"]["questions"][0]["state"],
        "handled"
    );
    let fresh = decoder
        .produce_baseline(
            BaselineInput {
                page_state: None,
                creator: rewritten.projection["meta"].get("creator"),
                questions: rewritten.projection["meta"].get("questions"),
                ..work_view(b"<p>changed</p>", None)
            },
            None,
        )
        .unwrap();
    gone(fresh.child_pid);
    let after = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &fresh.update,
                updates: &[],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    gone(after.child_pid);
    assert_eq!(after.projection, rewritten.projection);
}
#[test]
fn template_marks_survive_baselines_and_edits_and_the_mark_is_set_and_cleared_by_actions() {
    use tmt_colab::decoder::{ContentBatch, ContentEdit, StructuredEdit, StructuredKind};
    let from = serde_json::json!({"name":"report"});
    let mut decoder = owner();
    let baseline = decoder
        .produce_page(
            BaselineInput {
                from_template: Some(&from),
                ..work_view(b"<p>page</p>", None)
            },
            None,
        )
        .unwrap();
    gone(baseline.child_pid);
    let decode = |decoder: &mut Decoder, updates: &[&[u8]]| {
        let decoded = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &baseline.update,
                    updates,
                },
                Role::Editor,
                None,
            )
            .unwrap();
        gone(decoded.child_pid);
        decoded.projection
    };
    let before = decode(&mut decoder, &[]);
    assert_eq!(before["meta"]["fromTemplate"], from);
    assert!(before["meta"].get("template").is_none());
    let act =
        |decoder: &mut Decoder, base: &serde_json::Value, updates: &[&[u8]], action: &str, body| {
            let prepared = decoder.prepare_structured_batch(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &baseline.update,
                    updates,
                },
                base,
                &StructuredEdit {
                    kind: StructuredKind::Template,
                    action: action.into(),
                    body,
                    attribution: serde_json::json!({}),
                },
                None,
            );
            prepared.inspect(|prepared| gone(prepared.child_pid))
        };
    // Marking the page publishes one update and leaves the source and the origin alone.
    let mark = serde_json::json!({"name":"weekly-review","description":"Shipped, open"});
    let marked = act(&mut decoder, &before, &[], "template.set", mark.clone()).unwrap();
    assert_eq!(marked.projection["meta"]["template"], mark);
    assert_eq!(marked.projection["meta"]["fromTemplate"], from);
    assert_eq!(marked.projection["html"], "<p>page</p>");
    let ContentBatch::Updates(updates) = marked.batch else {
        panic!("expected a template update")
    };
    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
    assert_eq!(decode(&mut decoder, &refs), marked.projection);
    // Setting the same mark again changes nothing.
    let again = act(
        &mut decoder,
        &marked.projection,
        &refs,
        "template.set",
        mark.clone(),
    )
    .unwrap();
    assert!(matches!(again.batch, ContentBatch::Noop));
    // A source edit keeps both marks, and so does a new baseline made from the projection.
    let edited = decoder
        .prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline.update,
                updates: &refs,
            },
            &marked.projection,
            ContentEdit {
                source: "<p>changed</p>",
                publisher_agent: None,
                layout: None,
                attachments: None,
            },
            None,
        )
        .unwrap();
    gone(edited.child_pid);
    assert_eq!(edited.projection["meta"]["template"], mark);
    assert_eq!(edited.projection["meta"]["fromTemplate"], from);
    let fresh = decoder
        .produce_baseline(
            BaselineInput {
                template: edited.projection["meta"].get("template"),
                from_template: edited.projection["meta"].get("fromTemplate"),
                ..work_view(b"<p>changed</p>", None)
            },
            None,
        )
        .unwrap();
    gone(fresh.child_pid);
    let after = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &fresh.update,
                updates: &[],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    gone(after.child_pid);
    assert_eq!(after.projection, edited.projection);
    // Clearing removes the key from the document, not just from the projection.
    let cleared = act(
        &mut decoder,
        &marked.projection,
        &refs,
        "template.clear",
        serde_json::json!({}),
    )
    .unwrap();
    assert!(cleared.projection["meta"].get("template").is_none());
    assert_eq!(cleared.projection["meta"]["fromTemplate"], from);
    let ContentBatch::Updates(more) = cleared.batch else {
        panic!("expected a clearing update")
    };
    let all = updates
        .iter()
        .chain(&more)
        .map(Vec::as_slice)
        .collect::<Vec<_>>();
    assert_eq!(decode(&mut decoder, &all), cleared.projection);
    // A built-in name, an unknown action and a stray member are refused before any update.
    for (action, body) in [
        (
            "template.set",
            serde_json::json!({"name":"report","description":""}),
        ),
        (
            "template.set",
            serde_json::json!({"name":"Weekly","description":""}),
        ),
        (
            "template.set",
            serde_json::json!({"name":"weekly","description":"d","extra":1}),
        ),
        ("template.clear", serde_json::json!({"name":"weekly"})),
        ("template.rename", serde_json::json!({})),
    ] {
        assert!(
            act(&mut decoder, &before, &[], action, body).is_err(),
            "{action}"
        );
    }
    // A baseline never carries an invalid mark.
    for invalid in [
        BaselineInput {
            template: Some(&serde_json::json!({"name":"starter","description":""})),
            ..work_view(b"", None)
        },
        BaselineInput {
            from_template: Some(&serde_json::json!({"name":"report","revision":"007"})),
            ..work_view(b"", None)
        },
    ] {
        assert!(decoder.produce_page(invalid, None).is_err());
    }
}
fn work_view<'a>(source: &'a [u8], work: Option<&'a serde_json::Value>) -> BaselineInput<'a> {
    BaselineInput {
        page_state: None,
        creator: None,
        questions: None,
        template: None,
        from_template: None,
        work,
        layout: None,
        original_author: None,
        attachments: None,
        source,
        title: "Work",
        publisher_agent: None,
        creation_recipient: None,
        source_digest: tmt_colab_model::crypto::digest(source),
    }
}
fn work_edit(action: &str, body: serde_json::Value) -> tmt_colab::decoder::StructuredEdit {
    tmt_colab::decoder::StructuredEdit {
        kind: tmt_colab::decoder::StructuredKind::Work,
        action: action.into(),
        body,
        attribution: serde_json::json!({"writerId":WRITER,"timeMs":1000}),
    }
}
#[test]
fn work_shared_values_are_checked_by_the_actual_isolated_decoder() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/work-v2.json")).unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        if case.get("action").is_some() {
            continue;
        }
        let doc = Doc::new();
        let html = doc.get_or_insert_text("html");
        let meta = doc.get_or_insert_map("meta");
        let mut tx = doc.transact_mut();
        html.insert(&mut tx, 0, "<p>page</p>");
        meta.insert(&mut tx, "title", "Page");
        meta.insert(
            &mut tx,
            "work",
            yrs::Any::from_json(&case["document"].to_string()).unwrap(),
        );
        let bytes = tx.encode_state_as_update_v1(&StateVector::default());
        drop(tx);
        let result = owner().decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &bytes,
                updates: &[],
            },
            Role::Editor,
            None,
        );
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Ok(value) = result {
            assert_eq!(value.projection["meta"]["work"], case["document"]);
            assert_eq!(value.projection["html"], "<p>page</p>");
            gone(value.child_pid);
        }
    }
}

#[test]
fn work_never_shares_a_page_with_the_shell_page_type() {
    let doc = Doc::new();
    doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    let mut tx = doc.transact_mut();
    meta.insert(&mut tx, "title", "Shell");
    meta.insert(&mut tx, "pageType", "shell");
    meta.insert(
        &mut tx,
        "shell",
        yrs::Any::from_json(&tmt_colab::shell::empty().to_string()).unwrap(),
    );
    meta.insert(
        &mut tx,
        "work",
        yrs::Any::from_json(&tmt_colab::work::empty().to_string()).unwrap(),
    );
    let bytes = tx.encode_state_as_update_v1(&StateVector::default());
    drop(tx);
    assert!(
        owner()
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &bytes,
                    updates: &[],
                },
                Role::Editor,
                None,
            )
            .is_err()
    );
}

#[test]
fn work_rides_a_baseline_and_a_new_epoch_keeps_it() {
    let work = serde_json::json!({"version":1,"next":3,"tasks":{
        "T-2":{"title":"Ship it","state":"in-progress","assignee":{"kind":"agent","name":"feat-2"}}
    },"order":["T-2"],"summary":{"line1":"One task","line2":"Next: review",
        "attribution":{"writerId":WRITER,"timeMs":1000}}});
    let source = b"<p>the page</p>";
    let mut decoder = owner();
    let baseline = decoder
        .produce_baseline(work_view(source, Some(&work)), None)
        .unwrap();
    gone(baseline.child_pid);
    let verified = decoder
        .verify_baseline(
            work_view(source, None),
            &baseline.update,
            baseline.commitment,
            None,
        )
        .unwrap();
    gone(verified.child_pid);
    let decoded = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &verified.update,
                updates: &[],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(decoded.projection["meta"]["work"], work);
    assert_eq!(decoded.projection["html"], "<p>the page</p>");
    gone(decoded.child_pid);
    let mut changed = baseline.commitment;
    changed[0] ^= 1;
    assert!(
        decoder
            .verify_baseline(work_view(source, None), &baseline.update, changed, None)
            .is_err()
    );
    // A baseline without the value is a different page, and an invalid value is refused outright.
    let without = decoder
        .produce_baseline(work_view(source, None), None)
        .unwrap();
    assert_ne!(without.commitment, baseline.commitment);
    let invalid = serde_json::json!({"version":1,"next":1,"tasks":{},"order":["T-9"]});
    assert!(
        decoder
            .produce_baseline(work_view(source, Some(&invalid)), None)
            .is_err()
    );
    // Work rides the single baseline update: not the chunked creation of a new page, not a shell.
    assert!(
        decoder
            .produce_page(work_view(source, Some(&work)), None)
            .is_err()
    );
    assert!(
        decoder
            .produce_shell(
                work_view(b"", Some(&work)),
                &tmt_colab::shell::empty(),
                false,
                None
            )
            .is_err()
    );
}

#[test]
fn work_actions_prepare_replayable_updates_and_leave_the_page_alone() {
    use tmt_colab::decoder::{ContentBatch, ContentEdit};
    let source = b"<p>the page</p>";
    let mut decoder = owner();
    let baseline = decoder
        .produce_baseline(work_view(source, None), None)
        .unwrap();
    let batch = || UpdateBatch {
        namespace: Namespace::Content,
        baseline: baseline.update.as_slice(),
        updates: &[],
    };
    let before = decoder.decode(batch(), Role::Editor, None).unwrap();
    assert!(before.projection["meta"].get("work").is_none());
    // The first action starts from an empty value; the decoder re-derives and checks the result.
    let child = "20000000-0000-4000-8000-00000000000a";
    let add = work_edit("subpage.attach", serde_json::json!({"pageId":child}));
    let prepared = decoder
        .prepare_structured_batch(batch(), &before.projection, &add, None)
        .unwrap();
    let ContentBatch::Updates(updates) = prepared.batch else {
        panic!("expected update")
    };
    assert_eq!(updates.len(), 1);
    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let after_batch = || UpdateBatch {
        namespace: Namespace::Content,
        baseline: &baseline.update,
        updates: &refs,
    };
    let after = decoder.decode(after_batch(), Role::Editor, None).unwrap();
    assert_eq!(after.projection, prepared.projection);
    assert_eq!(after.projection["html"], "<p>the page</p>");
    assert_eq!(
        after.projection["meta"]["work"],
        serde_json::json!({"version":2,"order":[child]})
    );
    // An action that leaves a page with no work needs no write and gives it no empty value.
    let clear = work_edit("summary.clear", serde_json::json!({}));
    let noop = decoder
        .prepare_structured_batch(batch(), &before.projection, &clear, None)
        .unwrap();
    assert!(matches!(noop.batch, ContentBatch::Noop));
    // A later source write keeps the work, in the expected projection and in the document.
    let rewritten = decoder
        .prepare_content_batch(
            after_batch(),
            &after.projection,
            ContentEdit {
                layout: None,
                source: "<p>new</p>",
                publisher_agent: None,
                attachments: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(
        rewritten.projection["meta"]["work"],
        after.projection["meta"]["work"]
    );
    assert_eq!(rewritten.projection["html"], "<p>new</p>");
    let ContentBatch::Updates(more) = rewritten.batch else {
        panic!("expected update")
    };
    let all = updates
        .iter()
        .chain(&more)
        .map(Vec::as_slice)
        .collect::<Vec<_>>();
    let replayed = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline.update,
                updates: &all,
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(replayed.projection, rewritten.projection);
    // Refusals: a caller-supplied attribution, a shell action on an ordinary page, and a work
    // action on a shell.
    let mut bad = add.clone();
    bad.body["author"] = serde_json::json!("caller");
    assert!(
        decoder
            .prepare_structured_batch(batch(), &before.projection, &bad, None)
            .is_err()
    );
    let mut shell = add.clone();
    shell.kind = tmt_colab::decoder::StructuredKind::Shell;
    assert!(
        decoder
            .prepare_structured_batch(batch(), &before.projection, &shell, None)
            .is_err()
    );
    let shell_page = decoder
        .produce_shell(
            work_view(b"", None),
            &tmt_colab::shell::empty(),
            false,
            None,
        )
        .unwrap();
    let shell_batch = || UpdateBatch {
        namespace: Namespace::Content,
        baseline: shell_page.update.as_slice(),
        updates: &[],
    };
    let shell_before = decoder.decode(shell_batch(), Role::Editor, None).unwrap();
    assert!(
        decoder
            .prepare_structured_batch(shell_batch(), &shell_before.projection, &add, None)
            .is_err()
    );
}
#[test]
fn content_history_rebuild_drops_deleted_text_and_preserves_clocks_and_in_flight_edits() {
    let doc = Doc::with_client_id(53482);
    let html = doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    let original = {
        let mut tx = doc.transact_mut();
        html.insert(&mut tx, 0, &"discarded history ".repeat(4_000));
        meta.insert(&mut tx, "title", "Kept title");
        tx.encode_update_v1()
    };
    // An offline peer inserts inside the text that this writer is about to delete.
    let concurrent = Doc::with_client_id(53483);
    let concurrent_html = concurrent.get_or_insert_text("html");
    concurrent.get_or_insert_map("meta");
    concurrent
        .transact_mut()
        .apply_update(Update::decode_v1(&original).unwrap())
        .unwrap();
    let concurrent_edit = {
        let mut tx = concurrent.transact_mut();
        concurrent_html.insert(&mut tx, 5, "offline peer edit");
        tx.encode_update_v1()
    };
    let deletion = {
        let mut tx = doc.transact_mut();
        let length = html.len(&tx);
        html.remove_range(&mut tx, 0, length);
        html.insert(&mut tx, 0, "Current source");
        tx.encode_update_v1()
    };
    let vector = doc.transact().state_vector();
    let updates = [original.as_slice(), deletion.as_slice()];
    let mut decoder = owner();
    let merged = decoder.merge(Namespace::Content, &updates, None).unwrap();
    let rebuilt = decoder.rebuild_content(&updates, None).unwrap();
    assert!(rebuilt.len() < merged.len() / 4);
    let peer = Doc::new();
    let peer_html = peer.get_or_insert_text("html");
    peer.get_or_insert_map("meta");
    peer.transact_mut()
        .apply_update(Update::decode_v1(&rebuilt).unwrap())
        .unwrap();
    assert_eq!(peer.transact().state_vector(), vector);
    assert_eq!(peer_html.get_string(&peer.transact()), "Current source");
    // An edit prepared against the old identities still applies after the rebuild.
    let in_flight = {
        let mut tx = doc.transact_mut();
        html.push(&mut tx, " plus an edit");
        tx.encode_update_v1()
    };
    peer.transact_mut()
        .apply_update(Update::decode_v1(&in_flight).unwrap())
        .unwrap();
    assert_eq!(
        peer_html.get_string(&peer.transact()),
        html.get_string(&doc.transact())
    );
    let projection = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &rebuilt,
                updates: &[&in_flight],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(projection.projection["meta"]["title"], "Kept title");
    gone(projection.child_pid);
    doc.transact_mut()
        .apply_update(Update::decode_v1(&concurrent_edit).unwrap())
        .unwrap();
    peer.transact_mut()
        .apply_update(Update::decode_v1(&concurrent_edit).unwrap())
        .unwrap();
    assert_eq!(
        peer_html.get_string(&peer.transact()),
        html.get_string(&doc.transact())
    );
    assert!(
        peer_html
            .get_string(&peer.transact())
            .contains("offline peer edit")
    );
}

#[test]
fn content_history_rebuild_keeps_pending_foreign_structs_and_delete_sets_merge_only() {
    let (baseline, updates) = source();
    let mut decoder = owner();
    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let merged = decoder.merge(Namespace::Content, &refs, None).unwrap();
    assert_eq!(decoder.rebuild_content(&refs, None).unwrap(), merged);
    let decoded = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline,
                updates: &[&merged],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(decoded.projection["html"], "ello! world");
    gone(decoded.child_pid);

    let doc = Doc::new();
    let html = doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    doc.transact_mut()
        .apply_update(Update::decode_v1(&baseline).unwrap())
        .unwrap();
    let delete_only = {
        let mut tx = doc.transact_mut();
        html.remove_range(&mut tx, 0, 1);
        tx.encode_update_v1()
    };
    let merged = decoder
        .merge(Namespace::Content, &[&delete_only], None)
        .unwrap();
    assert_eq!(
        decoder.rebuild_content(&[&delete_only], None).unwrap(),
        merged
    );
    let decoded = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline,
                updates: &[&merged],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(decoded.projection["html"], "ello");
    gone(decoded.child_pid);
}

#[test]
fn child_materializes_and_merges_only_author_updates_preserving_dependencies_and_deletes() {
    let (baseline, updates) = source();
    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let decoded = owner()
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline,
                updates: &refs,
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(decoded.projection["html"], "ello! world");
    assert_eq!(decoded.projection["meta"]["title"], "test");
    assert_eq!(
        decoded.memory_limit,
        if cfg!(target_os = "linux") {
            MemoryLimit::Enforced
        } else {
            MemoryLimit::Unavailable
        }
    );
    gone(decoded.child_pid);
    let doc = Doc::new();
    let html = doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    doc.transact_mut()
        .apply_update(Update::decode_v1(&baseline).unwrap())
        .unwrap();
    doc.transact_mut()
        .apply_update(Update::decode_v1(&decoded.merged).unwrap())
        .unwrap();
    assert_eq!(html.get_string(&doc.transact()), "ello! world");
    let empty = Doc::new();
    empty.get_or_insert_text("html");
    empty.get_or_insert_map("meta");
    empty
        .transact_mut()
        .apply_update(Update::decode_v1(&decoded.merged).unwrap())
        .unwrap();
    assert!(
        empty.transact().store().pending_update().is_some(),
        "baseline was folded into author's merge"
    );
}
#[test]
fn mixed_roots_wrong_types_and_incomplete_dependencies_apply_nothing() {
    for root in ["threads", "html", "meta"] {
        let doc = Doc::new();
        if root == "meta" {
            doc.get_or_insert_text(root)
                .insert(&mut doc.transact_mut(), 0, "wrong");
        } else {
            doc.get_or_insert_map(root)
                .insert(&mut doc.transact_mut(), "bad", true);
        }
        let input = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(matches!(
            owner().decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[&input]
                },
                Role::Editor,
                None
            ),
            Err(DecodeFault::Rejected)
        ));
    }
    let (_, updates) = source();
    assert!(
        owner()
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[&updates[1]]
                },
                Role::Editor,
                None
            )
            .is_err()
    );
    let stop = AtomicBool::new(true);
    assert!(
        matches!(owner().decode(UpdateBatch { namespace: Namespace::Content, baseline: &[], updates: &[] }, Role::Editor, Some(&stop)), Err(DecodeFault::Invoke(e)) if e.kind == FailureKind::Interrupted)
    );
}
#[test]
fn archived_hostile_corpus_is_contained_with_confirmed_cleanup_twice() {
    let valid = include_bytes!("fixtures/hostile/valid.bin").to_vec();
    let mut seed = 0x830c01ab_u64;
    let mut corpus = vec![
        vec![],
        vec![255; 32],
        vec![1, 255, 255, 255, 255, 15],
        vec![0, 255, 255, 255, 255, 15],
        valid.clone(),
    ];
    for n in 0..256 {
        let mut bytes = if n % 2 == 0 {
            valid.clone()
        } else {
            Vec::new()
        };
        for _ in 0..(n % 256 + 1) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            if n % 2 == 0 {
                let i = seed as usize % bytes.len();
                bytes[i] ^= (seed >> 32) as u8;
            } else {
                bytes.push(seed as u8);
            }
        }
        if n % 7 == 0 {
            bytes.truncate(n % bytes.len().max(1));
        }
        corpus.push(bytes);
    }
    for (index, bytes) in [
        (26, include_bytes!("fixtures/hostile/26.bin").as_slice()),
        (60, include_bytes!("fixtures/hostile/60.bin")),
        (106, include_bytes!("fixtures/hostile/106.bin")),
        (147, include_bytes!("fixtures/hostile/147.bin")),
        (157, include_bytes!("fixtures/hostile/157.bin")),
        (192, include_bytes!("fixtures/hostile/192.bin")),
    ] {
        assert_eq!(corpus[index], bytes, "generator drift at {index}");
    }
    // Record the exec-preserved PID so every contained outcome proves reaping.
    let path = program().to_string_lossy().replace('\'', "'\\''");
    let fixture = FixtureProgram::new(&format!("exec '{path}' \"$@\""));
    for run in 0..2 {
        let start = Instant::now();
        let mut timeout = 0;
        let mut panic = 0;
        for (index, bytes) in corpus.iter().enumerate() {
            assert!(
                start.elapsed() < Duration::from_secs(45),
                "corpus suite exceeded budget"
            );
            let input=serde_json::to_vec(&serde_json::json!({"version":1,"namespace":"content","baseline":"","updates":[URL_SAFE_NO_PAD.encode(bytes)]})).unwrap();
            let program = &fixture.script;
            if fixture.directory.join("pid").exists() {
                std::fs::remove_file(fixture.directory.join("pid")).unwrap();
            }
            let args = ["__decoder".into()];
            let outcome = match tmt_invoke::invoke(
                Request {
                    program,
                    args: &args,
                    input: &input,
                    deadline: Instant::now() + Duration::from_secs(1),
                    max_stream_bytes: STREAM_BYTES,
                    launch: LaunchOptions {
                        environment: EnvironmentPolicy::ClearAllowlist(&[]),
                        ..Default::default()
                    },
                },
                None,
            ) {
                Ok(output) => {
                    // A warm decode may reject the saved timeout before its deadline.
                    assert!(
                        output.stdout.is_empty(),
                        "rejected fixture returned result bytes: {index}"
                    );
                    assert!(
                        !output.status.success(),
                        "fixture schema cannot be admitted: {index}"
                    );
                    let diagnostic = String::from_utf8_lossy(&output.stderr);
                    let panicked = diagnostic.contains("decoder panic");
                    if cfg!(target_os = "linux") && [26, 60, 106, 147, 157, 192].contains(&index) {
                        assert!(
                            panicked
                                || diagnostic.contains("decoder rejected")
                                || diagnostic.contains("decoder memory limit failed")
                                || (diagnostic.contains("memory allocation of")
                                    && diagnostic.contains("failed")),
                            "Linux saved dump exited without a containment diagnostic: {diagnostic}"
                        );
                    }
                    if !cfg!(target_os = "linux") && [60, 106, 147, 157, 192].contains(&index) {
                        assert!(panicked, "saved panic was not reproduced: {index}");
                    }
                    if panicked {
                        panic += 1;
                    }
                    if !cfg!(target_os = "linux") {
                        assert!(diagnostic.contains("memory limit unavailable"));
                    }
                    if panicked {
                        "panic"
                    } else if diagnostic.contains("decoder rejected") {
                        "rejected"
                    } else {
                        "memory-limit"
                    }
                }
                Err(e) => {
                    assert!(
                        cfg!(target_os = "linux") || ![60, 106, 147, 157, 192].contains(&index),
                        "saved panic timed out instead: {index}"
                    );
                    assert!(
                        matches!(e.cleanup, Cleanup::Confirmed),
                        "cleanup failed at {index}: {e}"
                    );
                    assert_eq!(e.kind, FailureKind::Deadline);
                    timeout += 1;
                    "deadline"
                }
            };
            match fixture.recorded_pid() {
                Some(pid) => gone(pid),
                None => assert_eq!(outcome, "deadline", "no pid without a deadline at {index}"),
            }
            if [26, 60, 106, 147, 157, 192].contains(&index) {
                eprintln!("hostile saved run {run}: dump {index}, {outcome}, reaped, no result");
            }
        }
        assert!(timeout + panic >= 1, "hostile evidence unexpectedly lost");
        eprintln!(
            "hostile run {run}: 261 cases, {panic} panics, {timeout} deadlines; confirmed cleanup; {:?}",
            start.elapsed()
        );
    }
    let mut decoder = Decoder::new(fixture.script.clone()).unwrap();
    for index in [26, 60, 106, 147, 157, 192] {
        decoder.set_deadline(tmt_colab::decoder::DEADLINE).unwrap();
        match decoder.decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[&corpus[index]],
            },
            Role::Editor,
            None,
        ) {
            Err(DecodeFault::Invoke(e)) => {
                assert!(cfg!(target_os = "linux") || index == 26);
                assert_eq!(e.kind, FailureKind::Deadline);
                assert!(matches!(e.cleanup, Cleanup::Confirmed));
            }
            // invoke has waited/reaped a non-success exit; the corpus assertion above
            // checks its diagnostic. The public runner deliberately hides child stderr.
            // Dump 26 may finish inside the runner's two-second deadline on a fast host,
            // so a clean rejection is contained here on every platform.
            Err(DecodeFault::Rejected) => {}
            Ok(_) => panic!("saved dump {index} unexpectedly succeeded"),
            Err(other) => panic!("saved dump {index} was not contained: {other:?}"),
        }
        gone(fixture.pid());
        decoder.set_deadline(support::DECODER_DEADLINE).unwrap();
        let reply = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[],
                },
                Role::Editor,
                None,
            )
            .unwrap();
        gone(reply.child_pid);
    }
}

struct FixtureProgram {
    directory: PathBuf,
    script: PathBuf,
    barrier: Option<(std::fs::File, std::fs::File)>,
}
impl FixtureProgram {
    fn new(body: &str) -> Self {
        use std::{
            os::unix::fs::PermissionsExt,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "tmt-decoder-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let script = core_fixture::link(&directory, "child", core_fixture::Program::Decoder);
        let pid_path = directory
            .join("pid")
            .to_string_lossy()
            .replace('\'', "'\\''");
        std::fs::write(
            directory.join("behavior"),
            format!("#!/bin/sh\nprintf '%s\\n' \"$$\" > '{pid_path}'\n{body}\n").as_bytes(),
        )
        .unwrap();
        Self {
            directory,
            script,
            barrier: None,
        }
    }
    fn blocked() -> Self {
        use nix::{sys::stat::Mode, unistd::mkfifo};
        let mut fixture = Self::new("");
        let ready = fixture.directory.join("ready");
        let release = fixture.directory.join("release");
        for path in [&ready, &release] {
            mkfifo(path, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
        }
        let open = |path| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap()
        };
        fixture.barrier = Some((open(&ready), open(&release)));
        let directory = fixture.directory.to_string_lossy().replace('\'', "'\\''");
        let actual = program().to_string_lossy().replace('\'', "'\\''");
        std::fs::write(
            fixture.directory.join("behavior"),
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{directory}/pid'\nexec 3< '{directory}/release'\nprintf x > '{directory}/ready'\nread line <&3\nexec 3<&-\nexec '{actual}' \"$@\"\n"
            ).as_bytes(),
        )
        .unwrap();
        fixture
    }
    fn wait_ready(&self) {
        use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
        use std::{io::Read, os::fd::AsFd};
        let (ready, _) = self.barrier.as_ref().unwrap();
        let mut events = [PollFd::new(ready.as_fd(), PollFlags::POLLIN)];
        assert_eq!(
            poll(&mut events, PollTimeout::try_from(30_000).unwrap()).unwrap(),
            1,
            "blocked child never reached its FIFO"
        );
        let mut byte = [0];
        (&*ready).read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'x']);
    }
    fn release(&self) {
        use std::io::Write;
        let (_, release) = self.barrier.as_ref().unwrap();
        (&*release).write_all(b"continue\n").unwrap();
    }
    fn assert_blocked_child_was_reaped(&self) {
        self.wait_ready();
        gone(self.pid());
    }
    fn pid(&self) -> u32 {
        self.recorded_pid().expect("fixture child recorded its pid")
    }
    /// None when a deadline killed the wrapper's process group before it wrote
    /// its pid; Confirmed cleanup then already proves termination.
    fn recorded_pid(&self) -> Option<u32> {
        match std::fs::read_to_string(self.directory.join("pid")) {
            Ok(text) => Some(text.trim().parse().unwrap()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => panic!("read fixture pid: {e}"),
        }
    }
    fn actual(&self) {
        let path = program().to_string_lossy().replace('\'', "'\\''");
        std::fs::write(
            self.directory.join("behavior"),
            format!("#!/bin/sh\nexec '{path}' \"$@\"\n").as_bytes(),
        )
        .unwrap();
    }
}
impl Drop for FixtureProgram {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
#[test]
fn environment_is_cleared_and_successful_invalid_output_is_rejected() {
    assert!(
        std::env::var_os("HOME").is_some(),
        "inherited control needs HOME"
    );
    let fixture = FixtureProgram::new("if [ -n \"${HOME+x}\" ]; then exit 7; fi\nprintf '{}'");
    assert_eq!(
        std::process::Command::new(&fixture.script)
            .status()
            .unwrap()
            .code(),
        Some(7)
    );
    assert!(matches!(
        Decoder::with_config(support::decoder_config(fixture.script.clone()))
            .unwrap()
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[]
                },
                Role::Editor,
                None
            ),
        Err(DecodeFault::InvalidOutput)
    ));
}
#[test]
fn fifo_blocked_child_completes_when_explicitly_released() {
    let fixture = FixtureProgram::blocked();
    let mut decoder =
        Decoder::with_config(support::decoder_config(fixture.script.clone())).unwrap();
    std::thread::scope(|scope| {
        let work = scope.spawn(|| {
            decoder.decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[],
                },
                Role::Editor,
                None,
            )
        });
        fixture.wait_ready();
        fixture.release();
        let reply = work.join().unwrap().unwrap();
        assert_eq!(reply.projection["html"], "");
        assert_eq!(reply.child_pid, fixture.pid());
        gone(reply.child_pid);
    });
}

#[test]
fn deadline_confirms_cleanup_before_owner_reuse() {
    let fixture = FixtureProgram::blocked();
    let mut decoder = Decoder::new(fixture.script.clone()).unwrap();
    // The FIFO cannot complete on its own. Keep the request minimal so this
    // measures the deadline rather than the time to serialize a large input.
    let baseline = [];
    let error = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline,
                updates: &[],
            },
            Role::Editor,
            None,
        )
        .err()
        .unwrap();
    assert!(matches!(error, DecodeFault::Invoke(ref e)
        if e.kind == FailureKind::Deadline && matches!(e.cleanup, Cleanup::Confirmed)));
    fixture.assert_blocked_child_was_reaped();
    fixture.actual();
    decoder.set_deadline(support::DECODER_DEADLINE).unwrap();
    let reply = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    gone(reply.child_pid);
}

#[test]
fn interrupted_input_backpressure_confirms_cleanup_before_owner_reuse() {
    use std::sync::atomic::Ordering;
    let fixture = FixtureProgram::blocked();
    let mut decoder =
        Decoder::with_config(support::decoder_config(fixture.script.clone())).unwrap();
    let stop = AtomicBool::new(false);
    // Larger than a pipe buffer; the child signals readiness but never reads stdin.
    let baseline = vec![0; tmt_colab::decoder::BASELINE_BYTES];
    std::thread::scope(|scope| {
        let work = scope.spawn(|| {
            decoder.decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &baseline,
                    updates: &[],
                },
                Role::Editor,
                Some(&stop),
            )
        });
        fixture.wait_ready();
        stop.store(true, Ordering::Relaxed);
        let error = work.join().unwrap().err().unwrap();
        assert!(matches!(error, DecodeFault::Invoke(ref e)
            if e.kind == FailureKind::Interrupted && matches!(e.cleanup, Cleanup::Confirmed)));
        gone(fixture.pid());
    });
    fixture.actual();
    let reply = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    gone(reply.child_pid);
}

#[test]
fn output_backpressure_confirms_exact_limit_and_cleanup_before_owner_reuse() {
    let fixture = FixtureProgram::new(&format!(
        "exec /usr/bin/head -c {} /dev/zero",
        tmt_colab::decoder::STREAM_BYTES + 1
    ));
    let mut decoder =
        Decoder::with_config(support::decoder_config(fixture.script.clone())).unwrap();
    let baseline = vec![0; tmt_colab::decoder::BASELINE_BYTES];
    let error = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline,
                updates: &[],
            },
            Role::Editor,
            None,
        )
        .err()
        .unwrap();
    assert!(matches!(error, DecodeFault::Invoke(ref e)
        if e.kind == FailureKind::OutputLimit(tmt_invoke::Stream::Stdout)
            && matches!(e.cleanup, Cleanup::Confirmed)));
    gone(fixture.pid());
    fixture.actual();
    let reply = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    gone(reply.child_pid);
}

#[test]
fn own_maps_have_a_positive_control_and_array_substitution_rejects() {
    let doc = Doc::new();
    for name in ["threads", "messages", "intents", "replies"] {
        doc.get_or_insert_map(name);
    }
    doc.get_or_insert_map("threads")
        .insert(&mut doc.transact_mut(), "id", "thread");
    let input = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let reply = owner()
        .decode(
            UpdateBatch {
                namespace: Namespace::Own,
                baseline: &[],
                updates: &[&input],
            },
            Role::Commenter,
            None,
        )
        .unwrap();
    assert_eq!(reply.projection["threads"]["id"], "thread");
    gone(reply.child_pid);
    let wrong = Doc::new();
    wrong
        .get_or_insert_array("html")
        .insert(&mut wrong.transact_mut(), 0, "wrong");
    let input = wrong
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    assert!(matches!(
        owner().decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[&input]
            },
            Role::Editor,
            None
        ),
        Err(DecodeFault::Rejected)
    ));
}

fn view<'a>(source: &'a [u8], title: &'a str) -> BaselineInput<'a> {
    use sha2::{Digest, Sha256};
    BaselineInput {
        page_state: None,
        creator: None,
        questions: None,
        template: None,
        from_template: None,
        work: None,
        layout: None,
        original_author: None,
        attachments: None,
        creation_recipient: None,
        source,
        title,
        publisher_agent: None,
        source_digest: Sha256::digest(source).into(),
    }
}
fn apply_baseline(doc: &Doc, update: &[u8]) {
    doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    doc.transact_mut()
        .apply_update(Update::decode_v1(update).unwrap())
        .unwrap();
}
#[test]
fn baseline_exact_vectors_materialize_and_concurrent_clients_converge() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/baseline-v1.json")).unwrap();
    for vector in vectors.as_array().unwrap() {
        let source = vector["source"].as_str().unwrap();
        let title = vector["title"].as_str().unwrap();
        let update = URL_SAFE_NO_PAD
            .decode(vector["update"].as_str().unwrap())
            .unwrap();
        let commitment: [u8; 32] = URL_SAFE_NO_PAD
            .decode(vector["commitment"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let recipient = vector.get("creationRecipient").map(|v| {
            serde_json::from_value::<tmt_colab::decoder::CreationRecipient>(v.clone()).unwrap()
        });
        let layout = vector["layout"]
            .as_str()
            .map(|kind| tmt_colab::decoder::PageLayout {
                kind: tmt_colab::decoder::LayoutKind::parse(kind).unwrap(),
                reason: vector["layoutReason"].as_str().map(str::to_owned),
            });
        let mut decoder = owner();
        let verified = decoder
            .verify_baseline(view(source.as_bytes(), title), &update, commitment, None)
            .unwrap();
        assert_eq!(verified.update, update);
        gone(verified.child_pid);
        if let Some(alternate) = vector["alternateUpdate"].as_str() {
            let mismatched = URL_SAFE_NO_PAD
                .decode(vector["alternateCommitment"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            assert!(matches!(
                decoder.verify_baseline(view(source.as_bytes(), title), &update, mismatched, None),
                Err(DecodeFault::Rejected)
            ));
            let alternate = URL_SAFE_NO_PAD.decode(alternate).unwrap();
            let commitment = URL_SAFE_NO_PAD
                .decode(vector["alternateCommitment"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            let verified = decoder
                .verify_baseline(view(source.as_bytes(), title), &alternate, commitment, None)
                .unwrap();
            assert_eq!(verified.update, alternate);
            gone(verified.child_pid);
        }
        let produced = decoder
            .produce_baseline(
                BaselineInput {
                    page_state: None,
                    creator: None,
                    questions: None,
                    template: None,
                    from_template: None,
                    work: None,
                    layout: layout.as_ref(),
                    original_author: vector["originalAuthor"].as_str(),
                    attachments: None,
                    creation_recipient: recipient.as_ref(),
                    publisher_agent: vector["publisherAgent"].as_str(),
                    ..view(source.as_bytes(), title)
                },
                None,
            )
            .unwrap();
        gone(produced.child_pid);
        // Both clients start from the identical owner-produced struct identity.
        let a = Doc::with_client_id(41);
        let b = Doc::with_client_id(42);
        apply_baseline(&a, &produced.update);
        apply_baseline(&b, &produced.update);
        for doc in [&a, &b] {
            let original = doc
                .get_or_insert_map("meta")
                .get(&doc.transact(), "originalAuthor")
                .map(|v| v.to_string(&doc.transact()));
            assert_eq!(original.as_deref(), vector["originalAuthor"].as_str());
            let html = doc.get_or_insert_text("html");
            let meta = doc.get_or_insert_map("meta");
            let txn = doc.transact();
            assert_eq!(html.get_string(&txn), source);
            assert_eq!(meta.get(&txn, "title").unwrap().to_string(&txn), title);
            assert_eq!(
                meta.get(&txn, "publisherAgent")
                    .map(|value| value.to_string(&txn)),
                vector["publisherAgent"].as_str().map(str::to_owned)
            );
            for key in ["layout", "layoutReason"] {
                assert_eq!(
                    meta.get(&txn, key).map(|value| value.to_string(&txn)),
                    vector[key].as_str().map(str::to_owned),
                    "{key}"
                );
            }
        }
        let change = |doc: &Doc, text: &str| {
            let html = doc.get_or_insert_text("html");
            let mut txn = doc.transact_mut();
            html.insert(&mut txn, 0, text);
            txn.encode_update_v1()
        };
        let first = change(&a, "A");
        let second = change(&b, "B");
        apply_baseline(&a, &second);
        apply_baseline(&b, &first);
        // Replay the baseline never independently inserts or duplicates source.
        apply_baseline(&a, &produced.update);
        apply_baseline(&b, &produced.update);
        assert_eq!(a.transact().state_vector(), b.transact().state_vector());
        let actual = a.get_or_insert_text("html").get_string(&a.transact());
        assert_eq!(
            actual,
            b.get_or_insert_text("html").get_string(&b.transact())
        );
        assert_eq!(actual, format!("AB{source}"));
    }
}
#[test]
fn baseline_digest_commitment_and_materialization_mismatches_return_no_result() {
    use sha2::{Digest, Sha256};
    let mut decoder = owner();
    let baseline = decoder
        .produce_baseline(view(b"exact", "title"), None)
        .unwrap();
    gone(baseline.child_pid);
    let wrong_digest = BaselineInput {
        page_state: None,
        creator: None,
        questions: None,
        template: None,
        from_template: None,
        work: None,
        layout: None,
        original_author: None,
        attachments: None,
        creation_recipient: None,
        source: b"exact",
        title: "title",
        source_digest: [0; 32],
        publisher_agent: None,
    };
    assert!(matches!(
        decoder.produce_baseline(wrong_digest, None),
        Err(DecodeFault::InvalidInput)
    ));
    assert!(matches!(
        decoder.verify_baseline(view(b"exact", "title"), &baseline.update, [0; 32], None),
        Err(DecodeFault::Rejected)
    ));
    assert!(matches!(
        decoder.verify_baseline(
            view(b"exact", "different title"),
            &baseline.update,
            baseline.commitment,
            None
        ),
        Err(DecodeFault::Rejected)
    ));
    let changed_source = b"other";
    // Valid hashes for the changed claim cannot substitute materialized text.
    let framed = tmt_colab_model::framing::frame(&[
        b"tmt-colab-baseline-v1",
        b"1",
        changed_source,
        &baseline.update,
    ])
    .unwrap();
    assert!(matches!(
        decoder.verify_baseline(
            view(changed_source, "title"),
            &baseline.update,
            Sha256::digest(framed).into(),
            None
        ),
        Err(DecodeFault::Rejected)
    ));
    for update in [&[255u8; 32][..], &[0u8, 0][..]] {
        let framed =
            tmt_colab_model::framing::frame(&[b"tmt-colab-baseline-v1", b"1", b"exact", update])
                .unwrap();
        assert!(matches!(
            decoder.verify_baseline(
                view(b"exact", "title"),
                update,
                Sha256::digest(framed).into(),
                None
            ),
            Err(DecodeFault::Rejected)
        ));
    }
    // A normal child remains usable after every clean rejection.
    let checked = decoder
        .verify_baseline(
            view(b"exact", "title"),
            &baseline.update,
            baseline.commitment,
            None,
        )
        .unwrap();
    gone(checked.child_pid);
}
#[test]
fn baseline_bounds_and_cleanup_use_the_existing_runner() {
    use tmt_colab::decoder::{BASELINE_BYTES, BASELINE_TITLE_BYTES};
    let mut decoder = owner();
    let source = vec![b'x'; BASELINE_BYTES];
    let produced = decoder
        .produce_baseline(view(&source, "title"), None)
        .unwrap();
    gone(produced.child_pid);
    assert!(produced.update.len() > BASELINE_BYTES);
    assert!(matches!(
        decoder.produce_baseline(view(&vec![b'x'; BASELINE_BYTES + 1], "title"), None),
        Err(DecodeFault::InvalidInput)
    ));
    assert!(matches!(
        decoder.produce_baseline(view(&[255], "title"), None),
        Err(DecodeFault::InvalidInput)
    ));
    assert!(matches!(
        decoder.produce_baseline(view(b"", &"x".repeat(BASELINE_TITLE_BYTES + 1)), None),
        Err(DecodeFault::InvalidInput)
    ));
    let checked = decoder
        .verify_baseline(
            view(&source, "title"),
            &produced.update,
            produced.commitment,
            None,
        )
        .unwrap();
    assert_eq!(checked.update, produced.update);
    gone(checked.child_pid);
    for body in [
        &format!(
            "exec /usr/bin/head -c {} /dev/zero",
            tmt_colab::decoder::STREAM_BYTES + 1
        ),
        "cat >/dev/null; printf '{}'",
    ] {
        let fixture = FixtureProgram::new(body);
        let mut decoder =
            Decoder::with_config(support::decoder_config(fixture.script.clone())).unwrap();
        let error = decoder
            .produce_baseline(view(&source, "title"), None)
            .err()
            .unwrap();
        if body == "cat >/dev/null; printf '{}'" {
            assert!(matches!(error, DecodeFault::InvalidOutput));
        } else {
            assert!(matches!(error, DecodeFault::Invoke(ref e)
                if e.kind == FailureKind::OutputLimit(tmt_invoke::Stream::Stdout)
                    && matches!(e.cleanup, Cleanup::Confirmed)));
        }
        gone(fixture.pid());
        fixture.actual();
        let produced = decoder
            .produce_baseline(view(b"reusable", "title"), None)
            .unwrap();
        gone(produced.child_pid);
    }
    let stop = AtomicBool::new(true);
    assert!(
        matches!(decoder.produce_baseline(view(b"", ""), Some(&stop)), Err(DecodeFault::Invoke(e)) if e.kind == FailureKind::Interrupted)
    );
}

#[test]
fn baseline_deadline_confirms_cleanup_before_owner_reuse() {
    let fixture = FixtureProgram::blocked();
    let mut decoder = Decoder::new(fixture.script.clone()).unwrap();
    let error = decoder.produce_baseline(view(b"", ""), None).err().unwrap();
    assert!(matches!(error, DecodeFault::Invoke(ref e)
        if e.kind == FailureKind::Deadline && matches!(e.cleanup, Cleanup::Confirmed)));
    fixture.assert_blocked_child_was_reaped();
    fixture.actual();
    decoder.set_deadline(support::DECODER_DEADLINE).unwrap();
    let produced = decoder
        .produce_baseline(view(b"reusable", "title"), None)
        .unwrap();
    gone(produced.child_pid);
}

#[test]
fn baseline_private_child_rejects_digest_and_strict_wire_mutations() {
    use sha2::{Digest, Sha256};
    let valid = serde_json::json!({"version":1,"source":URL_SAFE_NO_PAD.encode(b"exact"),
        "title":"title", "source_digest":URL_SAFE_NO_PAD.encode(Sha256::digest(b"exact")),
        "action":{"mode":"produce"}});
    let run = |input: &[u8]| {
        let program = program();
        let args = ["__decoder".into(), "baseline".into()];
        tmt_invoke::invoke(
            Request {
                program: &program,
                args: &args,
                input,
                deadline: Instant::now() + support::DECODER_DEADLINE,
                max_stream_bytes: STREAM_BYTES,
                launch: LaunchOptions {
                    environment: EnvironmentPolicy::ClearAllowlist(&[]),
                    ..Default::default()
                },
            },
            None,
        )
        .unwrap()
    };
    let control = run(&serde_json::to_vec(&valid).unwrap());
    assert!(control.status.success());
    let reply: serde_json::Value = serde_json::from_slice(&control.stdout).unwrap();
    gone(reply["pid"].as_u64().unwrap() as u32);
    for (field, value) in [
        (
            "source_digest",
            serde_json::json!(URL_SAFE_NO_PAD.encode([0; 32])),
        ),
        ("version", serde_json::json!(2)),
        ("extra", serde_json::json!(true)),
        ("source", serde_json::json!("ZXhhY3Q=")),
        ("action", serde_json::json!({"mode":"produce","extra":true})),
    ] {
        let mut changed = valid.clone();
        changed[field] = value;
        let rejected = run(&serde_json::to_vec(&changed).unwrap());
        assert!(!rejected.status.success(), "accepted mutation {field}");
        assert!(rejected.stdout.is_empty(), "rejected input returned bytes");
    }
    let mut raw = serde_json::to_string(&valid).unwrap();
    raw.insert_str(1, "\"version\":1,");
    let rejected = run(raw.as_bytes());
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
}

#[test]
fn publisher_metadata_updates_and_unknown_cli_edits_clear_it() {
    use tmt_colab::decoder::ContentEdit;
    let mut decoder = owner();
    let baseline = decoder
        .produce_baseline(
            BaselineInput {
                page_state: None,
                creator: None,
                questions: None,
                template: None,
                from_template: None,
                work: None,
                layout: None,
                original_author: Some("original-author"),
                attachments: None,
                creation_recipient: None,
                publisher_agent: Some("publisher"),
                ..view(b"before", "Title")
            },
            None,
        )
        .unwrap();
    let mut update = baseline.update;
    let mut base = serde_json::json!({"html":"before","meta":{"title":"Title","publisherAgent":"publisher","originalAuthor":"original-author"}});
    for (source, publisher) in [("after", Some("next-agent")), ("after", None)] {
        let edited = decoder
            .prepare_content_batch(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &update,
                    updates: &[],
                },
                &base,
                ContentEdit {
                    layout: None,
                    source,
                    publisher_agent: publisher,
                    attachments: None,
                },
                None,
            )
            .unwrap();
        let tmt_colab::decoder::ContentBatch::Updates(updates) = &edited.batch else {
            panic!("publisher change must produce causal updates");
        };
        let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let folded = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &update,
                    updates: &refs,
                },
                Role::Editor,
                None,
            )
            .unwrap();
        assert_eq!(folded.projection["html"], source);
        assert_eq!(
            folded.projection["meta"]["originalAuthor"],
            "original-author"
        );
        assert_eq!(
            folded.projection["meta"]["publisherAgent"].as_str(),
            publisher
        );
        let doc = Doc::new();
        apply_baseline(&doc, &update);
        for delta in updates {
            apply_baseline(&doc, delta);
        }
        base = folded.projection;
        update = doc
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default());
    }
    for invalid in [
        "".to_owned(),
        "x".repeat(129),
        "line\nbreak".to_owned(),
        "🚀".repeat(33),
    ] {
        assert!(
            decoder
                .produce_baseline(
                    BaselineInput {
                        work: None,
                        layout: None,
                        original_author: Some(&invalid),
                        ..view(b"after", "Title")
                    },
                    None
                )
                .is_err()
        );
        assert!(
            decoder
                .prepare_content_batch(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &update,
                        updates: &[]
                    },
                    &base,
                    ContentEdit {
                        layout: None,
                        source: "after",
                        publisher_agent: Some(&invalid),
                        attachments: None,
                    },
                    None
                )
                .is_err()
        );
    }
}

#[test]
fn a_page_source_larger_than_one_update_is_produced_as_ordered_updates_that_rebuild_it() {
    let mut decoder = Decoder::with_config(support::decoder_config(program())).unwrap();
    // Multi-byte text across a chunk boundary: chunks must never split a character.
    let source = "héllo wörld 🌍 <p>text</p>\n".repeat(60_000);
    assert!(source.len() > 1_500_000);
    let made = decoder
        .produce_page(
            BaselineInput {
                page_state: None,
                work: None,
                layout: None,
                original_author: Some("original-author"),
                ..view(source.as_bytes(), "T")
            },
            None,
        )
        .unwrap();
    assert!(made.chunks.len() > 1);
    let doc = Doc::new();
    doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    for chunk in &made.chunks {
        assert!(
            chunk.len() <= tmt_colab::decoder::UPDATE_BYTES,
            "{} bytes",
            chunk.len()
        );
        doc.transact_mut()
            .apply_update(Update::decode_v1(chunk).unwrap())
            .unwrap();
    }
    let rebuilt = doc.get_or_insert_text("html").get_string(&doc.transact());
    assert!(rebuilt == source, "the chunks rebuild a different source");
    assert_eq!(
        doc.get_or_insert_map("meta")
            .get(&doc.transact(), "originalAuthor")
            .unwrap()
            .to_string(&doc.transact()),
        "original-author"
    );
    // The merged update is the same page in one piece, for the baseline commitment.
    let merged = Doc::new();
    apply_baseline(&merged, &made.update);
    assert!(
        merged
            .get_or_insert_text("html")
            .get_string(&merged.transact())
            == source
    );
    gone(made.child_pid);
}

#[test]
fn merging_one_devices_updates_keeps_structs_that_depend_on_another_device() {
    let mut decoder = Decoder::with_config(support::decoder_config(program())).unwrap();
    // Device one writes the text; device two edits after it, so its updates alone are not a page.
    let one = Doc::with_client_id(1);
    one.get_or_insert_map("meta");
    one.get_or_insert_text("html")
        .insert(&mut one.transact_mut(), 0, "base");
    let base = one
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let two = Doc::with_client_id(2);
    two.get_or_insert_map("meta");
    two.get_or_insert_text("html");
    two.transact_mut()
        .apply_update(Update::decode_v1(&base).unwrap())
        .unwrap();
    let mut mine = Vec::new();
    for piece in [" edit", " more"] {
        let mut txn = two.transact_mut();
        let html = txn.get_text("html").unwrap();
        let end = html.len(&txn);
        html.insert(&mut txn, end, piece);
        mine.push(txn.encode_update_v1());
    }
    let refs = mine.iter().map(Vec::as_slice).collect::<Vec<_>>();
    // Read as a page on its own, it is incomplete and rejected...
    assert!(
        decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &refs,
                },
                Role::Editor,
                None,
            )
            .is_err()
    );
    // ...but merging keeps every struct, and with the other device's update it is the full page.
    let merged = decoder.merge(Namespace::Content, &refs, None).unwrap();
    let page = Doc::new();
    page.get_or_insert_map("meta");
    page.get_or_insert_text("html");
    for update in [base.as_slice(), merged.as_slice()] {
        page.transact_mut()
            .apply_update(Update::decode_v1(update).unwrap())
            .unwrap();
    }
    assert_eq!(
        page.get_or_insert_text("html").get_string(&page.transact()),
        "base edit more"
    );
    // A malformed update is refused, not merged.
    assert!(
        decoder
            .merge(Namespace::Content, &[b"not an update"], None)
            .is_err()
    );
}

#[test]
fn creation_recipient_survives_chunked_baselines_and_known_or_unknown_source_edits() {
    use tmt_colab::decoder::{ContentEdit, CreationRecipient};
    let recipient = CreationRecipient {
        machine_id: "40000000-0000-4000-8000-000000000001".into(),
        agent_id: "50000000-0000-1000-8000-000000000001".into(),
    };
    let source = "<p>Unicode 🐈</p>".repeat(20_000);
    let mut decoder = owner();
    for hint in [Some(&recipient), None] {
        let baseline = decoder
            .produce_page(
                BaselineInput {
                    page_state: None,
                    creator: None,
                    questions: None,
                    template: None,
                    from_template: None,
                    work: None,
                    layout: None,
                    attachments: None,
                    creation_recipient: hint,
                    ..view(source.as_bytes(), "Title")
                },
                None,
            )
            .unwrap();
        gone(baseline.child_pid);
        assert!(baseline.update.len() > 256 * 1024);
        assert!(!baseline.chunks.is_empty());
        let doc = Doc::new();
        for chunk in &baseline.chunks {
            apply_baseline(&doc, chunk);
        }
        let merged = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let expected = hint.map(|v| serde_json::to_value(v).unwrap());
        let folded = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &merged,
                    updates: &[],
                },
                Role::Editor,
                None,
            )
            .unwrap();
        assert_eq!(
            folded.projection["meta"].get("creationRecipient"),
            expected.as_ref()
        );
        gone(folded.child_pid);
        for publisher in [Some("later-agent"), None] {
            let prepared = decoder
                .prepare_content_batch(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &merged,
                        updates: &[],
                    },
                    &folded.projection,
                    ContentEdit {
                        layout: None,
                        source: "later",
                        publisher_agent: publisher,
                        attachments: None,
                    },
                    None,
                )
                .unwrap();
            let tmt_colab::decoder::ContentBatch::Updates(updates) = &prepared.batch else {
                panic!("source replacement must produce causal updates");
            };
            let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
            let folded = decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &merged,
                        updates: &refs,
                    },
                    Role::Editor,
                    None,
                )
                .unwrap();
            assert_eq!(
                folded.projection["meta"].get("creationRecipient"),
                expected.as_ref()
            );
            gone(prepared.child_pid);
            gone(folded.child_pid);
        }
    }
}

#[test]
fn creation_metadata_rejects_partial_unknown_nested_or_noncanonical_values() {
    use std::collections::HashMap;
    let pair = || {
        HashMap::from([
            (
                "machineId".into(),
                yrs::Any::String("40000000-0000-4000-8000-000000000001".into()),
            ),
            (
                "agentId".into(),
                yrs::Any::String("50000000-0000-1000-8000-000000000001".into()),
            ),
        ])
    };
    let mut partial = pair();
    partial.remove("agentId");
    let mut extra = pair();
    extra.insert("extra".into(), yrs::Any::Bool(true));
    let mut nested = pair();
    nested.insert("agentId".into(), yrs::Any::Map(pair().into()));
    let mut invalid = pair();
    invalid.insert(
        "machineId".into(),
        yrs::Any::String("40000000-0000-1000-8000-000000000001".into()),
    );
    for value in [
        yrs::Any::Null,
        yrs::Any::Map(partial.into()),
        yrs::Any::Map(extra.into()),
        yrs::Any::Map(nested.into()),
        yrs::Any::Map(invalid.into()),
    ] {
        let doc = Doc::new();
        doc.get_or_insert_text("html");
        let meta = doc.get_or_insert_map("meta");
        meta.insert(&mut doc.transact_mut(), "title", "Title");
        meta.insert(&mut doc.transact_mut(), "creationRecipient", value);
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(matches!(
            owner().decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &update,
                    updates: &[]
                },
                Role::Editor,
                None
            ),
            Err(DecodeFault::Rejected)
        ));
    }
}

#[test]
fn content_batch_full_replacement_replays_without_reattributing_foreign_structs() {
    use tmt_colab::decoder::{ContentBatch, ContentEdit};
    let old = "A".repeat(1_572_864);
    let author = Doc::with_client_id(123);
    author
        .get_or_insert_text("html")
        .insert(&mut author.transact_mut(), 0, &old);
    let meta = author.get_or_insert_map("meta");
    meta.insert(&mut author.transact_mut(), "title", "Keep title");
    meta.insert(
        &mut author.transact_mut(),
        "publisherAgent",
        "Old publisher",
    );
    let baseline = author
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let foreign = Doc::with_client_id(456);
    apply_baseline(&foreign, &baseline);
    let html = foreign.get_or_insert_text("html");
    let mut tx = foreign.transact_mut();
    let end = html.len(&tx);
    html.insert(&mut tx, end, "<p>foreign</p>");
    let admitted = tx.encode_update_v1();
    drop(tx);
    let before = serde_json::json!({"html":format!("{old}<p>foreign</p>"),"meta":{"title":"Keep title","publisherAgent":"Old publisher"}});
    let refs = [admitted.as_slice()];
    let source = "B".repeat(1_572_864);
    // This regression uses the unchanged production deadline, not the test's larger deadline.
    let mut decoder = Decoder::new(program()).unwrap();
    let made = decoder
        .prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline,
                updates: &refs,
            },
            &before,
            ContentEdit {
                layout: None,
                source: &source,
                publisher_agent: Some("New publisher"),
                attachments: None,
            },
            None,
        )
        .unwrap();
    let ContentBatch::Updates(updates) = &made.batch else {
        panic!("expected full replacement")
    };
    assert!(updates.len() > 1);
    assert!(
        updates
            .iter()
            .all(|v| v.len() <= tmt_colab::decoder::UPDATE_BYTES)
    );
    assert!(updates.iter().map(Vec::len).sum::<usize>() <= tmt_colab::decoder::WRITE_TAIL_BYTES);
    let replay = Doc::new();
    apply_baseline(&replay, &baseline);
    apply_baseline(&replay, &admitted);
    let foreign_clock = replay.transact().state_vector().get(&foreign.client_id());
    for update in updates {
        apply_baseline(&replay, update);
        assert!(replay.transact().store().pending_update().is_none());
        assert!(replay.transact().store().pending_ds().is_none());
    }
    assert_eq!(
        replay
            .get_or_insert_text("html")
            .get_string(&replay.transact()),
        source
    );
    assert_eq!(
        replay.transact().state_vector().get(&foreign.client_id()),
        foreign_clock
    );
    assert_eq!(
        made.projection,
        serde_json::json!({"html":source,"meta":{"title":"Keep title","publisherAgent":"New publisher"}})
    );
    gone(made.child_pid);
    // Deltas alone are dependent on the admitted source; they are not a fresh baseline.
    let detached = Doc::new();
    for update in updates {
        apply_baseline(&detached, update);
    }
    assert!(
        detached.transact().store().pending_update().is_some()
            || detached.transact().store().pending_ds().is_some()
    );
    let unchanged = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline,
                updates: &refs,
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(unchanged.projection, before);
    gone(unchanged.child_pid);
}

#[test]
fn content_batch_handles_unicode_deletion_noop_and_publisher_only_changes() {
    use tmt_colab::decoder::{ContentBatch, ContentEdit};
    for (old, source) in [
        (
            "start 😀 middle 🐈 end".to_owned(),
            "START 😀 middle 🐈 end".to_owned(),
        ),
        (
            "start 😀 middle 🐈 end".to_owned(),
            "start 😀 MIDDLE 🐈 end".to_owned(),
        ),
        (
            "start 😀 middle 🐈 end".to_owned(),
            "start 😀 middle 🐈 END".to_owned(),
        ),
        ("x😀end".to_owned(), "x😁end".to_owned()),
        (
            "old".to_owned(),
            format!("{}🌍漢字{}", "é".repeat(96 * 1024 - 1), "界".repeat(80_000)),
        ),
        ("delete me".to_owned(), String::new()),
        (String::new(), String::new()),
        ("same 🐈".to_owned(), "same 🐈".to_owned()),
    ] {
        let doc = Doc::with_client_id(123);
        doc.get_or_insert_text("html")
            .insert(&mut doc.transact_mut(), 0, &old);
        doc.get_or_insert_map("meta")
            .insert(&mut doc.transact_mut(), "title", "T");
        let baseline = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let before = serde_json::json!({"html":old,"meta":{"title":"T"}});
        let made = owner()
            .prepare_content_batch(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &baseline,
                    updates: &[],
                },
                &before,
                ContentEdit {
                    layout: None,
                    source: &source,
                    publisher_agent: None,
                    attachments: None,
                },
                None,
            )
            .unwrap();
        match made.batch {
            ContentBatch::Noop => assert_eq!(old, source),
            ContentBatch::Updates(updates) => {
                assert_ne!(old, source);
                for update in &updates {
                    apply_baseline(&doc, update);
                }
                assert_eq!(
                    doc.get_or_insert_text("html").get_string(&doc.transact()),
                    source
                );
            }
        }
        assert_eq!(
            made.projection,
            serde_json::json!({"html":source,"meta":{"title":"T"}})
        );
        gone(made.child_pid);
        for publisher in [Some("Publisher"), None] {
            let baseline = doc
                .transact()
                .encode_state_as_update_v1(&StateVector::default());
            let made = owner().prepare_content_batch(
                UpdateBatch { namespace: Namespace::Content, baseline: &baseline, updates: &[] },
                &serde_json::json!({"html":source,"meta":if publisher.is_some() { serde_json::json!({"title":"T"}) } else { serde_json::json!({"title":"T","publisherAgent":"Publisher"}) }}),
                ContentEdit { source: &source, publisher_agent: publisher, layout: None, attachments: None }, None,
            ).unwrap();
            let ContentBatch::Updates(updates) = made.batch else {
                panic!("publisher change is not a no-op")
            };
            for update in &updates {
                apply_baseline(&doc, update);
            }
            assert_eq!(
                made.projection["meta"]["publisherAgent"].as_str(),
                publisher
            );
            assert_eq!(
                doc.get_or_insert_text("html").get_string(&doc.transact()),
                source
            );
            gone(made.child_pid);
        }
    }
}

#[test]
fn content_batch_rejection_and_deadline_keep_the_runner_reusable() {
    use tmt_colab::decoder::{BASELINE_BYTES, ContentBatch, ContentEdit};
    let before = serde_json::json!({"html":"","meta":{}});
    let source = "x".repeat(BASELINE_BYTES + 1);
    let mut decoder = owner();
    assert!(matches!(
        decoder.prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            &before,
            ContentEdit {
                layout: None,
                source: &source,
                publisher_agent: None,
                attachments: None,
            },
            None,
        ),
        Err(DecodeFault::InvalidInput)
    ));
    assert!(matches!(
        decoder.prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            &serde_json::json!({"html":"wrong","meta":{}}),
            ContentEdit {
                layout: None,
                source: "new",
                publisher_agent: None,
                attachments: None,
            },
            None,
        ),
        Err(DecodeFault::Rejected)
    ));
    let fixture = FixtureProgram::new("cat >/dev/null; printf '{}' ");
    let mut decoder =
        Decoder::with_config(support::decoder_config(fixture.script.clone())).unwrap();
    assert!(matches!(
        decoder.prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            &before,
            ContentEdit {
                layout: None,
                source: "new",
                publisher_agent: None,
                attachments: None,
            },
            None,
        ),
        Err(DecodeFault::InvalidOutput)
    ));
    gone(fixture.pid());
    fixture.actual();
    let made = decoder
        .prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[],
            },
            &before,
            ContentEdit {
                layout: None,
                source: "",
                publisher_agent: None,
                attachments: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(made.batch, ContentBatch::Noop);
    gone(made.child_pid);
    let fixture = FixtureProgram::blocked();
    let mut decoder = Decoder::new(fixture.script.clone()).unwrap();
    let error = decoder
        .prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[],
            },
            &before,
            ContentEdit {
                layout: None,
                source: "new",
                publisher_agent: None,
                attachments: None,
            },
            None,
        )
        .err()
        .unwrap();
    assert!(matches!(error, DecodeFault::Invoke(ref e)
        if e.kind == FailureKind::Deadline && matches!(e.cleanup, Cleanup::Confirmed)));
    fixture.assert_blocked_child_was_reaped();
    fixture.actual();
    let made = decoder
        .prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[],
            },
            &before,
            ContentEdit {
                layout: None,
                source: "new",
                publisher_agent: None,
                attachments: None,
            },
            None,
        )
        .unwrap();
    assert!(matches!(made.batch, ContentBatch::Updates(_)));
    gone(made.child_pid);
}

#[test]
fn content_batch_preserves_creation_recipient_through_replay_and_noop() {
    use tmt_colab::decoder::{ContentBatch, ContentEdit, CreationRecipient};
    let recipient = CreationRecipient {
        machine_id: "40000000-0000-4000-8000-000000000001".into(),
        agent_id: "50000000-0000-1000-8000-000000000001".into(),
    };
    let mut decoder = owner();
    for hint in [Some(&recipient), None] {
        let baseline = decoder
            .produce_page(
                BaselineInput {
                    page_state: None,
                    creator: None,
                    questions: None,
                    template: None,
                    from_template: None,
                    work: None,
                    layout: None,
                    attachments: None,
                    creation_recipient: hint,
                    ..view(b"old", "T")
                },
                None,
            )
            .unwrap();
        gone(baseline.child_pid);
        let folded = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &baseline.update,
                    updates: &[],
                },
                Role::Editor,
                None,
            )
            .unwrap();
        gone(folded.child_pid);
        for source in ["old", "new"] {
            let made = decoder
                .prepare_content_batch(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &baseline.update,
                        updates: &[],
                    },
                    &folded.projection,
                    ContentEdit {
                        layout: None,
                        source,
                        publisher_agent: None,
                        attachments: None,
                    },
                    None,
                )
                .unwrap();
            gone(made.child_pid);
            assert_eq!(made.projection["meta"], folded.projection["meta"]);
            match made.batch {
                ContentBatch::Noop => assert_eq!(source, "old"),
                ContentBatch::Updates(updates) => {
                    assert_eq!(source, "new");
                    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
                    let replay = decoder
                        .decode(
                            UpdateBatch {
                                namespace: Namespace::Content,
                                baseline: &baseline.update,
                                updates: &refs,
                            },
                            Role::Editor,
                            None,
                        )
                        .unwrap();
                    assert_eq!(replay.projection, made.projection);
                    gone(replay.child_pid);
                }
            }
            let unchanged = decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &baseline.update,
                        updates: &[],
                    },
                    Role::Editor,
                    None,
                )
                .unwrap();
            assert_eq!(unchanged.projection, folded.projection);
            gone(unchanged.child_pid);
        }
    }
}

/// Lines of ASCII, CJK and emoji, so a character's UTF-8 byte offset is not its UTF-16 offset.
fn mixed_page(lines: usize) -> String {
    (0..lines)
        .map(|n| format!("<p>第{n}行 🐈 row {n} 漢字 ✓</p>\n"))
        .collect()
}
/// The ordered updates that turn a page of source `old` into `new`, prepared by the isolated
/// decoder against the baseline of `old`, with the baseline they apply to.
fn prepared_edit(decoder: &mut Decoder, old: &str, new: &str) -> (Vec<u8>, Vec<Vec<u8>>) {
    use tmt_colab::decoder::{ContentBatch, ContentEdit};
    let baseline = decoder
        .produce_baseline(
            BaselineInput {
                page_state: None,
                work: None,
                template: None,
                from_template: None,
                layout: None,
                original_author: None,
                attachments: None,
                source: old.as_bytes(),
                title: "Page",
                publisher_agent: None,
                creation_recipient: None,
                creator: None,
                questions: None,
                source_digest: tmt_colab_model::crypto::digest(old.as_bytes()),
            },
            None,
        )
        .unwrap();
    let batch = || UpdateBatch {
        namespace: Namespace::Content,
        baseline: baseline.update.as_slice(),
        updates: &[],
    };
    let before = decoder.decode(batch(), Role::Editor, None).unwrap();
    assert_eq!(before.projection["html"], old);
    let prepared = decoder
        .prepare_content_batch(
            batch(),
            &before.projection,
            ContentEdit {
                layout: None,
                source: new,
                publisher_agent: None,
                attachments: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(prepared.projection["html"], new);
    let ContentBatch::Updates(updates) = prepared.batch else {
        panic!("expected updates")
    };
    (baseline.update, updates)
}
#[test]
fn far_apart_edits_publish_one_delta_per_changed_region_and_the_text_is_exact() {
    use yrs::{OffsetKind, Options};
    let old = mixed_page(400);
    assert!(old.len() > 15_000, "{}", old.len());
    let cases: [(&str, String); 4] = [
        (
            "two replacements far apart",
            old.replacen("row 5 ", "ROW FIVE 🐕 ", 1)
                .replacen("row 395 ", "ROW 395 🐕🐕 ", 1),
        ),
        (
            "an insertion at the start, a deletion near the end and a change between",
            format!("<h1>标题 🚀</h1>\n{}", old)
                .replacen("第200行", "第二百行", 1)
                .replacen("<p>第390行 🐈 row 390 漢字 ✓</p>\n", "", 1),
        ),
        (
            "a pure insertion and a pure deletion",
            old.replacen("<p>第100行", "<p>🆕</p>\n<p>第100行", 1)
                .replacen("<p>第300行 🐈 row 300 漢字 ✓</p>\n", "", 1),
        ),
        (
            "one word inside a line",
            old.replacen("漢字 ✓</p>\n<p>第201行", "漢字 ✗ 🐈</p>\n<p>第201行", 1),
        ),
    ];
    let mut decoder = owner();
    for (name, new) in cases {
        assert_ne!(new, old, "{name}");
        let (baseline, updates) = prepared_edit(&mut decoder, &old, &new);
        let bytes: usize = updates.iter().map(Vec::len).sum();
        // The text between the regions is not rewritten: the update carries the changed regions
        // (a few hundred bytes), not the ~16 KB the page holds between them.
        assert!(
            bytes < 1_200,
            "{name}: {bytes} bytes in {} updates",
            updates.len()
        );
        // The source is byte-identical after the isolated decoder applies them...
        let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let replayed = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &baseline,
                    updates: &refs,
                },
                Role::Editor,
                None,
            )
            .unwrap();
        assert_eq!(replayed.projection["html"], new, "{name}");
        // ...and in a document that counts characters the way the browser's Yjs does, in UTF-16
        // code units, where an emoji is two and a CJK character one.
        let browser = Doc::with_options(Options {
            offset_kind: OffsetKind::Utf16,
            ..Options::default()
        });
        let text = browser.get_or_insert_text("html");
        for bytes in std::iter::once(&baseline).chain(&updates) {
            browser
                .transact_mut()
                .apply_update(Update::decode_v1(bytes).unwrap())
                .unwrap();
        }
        assert_eq!(text.get_string(&browser.transact()), new, "{name}");
    }
}

#[test]
fn question_metadata_belongs_only_to_ordinary_pages() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/question-metadata-v1.json"
    ))
    .unwrap();
    for case in vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["valid"] == true)
    {
        let doc = Doc::new();
        doc.get_or_insert_text("html");
        let meta = doc.get_or_insert_map("meta");
        let mut tx = doc.transact_mut();
        meta.insert(&mut tx, "title", "Dashboard");
        meta.insert(&mut tx, "pageType", "shell");
        meta.insert(
            &mut tx,
            "shell",
            yrs::Any::from_json(&tmt_colab::shell::empty().to_string()).unwrap(),
        );
        let field = if case["kind"] == "creator" {
            "creator"
        } else {
            "questions"
        };
        meta.insert(
            &mut tx,
            field,
            yrs::Any::from_json(&case["value"].to_string()).unwrap(),
        );
        let bytes = tx.encode_state_as_update_v1(&StateVector::default());
        drop(tx);
        assert!(
            owner()
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &bytes,
                        updates: &[]
                    },
                    Role::Editor,
                    None
                )
                .is_err(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn own_merge_preserves_pending_structs_and_delete_sets_as_exact_raw_merges() {
    for deletion in [false, true] {
        let doc = Doc::with_client_id(5293);
        for root in ["threads", "messages", "intents", "replies"] {
            doc.get_or_insert_map(root);
        }
        let map = doc.get_or_insert_map("messages");
        map.insert(&mut doc.transact_mut(), "legacy", "Historical own value");
        let prerequisite = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let later = Doc::with_client_id(5294);
        later
            .transact_mut()
            .apply_update(Update::decode_v1(&prerequisite).unwrap())
            .unwrap();
        let messages = later.get_or_insert_map("messages");
        let vector = later.transact().state_vector();
        if deletion {
            messages.remove(&mut later.transact_mut(), "legacy");
        } else {
            messages.insert(&mut later.transact_mut(), "legacy", "Dependent own value");
        }
        let tail = later.transact().encode_state_as_update_v1(&vector);
        let incomplete = Doc::new();
        incomplete
            .transact_mut()
            .apply_update(Update::decode_v1(&tail).unwrap())
            .unwrap();
        assert!(if deletion {
            incomplete.transact().store().pending_ds().is_some()
        } else {
            incomplete.transact().store().pending_update().is_some()
        });
        let expected = yrs::merge_updates_v1([tail.as_slice()]).unwrap();
        let mut decoder = owner();
        let merged = decoder.merge(Namespace::Own, &[&tail], None).unwrap();
        assert_eq!(merged, expected, "deletion={deletion}");
        let replay = Doc::new();
        let messages = replay.get_or_insert_map("messages");
        replay
            .transact_mut()
            .apply_update(Update::decode_v1(&merged).unwrap())
            .unwrap();
        assert!(if deletion {
            replay.transact().store().pending_ds().is_some()
        } else {
            replay.transact().store().pending_update().is_some()
        });
        replay
            .transact_mut()
            .apply_update(Update::decode_v1(&prerequisite).unwrap())
            .unwrap();
        assert!(replay.transact().store().pending_update().is_none());
        assert!(replay.transact().store().pending_ds().is_none());
        if deletion {
            assert!(messages.get(&replay.transact(), "legacy").is_none());
        } else {
            assert_eq!(
                messages
                    .get(&replay.transact(), "legacy")
                    .unwrap()
                    .to_string(&replay.transact()),
                "Dependent own value"
            );
        }
    }
}

#[test]
fn page_state_metadata_accepts_only_the_three_explicit_values() {
    for (state, valid) in [
        (serde_json::json!("todo"), true),
        (serde_json::json!("in-progress"), true),
        (serde_json::json!("done"), true),
        (serde_json::json!("blocked"), false),
        (serde_json::Value::Null, false),
        (serde_json::json!(42), false),
    ] {
        let doc = Doc::new();
        let html = doc.get_or_insert_text("html");
        let meta = doc.get_or_insert_map("meta");
        let mut tx = doc.transact_mut();
        html.insert(&mut tx, 0, "<p>page</p>");
        meta.insert(&mut tx, "title", "Page");
        meta.insert(
            &mut tx,
            "pageState",
            yrs::Any::from_json(&state.to_string()).unwrap(),
        );
        let bytes = tx.encode_state_as_update_v1(&StateVector::default());
        drop(tx);
        let result = owner().decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &bytes,
                updates: &[],
            },
            Role::Editor,
            None,
        );
        assert_eq!(result.is_ok(), valid, "pageState {state}");
        if let Ok(value) = result {
            assert_eq!(value.projection["meta"]["pageState"], state);
            gone(value.child_pid);
        }
    }
}

#[test]
fn page_state_action_preserves_source_and_remains_in_the_next_baseline() {
    use tmt_colab::decoder::{ContentBatch, StructuredEdit, StructuredKind};
    let source = b"<p>the page</p>";
    let mut decoder = owner();
    let baseline = decoder
        .produce_baseline(work_view(source, None), None)
        .unwrap();
    let batch = || UpdateBatch {
        namespace: Namespace::Content,
        baseline: &baseline.update,
        updates: &[],
    };
    let before = decoder.decode(batch(), Role::Editor, None).unwrap();
    assert!(before.projection["meta"].get("pageState").is_none());
    let edit = |state: serde_json::Value| StructuredEdit {
        kind: StructuredKind::PageState,
        action: "page.state".into(),
        body: serde_json::json!({"state": state}),
        attribution: serde_json::json!({"writerId": WRITER,"timeMs": 1000}),
    };
    let noop = decoder
        .prepare_structured_batch(
            batch(),
            &before.projection,
            &edit(serde_json::json!("todo")),
            None,
        )
        .unwrap();
    assert!(matches!(noop.batch, ContentBatch::Noop));
    let prepared = decoder
        .prepare_structured_batch(
            batch(),
            &before.projection,
            &edit(serde_json::json!("done")),
            None,
        )
        .unwrap();
    let ContentBatch::Updates(updates) = prepared.batch else {
        panic!("expected update")
    };
    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let after = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline.update,
                updates: &refs,
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(after.projection["meta"]["pageState"], "done");
    assert_eq!(after.projection["html"], "<p>the page</p>");
    let next = decoder
        .produce_baseline(
            BaselineInput {
                page_state: after.projection["meta"].get("pageState"),
                ..work_view(source, None)
            },
            None,
        )
        .unwrap();
    let rotated = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &next.update,
                updates: &[],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(rotated.projection["meta"]["pageState"], "done");
    assert!(
        decoder
            .prepare_structured_batch(
                batch(),
                &before.projection,
                &edit(serde_json::json!("blocked")),
                None
            )
            .is_err()
    );
}
