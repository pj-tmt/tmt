//! Fixed owner→member/link→device chain syntax; issuer bindings are caller-verified log facts.
use crate::{
    Invalid, Result, crypto,
    framing::{fields, frame, text},
    require, values,
};
use serde::Deserialize;
#[derive(Debug, PartialEq, Eq)]
pub struct Certificate<'a> {
    pub space: &'a str,
    pub issuer_kind: &'a str,
    pub issuer_id: &'a str,
    pub device_id: &'a str,
    pub signing_key: &'a [u8; 32],
    pub encryption_key: &'a [u8; 32],
    pub membership_revision: &'a str,
    pub issued_at: u64,
    pub expires_at: u64,
}
pub fn input(v: &Certificate<'_>) -> Result<Vec<u8>> {
    values::space_id(v.space)?;
    values::generated_id(v.issuer_id)?;
    values::generated_id(v.device_id)?;
    require(matches!(v.issuer_kind, "member" | "link"))?;
    crypto::public_key(v.signing_key)?;
    values::decimal(v.membership_revision, false)?;
    values::time(v.issued_at)?;
    values::time(v.expires_at)?;
    require(v.expires_at > v.issued_at)?;
    frame(&[
        b"tmt-colab-device-cert-v1",
        b"1",
        v.space.as_bytes(),
        v.issuer_kind.as_bytes(),
        v.issuer_id.as_bytes(),
        v.device_id.as_bytes(),
        v.signing_key,
        v.encryption_key,
        v.membership_revision.as_bytes(),
        v.issued_at.to_string().as_bytes(),
        v.expires_at.to_string().as_bytes(),
    ])
}
pub fn decode(bytes: &[u8]) -> Result<Certificate<'_>> {
    let f = fields(bytes, 11, 1024)?;
    require(f[0] == b"tmt-colab-device-cert-v1" && f[1] == b"1")?;
    let time = |b| {
        let n = values::decimal(text(b)?, true)?;
        values::time(n)?;
        Ok(n)
    };
    let v = Certificate {
        space: text(f[2])?,
        issuer_kind: text(f[3])?,
        issuer_id: text(f[4])?,
        device_id: text(f[5])?,
        signing_key: f[6].try_into().map_err(|_| Invalid)?,
        encryption_key: f[7].try_into().map_err(|_| Invalid)?,
        membership_revision: text(f[8])?,
        issued_at: time(f[9])?,
        expires_at: time(f[10])?,
    };
    require(input(&v)? == bytes)?;
    Ok(v)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Wire {
    version: u8,
    issuer_statement: String,
    device_certificate: String,
    issuer_signature: String,
}
pub struct Chain {
    issuer_statement: [u8; 32],
    certificate: Vec<u8>,
    signature: [u8; 64],
}
impl Chain {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= 16 * 1024)?;
        let w: Wire = serde_json::from_slice(bytes).map_err(|_| Invalid)?;
        require(w.version == 1)?;
        let certificate = values::binary(&w.device_certificate, 1024)?;
        decode(&certificate)?;
        Ok(Self {
            issuer_statement: values::binary(&w.issuer_statement, 32)?
                .try_into()
                .map_err(|_| Invalid)?,
            certificate,
            signature: values::binary(&w.issuer_signature, 64)?
                .try_into()
                .map_err(|_| Invalid)?,
        })
    }
    pub fn certificate(&self) -> Result<Certificate<'_>> {
        decode(&self.certificate)
    }
    pub fn digest(&self) -> Result<[u8; 32]> {
        Ok(crypto::digest(&frame(&[
            b"tmt-colab-chain-v1",
            b"1",
            &self.issuer_statement,
            &self.certificate,
            &self.signature,
        ])?))
    }
    /// Caller first resolves a live member/link issuer from this exact owner statement hash.
    pub fn verify(
        &self,
        statement: &[u8; 32],
        expected: &Certificate<'_>,
        issuer_key: &[u8; 32],
    ) -> Result<()> {
        require(self.issuer_statement == *statement && self.certificate()?.eq(expected))?;
        crypto::verify_signature(issuer_key, &self.certificate, &self.signature)
    }
}
