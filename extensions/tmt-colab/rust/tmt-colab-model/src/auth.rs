//! Sign-in/management byte builders. Server-owned expiry, replay and owner fences are separate.
use crate::{
    Invalid, Result, crypto,
    framing::{fields, frame, text},
    require, values,
};

pub struct SignIn<'a> {
    pub code_id: &'a str,
    pub space: &'a str,
    pub device: &'a str,
    pub signing_key: &'a [u8; 32],
    pub encryption_key: &'a [u8; 32],
    pub nonce: &'a [u8; 16],
}
pub fn signin_input(v: &SignIn<'_>) -> Result<Vec<u8>> {
    values::generated_id(v.code_id)?;
    values::space_id(v.space)?;
    values::generated_id(v.device)?;
    crypto::public_key(v.signing_key)?;
    frame(&[
        b"tmt-colab-signin-v1",
        b"1",
        v.code_id.as_bytes(),
        v.space.as_bytes(),
        v.device.as_bytes(),
        v.signing_key,
        v.encryption_key,
        v.nonce,
    ])
}
/// Decode exact framed sign-in bytes without replacing their keys with transport hints.
pub fn decode_signin(input: &[u8]) -> Result<SignIn<'_>> {
    let f = fields(input, 8, 1024)?;
    require(f[0] == b"tmt-colab-signin-v1" && f[1] == b"1")?;
    let v = SignIn {
        code_id: text(f[2])?,
        space: text(f[3])?,
        device: text(f[4])?,
        signing_key: f[5].try_into().map_err(|_| Invalid)?,
        encryption_key: f[6].try_into().map_err(|_| Invalid)?,
        nonce: f[7].try_into().map_err(|_| Invalid)?,
    };
    require(signin_input(&v)? == input)?;
    Ok(v)
}
pub fn signin_proof(code: &[u8; 16], v: &SignIn<'_>) -> Result<[u8; 32]> {
    Ok(crypto::mac(code, &signin_input(v)?))
}
pub fn signin_possession_input(v: &SignIn<'_>) -> Result<Vec<u8>> {
    frame(&[b"tmt-colab-signin-possession-v1", b"1", &signin_input(v)?])
}
pub fn verify_signin(
    code: &[u8; 16],
    v: &SignIn<'_>,
    proof: &[u8],
    signature: &[u8],
) -> Result<()> {
    crypto::verify_mac(code, &signin_input(v)?, proof)?;
    crypto::verify_signature(v.signing_key, &signin_possession_input(v)?, signature)
}
pub fn operation(value: &str) -> Result<()> {
    require(matches!(
        value,
        "member.add"
            | "member.remove"
            | "member.role"
            | "link.add"
            | "link.remove"
            | "device.revoke"
            | "bridge.add"
            | "epoch.advance"
            | "page.share"
            | "page.scripts"
            | "retention.set"
            | "page.archive"
            | "page.delete"
    ))
}
pub struct Management<'a> {
    pub space: &'a str,
    pub page: &'a str,
    pub expected_revision: &'a str,
    pub operation_id: &'a str,
    pub operation: &'a str,
    pub payload: &'a [u8],
    pub sender_device: &'a str,
    pub issued_at: u64,
    pub expires_at: u64,
}
/// Binds exact payload bytes; operation-specific strict JSON schema admission is a caller prerequisite.
pub fn management_input(v: &Management<'_>) -> Result<Vec<u8>> {
    values::space_id(v.space)?;
    for id in [v.page, v.operation_id, v.sender_device] {
        values::generated_id(id)?;
    }
    values::decimal(v.expected_revision, false)?;
    operation(v.operation)?;
    values::time(v.issued_at)?;
    values::time(v.expires_at)?;
    require(
        v.expires_at > v.issued_at
            && v.expires_at - v.issued_at <= 600_000
            && v.payload.len() <= 16 * 1024,
    )?;
    frame(&[
        b"tmt-colab-management-v1",
        b"1",
        v.space.as_bytes(),
        v.page.as_bytes(),
        v.expected_revision.as_bytes(),
        v.operation_id.as_bytes(),
        v.operation.as_bytes(),
        &crypto::digest(v.payload),
        v.sender_device.as_bytes(),
        v.issued_at.to_string().as_bytes(),
        v.expires_at.to_string().as_bytes(),
    ])
}
