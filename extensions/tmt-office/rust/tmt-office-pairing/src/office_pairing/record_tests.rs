use super::*;
use serde_json::json;

const INSTALLATION: &str = "00000000-0000-4000-8000-000000000001";
const IDENTITY: &str = "00000000-0000-4000-8000-000000000002";

fn fixture() -> (WorldTarget, PairingRecord) {
    let target = WorldTarget::parse(
        "https://office.example/worlds/abcdefghijklmnopqrst",
        DeploymentMode::Cloud,
    )
    .unwrap();
    let deployment = OfficeDeployment::decode(target.clone(), br#"{"version":1,"mode":"cloud","projectId":"example-office","apiKey":"public-key","pairingUrl":"https://issuer.example"}"#).unwrap();
    let proof = Proof::generate().unwrap();
    let approval = Approval::new(&target, INSTALLATION, IDENTITY, "Alice", false, &proof).unwrap();
    let record = PairingRecord::pending(&deployment, approval, proof, 1000).unwrap();
    (target, record)
}

#[test]
fn revoked_receipt_has_no_secret_and_can_resume_without_remote_authority() {
    let (target, mut record) = fixture();
    assert!(record.is_pending());
    assert!(!record.is_revoked());
    let secret = record.reserve_claim(1000).unwrap().secret();
    // Remote confirmation is exercised by the emulator test; this assertion
    // isolates the durable terminal representation and its retry behavior.
    record.phase = Phase::Revoked {};
    let bytes = record.encode().unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(&secret));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["phase"],
        json!({"state":"revoked"})
    );
    let mut resumed = PairingRecord::decode(&bytes, &target, INSTALLATION, IDENTITY).unwrap();
    assert_eq!(resumed.local_state(u64::MAX), "revoked");
    assert!(resumed.is_revoked());
    assert!(!resumed.is_pending());
    assert!(!resumed.has_credentials());
    // An elapsed deadline proves a terminal retry needs no network exchange.
    resumed.revoke(&target, std::time::Instant::now()).unwrap();
    assert_eq!(resumed.encode().unwrap(), bytes);
    assert_eq!(PairingRecord::hook_target(&bytes).unwrap(), target);
    let mut invalid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    invalid["phase"]["secret"] = json!(secret);
    assert!(
        PairingRecord::decode(
            &serde_json::to_vec(&invalid).unwrap(),
            &target,
            INSTALLATION,
            IDENTITY
        )
        .is_err()
    );
}

#[test]
fn protected_record_round_trip_preserves_proof_deadline_and_full_deployment() {
    let (target, mut original) = fixture();
    let proof = original.reserve_claim(1000).unwrap();
    let bytes = original.encode().unwrap();
    let mut resumed = PairingRecord::decode(&bytes, &target, INSTALLATION, IDENTITY).unwrap();
    assert_eq!(resumed.expires_at, 301_000);
    assert_eq!(
        resumed.deployment(&target).unwrap(),
        original.deployment(&target).unwrap()
    );
    assert_eq!(resumed.approval(), original.approval());
    assert!(matches!(
        resumed.reserve_claim(5999),
        Err(OfficeError::PairingPending)
    ));
    assert_eq!(
        resumed.reserve_claim(6000).unwrap().secret(),
        proof.secret()
    );
    assert_eq!(resumed.expires_at, 301_000);
    assert_eq!(resumed.local_state(300_999), "pending");
    assert_eq!(resumed.local_state(301_000), "expired");
    assert!(matches!(
        resumed.reserve_claim(301_000),
        Err(OfficeError::PairingExpired)
    ));
}

#[test]
fn wrong_scope_replacement_and_corrupt_records_fail_closed() {
    let (target, record) = fixture();
    let bytes = record.encode().unwrap();
    let other_origin = WorldTarget::parse(
        "https://other.example/worlds/abcdefghijklmnopqrst",
        DeploymentMode::Cloud,
    )
    .unwrap();
    let other_world = WorldTarget::parse(
        "https://office.example/worlds/AbCdEfGhIjKlMnOpQrSt",
        DeploymentMode::Cloud,
    )
    .unwrap();
    for (selected, installation, identity) in [
        (&other_origin, INSTALLATION, IDENTITY),
        (&other_world, INSTALLATION, IDENTITY),
        (&target, IDENTITY, IDENTITY),
        (&target, INSTALLATION, INSTALLATION),
    ] {
        assert!(PairingRecord::decode(&bytes, selected, installation, identity).is_err());
    }
    for (key, value) in [
        ("version", json!(2)),
        ("mode", json!("emulator")),
        ("descriptor", json!("{}")),
        ("expiresAt", json!(u64::MAX)),
        (
            "phase",
            json!({"state":"pending","secret":"invalid","nextClaimAt":1000}),
        ),
        ("customToken", json!("must-not-persist")),
    ] {
        let mut changed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        changed[key] = value;
        assert!(
            PairingRecord::decode(
                &serde_json::to_vec(&changed).unwrap(),
                &target,
                INSTALLATION,
                IDENTITY
            )
            .is_err(),
            "{key}"
        );
    }
    let duplicate = String::from_utf8(bytes)
        .unwrap()
        .replacen('{', "{\"version\":1,", 1);
    assert!(PairingRecord::decode(duplicate.as_bytes(), &target, INSTALLATION, IDENTITY).is_err());
    assert!(
        PairingRecord::decode(
            &vec![b' '; RECORD_LIMIT + 1],
            &target,
            INSTALLATION,
            IDENTITY
        )
        .is_err()
    );
}
