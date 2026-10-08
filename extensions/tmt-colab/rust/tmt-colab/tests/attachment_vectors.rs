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
