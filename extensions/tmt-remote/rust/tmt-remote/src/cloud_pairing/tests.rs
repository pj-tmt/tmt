use super::*;
use ed25519_dalek::{Signer, SigningKey};

pub(super) fn fixture() -> (Challenge, Candidate, SigningKey) {
    let key = SigningKey::from_bytes(&[7; 32]);
    (
        Challenge {
            project: "demo-pairing-test".into(),
            owner: "11111111-1111-4111-8111-111111111111".into(),
            owner_key: SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes(),
            offer: "22222222-2222-4222-8222-222222222222".into(),
            nonce: [2; 16],
            generation: 1,
            expires_at_ms: 600_000,
        },
        Candidate {
            peer: "33333333-3333-4333-8333-333333333333".into(),
            public_key: key.verifying_key().to_bytes(),
            uid: "sender-uid".into(),
            name: "Peer machine".into(),
            nonce: [3; 16],
        },
        key,
    )
}

pub(super) fn verified() -> VerifiedCandidate {
    let (challenge, candidate, key) = fixture();
    let bytes = enrollment(&challenge, &candidate).unwrap();
    let mac = crypto::enrollment_mac(&[4; 16], &bytes);
    let signature = key
        .sign(&canonical::possession(&bytes, &mac).unwrap())
        .to_bytes();
    verify_enrollment(
        &challenge,
        &candidate,
        &candidate.uid,
        &[4; 16],
        &mac,
        &signature,
        1,
    )
    .unwrap()
}

#[test]
fn code_and_native_possession_are_both_required_before_comparing_words() {
    let (challenge, candidate, key) = fixture();
    let bytes = enrollment(&challenge, &candidate).unwrap();
    let mac = crypto::enrollment_mac(&[4; 16], &bytes);
    let signature = key
        .sign(&canonical::possession(&bytes, &mac).unwrap())
        .to_bytes();
    let verify = |code, mac, signature, now| {
        verify_enrollment(
            &challenge,
            &candidate,
            &candidate.uid,
            code,
            mac,
            signature,
            now,
        )
    };
    let admitted = verify(&[4; 16], &mac, &signature, 1).unwrap();
    assert_eq!(
        admitted.words().unwrap(),
        canonical::fingerprint_words(&candidate.public_key).unwrap()
    );
    assert!(verify(&[5; 16], &mac, &signature, 1).is_err());
    assert!(verify(&[4; 16], &[0; 32], &signature, 1).is_err());
    assert!(verify(&[4; 16], &mac, &[0; 64], 1).is_err());
    assert!(verify(&[4; 16], &mac, &signature, challenge.expires_at_ms).is_err());
    assert!(
        verify_enrollment(
            &challenge,
            &candidate,
            "another-uid",
            &[4; 16],
            &mac,
            &signature,
            1
        )
        .is_err()
    );
}

#[test]
fn proof_binds_every_deployment_identity_challenge_and_candidate_field() {
    let (challenge, candidate, key) = fixture();
    let bytes = enrollment(&challenge, &candidate).unwrap();
    let mac = crypto::enrollment_mac(&[4; 16], &bytes);
    let signature = key
        .sign(&canonical::possession(&bytes, &mac).unwrap())
        .to_bytes();
    let mut challenges = vec![challenge.clone(); 7];
    challenges[0].project = "demo-other-test".into();
    challenges[1].owner = "44444444-4444-4444-8444-444444444444".into();
    challenges[2].owner_key = SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes();
    challenges[3].offer = "55555555-5555-4555-8555-555555555555".into();
    challenges[4].nonce[0] ^= 1;
    challenges[5].generation += 1;
    challenges[6].expires_at_ms += 1;
    for changed in challenges {
        assert!(
            verify_enrollment(
                &changed,
                &candidate,
                &candidate.uid,
                &[4; 16],
                &mac,
                &signature,
                1
            )
            .is_err()
        );
    }
    let mut candidates = vec![candidate.clone(); 5];
    candidates[0].peer = "66666666-6666-4666-8666-666666666666".into();
    candidates[1].public_key = SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes();
    candidates[2].uid = "changed-uid".into();
    candidates[3].name = "Other name".into();
    candidates[4].nonce[0] ^= 1;
    for changed in candidates {
        assert!(
            verify_enrollment(
                &challenge,
                &changed,
                &changed.uid,
                &[4; 16],
                &mac,
                &signature,
                1
            )
            .is_err()
        );
    }
}

