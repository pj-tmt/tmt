use super::*;
use crate::office_deployment::DeploymentMode;
use serde_json::{Value, json};

fn examples() -> Value {
    serde_json::from_str(include_str!("../../../../contracts/pairing-examples.json")).unwrap()
}

fn expected() -> Approval {
    Approval::decode(&serde_json::to_vec(&examples()["valid"][0]).unwrap()).unwrap()
}

fn claim() -> Value {
    let mut value = examples()["valid"][0].clone();
    value["principalUid"] = json!("office-agent:00000000-0000-4000-8000-000000000003");
    value["blockId"] = json!("00000000-0000-4000-8000-000000000004");
    value["expiresAt"] = json!(301_000);
    value["grantExpiresAt"] = json!(86_401_000);
    value["customToken"] = json!("private-custom-token");
    value
}

#[test]
fn renewal_readback_preserves_scope_and_rejects_rollback_or_ambiguous_json() {
    let mut value = claim();
    let fields = value.as_object_mut().unwrap();
    for key in [
        "installationLabel",
        "identityLabel",
        "expiresAt",
        "customToken",
    ] {
        fields.remove(key);
    }
    let principal = value["principalUid"].as_str().unwrap().to_owned();
    let block = value["blockId"].as_str().unwrap().to_owned();
    let decode = |bytes: &[u8]| Renewal::decode(bytes, &expected(), &principal, &block, 1000);
    assert_eq!(
        decode(&serde_json::to_vec(&value).unwrap()).unwrap(),
        86_401_000
    );
    for (key, invalid) in [
        ("version", json!(2)),
        ("pairingId", json!("0".repeat(64))),
        ("worldId", json!("abcdefghijklmnopqrst")),
        (
            "installationId",
            json!("00000000-0000-4000-8000-000000000009"),
        ),
        ("identityId", json!("00000000-0000-4000-8000-000000000009")),
        (
            "principalUid",
            json!("office-agent:00000000-0000-4000-8000-000000000009"),
        ),
        ("blockId", json!("00000000-0000-4000-8000-000000000009")),
        ("capabilities", json!(["layout.read", "notebook.write"])),
        ("grantExpiresAt", json!(999)),
        ("grantExpiresAt", json!(MAX_TIMESTAMP + 1)),
        ("customToken", json!("unexpected-token")),
    ] {
        let mut changed = value.clone();
        changed[key] = invalid;
        assert!(
            decode(&serde_json::to_vec(&changed).unwrap()).is_err(),
            "{key}"
        );
    }
    let text = serde_json::to_string(&value).unwrap();
    assert!(decode(text.replacen('{', "{\"version\":1,", 1).as_bytes()).is_err());
    assert!(decode(&vec![b' '; RESPONSE_LIMIT + 1]).is_err());
    // A stale request may read back another caller's already-expired lease;
    // the record persists it, but resource use still requires a live lease.
    value["grantExpiresAt"] = json!(1000);
    assert_eq!(decode(&serde_json::to_vec(&value).unwrap()).unwrap(), 1000);
}

#[test]
fn native_decoder_conforms_to_independent_browser_and_service_literals() {
    let corpus = examples();
    for raw in corpus["invalidJson"].as_array().unwrap() {
        assert!(Approval::decode(raw.as_str().unwrap().as_bytes()).is_err());
    }
    for value in corpus["valid"].as_array().unwrap() {
        let decoded = Approval::decode(&serde_json::to_vec(value).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), *value);
    }
    for value in corpus["invalid"].as_array().unwrap() {
        assert!(
            Approval::decode(&serde_json::to_vec(value).unwrap()).is_err(),
            "{value}"
        );
    }
    let text = serde_json::to_string(&corpus["valid"][0]).unwrap();
    assert!(Approval::decode(text.replacen('{', "{\"version\":1,", 1).as_bytes()).is_err());
    let mut padded = text.clone();
    padded.extend(std::iter::repeat_n(' ', APPROVAL_LIMIT - text.len()));
    assert!(Approval::decode(padded.as_bytes()).is_ok());
    padded.push(' ');
    assert!(Approval::decode(padded.as_bytes()).is_err());
}

