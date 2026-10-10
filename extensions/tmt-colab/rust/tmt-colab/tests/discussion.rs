mod support;
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use serde_json::{Value, json};
use tmt_colab::decoder::{Decoder, Namespace, Role, UpdateBatch};
use yrs::{Any, Doc, Map, ReadTxn, StateVector, Transact};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../contracts/vectors/discussion-v1.json"
    ))
    .unwrap()
}
fn update(root: &str, key: &str, value: &Value) -> Vec<u8> {
    let doc = Doc::new();
    for name in ["threads", "messages", "intents", "replies"] {
        doc.get_or_insert_map(name);
    }
    doc.get_or_insert_map(root).insert(
        &mut doc.transact_mut(),
        key,
        Any::from_json(&value.to_string()).unwrap(),
    );
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
}
#[test]
fn real_child_admits_literal_records_and_tombstones_and_rejects_typed_mutations() {
    let f = fixture();
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    for (root, id, name) in [
        ("threads", "threadId", "thread"),
        ("messages", "messageId", "comment"),
    ] {
        let value = &f[name];
        let key = format!("{}:1", value[id].as_str().unwrap());
        let decode = |decoder: &mut Decoder, bytes: &[u8]| {
            decoder.decode(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &[],
                    updates: &[bytes],
                },
                Role::Commenter,
                None,
            )
        };
        let valid = update(root, &key, value);
        let result = decode(&mut decoder, &valid).unwrap();
        assert_eq!(result.projection[root][&key], *value);
        assert_eq!(
            kill(Pid::from_raw(result.child_pid as i32), None),
            Err(Errno::ESRCH)
        );
        let mut tombstone = value.clone();
        tombstone["deleted"] = json!(true);
        if name == "thread" {
            tombstone["anchor"] = Value::Null;
        } else {
            tombstone["body"] = json!("");
        }
        assert!(decode(&mut decoder, &update(root, &key, &tombstone)).is_ok());
        for (field, invalid) in [
            ("version", json!(2)),
            ("epoch", json!("01")),
            ("revision", json!("0")),
            ("senderDevice", json!("not-a-device")),
            ("deleted", json!("false")),
            ("deviceName", json!("é".repeat(65))),
            ("at", json!("01")),
            ("at", json!("-1")),
            ("at", json!("1.5")),
            ("at", json!(1791072000000u64)),
            ("at", json!("8640000000000001")),
            ("at", json!(null)),
            ("unexpected", json!(true)),
        ] {
            let mut malformed = value.clone();
            malformed[field] = invalid;
            assert!(
                decode(&mut decoder, &update(root, &key, &malformed)).is_err(),
                "{name}/{field}"
            );
        }
        for at in ["0", "8640000000000000"] {
            let mut boundary = value.clone();
            boundary["at"] = json!(at);
            assert!(decode(&mut decoder, &update(root, &key, &boundary)).is_ok());
        }
        let mut missing_time = value.clone();
        missing_time.as_object_mut().unwrap().remove("at");
        assert!(decode(&mut decoder, &update(root, &key, &missing_time)).is_err());
        assert!(decode(&mut decoder, &update("replies", &key, value)).is_err());
        assert!(decode(&mut decoder, &update(root, "wrong-key", value)).is_err());
        let mut malformed = value.clone();
        if name == "thread" {
            malformed["anchor"]["prefix"] = json!("🐈".repeat(33));
        } else {
            malformed["body"] = json!("é".repeat(8193));
        }
        assert!(decode(&mut decoder, &update(root, &key, &malformed)).is_err());
        let mut boundary = value.clone();
        if name == "thread" {
            boundary["anchor"]["prefix"] = json!("🐈".repeat(32));
        } else {
            boundary["body"] = json!("é".repeat(8192));
        }
        assert!(decode(&mut decoder, &update(root, &key, &boundary)).is_ok());
        if name == "thread" {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove("anchor");
            assert!(decode(&mut decoder, &update(root, &key, &missing)).is_err());
        }
        let mut invalid_delete = value.clone();
        invalid_delete["deleted"] = json!(true);
        assert!(decode(&mut decoder, &update(root, &key, &invalid_delete)).is_err());
        // A valid control follows failures: no rejected projection poisons later work.
        assert_eq!(
            decode(&mut decoder, &valid).unwrap().projection[root][&key],
            *value
        );
    }
}

