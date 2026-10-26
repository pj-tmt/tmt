use super::*;
use ed25519_dalek::{Signer, SigningKey};

pub(super) fn fixture() -> (Context, Candidate, SigningKey) {
    let key = SigningKey::from_bytes(&[7; 32]);
    (
        Context {
            binding: BackendBinding {
                backend: "mailbox".into(),
                reference: b"opaque deployment mailbox".to_vec(),
                owner_principal: b"owner-principal".to_vec(),
            },
            owner: "11111111-1111-4111-8111-111111111111".into(),
            owner_key: SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes(),
            offer: "22222222-2222-4222-8222-222222222222".into(),
            nonce: [2; 16],
            issued_at_ms: 1,
            expires_at_ms: 600_001,
        },
        Candidate {
            machine: "33333333-3333-4333-8333-333333333333".into(),
            public_key: key.verifying_key().to_bytes(),
            principal: b"candidate-principal".to_vec(),
            name: "Candidate machine".into(),
            nonce: [3; 16],
        },
        key,
    )
}

#[test]
fn proof_binds_backend_locator_both_principals_and_original_expiry() {
    let (context, candidate, key) = fixture();
    let transcript = enrollment(&context, &candidate).unwrap();
    let code = [4; 16];
    let mac = crypto::enrollment_mac(&code, &transcript);
    let signature = key
        .sign(&canonical::possession(&transcript, &mac).unwrap())
        .to_bytes();
    let verify = |context: &Context, principal: &[u8], mac, signature, now| {
        verify_enrollment(context, &candidate, principal, &code, mac, signature, now)
    };
    let admitted = verify(&context, &candidate.principal, &mac, &signature, 2).unwrap();
    assert_eq!(
        admitted.words().unwrap(),
        canonical::fingerprint_words(&candidate.public_key).unwrap()
    );
    assert!(verify(&context, b"another-principal", &mac, &signature, 2).is_err());
    assert!(verify(&context, &candidate.principal, &[0; 32], &signature, 2).is_err());
    assert!(verify(&context, &candidate.principal, &mac, &[0; 64], 2).is_err());
    assert!(
        verify(
            &context,
            &candidate.principal,
            &mac,
            &signature,
            context.expires_at_ms
        )
        .is_err()
    );
    for changed in [
        Context {
            binding: BackendBinding {
                backend: "append-log".into(),
                ..context.binding.clone()
            },
            ..context.clone()
        },
        Context {
            binding: BackendBinding {
                reference: b"other reference".to_vec(),
                ..context.binding.clone()
            },
            ..context.clone()
        },
        Context {
            binding: BackendBinding {
                owner_principal: b"other owner".to_vec(),
                ..context.binding.clone()
            },
            ..context.clone()
        },
        Context {
            expires_at_ms: context.expires_at_ms - 1,
            ..context.clone()
        },
    ] {
        assert!(verify(&changed, &candidate.principal, &mac, &signature, 2).is_err());
    }
}

