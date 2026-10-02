//! Owner-authenticated RFC9180 Base wraps. Syntax/key agreement alone grants no authority.
use crate::{
    Invalid, Result, crypto,
    framing::{fields, frame, text},
    require, values,
};
use ed25519_dalek::{Signer, SigningKey};
use hpke::{
    Deserializable, Kem, OpModeR, OpModeS, Serializable, aead::AesGcm256, kdf::HkdfSha256,
    kem::X25519HkdfSha256 as X,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub space: String,
    pub page: String,
    pub epoch: String,
    pub recipient_kind: String,
    pub recipient_id: String,
    pub recipient_key: [u8; 32],
    pub signer_key: [u8; 32],
    pub membership_revision: String,
}
impl Header {
    pub fn encode(&self) -> Result<Vec<u8>> {
        values::space_id(&self.space)?;
        values::generated_id(&self.page)?;
        values::decimal(&self.epoch, false)?;
        values::generated_id(&self.recipient_id)?;
        values::decimal(&self.membership_revision, false)?;
        require(matches!(
            self.recipient_kind.as_str(),
            "member" | "device" | "link" | "bridge"
        ))?;
        crypto::public_key(&self.signer_key)?;
        let out = frame(&[
            b"tmt-colab-wrap-v1",
            b"1",
            b"base-x25519-hkdfsha256-aes256gcm",
            self.space.as_bytes(),
            self.page.as_bytes(),
            self.epoch.as_bytes(),
            self.recipient_kind.as_bytes(),
            self.recipient_id.as_bytes(),
            &self.recipient_key,
            &self.signer_key,
            self.membership_revision.as_bytes(),
            b"epoch-key",
        ])?;
        require(out.len() <= 1024)?;
        Ok(out)
    }
    pub fn decode(input: &[u8]) -> Result<Self> {
        let f = fields(input, 12, 1024)?;
        require(
            f[0] == b"tmt-colab-wrap-v1"
                && f[1] == b"1"
                && f[2] == b"base-x25519-hkdfsha256-aes256gcm"
                && f[11] == b"epoch-key",
        )?;
        let h = Self {
            space: text(f[3])?.into(),
            page: text(f[4])?.into(),
            epoch: text(f[5])?.into(),
            recipient_kind: text(f[6])?.into(),
            recipient_id: text(f[7])?.into(),
            recipient_key: f[8].try_into().map_err(|_| Invalid)?,
            signer_key: f[9].try_into().map_err(|_| Invalid)?,
            membership_revision: text(f[10])?.into(),
        };
        require(h.encode()? == input)?;
        Ok(h)
    }
}
/// Native keyring imports a long-term seed; no private export or fixture-ephemeral API.
pub struct RecipientKey(<X as Kem>::PrivateKey);
impl RecipientKey {
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self> {
        Ok(Self(
            <X as Kem>::PrivateKey::from_bytes(seed).map_err(|_| Invalid)?,
        ))
    }
    pub fn public_key(&self) -> [u8; 32] {
        X::sk_to_pk(&self.0).to_bytes().into()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    header: Vec<u8>,
    enc: [u8; 32],
    ciphertext: [u8; 48],
    signature: [u8; 64],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    header: String,
    enc: String,
    ciphertext: String,
    signature: String,
}
impl Envelope {
    fn wire(&self) -> Wire {
        Wire {
            header: values::encode_binary(&self.header),
            enc: values::encode_binary(&self.enc),
            ciphertext: values::encode_binary(&self.ciphertext),
            signature: values::encode_binary(&self.signature),
        }
    }
    fn from_wire(w: Wire) -> Result<Self> {
        let header = values::binary(&w.header, 1024)?;
        Header::decode(&header)?;
        Ok(Self {
            header,
            enc: values::binary(&w.enc, 32)?
                .try_into()
                .map_err(|_| Invalid)?,
            ciphertext: values::binary(&w.ciphertext, 48)?
                .try_into()
                .map_err(|_| Invalid)?,
            signature: values::binary(&w.signature, 64)?
                .try_into()
                .map_err(|_| Invalid)?,
        })
    }
    pub fn from_json(input: &[u8]) -> Result<Self> {
        require(input.len() <= 2048)?;
        Self::from_wire(serde_json::from_slice(input).map_err(|_| Invalid)?)
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self.wire()).map_err(|_| Invalid)
    }
    pub fn header(&self) -> Result<Header> {
        Header::decode(&self.header)
    }
    pub fn verify_owner(&self, owner: &[u8; 32]) -> Result<()> {
        let h = self.header()?;
        require(h.signer_key == *owner && crypto::space_id(owner)? == h.space)?;
        crypto::verify_signature(owner, &self.signature_input()?, &self.signature)
    }
    pub fn signature_input(&self) -> Result<Vec<u8>> {
        frame(&[
            b"tmt-colab-wrap-signature-v1",
            b"1",
            &self.header,
            &self.enc,
            &crypto::digest(&self.ciphertext),
        ])
    }
}
impl Serialize for Envelope {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.wire().serialize(s)
    }
}
impl<'de> Deserialize<'de> for Envelope {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::from_wire(Wire::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
/// Fresh ephemeral randomness stays inside the pinned library, never a caller input.
/// The library panics if OS entropy fails; no signature/authority operation occurs before setup.
pub fn seal(header: &Header, key: &[u8; 32], owner: &SigningKey) -> Result<Envelope> {
    require(
        header.signer_key == owner.verifying_key().to_bytes()
            && crypto::space_id(&header.signer_key)? == header.space,
    )?;
    let bytes = header.encode()?;
    let info = frame(&[b"tmt-colab-hpke-info-v1", &bytes])?;
    let recipient =
        <X as Kem>::PublicKey::from_bytes(&header.recipient_key).map_err(|_| Invalid)?;
    let (enc, ciphertext) = hpke::single_shot_seal::<AesGcm256, HkdfSha256, X>(
        &OpModeS::Base,
        &recipient,
        &info,
        key,
        &bytes,
    )
    .map_err(|_| Invalid)?;
    let mut out = Envelope {
        header: bytes,
        enc: enc.to_bytes().into(),
        ciphertext: ciphertext.try_into().map_err(|_| Invalid)?,
        signature: [0; 64],
    };
    out.signature = owner.sign(&out.signature_input()?).to_bytes();
    Ok(out)
}
/// Expected context and owner key must resolve from the caller's latest verified log.
pub fn open(
    envelope: &Envelope,
    expected: &Header,
    recipient: &RecipientKey,
    owner: &[u8; 32],
) -> Result<[u8; 32]> {
    require(
        envelope.header()? == *expected
            && expected.signer_key == *owner
            && expected.recipient_key == recipient.public_key(),
    )?;
    envelope.verify_owner(owner)?;
    let enc = <X as Kem>::EncappedKey::from_bytes(&envelope.enc).map_err(|_| Invalid)?;
    let info = frame(&[b"tmt-colab-hpke-info-v1", &envelope.header])?;
    hpke::single_shot_open::<AesGcm256, HkdfSha256, X>(
        &OpModeR::Base,
        &recipient.0,
        &enc,
        &info,
        &envelope.ciphertext,
        &envelope.header,
    )
    .map_err(|_| Invalid)?
    .try_into()
    .map_err(|_| Invalid)
}
