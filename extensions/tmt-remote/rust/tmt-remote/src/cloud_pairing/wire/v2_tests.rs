use super::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;

#[test]
fn private_instruction_carrier_preserves_the_exact_native_code_pin_and_deadline() {
    let vector: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/invitations/invite-v2.json"
    ))
    .unwrap();
    let token = vector["token"].as_str().unwrap();
    let invite = Invite::read(token, vector["now"].as_u64().unwrap()).unwrap();
    assert_eq!(invite.token().unwrap(), token);
    assert_eq!(serde_json::to_value(&invite).unwrap(), vector["invite"]);
    assert_eq!(invite.context.expires_at_ms, 1_700_000_600_000);
    assert!(Invite::read(token, invite.context.expires_at_ms).is_err());
}

fn fixture() -> (Invite, port::Submission) {
    let key = SigningKey::from_bytes(&[7; 32]);
    let context = Context {
        binding: invitations::BackendBinding {
            backend: "firestore".into(),
            reference: b"demo-invitation-v2".to_vec(),
            owner_principal: b"owner-uid".to_vec(),
        },
        owner: "11111111-1111-4111-8111-111111111111".into(),
        owner_key: SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes(),
        offer: "22222222-2222-4222-8222-222222222222".into(),
        nonce: [2; 16],
        issued_at_ms: 1,
        expires_at_ms: 600_001,
    };
    let candidate = invitations::Candidate {
        machine: "33333333-3333-4333-8333-333333333333".into(),
        public_key: key.verifying_key().to_bytes(),
        principal: b"joiner-uid".to_vec(),
        name: "Joiner".into(),
        nonce: [3; 16],
    };
    let invite = Invite {
        version: 2,
        context,
        public_config: json!({"projectId":"demo-invitation-v2","appId":"1:123:web:test","apiKey":"public-test"}),
        machine_name: "Owner".into(),
        code: [4; 16],
    };
    let transcript = invitations::enrollment(&invite.context, &candidate).unwrap();
    let mac = crypto::enrollment_mac(&invite.code, &transcript);
    let signature = key
        .sign(&canonical::possession(&transcript, &mac).unwrap())
        .to_bytes();
    let submitted = port::Submission {
        candidate,
        gate: invitations::service::gate(&invite.context, &invite.code).unwrap(),
        mac,
        signature,
    };
    (invite, submitted)
}

#[test]
fn new_profile_binds_project_owner_principal_and_original_remaining_lifetime() {
    let (invite, _) = fixture();
    let token = invite.token().unwrap();
    let admitted = Invite::read(&token, invite.context.expires_at_ms - 1).unwrap();
    assert!(admitted.context == invite.context);
    assert_eq!(admitted.token().unwrap(), token);
    assert_eq!(
        Invite::read(&token, invite.context.expires_at_ms)
            .err()
            .unwrap()
            .code,
        "REMOTE_CLOUD_PAIR_EXPIRED"
    );
    assert_eq!(
        super::super::Invite::read(&token, 2).err().unwrap().code,
        "REMOTE_CLOUD_UPGRADE_REQUIRED"
    );
    let value: Value =
        serde_json::from_slice(&canonical::base64url_decode(&token).unwrap()).unwrap();
    for changed in [
        {
            let mut v = value.clone();
            v["version"] = json!(1);
            v
        },
        {
            let mut v = value.clone();
            v["version"] = json!(99);
            v
        },
        {
            let mut v = value.clone();
            v["context"]["binding"]["backend"] = json!("mailbox");
            v
        },
        {
            let mut v = value.clone();
            v["publicConfig"]["projectId"] = json!("demo-other-project");
            v
        },
        {
            let mut v = value.clone();
            v["context"]["binding"]["owner_principal"] = json!(b"bad/uid".to_vec());
            v
        },
        {
            let mut v = value.clone();
            v["context"]["unknown"] = json!(true);
            v
        },
        {
            let mut v = value.clone();
            v["context"]["binding"]["unknown"] = json!(true);
            v
        },
    ] {
        let raw = canonical::base64url(&serde_json::to_vec(&changed).unwrap());
        assert!(Invite::read(&raw, 2).is_err());
    }
}

#[test]
fn enrollment_roundtrip_requires_native_possession_exact_context_and_rules_uid() {
    let (invite, submitted) = fixture();
    let bytes = submission(&invite.context, &submitted).unwrap();
    let observed = read_submission(&bytes, &invite.context, "joiner-uid", submitted.gate).unwrap();
    assert!(observed == submitted);
    invitations::verify_enrollment(
        &invite.context,
        &observed.candidate,
        b"joiner-uid",
        &invite.code,
        &observed.mac,
        &observed.signature,
        2,
    )
    .unwrap();
    assert!(read_submission(&bytes, &invite.context, "foreign-uid", submitted.gate).is_err());
    let mut other_owner = invite.context.clone();
    other_owner.binding.owner_principal = b"other-owner-uid".to_vec();
    assert!(read_submission(&bytes, &other_owner, "joiner-uid", submitted.gate).is_err());
    let original: Value = serde_json::from_slice(&bytes).unwrap();
    for changed in [
        {
            let mut v = original.clone();
            v["version"] = json!(1);
            v
        },
        {
            let mut v = original.clone();
            v["kind"] = json!("reroute");
            v
        },
        {
            let mut v = original.clone();
            v["signature"] = json!(canonical::base64url(&[0; 64]));
            v
        },
        {
            let mut v = original.clone();
            v["candidate"]["unknown"] = json!(true);
            v
        },
    ] {
        assert!(
            read_submission(
                &serde_json::to_vec(&changed).unwrap(),
                &invite.context,
                "joiner-uid",
                submitted.gate
            )
            .is_err()
        );
    }
    assert!(
        read_submission(
            &vec![b' '; limits::CLOUD_ENROLLMENT_BYTES + 1],
            &invite.context,
            "joiner-uid",
            submitted.gate
        )
        .is_err()
    );
}

#[test]
fn receipt_envelope_preserves_exact_bytes_and_refuses_cross_profile_or_extra_fields() {
    let (_, submitted) = fixture();
    let bytes = b"exact receipt\0\xe9\x9b\xaa";
    let snapshot = [5; 32];
    let raw = receipt(&snapshot, bytes, &submitted.mac).unwrap();
    assert!(
        read_receipt(&raw).unwrap()
            == port::Payload::Receipt {
                snapshot,
                bytes: bytes.to_vec(),
                proof: submitted.mac,
            }
    );
    let original: Value = serde_json::from_slice(&raw).unwrap();
    for changed in [
        {
            let mut v = original.clone();
            v["version"] = json!(1);
            v
        },
        {
            let mut v = original.clone();
            v["kind"] = json!("route-response");
            v
        },
        {
            let mut v = original.clone();
            v["unknown"] = json!(true);
            v
        },
        {
            let mut v = original.clone();
            v["snapshot"] = json!("bad");
            v
        },
    ] {
        assert!(read_receipt(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
}