#[test]
fn framing_and_receipt_primitives_match_an_independent_node_vector() {
    // Generated independently with Node26 crypto and u32-BE framing, using fixed
    // synthetic seeds 7/8 and code 4. No Rust framing/helper output was consumed.
    let (context, candidate, key) = fixture();
    let transcript = enrollment(&context, &candidate).unwrap();
    let mac = crypto::enrollment_mac(&[4; 16], &transcript);
    let possession = canonical::possession(&transcript, &mac).unwrap();
    let signature = key.sign(&possession).to_bytes();
    let response_key = crypto::response_key(&[4; 16], &transcript).unwrap();
    assert_eq!(
        crate::state::hex(&transcript),
        "00000011746d742d696e7669746174696f6e2d76320000000a656e726f6c6c6d656e74000000076d61696c626f78000000196f7061717565206465706c6f796d656e74206d61696c626f780000000f6f776e65722d7072696e636970616c0000002431313131313131312d313131312d343131312d383131312d313131313131313131313131000000201398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca0000002432323232323232322d323232322d343232322d383232322d32323232323232323232323200000010020202020202020202020202020202020000000131000000063630303030310000002433333333333333332d333333332d343333332d383333332d33333333333333333333333300000020ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c0000001363616e6469646174652d7072696e636970616c0000001143616e646964617465206d616368696e650000001003030303030303030303030303030303"
    );
    assert_eq!(
        crate::state::hex(&mac),
        "9f24091adf4aea675f1aacbcbd9c6a0fb979a48c910e723abc52e5da81312eff"
    );
    assert_eq!(
        crate::state::hex(&signature),
        "0ecf6b6b416d0d4fd4909d791e9fa41e4aa2c44ce9f8e5691b3e0840cc2b6d00ca28f646383a902e7fb8cd3b71908ee663e8c23aaaab9742e20ecd2a4925b901"
    );
    assert_eq!(
        crate::state::hex(&response_key),
        "3aecb43bae97f4ec5ea1b577a989b818e5993cabef5c43ab0bc2dec964197ddf"
    );
    let receipt = br#"{"member":"44444444-4444-4444-8444-444444444444","sending":false}"#;
    assert_eq!(
        crate::state::hex(&crypto::server_proof(&response_key, receipt).unwrap()),
        "4f6d3f5de2e97f52b60be508d6207bbc05f245571a1f85a4c28766d3c05411d5"
    );
    let verified = verify_enrollment(
        &context,
        &candidate,
        &candidate.principal,
        &[4; 16],
        &mac,
        &signature,
        2,
    )
    .unwrap();
    assert_eq!(
        crate::state::hex(verified.digest()),
        "20f9bffcc2886b73e398607d21b89653a0620be3a8d722f0806e80586733718a"
    );
}

#[test]
fn native_possession_does_not_replace_the_original_invitation_code() {
    let (context, candidate, key) = fixture();
    let transcript = enrollment(&context, &candidate).unwrap();
    let mac = crypto::enrollment_mac(&[4; 16], &transcript);
    let signature = key
        .sign(&canonical::possession(&transcript, &mac).unwrap())
        .to_bytes();
    assert!(
        verify_enrollment(
            &context,
            &candidate,
            &candidate.principal,
            &[5; 16],
            &mac,
            &signature,
            2,
        )
        .is_err(),
        "native possession cannot substitute for the original invitation MAC key"
    );
}

#[test]
fn scoped_firestore_tokens_keep_existing_direction_framing_without_key_export() {
    let (mut context, candidate, _) = fixture();
    context.binding.backend = "firestore".into();
    context.binding.reference = b"demo-invitation-v2".to_vec();
    context.binding.owner_principal = b"owner-uid".to_vec();
    let transcript = enrollment(&context, &candidate).unwrap();
    let response = crypto::response_key(&[4; 16], &transcript).unwrap();
    let scoped = port::ReceiptKey(response);
    let (forward, reverse) =
        crate::cloud_pairing::wire::v2::reroute_tokens(&context, &candidate, &scoped).unwrap();
    let project = std::str::from_utf8(&context.binding.reference).unwrap();
    // Independent LP inputs also pin that only the direction part/order differ.
    let forward_input = canonical::framed(&[
        b"tmt-cloud-reroute-token-v1",
        project.as_bytes(),
        context.owner.as_bytes(),
        candidate.machine.as_bytes(),
    ])
    .unwrap();
    let reverse_input = canonical::framed(&[
        b"tmt-cloud-reroute-token-v1",
        b"reverse",
        project.as_bytes(),
        candidate.machine.as_bytes(),
        context.owner.as_bytes(),
    ])
    .unwrap();
    assert_eq!(forward, crypto::mac(&response, &forward_input));
    assert_eq!(reverse, crypto::mac(&response, &reverse_input));
    assert_ne!(forward, reverse);
    assert_ne!(forward, response);
}