#[test]
fn real_child_refuses_overwritten_and_removed_immutable_discussion_keys() {
    let f = fixture();
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    for remove in [false, true] {
        let doc = Doc::new();
        let map = doc.get_or_insert_map("threads");
        let key = format!("{}:1", f["thread"]["threadId"].as_str().unwrap());
        map.insert(
            &mut doc.transact_mut(),
            key.as_str(),
            Any::from_json(&f["thread"].to_string()).unwrap(),
        );
        let baseline = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let vector = doc.transact().state_vector();
        if remove {
            map.remove(&mut doc.transact_mut(), &key);
        } else {
            let mut changed = f["thread"].clone();
            changed["resolved"] = json!(true);
            map.insert(
                &mut doc.transact_mut(),
                key.as_str(),
                Any::from_json(&changed.to_string()).unwrap(),
            );
        }
        let tail = doc.transact().encode_state_as_update_v1(&vector);
        assert!(
            decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Own,
                        baseline: &baseline,
                        updates: &[&tail]
                    },
                    Role::Commenter,
                    None
                )
                .is_err()
        );
        assert!(
            decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Own,
                        baseline: &baseline,
                        updates: &[]
                    },
                    Role::Commenter,
                    None
                )
                .is_ok()
        );
    }
}

#[test]
fn real_child_admits_status_actions_and_failures_and_refuses_their_mutation() {
    let f = fixture();
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    for value in [&f["statusCases"][2]["actions"][0], &f["notification"]] {
        let kind = value["kind"].as_str().unwrap();
        let id = value[if kind == "thread-status" {
            "actionId"
        } else {
            "operationId"
        }]
        .as_str()
        .unwrap();
        let key = format!("{id}:{kind}");
        let valid = update("messages", &key, value);
        let mut decode = |baseline: &[u8], updates: &[&[u8]]| {
            decoder.decode(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline,
                    updates,
                },
                Role::Commenter,
                None,
            )
        };
        assert_eq!(
            decode(&[], &[&valid]).unwrap().projection["messages"][&key],
            *value
        );
        for (field, bad) in [
            ("revision", json!("2")),
            ("deleted", json!(true)),
            ("at", json!("01")),
            ("unexpected", json!(true)),
        ] {
            let mut malformed = value.clone();
            malformed[field] = bad;
            assert!(
                decode(&[], &[&update("messages", &key, &malformed)]).is_err(),
                "{kind}/{field}"
            );
        }
        assert!(decode(&[], &[&update("threads", &key, value)]).is_err());
        assert!(decode(&[], &[&update("messages", "wrong-key", value)]).is_err());
        for remove in [false, true] {
            let doc = Doc::new();
            for root in ["threads", "messages", "intents", "replies"] {
                doc.get_or_insert_map(root);
            }
            let map = doc.get_or_insert_map("messages");
            map.insert(
                &mut doc.transact_mut(),
                key.as_str(),
                Any::from_json(&value.to_string()).unwrap(),
            );
            let baseline = doc
                .transact()
                .encode_state_as_update_v1(&StateVector::default());
            let vector = doc.transact().state_vector();
            if remove {
                map.remove(&mut doc.transact_mut(), &key);
            } else {
                let mut changed = value.clone();
                changed["deviceName"] = json!("Changed label");
                map.insert(
                    &mut doc.transact_mut(),
                    key.as_str(),
                    Any::from_json(&changed.to_string()).unwrap(),
                );
            }
            let tail = doc.transact().encode_state_as_update_v1(&vector);
            assert!(
                decode(&baseline, &[&tail]).is_err(),
                "{kind}/remove={remove}"
            );
            assert_eq!(
                decode(&baseline, &[]).unwrap().projection["messages"][&key],
                *value
            );
        }
    }
}

