//! Developer-only differential transport, not a product CLI or model I/O dependency.
use ed25519_dalek::SigningKey;
use serde::Deserialize;
use std::io::{self, Read};
use tmt_colab_model::object::{self, Envelope, Header};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    seal: bool,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    envelope: serde_json::Value,
    public: String,
    secret: String,
    plaintext: String,
    seed: Option<String>,
}
fn hex(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2) && value.len() <= 128);
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).expect("fixture hex"))
        .collect()
}
fn main() {
    let mut bytes = String::new();
    io::stdin()
        .take(2 * 1024 * 1024)
        .read_to_string(&mut bytes)
        .expect("fixture input");
    let input: Input = serde_json::from_str(&bytes).expect("fixture schema");
    assert!(!input.cases.is_empty() && input.cases.len() <= 6);
    let mut results = Vec::new();
    for case in input.cases {
        let envelope = Envelope::from_json(&serde_json::to_vec(&case.envelope).unwrap()).unwrap();
        let context = Header::decode(envelope.header()).unwrap().context;
        let secret = hex(&case.secret).try_into().expect("secret width");
        let public = hex(&case.public).try_into().expect("public width");
        let plaintext = hex(&case.plaintext);
        if input.seal {
            let signer = SigningKey::from_bytes(
                &hex(case.seed.as_deref().expect("public fixture seed"))
                    .try_into()
                    .unwrap(),
            );
            assert_eq!(signer.verifying_key().to_bytes(), public);
            let sealed = object::seal(&context, &secret, &signer, &plaintext).unwrap();
            results.push(
                serde_json::from_slice::<serde_json::Value>(&sealed.to_json().unwrap()).unwrap(),
            );
        } else {
            assert_eq!(
                object::open(&envelope, &context, &secret, &public).unwrap(),
                plaintext
            );
            results.push(serde_json::Value::Bool(true));
        }
    }
    println!("{}", serde_json::to_string(&results).unwrap());
}
