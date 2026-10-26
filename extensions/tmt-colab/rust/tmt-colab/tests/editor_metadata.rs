//! Closed request/result grammar through the real isolated child, including immutable receipts.
mod support;
use serde_json::Value;
use tmt_colab::decoder::{Decoder, Namespace, OwnRecord, UpdateBatch};
#[test]
fn metadata_requests_and_terminal_receipts_follow_the_shared_vectors() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/editor-metadata-v1.json"
    ))
    .unwrap();
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    for list in ["requests", "results"] {
        for case in vectors[list].as_array().unwrap() {
            let record = OwnRecord {
                root: case["root"].as_str().unwrap().into(),
                key: case["key"].as_str().unwrap().into(),
                value: case["record"].clone(),
            };
            if list == "requests" && case["valid"] == true {
                assert_eq!(
                    tmt_colab_model::values::encode_binary(&tmt_colab_model::crypto::digest(
                        &tmt_colab::closed::canonical(&case["record"]).unwrap()
                    )),
                    case["requestSha256"]
                );
            }
            let result = decoder.prepare_own(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &[],
                    updates: &[],
                },
                std::slice::from_ref(&record),
                None,
            );
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if let Ok(prepared) = result {
                let batch = UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &[],
                    updates: &[&prepared.merged],
                };
                assert!(
                    decoder
                        .prepare_own(batch, std::slice::from_ref(&record), None)
                        .is_ok()
                );
                let mut changed = record.clone();
                if list == "results" {
                    if changed.value["result"]["state"] == "rejected" {
                        changed.value["result"]["message"] =
                            serde_json::json!("Another valid refusal");
                    } else {
                        changed.value["result"]["revision"] =
                            serde_json::json!(format!("v1:{}", "1".repeat(64)));
                    }
                } else if changed.value["action"] == "creator.set" {
                    changed.value["body"]["selector"] = serde_json::json!("another-agent");
                } else {
                    changed.value["target"]["title"] = serde_json::json!("Another title");
                }
                assert!(
                    decoder
                        .prepare_own(
                            UpdateBatch {
                                namespace: Namespace::Own,
                                baseline: &[],
                                updates: &[&prepared.merged]
                            },
                            &[changed],
                            None
                        )
                        .is_err()
                );
            }
        }
    }
}