#[test]
fn isolated_own_preparation_is_a_bounded_immutable_delta_and_does_not_commit() {
    use tmt_colab::decoder::OwnRecord;
    let f = fixture();
    let mut action = f["statusCases"][2]["actions"][0].clone();
    let mut previous = action.clone();
    previous["actionId"] = json!("00000000-0000-4000-8000-000000000009");
    action["previous"] = json!({"writer":previous["senderDevice"],"id":previous["actionId"]});
    let previous_key = format!("{}:thread-status", previous["actionId"].as_str().unwrap());
    let key = format!("{}:thread-status", action["actionId"].as_str().unwrap());
    let baseline = update("messages", &previous_key, &previous);
    let records = [OwnRecord {
        root: "messages".into(),
        key: key.clone(),
        value: action.clone(),
    }];
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    let prepared = decoder
        .prepare_own(
            UpdateBatch {
                namespace: Namespace::Own,
                baseline: &baseline,
                updates: &[],
            },
            &records,
            None,
        )
        .unwrap();
    assert_eq!(prepared.projection["messages"][&previous_key], previous);
    assert_eq!(prepared.projection["messages"][&key], action);
    assert!(prepared.merged.len() <= tmt_colab::decoder::UPDATE_BYTES);
    let original = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Own,
                baseline: &baseline,
                updates: &[],
            },
            Role::Commenter,
            None,
        )
        .unwrap();
    assert!(original.projection["messages"].get(&key).is_none());
    let committed = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Own,
                baseline: &baseline,
                updates: &[&prepared.merged],
            },
            Role::Commenter,
            None,
        )
        .unwrap();
    assert_eq!(committed.projection, prepared.projection);
    // Replaying the exact record is harmless; replacing its timestamp or label is not.
    assert!(
        decoder
            .prepare_own(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &baseline,
                    updates: &[&prepared.merged]
                },
                &records,
                None
            )
            .is_ok()
    );
    let mut changed = records[0].clone();
    changed.value["deviceName"] = json!("Changed");
    assert!(
        decoder
            .prepare_own(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &baseline,
                    updates: &[&prepared.merged]
                },
                &[changed],
                None
            )
            .is_err()
    );
    for records in [
        vec![],
        vec![records[0].clone(); 2],
        vec![records[0].clone(); 33],
    ] {
        assert!(
            decoder
                .prepare_own(
                    UpdateBatch {
                        namespace: Namespace::Own,
                        baseline: &baseline,
                        updates: &[]
                    },
                    &records,
                    None
                )
                .is_err()
        );
    }
    assert!(
        decoder
            .prepare_own(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[]
                },
                &records,
                None
            )
            .is_err()
    );
    assert_eq!(
        decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &baseline,
                    updates: &[&prepared.merged]
                },
                Role::Commenter,
                None
            )
            .unwrap()
            .projection,
        prepared.projection
    );
}

#[test]
fn shared_status_recipient_vectors_accept_core_ids_and_refuse_ambiguous_or_unbounded_batches() {
    let f = fixture();
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    let decode = |decoder: &mut Decoder, record: &Value| {
        let key = format!("{}:thread-status", record["actionId"].as_str().unwrap());
        let bytes = update("messages", &key, record);
        decoder.decode(
            UpdateBatch {
                namespace: Namespace::Own,
                baseline: &[],
                updates: &[&bytes],
            },
            Role::Commenter,
            None,
        )
    };
    for row in f["recipientCases"].as_array().unwrap() {
        assert_eq!(
            decode(&mut decoder, &row["record"]).is_ok(),
            row["valid"].as_bool().unwrap(),
            "{}",
            row["name"]
        );
    }
    let mut record = f["recipientCases"][0]["record"].clone();
    record["recipients"] = json!(vec![record["recipients"][0].clone(); 1001]);
    assert!(decode(&mut decoder, &record).is_err());
    assert!(decode(&mut decoder, &f["recipientCases"][0]["record"]).is_ok());
}

