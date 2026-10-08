//! Bounded attachment syntax and asset verification. References and current
//! membership/history authority are admitted by the caller, never by a label.
use crate::{Invalid, Result, bounded::List, crypto, framing, object, require, values};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const DESCRIPTOR_BYTES: usize = 2048;
pub const PLAINTEXT_BYTES: usize = 8 * 1024 * 1024;
pub const PAYLOAD_BYTES: usize = 12 * 1024 * 1024;
pub const MESSAGE_ATTACHMENTS: usize = 16;
pub const DOCUMENT_ATTACHMENTS: usize = 128;
pub const MANIFEST_BYTES: usize = DOCUMENT_ATTACHMENTS * DESCRIPTOR_BYTES + 1024;
pub type DocumentAttachments = List<Descriptor, DOCUMENT_ATTACHMENTS>;
pub type MessageAttachments = List<Descriptor, MESSAGE_ATTACHMENTS>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Source {
    Document {
        #[serde(rename = "sourceDigest")]
        source_digest: String,
    },
    Message {
        #[serde(rename = "writerId")]
        writer_id: String,
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "messageRevision")]
        message_revision: String,
    },
}
impl Source {
    fn input(&self) -> Result<Vec<u8>> {
        match self {
            Self::Document { source_digest } => {
                values::object_id(source_digest)?;
                framing::frame(&[b"document", source_digest.as_bytes()])
            }
            Self::Message {
                writer_id,
                message_id,
                message_revision,
            } => {
                values::generated_id(writer_id)?;
                values::generated_id(message_id)?;
                values::decimal(message_revision, false)?;
                framing::frame(&[
                    b"message",
                    writer_id.as_bytes(),
                    message_id.as_bytes(),
                    message_revision.as_bytes(),
                ])
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Descriptor {
    pub version: u8,
    pub attachment_id: String,
    pub space: String,
    pub page: String,
    pub epoch: String,
    pub namespace: String,
    pub object_id: String,
    pub author_device: String,
    pub membership_revision: String,
    pub source: Source,
    pub envelope_hash: String,
    pub signature: String,
    pub payload_sha256: String,
    pub payload_bytes: String,
    pub plaintext_bytes: String,
    pub filename: String,
    pub media_type: String,
}
impl Descriptor {
    pub fn from_json(raw: &[u8]) -> Result<Self> {
        require(raw.len() <= DESCRIPTOR_BYTES)?;
        let value: Self = serde_json::from_slice(raw).map_err(|_| Invalid)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        require(self.version == 1)?;
        values::generated_id(&self.attachment_id)?;
        object::Header {
            context: self.context(),
            object_id: self.object_id.clone(),
        }
        .encode()?;
        self.source.input()?;
        require(matches!(
            (&self.source, self.namespace.as_str()),
            (Source::Document { .. }, "content") | (Source::Message { .. }, "own")
        ))?;
        values::object_id(&self.envelope_hash)?;
        values::object_id(&self.payload_sha256)?;
        require(values::binary(&self.signature, 64)?.len() == 64)?;
        require(values::decimal(&self.payload_bytes, false)? <= PAYLOAD_BYTES as u64)?;
        require(values::decimal(&self.plaintext_bytes, true)? <= PLAINTEXT_BYTES as u64)?;
        require(
            !self.filename.is_empty()
                && self.filename.len() <= 255
                && !self.filename.chars().any(char::is_control),
        )?;
        // A type is an inert label, never a preview or content-sniffing permission.
        require(!self.media_type.is_empty() && self.media_type.len() <= 128)?;
        let parts: Vec<_> = self.media_type.split('/').collect();
        require(
            parts.len() == 2
                && parts.iter().all(|part| {
                    !part.is_empty()
                        && part.bytes().all(|b| {
                            b.is_ascii_lowercase()
                                || b.is_ascii_digit()
                                || b"!#$&^_.+-".contains(&b)
                        })
                }),
        )?;
        require(serde_json::to_vec(self).map_err(|_| Invalid)?.len() <= DESCRIPTOR_BYTES)
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| Invalid)
    }
    pub fn input(&self) -> Result<Vec<u8>> {
        self.validate()?;
        framing::frame(&[
            b"tmt-colab-attachment-v1",
            b"1",
            self.attachment_id.as_bytes(),
            self.space.as_bytes(),
            self.page.as_bytes(),
            self.epoch.as_bytes(),
            self.namespace.as_bytes(),
            self.object_id.as_bytes(),
            self.author_device.as_bytes(),
            self.membership_revision.as_bytes(),
            &self.source.input()?,
            self.envelope_hash.as_bytes(),
            self.signature.as_bytes(),
            self.payload_sha256.as_bytes(),
            self.payload_bytes.as_bytes(),
            self.plaintext_bytes.as_bytes(),
            self.filename.as_bytes(),
            self.media_type.as_bytes(),
        ])
    }
    pub fn hash(&self) -> Result<[u8; 32]> {
        Ok(crypto::digest(&self.input()?))
    }
    pub fn context(&self) -> object::Context {
        object::Context {
            space: self.space.clone(),
            page: self.page.clone(),
            epoch: self.epoch.clone(),
            kind: "asset".into(),
            namespace: self.namespace.clone(),
            author_device: self.author_device.clone(),
            membership_revision: self.membership_revision.clone(),
            stream_seq: "0".into(),
            prev_hash: [0; 32],
        }
    }
    /// The caller supplies the independently admitted creator context/key and
    /// eligible epoch secret. A descriptor or server-held secret is not authority.
    pub fn open(
        &self,
        payload: &[u8],
        admitted: &object::Context,
        secret: &[u8; 32],
        public_key: &[u8; 32],
    ) -> Result<Vec<u8>> {
        self.validate()?;
        require(
            &self.context() == admitted
                && payload.len() as u64 == values::decimal(&self.payload_bytes, false)?,
        )?;
        require(hex(&crypto::digest(payload)) == self.payload_sha256)?;
        let envelope = object::Envelope::from_json(payload)?;
        require(
            envelope.ciphertext().len() as u64
                == values::decimal(&self.plaintext_bytes, true)? + 16,
        )?;
        let header = object::Header::decode(envelope.header())?;
        require(header.object_id == self.object_id && header.context == *admitted)?;
        require(
            hex(&envelope.hash()?) == self.envelope_hash
                && values::encode_binary(envelope.signature()) == self.signature,
        )?;
        let mut plain = object::open(&envelope, admitted, secret, public_key)?;
        if plain.len() as u64 != values::decimal(&self.plaintext_bytes, true)? {
            plain.fill(0);
            return Err(Invalid);
        }
        Ok(plain)
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn validate_attachment_list(
    values: &[Descriptor],
    limit: usize,
    scope: Option<(&str, &str)>,
) -> Result<()> {
    require(values.len() <= limit)?;
    let mut ids = HashSet::new();
    for value in values {
        value.validate()?;
        require(
            scope.is_none_or(|(space, page)| value.space == space && value.page == page)
                && ids.insert(&value.attachment_id),
        )?;
    }
    Ok(())
}

/// Exact ordered list bound by a snapshot's authenticated record. This codec
/// neither creates a snapshot nor grants access to its historical asset keys.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub version: u8,
    pub space: String,
    pub page: String,
    pub snapshot_id: String,
    pub author_device: String,
    pub membership_revision: String,
    pub source_digest: String,
    pub attachments: List<Descriptor, DOCUMENT_ATTACHMENTS>,
}
impl Manifest {
    pub fn from_json(raw: &[u8]) -> Result<Self> {
        require(raw.len() <= MANIFEST_BYTES)?;
        let value: Self = serde_json::from_slice(raw).map_err(|_| Invalid)?;
        value.input()?;
        Ok(value)
    }
    pub fn input(&self) -> Result<Vec<u8>> {
        require(self.version == 1)?;
        values::space_id(&self.space)?;
        values::generated_id(&self.page)?;
        values::generated_id(&self.snapshot_id)?;
        values::generated_id(&self.author_device)?;
        values::decimal(&self.membership_revision, false)?;
        values::object_id(&self.source_digest)?;
        validate_attachment_list(
            self.attachments.as_slice(),
            DOCUMENT_ATTACHMENTS,
            Some((&self.space, &self.page)),
        )?;
        let entries: Vec<_> = self
            .attachments
            .as_slice()
            .iter()
            .map(Descriptor::input)
            .collect::<Result<_>>()?;
        let count = (entries.len() as u32).to_be_bytes();
        let mut list = count.to_vec();
        list.extend(framing::frame(
            &entries.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        )?);
        framing::frame(&[
            b"tmt-colab-attachment-manifest-v1",
            b"1",
            self.space.as_bytes(),
            self.page.as_bytes(),
            self.snapshot_id.as_bytes(),
            self.author_device.as_bytes(),
            self.membership_revision.as_bytes(),
            self.source_digest.as_bytes(),
            &list,
        ])
    }
    pub fn hash(&self) -> Result<[u8; 32]> {
        Ok(crypto::digest(&self.input()?))
    }
}
