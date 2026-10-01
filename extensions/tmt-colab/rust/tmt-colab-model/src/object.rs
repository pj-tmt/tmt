//! Immutable objects: one internally generated ID per seal; retries retain the envelope.
use crate::{
    Invalid, Result, crypto,
    framing::{fields, frame, text},
    require, values,
};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};

pub const SUITE: &str = "aes256gcm-hkdfsha256-ed25519-v1";
pub const MAX_PLAINTEXT: usize = 16 * 1024 * 1024;
/// Serialized envelope cap: 16 MiB plaintext plus 2 KiB binary overhead reserve
/// (tag, header, nonce and signature), base64 expansion, then 2 KiB JSON reserve.
/// Includes whitespace/field syntax; this is a per-object cap, not a page quota.
pub const MAX_ENVELOPE_JSON: usize = (MAX_PLAINTEXT + 2048) * 4 / 3 + 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub space: String,
    pub page: String,
    pub epoch: String,
    pub kind: String,
    pub namespace: String,
    pub author_device: String,
    pub membership_revision: String,
    pub stream_seq: String,
    pub prev_hash: [u8; 32],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub context: Context,
    pub object_id: String,
}
impl Header {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let v = &self.context;
        values::space_id(&v.space)?;
        values::generated_id(&v.page)?;
        values::generated_id(&v.author_device)?;
        values::object_id(&self.object_id)?;
        values::namespace(&v.namespace)?;
        values::decimal(&v.epoch, false)?;
        values::decimal(&v.membership_revision, false)?;
        let seq = values::decimal(&v.stream_seq, true)?;
        require(match v.kind.as_str() {
            "update" | "checkpoint" => seq > 0,
            "html" => seq == 0 && v.prev_hash == [0; 32] && v.namespace == "content",
            "asset" => seq == 0 && v.prev_hash == [0; 32],
            _ => false,
        })?;
        if v.kind == "update" && seq == 1 {
            require(v.prev_hash == [0; 32])?;
        }
        let bytes = frame(&[
            b"tmt-colab-object-v1",
            b"1",
            SUITE.as_bytes(),
            v.space.as_bytes(),
            v.page.as_bytes(),
            v.epoch.as_bytes(),
            v.kind.as_bytes(),
            v.namespace.as_bytes(),
            self.object_id.as_bytes(),
            v.author_device.as_bytes(),
            v.membership_revision.as_bytes(),
            v.stream_seq.as_bytes(),
            &v.prev_hash,
        ])?;
        require(bytes.len() <= 1024)?;
        Ok(bytes)
    }
    pub fn decode(input: &[u8]) -> Result<Self> {
        let f = fields(input, 13, 1024)?;
        require(f[0] == b"tmt-colab-object-v1" && f[1] == b"1" && f[2] == SUITE.as_bytes())?;
        let value = Self {
            context: Context {
                space: text(f[3])?.into(),
                page: text(f[4])?.into(),
                epoch: text(f[5])?.into(),
                kind: text(f[6])?.into(),
                namespace: text(f[7])?.into(),
                author_device: text(f[9])?.into(),
                membership_revision: text(f[10])?.into(),
                stream_seq: text(f[11])?.into(),
                prev_hash: f[12].try_into().map_err(|_| Invalid)?,
            },
            object_id: text(f[8])?.into(),
        };
        require(value.encode()? == input)?;
        Ok(value)
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Envelope {
    header: Vec<u8>,
    ciphertext: Vec<u8>,
    signature: [u8; 64],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    header: String,
    nonce: String,
    ciphertext: String,
    signature: String,
}
impl Envelope {
    pub fn header(&self) -> &[u8] {
        &self.header
    }
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&Wire {
            header: values::encode_binary(&self.header),
            nonce: values::encode_binary(&[0; 12]),
            ciphertext: values::encode_binary(&self.ciphertext),
            signature: values::encode_binary(&self.signature),
        })
        .map_err(|_| Invalid)
    }
    /// Syntax only: never establishes certificate, writer, epoch, role or stream authority.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_ENVELOPE_JSON)?;
        let wire: Wire = serde_json::from_slice(bytes).map_err(|_| Invalid)?;
        let header = values::binary(&wire.header, 1024)?;
        let decoded = Header::decode(&header)?;
        require(values::binary(&wire.nonce, 12)? == [0; 12])?;
        let ciphertext = values::binary(
            &wire.ciphertext,
            plaintext_limit(&decoded.context.kind) + 16,
        )?;
        require(ciphertext.len() >= 16)?;
        let signature = values::binary(&wire.signature, 64)?
            .try_into()
            .map_err(|_| Invalid)?;
        Ok(Self {
            header,
            ciphertext,
            signature,
        })
    }
    pub fn signature_input(&self) -> Result<Vec<u8>> {
        frame(&[
            b"tmt-colab-signature-v1",
            &self.header,
            &[0; 12],
            &crypto::digest(&self.ciphertext),
        ])
    }
    pub fn hash(&self) -> Result<[u8; 32]> {
        Ok(crypto::digest(&frame(&[
            b"tmt-colab-envelope-hash-v1",
            &self.header,
            &[0; 12],
            &self.ciphertext,
            &self.signature,
        ])?))
    }
}
fn plaintext_limit(kind: &str) -> usize {
    match kind {
        "update" => 256 * 1024,
        _ => MAX_PLAINTEXT,
    }
}
fn cipher(secret: &[u8; 32], header: &[u8]) -> Result<Aes256Gcm> {
    let key = crypto::derive_key(secret, &[], &frame(&[b"tmt-colab-object-key-v1", header])?);
    Aes256Gcm::new_from_slice(&key).map_err(|_| Invalid)
}
/// Does not accept an object ID or fixture entropy. Caller enforces the 2^32 per-epoch ceiling.
/// A caller-supplied Header/object ID is not a seal input:
/// ```compile_fail
/// use tmt_colab_model::object::{seal, Header};
/// fn inject(h: &Header, k: &[u8; 32], s: &ed25519_dalek::SigningKey) {
///     let _ = seal(h, k, s, b"text");
/// }
/// ```
pub fn seal(
    context: &Context,
    secret: &[u8; 32],
    signer: &SigningKey,
    plaintext: &[u8],
) -> Result<Envelope> {
    require(plaintext.len() <= plaintext_limit(&context.kind))?;
    let mut id = [0; 32];
    getrandom::fill(&mut id).map_err(|_| Invalid)?;
    let object_id = id.iter().map(|b| format!("{b:02x}")).collect();
    let header = Header {
        context: context.clone(),
        object_id,
    }
    .encode()?;
    let nonce: &Nonce<aes_gcm::aead::consts::U12> =
        (&[0u8; 12][..]).try_into().map_err(|_| Invalid)?;
    let ciphertext = cipher(secret, &header)?
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: &header,
            },
        )
        .map_err(|_| Invalid)?;
    let mut out = Envelope {
        header,
        ciphertext,
        signature: [0; 64],
    };
    out.signature = signer.sign(&out.signature_input()?).to_bytes();
    Ok(out)
}
/// Caller first admits chain, epoch, role and stream order, then supplies that exact context/key.
/// Neither a valid signature nor this context comparison establishes those policy facts.
pub fn open(
    envelope: &Envelope,
    expected: &Context,
    secret: &[u8; 32],
    author_key: &[u8; 32],
) -> Result<Vec<u8>> {
    let header = Header::decode(&envelope.header)?;
    require(
        &header.context == expected
            && envelope.ciphertext.len() >= 16
            && envelope.ciphertext.len() - 16 <= plaintext_limit(&expected.kind),
    )?;
    crypto::verify_signature(
        author_key,
        &envelope.signature_input()?,
        &envelope.signature,
    )?;
    let nonce: &Nonce<aes_gcm::aead::consts::U12> =
        (&[0u8; 12][..]).try_into().map_err(|_| Invalid)?;
    cipher(secret, &envelope.header)?
        .decrypt(
            nonce,
            Payload {
                msg: &envelope.ciphertext,
                aad: &envelope.header,
            },
        )
        .map_err(|_| Invalid)
}