#[test]
fn attachment_publications_survive_checkpoints_and_reject_mutation_or_wrong_roots() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/attachment-v1.json"
    ))
    .unwrap();
    let row = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "publication-document")
        .unwrap();
    let publication: Value = serde_json::from_str(row["input"].as_str().unwrap()).unwrap();
    let key = publication["attachmentId"].as_str().unwrap();
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    for remove in [false, true] {
        let doc = Doc::new();
        let map = doc.get_or_insert_map("intents");
        map.insert(
            &mut doc.transact_mut(),
            key,
            Any::from_json(&publication.to_string()).unwrap(),
        );
        let baseline = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let positive = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &baseline,
                    updates: &[],
                },
                Role::Commenter,
                None,
            )
            .unwrap();
        assert_eq!(positive.projection["intents"][key], publication);
        let vector = doc.transact().state_vector();
        if remove {
            map.remove(&mut doc.transact_mut(), key);
        } else {
            let mut changed = publication.clone();
            changed["descriptorHash"] = json!("00".repeat(32));
            map.insert(
                &mut doc.transact_mut(),
                key,
                Any::from_json(&changed.to_string()).unwrap(),
            );
        }
        let tail = doc.transact().encode_state_as_update_v1(&vector);
        assert!(
            decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Own,
                        baseline: &baseline,
                        updates: &[&tail]
                    },
                    Role::Commenter,
                    None
                )
                .is_err()
        );
        assert_eq!(
            decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Own,
                        baseline: &baseline,
                        updates: &[]
                    },
                    Role::Commenter,
                    None
                )
                .unwrap()
                .projection["intents"][key],
            publication
        );
    }
    for (root, name) in [("messages", key), ("intents", "wrong-key")] {
        let doc = Doc::new();
        doc.get_or_insert_map(root).insert(
            &mut doc.transact_mut(),
            name,
            Any::from_json(&publication.to_string()).unwrap(),
        );
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(
            decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Own,
                        baseline: &[],
                        updates: &[&update]
                    },
                    Role::Commenter,
                    None
                )
                .is_err()
        );
    }
}

#[test]
fn real_child_keeps_proposal_decision_keys_immutable() {
    let f = fixture();
    let value = f["decisionCases"][0]["actions"][1].clone();
    let key = format!("{}:proposal-decision", value["actionId"].as_str().unwrap());
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    for remove in [false, true] {
        let doc = Doc::new();
        for name in ["threads", "messages", "intents", "replies"] {
            doc.get_or_insert_map(name);
        }
        let map = doc.get_or_insert_map("messages");
        map.insert(
            &mut doc.transact_mut(),
            key.as_str(),
            Any::from_json(&value.to_string()).unwrap(),
        );
        let baseline = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let vector = doc.transact().state_vector();
        if remove {
            map.remove(&mut doc.transact_mut(), &key);
        } else {
            let mut changed = value.clone();
            changed["decision"] = json!("declined");
            map.insert(
                &mut doc.transact_mut(),
                key.as_str(),
                Any::from_json(&changed.to_string()).unwrap(),
            );
        }
        let tail = doc.transact().encode_state_as_update_v1(&vector);
        assert!(
            decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Own,
                        baseline: &baseline,
                        updates: &[]
                    },
                    Role::Commenter,
                    None
                )
                .is_ok()
        );
        assert!(
            decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Own,
                        baseline: &baseline,
                        updates: &[&tail]
                    },
                    Role::Commenter,
                    None
                )
                .is_err()
        );
    }
}
