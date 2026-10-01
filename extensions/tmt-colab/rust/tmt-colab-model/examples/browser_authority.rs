//! Developer-only fresh native authority material for browser interoperability.
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use std::io::{self, Read};
use tmt_colab_model::{statement, wrap};
fn hex(value: &str) -> [u8; 32] {
    assert_eq!(value.len(), 64);
    std::array::from_fn(|i| u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).unwrap())
}
fn wire(bytes: Vec<u8>) -> Value {
    serde_json::from_slice(&bytes).unwrap()
}
fn main() {
    let mut bytes = String::new();
    io::stdin()
        .take(16 * 1024)
        .read_to_string(&mut bytes)
        .unwrap();
    let v: Value = serde_json::from_str(&bytes).unwrap();
    let string = |name: &str| v[name].as_str().unwrap();
    let owner = SigningKey::from_bytes(&hex(string("seed")));
    let space = string("space");
    let genesis = statement::sign(
        space,
        None,
        "member.add",
        string("payload").as_bytes(),
        &owner,
    )
    .unwrap();
    let head = genesis
        .verify_next(space, owner.verifying_key().as_bytes(), None)
        .unwrap()
        .head;
    let payload = serde_json::to_vec(&json!({"pageId":string("page"),"mode":"static"})).unwrap();
    let successor = statement::sign(space, Some(&head), "page.scripts", &payload, &owner).unwrap();
    let fixture = wrap::Envelope::from_json(&serde_json::to_vec(&v["wrap"]).unwrap()).unwrap();
    let header = fixture.header().unwrap();
    let recipient = wrap::RecipientKey::from_seed(&hex(string("recipientSeed"))).unwrap();
    assert_eq!(header.recipient_key, recipient.public_key());
    let key = hex(string("epochKey"));
    let mut wraps = Vec::new();
    for _ in 0..2 {
        let sealed = wrap::seal(&header, &key, &owner).unwrap();
        assert_eq!(
            wrap::open(
                &sealed,
                &header,
                &recipient,
                owner.verifying_key().as_bytes()
            )
            .unwrap(),
            key
        );
        wraps.push(wire(sealed.to_json().unwrap()));
    }
    println!(
        "{}",
        json!({"statements":[wire(genesis.to_json().unwrap()),wire(successor.to_json().unwrap())],"wraps":wraps})
    );
}
