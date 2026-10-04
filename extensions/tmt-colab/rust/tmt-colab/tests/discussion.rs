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
