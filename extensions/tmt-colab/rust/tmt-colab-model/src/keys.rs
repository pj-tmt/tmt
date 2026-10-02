//! Deterministic public derivation only. Seed generation/storage belongs to the keyring.
//! No Ed25519-to-X25519 conversion or RNG.

pub fn ed25519_public(seed: &[u8; 32]) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(seed)
        .verifying_key()
        .to_bytes()
}
/// RFC7748 scalar clamping is performed by the pinned X25519 primitive.
pub fn x25519_public(seed: &[u8; 32]) -> [u8; 32] {
    x25519_dalek::x25519(*seed, x25519_dalek::X25519_BASEPOINT_BYTES)
}