#[test]
fn proof_uses_full_entropy_and_canonical_encoding_with_independent_digest() {
    // SHA-256 of 32 zero bytes, independent of the implementation's encoder.
    let proof = Proof::decode("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    assert_eq!(
        proof.challenge(),
        "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925"
    );
    assert_eq!(
        proof.secret(),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    );
    for input in [
        "",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA+",
    ] {
        assert!(Proof::decode(input).is_err());
    }
    let first = Proof::generate().unwrap();
    let second = Proof::generate().unwrap();
    assert_ne!(first.challenge(), second.challenge());
    assert_eq!(
        Proof::decode(&first.secret()).unwrap().challenge(),
        first.challenge()
    );
}

#[test]
fn public_link_contains_only_exact_consent_and_never_the_proof() {
    let target = WorldTarget::parse(
        "https://office.example/worlds/AbCdEfGhIjKlMnOpQrSt",
        DeploymentMode::Cloud,
    )
    .unwrap();
    let proof = Proof::generate().unwrap();
    let approval = Approval::new(
        &target,
        expected().installation_id(),
        expected().identity_id(),
        "Alice",
        false,
        &proof,
    )
    .unwrap();
    let link = approval.approval_url(&target).unwrap();
    assert!(link.starts_with("https://office.example/worlds/AbCdEfGhIjKlMnOpQrSt/pair#tmt-pair="));
    assert!(!link.contains(&proof.secret()));
    let bytes = URL_SAFE_NO_PAD
        .decode(link.split_once("#tmt-pair=").unwrap().1)
        .unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 8);
    assert_eq!(json["pairingId"], proof.challenge());
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains(&proof.secret())
    );
    assert_eq!(Approval::decode(&bytes).unwrap(), approval);
    let other = WorldTarget::parse(
        "https://office.example/worlds/abcdefghijklmnopqrst",
        DeploymentMode::Cloud,
    )
    .unwrap();
    assert_eq!(
        approval.approval_url(&other),
        Err(OfficeError::CredentialsInvalid)
    );
}

#[test]
fn claim_requires_complete_echo_and_keeps_grant_clock_separate() {
    let input = claim();
    let accepted = Claim::decode(&serde_json::to_vec(&input).unwrap(), &expected(), 1000).unwrap();
    assert_eq!(accepted.principal_uid, input["principalUid"]);
    assert_eq!(accepted.block_id, input["blockId"]);
    assert_eq!(accepted.grant_expires_at, 86_401_000);
    assert_eq!(accepted.custom_token, "private-custom-token");
    for (key, value) in [
        ("version", json!(2)),
        ("pairingId", json!("f".repeat(64))),
        ("worldId", json!("abcdefghijklmnopqrst")),
        (
            "installationId",
            json!("00000000-0000-4000-8000-000000000009"),
        ),
        ("identityId", json!("00000000-0000-4000-8000-000000000009")),
        ("installationLabel", json!("Different installation")),
        ("identityLabel", json!("Other")),
        ("capabilities", json!(["layout.read"])),
        ("principalUid", json!("human-owner")),
        ("blockId", json!("home")),
        ("expiresAt", json!(1000)),
        ("grantExpiresAt", json!(1000)),
        ("grantExpiresAt", json!(MAX_TIMESTAMP + 1)),
        ("customToken", json!("private\nmaterial")),
        ("customToken", json!("")),
        ("unknown", json!("private-material")),
    ] {
        let mut changed = input.clone();
        changed[key] = value;
        let error = Claim::decode(&serde_json::to_vec(&changed).unwrap(), &expected(), 1000)
            .err()
            .unwrap();
        assert_eq!(error, OfficeError::CredentialsInvalid, "{key}");
        assert!(!error.to_string().contains("private"));
    }
    for key in input.as_object().unwrap().keys() {
        let mut missing = input.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            Claim::decode(&serde_json::to_vec(&missing).unwrap(), &expected(), 1000).is_err(),
            "{key}"
        );
    }
    let duplicate = input
        .to_string()
        .replacen('{', "{\"customToken\":\"other\",", 1);
    assert!(Claim::decode(duplicate.as_bytes(), &expected(), 1000).is_err());
    assert!(Claim::decode(&vec![b' '; RESPONSE_LIMIT + 1], &expected(), 1000).is_err());
}
