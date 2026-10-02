//! Fixed local-v1 primitives, with no key storage, random generation or authority effects.
use crate::canonical::{InvalidBytes, Result, framed, require};
use ed25519_dalek::{Signature, VerifyingKey};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// Reject weak and noncanonical encodings before accepting a verification key.
pub fn public_key(raw: &[u8]) -> Result<VerifyingKey> {
    let bytes = raw.try_into().map_err(|_| InvalidBytes)?;
    let key = VerifyingKey::from_bytes(bytes).map_err(|_| InvalidBytes)?;
    require(!key.is_weak() && key.to_edwards().compress().as_bytes() == bytes)?;
    Ok(key)
}
/// Ordinary Ed25519 over exact canonical bytes, never Ed25519ph or a second prehash.
pub fn verify_signature(key: &[u8], message: &[u8], signature: &[u8]) -> Result<()> {
    let key = public_key(key)?;
    let signature = Signature::from_slice(signature).map_err(|_| InvalidBytes)?;
    key.verify_strict(message, &signature)
        .map_err(|_| InvalidBytes)
}
pub fn mac(key: &[u8], message: &[u8]) -> [u8; 32] {
    // HMAC accepts every key length; local-v1 callers use fixed-size code/derived-key APIs.
    let mut state = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    state.update(message);
    state.finalize().into_bytes().into()
}
pub fn verify_mac(key: &[u8], message: &[u8], tag: &[u8]) -> Result<()> {
    require(tag.len() == 32)?;
    let mut state = Hmac::<Sha256>::new_from_slice(key).map_err(|_| InvalidBytes)?;
    state.update(message);
    state.verify_slice(tag).map_err(|_| InvalidBytes)
}
pub fn enrollment_mac(code: &[u8; 16], enrollment: &[u8]) -> [u8; 32] {
    mac(code, enrollment)
}
/// `K_response`; derivation only, never a reusable enrollment token.
pub fn response_key(code: &[u8; 16], enrollment: &[u8]) -> Result<[u8; 32]> {
    Ok(mac(
        code,
        &framed(&[b"tmt-device-pair-response-key-v1", enrollment])?,
    ))
}
/// `serverProof` over the exact receipt JSON bytes.
pub fn server_proof(key: &[u8; 32], exact_receipt: &[u8]) -> Result<[u8; 32]> {
    Ok(mac(
        key,
        &framed(&[b"tmt-device-pair-response-v1", exact_receipt])?,
    ))
}
pub fn verify_server_proof(key: &[u8; 32], exact_receipt: &[u8], proof: &[u8]) -> Result<()> {
    verify_mac(
        key,
        &framed(&[b"tmt-device-pair-response-v1", exact_receipt])?,
        proof,
    )
}
