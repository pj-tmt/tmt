//! The typed `edit-request` own record through the real isolated decoder child: what the vectors
//! admit is prepared as one own update within the update cap, what they refuse is not, and a
//! stored request cannot be replaced.
mod support;
use serde_json::Value;
use tmt_colab::decoder::{Decoder, Namespace, OwnRecord, Role, UPDATE_BYTES, UpdateBatch};
use yrs::{Any, Doc, Map, ReadTxn, Transact, Update, updates::decoder::Decode};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../contracts/vectors/edit-request-v1.json"
    ))
    .unwrap()
}
/// Expands `{"fill": text, "count": n}` to the text repeated n times, anywhere in the value.
fn expand(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.len() == 2
                && let (Some(Value::String(text)), Some(count)) =
                    (map.get("fill"), map.get("count"))
            {
                *value = Value::String(text.repeat(count.as_u64().unwrap() as usize));
                return;
            }
            map.values_mut().for_each(expand);
        }
        Value::Array(items) => items.iter_mut().for_each(expand),
        _ => {}
    }
}
fn records(v: &Value, name: &str) -> OwnRecord {
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap();
    let mut value = case["record"].clone();
    expand(&mut value);
    OwnRecord {
        root: case["root"].as_str().unwrap().into(),
        key: case["key"].as_str().unwrap().into(),
        value,
    }
}
fn decoder() -> Decoder {
    Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap()
}
fn batch<'a>(updates: &'a [&'a [u8]]) -> UpdateBatch<'a> {
    UpdateBatch {
        namespace: Namespace::Own,
        baseline: &[],
        updates,
    }
}

#[test]
fn the_largest_edit_request_is_one_own_update_within_the_cap() {
    let v = vectors();
    let record = records(&v, "maximal");
    let source = record.value["source"].as_str().unwrap().len();
    let hunk_text: usize = record.value["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["old"].as_str().unwrap().len() + h["new"].as_str().unwrap().len())
        .sum();
    assert_eq!(
        (source, hunk_text),
        (
            tmt_colab::edit_request::SOURCE_BYTES,
            tmt_colab::edit_request::HUNK_BYTES
        )
    );
    let prepared = decoder()
        .prepare_own(batch(&[]), std::slice::from_ref(&record), None)
        .unwrap();
    assert!(prepared.merged.len() <= UPDATE_BYTES);
    assert_eq!(prepared.projection["intents"][&record.key], record.value);
}

#[test]
fn the_real_child_admits_and_refuses_what_the_vectors_list_and_keeps_a_request_immutable() {
    let v = vectors();
    let mut decoder = decoder();
    for case in v["cases"].as_array().unwrap() {
        let record = records(&v, case["name"].as_str().unwrap());
        let prepared = decoder.prepare_own(batch(&[]), &[record], None);
        assert_eq!(
            prepared.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
    // The exact request again is harmless, and the decoder's own edit never replaces one.
    let first = records(&v, "minimal");
    let stored = decoder
        .prepare_own(batch(&[]), std::slice::from_ref(&first), None)
        .unwrap()
        .merged;
    assert!(
        decoder
            .prepare_own(batch(&[&stored]), std::slice::from_ref(&first), None)
            .is_ok()
    );
    // A later update from the writer's stream that replaces or removes it is refused: another
    // valid request under the same operation ID, as a foreign update would carry it.
    let other = records(&v, "maximal").value;
    for replacement in [Some(other), None] {
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v1(&stored).unwrap())
            .unwrap();
        let vector = doc.transact().state_vector();
        let intents = doc.get_or_insert_map("intents");
        match replacement {
            Some(value) => {
                intents.insert(
                    &mut doc.transact_mut(),
                    first.key.as_str(),
                    Any::from_json(&value.to_string()).unwrap(),
                );
            }
            None => {
                intents.remove(&mut doc.transact_mut(), first.key.as_str());
            }
        }
        let tail = doc.transact().encode_state_as_update_v1(&vector);
        assert!(
            decoder
                .decode(batch(&[&stored, &tail]), Role::Commenter, None)
                .is_err()
        );
    }
    assert!(
        decoder
            .decode(batch(&[&stored]), Role::Commenter, None)
            .is_ok()
    );
}
