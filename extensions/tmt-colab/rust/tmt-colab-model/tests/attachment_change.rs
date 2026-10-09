//! The typed document attachment change against its independent oracle
//! (`contracts/vectors/attachment-change-reference.py`), shared with the browser.
use serde_json::Value;
use sha2::{Digest, Sha256};
use tmt_colab_model::attachment::{Descriptor, DocumentChange};

fn descriptor(value: &Value) -> Descriptor {
    Descriptor::from_json(value.to_string().as_bytes()).unwrap()
}

/// The deterministic fillers the oracle names `{"generated": n}`.
fn generated(template: &Value, count: usize) -> Vec<Descriptor> {
    (0..count)
        .map(|i| {
            let n = 0x1000 + i as u64;
            let mut value = template.clone();
            value["attachmentId"] = format!("00000000-0000-4000-8000-{n:012x}").into();
            value["objectId"] = format!("{:02x}", n & 0xff).repeat(32).into();
            value["filename"] = format!("file-{n}.bin").into();
            descriptor(&value)
        })
        .collect()
}

#[test]
fn typed_change_matches_the_shared_oracle() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/attachment-change-v1.json"
    ))
    .unwrap();
    let digest: [u8; 32] = Sha256::digest(corpus["source"].as_str().unwrap().as_bytes()).into();
    let named = &corpus["descriptors"];
    let template = &named["a"];
    for case in corpus["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let current: Vec<Descriptor> = match &case["current"] {
            Value::Array(keys) => keys
                .iter()
                .map(|k| descriptor(&named[k.as_str().unwrap()]))
                .collect(),
            other => generated(template, other["generated"].as_u64().unwrap() as usize),
        };
        let change: DocumentChange = serde_json::from_value(serde_json::json!({
            "set": case["set"].as_array().unwrap().iter()
                .map(|k| named[k.as_str().unwrap()].clone()).collect::<Vec<_>>(),
            "remove": case["remove"],
        }))
        .unwrap();
        let outcome = change
            .validate(&digest)
            .and_then(|()| change.apply(&current));
        match &case["result"] {
            Value::String(refused) => {
                assert_eq!(refused, "refused", "{name}");
                assert!(outcome.is_err(), "{name} must be refused");
            }
            ids => {
                let got: Vec<String> = outcome
                    .unwrap_or_else(|_| panic!("{name} must be admitted"))
                    .into_iter()
                    .map(|d| d.attachment_id)
                    .collect();
                let want: Vec<String> = ids
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(got, want, "{name}");
            }
        }
    }
}

#[test]
fn wire_shape_is_strict_and_bounded() {
    for raw in [
        r#"{"set":[],"remove":[],"extra":1}"#,
        r#"{"remove":"00000000-0000-4000-8000-000000000001"}"#,
        r#"{"set":{"kind":"document"}}"#,
    ] {
        assert!(
            serde_json::from_str::<DocumentChange>(raw).is_err(),
            "{raw}"
        );
    }
    let too_many = serde_json::json!({
        "remove": (0..129).map(|i| format!("00000000-0000-4000-8000-{i:012x}")).collect::<Vec<_>>()
    });
    assert!(serde_json::from_value::<DocumentChange>(too_many).is_err());
    let empty: DocumentChange = serde_json::from_str("{}").unwrap();
    assert!(empty.is_empty());
    assert_eq!(serde_json::to_string(&empty).unwrap(), "{}");
}
