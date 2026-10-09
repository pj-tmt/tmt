//! Real isolated decoder controls over the same attachment corpus as the browser.
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tmt_colab::decoder::{
    BaselineInput, ContentBatch, ContentEdit, Decoder, Namespace, Role, UpdateBatch,
};
use tmt_colab_model::attachment::DocumentAttachments;
use yrs::{Any, Doc, Map, ReadTxn, StateVector, Text, Transact};

#[test]
fn descriptors_survive_real_decode_source_edits_checkpoints_and_baselines() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/attachment-v1.json"
    ))
    .unwrap();
    let mut decoder = Decoder::new(env!("CARGO_BIN_EXE_tmt-colab").into()).unwrap();
    for case in corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["operation"] == "document")
    {
        let projection: Value = serde_json::from_str(case["input"].as_str().unwrap()).unwrap();
        let doc = Doc::with_client_id(1853);
        let html = doc.get_or_insert_text("html");
        let meta = doc.get_or_insert_map("meta");
        let update = {
            let mut tx = doc.transact_mut();
            html.insert(&mut tx, 0, projection["html"].as_str().unwrap());
            for (key, value) in projection["meta"].as_object().unwrap() {
                meta.insert(
                    &mut tx,
                    key.as_str(),
                    Any::from_json(&value.to_string()).unwrap(),
                );
            }
            tx.encode_state_as_update_v1(&StateVector::default())
        };
        let batch = || UpdateBatch {
            namespace: Namespace::Content,
            baseline: &[],
            updates: &[],
        };
        let result = decoder.decode(
            UpdateBatch {
                updates: &[&update],
                ..batch()
            },
            Role::Editor,
            None,
        );
        if !case["admit"].as_bool().unwrap() {
            assert!(result.is_err(), "{}", case["name"]);
            continue;
        }
        let result = result.unwrap();
        assert_eq!(result.projection, projection);
        assert_eq!(
            kill(Pid::from_raw(result.child_pid as i32), None),
            Err(Errno::ESRCH)
        );
        let prepared = decoder
            .prepare_content_batch(
                UpdateBatch {
                    updates: &[&update],
                    ..batch()
                },
                &projection,
                ContentEdit {
                    source: "changed source",
                    publisher_agent: None,
                    attachments: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(prepared.projection["meta"], projection["meta"]);
        let ContentBatch::Updates(deltas) = prepared.batch else {
            panic!("expected changed source")
        };
        let refs: Vec<_> = std::iter::once(update.as_slice())
            .chain(deltas.iter().map(Vec::as_slice))
            .collect();
        let merged = decoder.merge(Namespace::Content, &refs, None).unwrap();
        let checkpoint = decoder
            .decode(
                UpdateBatch {
                    baseline: &merged,
                    ..batch()
                },
                Role::Editor,
                None,
            )
            .unwrap();
        assert_eq!(checkpoint.projection["meta"], projection["meta"]);
        let attachments: DocumentAttachments =
            serde_json::from_value(projection["meta"]["attachments"].clone()).unwrap();
        let view = || BaselineInput {
            original_author: None,
            source: b"changed source",
            title: "Storage",
            publisher_agent: None,
            creation_recipient: None,
            attachments: Some(&attachments),
            source_digest: Sha256::digest(b"changed source").into(),
        };
        let baseline = decoder.produce_baseline(view(), None).unwrap();
        decoder
            .verify_baseline(view(), &baseline.update, baseline.commitment, None)
            .unwrap();
        let restored = decoder
            .decode(
                UpdateBatch {
                    baseline: &baseline.update,
                    ..batch()
                },
                Role::Editor,
                None,
            )
            .unwrap();
        assert_eq!(restored.projection["meta"], projection["meta"]);
        assert_eq!(restored.projection["html"], "changed source");
        let page = decoder.produce_page(view(), None).unwrap();
        let refs: Vec<_> = page.chunks.iter().map(Vec::as_slice).collect();
        let created = decoder
            .decode(
                UpdateBatch {
                    updates: &refs,
                    ..batch()
                },
                Role::Editor,
                None,
            )
            .unwrap();
        assert_eq!(created.projection, restored.projection);
    }
}

/// A typed set/remove change runs through the real isolated child: the deltas replay to the
/// edited list, the source is untouched, and a change bound to another source never prepares.
#[test]
fn typed_attachment_change_is_prepared_by_the_isolated_decoder() {
    use tmt_colab_model::attachment::DocumentChange;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/attachment-change-v1.json"
    ))
    .unwrap();
    let source = oracle["source"].as_str().unwrap();
    let descriptors = &oracle["descriptors"];
    let change = |set: &[&str], remove: &[&str]| -> DocumentChange {
        serde_json::from_value(serde_json::json!({
            "set": set.iter().map(|k| descriptors[*k].clone()).collect::<Vec<_>>(),
            "remove": remove.iter().map(|k| descriptors[*k]["attachmentId"].clone()).collect::<Vec<_>>(),
        }))
        .unwrap()
    };
    let doc = Doc::with_client_id(1855);
    let html = doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    let first = {
        let mut tx = doc.transact_mut();
        html.insert(&mut tx, 0, source);
        meta.insert(&mut tx, "title", "Files");
        tx.encode_state_as_update_v1(&StateVector::default())
    };
    let mut decoder = Decoder::new(env!("CARGO_BIN_EXE_tmt-colab").into()).unwrap();
    let mut log: Vec<Vec<u8>> = vec![first];
    let ids = |projection: &Value| -> Vec<String> {
        projection["meta"]["attachments"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|d| d["attachmentId"].as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let id = |key: &str| {
        descriptors[key]["attachmentId"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let mut step = |change: &DocumentChange, decoder: &mut Decoder| {
        let refs: Vec<&[u8]> = log.iter().map(Vec::as_slice).collect();
        let current = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &refs,
                },
                Role::Editor,
                None,
            )
            .unwrap()
            .projection;
        let prepared = decoder.prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &refs,
            },
            &current,
            ContentEdit {
                source,
                publisher_agent: None,
                attachments: Some(change),
            },
            None,
        )?;
        let ContentBatch::Updates(deltas) = prepared.batch else {
            panic!("the list changed, so a delta is expected")
        };
        log.extend(deltas);
        let refs: Vec<&[u8]> = log.iter().map(Vec::as_slice).collect();
        let replayed = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &refs,
                },
                Role::Editor,
                None,
            )
            .unwrap()
            .projection;
        // The preparer's projection is exactly what a fresh replay of the log yields.
        assert_eq!(replayed, prepared.projection);
        assert_eq!(replayed["html"], source);
        Ok::<_, tmt_colab::decoder::DecodeFault>(replayed)
    };
    let one = step(&change(&["a"], &[]), &mut decoder).unwrap();
    assert_eq!(ids(&one), vec![id("a")]);
    let swapped = step(&change(&["b"], &["a"]), &mut decoder).unwrap();
    assert_eq!(ids(&swapped), vec![id("b")]);
    // The last removal drops the key: absence is the no-attachment grammar.
    let none = step(&change(&[], &["b"]), &mut decoder).unwrap();
    assert!(none["meta"].get("attachments").is_none());
    // Bound to another source, in the wrong namespace, or removing what is absent: refused
    // by the parent before any child work.
    for refused in [
        change(&["foreignDigest"], &[]),
        change(&["ownNamespace"], &[]),
        change(&[], &["a"]),
    ] {
        assert!(step(&refused, &mut decoder).is_err());
    }
}
