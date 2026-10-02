//! Fixed suite; key storage and authority remain outside the model.
use crate::{Invalid, Result, framing::frame, require};
use ed25519_dalek::{Signature, VerifyingKey};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

pub fn digest(input: &[u8]) -> [u8; 32] {
    Sha256::digest(input).into()
}
pub fn public_key(raw: &[u8]) -> Result<VerifyingKey> {
    let bytes = raw.try_into().map_err(|_| Invalid)?;
    let key = VerifyingKey::from_bytes(bytes).map_err(|_| Invalid)?;
    require(!key.is_weak() && key.to_edwards().compress().as_bytes() == bytes)?;
    Ok(key)
}
pub fn verify_signature(key: &[u8], input: &[u8], signature: &[u8]) -> Result<()> {
    let key = public_key(key)?;
    let signature = Signature::from_slice(signature).map_err(|_| Invalid)?;
    key.verify_strict(input, &signature).map_err(|_| Invalid)
}
pub fn mac(key: &[u8], input: &[u8]) -> [u8; 32] {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(input);
    m.finalize().into_bytes().into()
}
pub fn verify_mac(key: &[u8], input: &[u8], tag: &[u8]) -> Result<()> {
    require(tag.len() == 32)?;
    let mut m = Hmac::<Sha256>::new_from_slice(key).map_err(|_| Invalid)?;
    m.update(input);
    m.verify_slice(tag).map_err(|_| Invalid)
}
/// RFC5869 extract and one-block expand for the contract's 32-byte outputs.
pub fn derive_key(secret: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let prk = mac(salt, secret);
    let mut block = info.to_vec();
    block.push(1);
    mac(&prk, &block)
}
pub fn space_id(owner: &[u8; 32]) -> Result<String> {
    public_key(owner)?;
    let hash = digest(&frame(&[b"tmt-colab-space-id-v1", owner])?);
    let alphabet = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity(32);
    for i in 0..32 {
        let bit = i * 5;
        let byte = bit / 8;
        let shift = bit % 8;
        let word = ((hash[byte] as u16) << 8) | hash[byte + 1] as u16;
        out.push(alphabet[((word >> (11 - shift)) & 31) as usize] as char);
    }
    Ok(out)
}
