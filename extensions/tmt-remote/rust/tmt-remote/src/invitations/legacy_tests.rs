//! Legacy codes keep their exact decoder and expiry; no profile fallback occurs.
use super::*;
use crate::cloud_pairing::{self, wire::Invite};
use ed25519_dalek::Signer;
use serde_json::json;

#[test]
fn firestore_v1_decoder_keeps_original_native_binding_and_remaining_lifetime() {
    let (context, _, _) = super::tests::fixture();
    let invite = Invite::new(
        "demo-pairing-test",
        &context.owner,
        &context.owner_key,
        json!({"projectId":"demo-pairing-test","appId":"1:123:web:test","apiKey":"public-test"}),
        "Legacy owner",
        1,
    )
    .unwrap();
    let token = invite.token().unwrap();
    let near_expiry = Invite::read(&token, invite.challenge.expires_at_ms - 1).unwrap();
    assert_eq!(near_expiry.challenge, invite.challenge);
    assert_eq!(near_expiry.challenge.owner, context.owner);
    assert_eq!(near_expiry.challenge.owner_key, context.owner_key);
    assert!(near_expiry.token().unwrap() == token);
    let expired = match Invite::read(&token, invite.challenge.expires_at_ms) {
        Ok(_) => panic!("old code received a fresh lifetime"),
        Err(error) => error,
    };
    assert_eq!(expired.code, "REMOTE_CLOUD_PAIR_EXPIRED");
    assert!(!expired.message.contains(&token));
    let raw = canonical::base64url_decode(&token).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    for version in [0, 2, 99] {
        let mut changed = value.clone();
        changed["version"] = json!(version);
        let text = canonical::base64url(&serde_json::to_vec(&changed).unwrap());
        let error = match Invite::read(&text, 2) {
            Ok(_) => panic!("unknown profile accepted"),
            Err(error) => error,
        };
        assert_eq!(error.code, "REMOTE_CLOUD_UPGRADE_REQUIRED");
    }
    // A new backend discriminator cannot be grafted into a v1 Firestore code.
    let mut changed = value;
    changed["backend"] = json!("append-log");
    assert!(
        Invite::read(
            &canonical::base64url(&serde_json::to_vec(&changed).unwrap()),
            2
        )
        .is_err()
    );
}

#[test]
fn legacy_and_backend_neutral_proofs_are_not_interchangeable_even_with_the_same_code_and_native_key()
 {
    let (mut context, candidate, key) = super::tests::fixture();
    let old_context = cloud_pairing::Challenge {
        project: "demo-pairing-test".into(),
        owner: context.owner.clone(),
        owner_key: context.owner_key,
        offer: context.offer.clone(),
        nonce: context.nonce,
        generation: 1,
        expires_at_ms: context.expires_at_ms,
    };
    let old_candidate = cloud_pairing::Candidate {
        peer: candidate.machine.clone(),
        public_key: candidate.public_key,
        uid: String::from_utf8(candidate.principal.clone()).unwrap(),
        name: candidate.name.clone(),
        nonce: candidate.nonce,
    };
    let code = [4; 16];
    let old_transcript = cloud_pairing::enrollment(&old_context, &old_candidate).unwrap();
    let old_mac = crypto::enrollment_mac(&code, &old_transcript);
    let old_signature = key
        .sign(&canonical::possession(&old_transcript, &old_mac).unwrap())
        .to_bytes();
    cloud_pairing::verify_enrollment(
        &old_context,
        &old_candidate,
        &old_candidate.uid,
        &code,
        &old_mac,
        &old_signature,
        2,
    )
    .unwrap();
    for backend in ["firestore", "mailbox", "append-log"] {
        context.binding.backend = backend.into();
        assert!(
            verify_enrollment(
                &context,
                &candidate,
                &candidate.principal,
                &code,
                &old_mac,
                &old_signature,
                2
            )
            .is_err()
        );
        let transcript = enrollment(&context, &candidate).unwrap();
        let mac = crypto::enrollment_mac(&code, &transcript);
        let signature = key
            .sign(&canonical::possession(&transcript, &mac).unwrap())
            .to_bytes();
        verify_enrollment(
            &context,
            &candidate,
            &candidate.principal,
            &code,
            &mac,
            &signature,
            2,
        )
        .unwrap();
        assert!(
            cloud_pairing::verify_enrollment(
                &old_context,
                &old_candidate,
                &old_candidate.uid,
                &code,
                &mac,
                &signature,
                2
            )
            .is_err()
        );
    }
}