pub(super) fn reroute_fixture(generation: u64, uid: &str) -> (Reroute, SigningKey) {
    let (challenge, candidate, key) = fixture();
    (
        Reroute {
            machine_id: challenge.owner,
            owner_key: challenge.owner_key,
            peer: candidate.peer,
            uid: uid.into(),
            generation,
            timestamp_ms: 1_000,
        },
        key,
    )
}

#[test]
fn reroute_binds_uid_owner_identity_counter_and_provider_creation_time() {
    let (request, key) = reroute_fixture(2, "new-uid");
    let public_key = key.verifying_key().to_bytes();
    let signature = key.sign(&reroute(&request).unwrap()).to_bytes();
    assert!(verify_reroute(&request, &request.uid, &public_key, &signature, 1_000).is_ok());
    assert!(verify_reroute(&request, &request.uid, &public_key, &signature, 61_000).is_ok());
    assert!(verify_reroute(&request, &request.uid, &public_key, &signature, 61_001).is_err());
    assert!(verify_reroute(&request, "foreign-uid", &public_key, &signature, 1_000).is_err());
    let other = SigningKey::from_bytes(&[8; 32]);
    assert!(
        verify_reroute(
            &request,
            &request.uid,
            &other.verifying_key().to_bytes(),
            &signature,
            1_000
        )
        .is_err()
    );
    let mut changes = vec![request.clone(); 6];
    changes[0].machine_id = "44444444-4444-4444-8444-444444444444".into();
    changes[1].owner_key = other.verifying_key().to_bytes();
    changes[2].peer = "55555555-5555-4555-8555-555555555555".into();
    changes[3].uid = "another-uid".into();
    changes[4].generation += 1;
    changes[5].timestamp_ms += 1;
    for changed in changes {
        assert!(verify_reroute(&changed, &changed.uid, &public_key, &signature, 1_000).is_err());
    }
    let (challenge, candidate, _) = fixture();
    let join = enrollment(&challenge, &candidate).unwrap();
    let mac = crypto::enrollment_mac(&[4; 16], &join);
    let signature = key
        .sign(&canonical::possession(&join, &mac).unwrap())
        .to_bytes();
    assert!(verify_reroute(&request, &request.uid, &public_key, &signature, 1_000).is_err());
}

#[test]
fn exact_receipt_proof_cannot_move_between_offers_or_change_receipt_bytes() {
    let (challenge, candidate, _) = fixture();
    let key = response_key(&[4; 16], &challenge, &candidate).unwrap();
    let receipt = b"exact owner receipt";
    let proof = crypto::server_proof(&key, receipt).unwrap();
    crypto::verify_server_proof(&key, receipt, &proof).unwrap();
    assert!(crypto::verify_server_proof(&key, b"changed receipt", &proof).is_err());
    let mut changed = challenge;
    changed.nonce[0] ^= 1;
    let key = response_key(&[4; 16], &changed, &candidate).unwrap();
    assert!(crypto::verify_server_proof(&key, receipt, &proof).is_err());
}

#[test]
fn malformed_uid_key_identity_and_numeric_bounds_refuse_before_proof() {
    let (challenge, candidate, _) = fixture();
    for uid in ["", ".", "..", "a/b", "line\nbreak", &"x".repeat(129)] {
        let mut changed = candidate.clone();
        changed.uid = uid.into();
        assert!(enrollment(&challenge, &changed).is_err());
    }
    let mut changed = candidate.clone();
    changed.public_key = [0; 32];
    assert!(enrollment(&challenge, &changed).is_err());
    changed = candidate.clone();
    changed.peer = challenge.owner.clone();
    assert!(enrollment(&challenge, &changed).is_err());
    let mut changed = challenge.clone();
    changed.generation = 1 << 53;
    assert!(enrollment(&changed, &candidate).is_err());
    changed.generation = 0;
    assert!(enrollment(&changed, &candidate).is_err());
}
