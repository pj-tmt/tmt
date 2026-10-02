use ed25519_dalek::{Signer, SigningKey};
use serde_json::Value;
use tmt_remote::{canonical, crypto};

fn bytes(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0, "whole bytes");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
// RFC 8032 section 7.1 TEST 1 (empty message); independent published known-answer vector.
const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const SIGNATURE: &str = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";
#[test]
fn rfc8032_and_strict_single_condition_refusals() {
    let key = bytes(KEY);
    let signature = bytes(SIGNATURE);
    assert!(crypto::verify_signature(&key, b"", &signature).is_ok());
    let signer = SigningKey::from_bytes(&bytes(SEED).try_into().unwrap());
    assert_eq!(signer.sign(b"").to_bytes().as_slice(), signature);
    assert!(crypto::verify_signature(&key, b"changed", &signature).is_err());
    for length in [0, 31, 33] {
        assert!(crypto::public_key(&vec![0; length]).is_err());
    }
    for length in [0, 63, 65] {
        assert!(crypto::verify_signature(&key, b"", &vec![0; length]).is_err());
    }
    assert!(crypto::public_key(&[0; 32]).is_err()); // Small order.
    let mut identity = [0; 32];
    identity[0] = 1;
    assert!(crypto::public_key(&identity).is_err());
    let mut alternate_identity = [255; 32];
    alternate_identity[0] = 238;
    alternate_identity[31] = 127;
    assert!(crypto::public_key(&alternate_identity).is_err()); // y=p+1, noncanonical.
    let other = SigningKey::from_bytes(&[42; 32]);
    assert!(crypto::verify_signature(other.verifying_key().as_bytes(), b"", &signature).is_err());
    let mut changed = signature.clone();
    changed[0] ^= 1;
    assert!(crypto::verify_signature(&key, b"", &changed).is_err());
    let mut weak_r = signature.clone();
    weak_r[..32].fill(0);
    weak_r[0] = 1;
    assert!(crypto::verify_signature(&key, b"", &weak_r).is_err());
    let mut noncanonical_r = signature.clone();
    noncanonical_r[..32].copy_from_slice(&alternate_identity);
    assert!(crypto::verify_signature(&key, b"", &noncanonical_r).is_err());
    // S + group order represents the same scalar modulo L, but strict verification rejects it.
    let order = bytes("edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010");
    let mut high_s = signature.clone();
    let mut carry = 0u16;
    for i in 0..32 {
        let sum = u16::from(high_s[32 + i]) + u16::from(order[i]) + carry;
        high_s[32 + i] = sum as u8;
        carry = sum >> 8;
    }
    assert!(crypto::verify_signature(&key, b"", &high_s).is_err());
}
#[test]
fn noncanonical_encoding_of_a_strong_point_is_rejected() {
    // Encodings y=p+n reduce to y=n under ZIP215. Unlike identity examples,
    // these positive controls are not small-order points.
    let mut controls = 0;
    for n in 2..19u8 {
        let mut canonical = [0; 32];
        canonical[0] = n;
        let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(&canonical) else {
            continue;
        };
        if key.is_weak() {
            continue;
        }
        assert!(crypto::public_key(&canonical).is_ok());
        let mut alternate = [255; 32];
        alternate[0] = 237 + n;
        alternate[31] = 127;
        assert!(crypto::public_key(&alternate).is_err());
        controls += 1;
    }
    assert!(controls > 0);
}

#[test]
fn rfc4231_full_hmac_and_wrong_or_truncated_tags() {
    // RFC 4231 section 4, cases 1 and 2; no truncated tags are accepted by local-v1.
    for (key, message, expected) in [
        (
            vec![11; 20],
            b"Hi There".as_slice(),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        ),
        (
            b"Jefe".to_vec(),
            b"what do ya want for nothing?".as_slice(),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        ),
    ] {
        let tag = bytes(expected);
        assert_eq!(crypto::mac(&key, message).as_slice(), tag);
        assert!(crypto::verify_mac(&key, message, &tag).is_ok());
        for length in [0, 16, 31] {
            assert!(crypto::verify_mac(&key, message, &tag[..length]).is_err());
        }
        let mut long = tag.clone();
        long.push(0);
        assert!(crypto::verify_mac(&key, message, &long).is_err());
        let mut wrong = tag;
        wrong[0] ^= 1;
        assert!(crypto::verify_mac(&key, message, &wrong).is_err());
        assert!(crypto::verify_mac(b"wrong", message, &crypto::mac(&key, message)).is_err());
    }
}
#[test]
fn independent_python_server_proof_derivation_and_domain_separation() {
    let v: Value = serde_json::from_str(include_str!("fixtures/mac-vectors.json")).unwrap();
    let field = |name: &str| bytes(v[name].as_str().unwrap());
    let code = field("code").try_into().unwrap();
    let enrollment = field("enrollment");
    let receipt = field("receipt");
    let mac = crypto::enrollment_mac(&code, &enrollment);
    assert_eq!(mac.as_slice(), field("enrollmentMac"));
    let key = crypto::response_key(&code, &enrollment).unwrap();
    assert_eq!(key.as_slice(), field("responseKey"));
    let proof = crypto::server_proof(&key, &receipt).unwrap();
    assert_eq!(proof.as_slice(), field("serverProof"));
    assert!(crypto::verify_server_proof(&key, &receipt, &proof).is_ok());
    assert!(crypto::verify_server_proof(&key, b"{}", &proof).is_err());
    assert!(crypto::verify_server_proof(&key, &receipt, &proof[..31]).is_err());
    assert_ne!(key, crypto::response_key(&[99; 16], &enrollment).unwrap());
    let mut changed = enrollment.clone();
    changed[0] ^= 1;
    assert_ne!(key, crypto::response_key(&code, &changed).unwrap());
    assert_ne!(proof, crypto::mac(&key, &receipt));
    assert_ne!(key, mac);
    let possession = canonical::possession(&enrollment, &mac).unwrap();
    let signature = SigningKey::from_bytes(&bytes(SEED).try_into().unwrap()).sign(&possession);
    assert!(crypto::verify_signature(&bytes(KEY), &possession, &signature.to_bytes()).is_ok());
    assert!(crypto::verify_signature(&bytes(KEY), &enrollment, &signature.to_bytes()).is_err());
}
#[test]
fn webcrypto_fixture_signatures_match_native_signing_and_verification() {
    let v: Value = serde_json::from_str(include_str!("fixtures/webcrypto-vectors.json")).unwrap();
    let signer = SigningKey::from_bytes(&bytes(SEED).try_into().unwrap());
    for case in v["cases"].as_array().unwrap() {
        let message = bytes(case["message"].as_str().unwrap());
        let signature = bytes(case["signature"].as_str().unwrap());
        assert!(crypto::verify_signature(&bytes(KEY), &message, &signature).is_ok());
        assert_eq!(signer.sign(&message).to_bytes().as_slice(), signature);
        // Each signed field is bound; changing any byte invalidates the signature.
        for i in 0..message.len() {
            let mut changed = message.clone();
            changed[i] ^= 1;
            assert!(crypto::verify_signature(&bytes(KEY), &changed, &signature).is_err());
        }
    }
}
