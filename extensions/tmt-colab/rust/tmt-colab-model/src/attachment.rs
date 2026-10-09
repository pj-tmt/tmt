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
pub const REFERENCE_BYTES: usize = 2048;
/// The base a message-source attachment is fenced by: the space, page and epoch it was sealed
/// under, the membership head it was admitted by and its author. Unlike a page revision it
/// covers no stream cut, so a foreign write while the upload runs leaves it valid, while a
/// membership or epoch change does not. The label is distinct, so a page revision (the base
/// of a document-source attachment) can never satisfy a fence or the reverse.
pub fn message_fence(
    space: &str,
    page: &str,
    epoch: &str,
    membership_revision: &str,
    membership_hash: &[u8; 32],
    author: &str,
) -> Result<String> {
    values::space_id(space)?;
    values::generated_id(page)?;
    values::decimal(epoch, false)?;
    values::decimal(membership_revision, false)?;
    values::generated_id(author)?;
    let digest = crypto::digest(&framing::frame(&[
        b"tmt-colab-attachment-fence-v1",
        b"1",
        space.as_bytes(),
        page.as_bytes(),
        epoch.as_bytes(),
        membership_revision.as_bytes(),
        membership_hash,
        author.as_bytes(),
    ])?);
    Ok(format!(
        "v1:{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}
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
/// The two display labels of an attachment, which grant nothing. A type is an inert label,
/// never a preview or content-sniffing permission.
pub fn validate_labels(filename: &str, media_type: &str) -> Result<()> {
    require(
        !filename.is_empty() && filename.len() <= 255 && !filename.chars().any(char::is_control),
    )?;
    require(!media_type.is_empty() && media_type.len() <= 128)?;
    let parts: Vec<_> = media_type.split('/').collect();
    require(
        parts.len() == 2
            && parts.iter().all(|part| {
                !part.is_empty()
                    && part.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b"!#$&^_.+-".contains(&b)
                    })
            }),
    )
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
        validate_labels(&self.filename, &self.media_type)?;
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
/// One typed change to a document's `meta.attachments`, never free-form metadata. `set`
/// adds descriptors the writer already committed and proved; `remove` drops references by
/// attachment ID. Descriptors are immutable, so a `set` of an existing ID must be identical.
/// The browser and native preparers share its vectors (`attachment-change-v1.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentChange {
    #[serde(default, skip_serializing_if = "List::is_empty")]
    pub set: List<Descriptor, DOCUMENT_ATTACHMENTS>,
    #[serde(default, skip_serializing_if = "List::is_empty")]
    pub remove: List<String, DOCUMENT_ATTACHMENTS>,
}
impl DocumentChange {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.remove.is_empty()
    }
    /// Structure and source binding, before any page is consulted: each added descriptor is a
    /// valid document asset whose recorded source digest is the source being written.
    pub fn validate(&self, source_sha256: &[u8; 32]) -> Result<()> {
        let digest = hex(source_sha256);
        let mut ids = HashSet::new();
        for descriptor in self.set.as_slice() {
            descriptor.validate()?;
            require(
                descriptor.namespace == "content"
                    && matches!(&descriptor.source, Source::Document { source_digest } if *source_digest == digest)
                    && ids.insert(descriptor.attachment_id.as_str()),
            )?;
        }
        for id in self.remove.as_slice() {
            values::generated_id(id)?;
            require(ids.insert(id.as_str()))?;
        }
        Ok(())
    }
    /// The list after the change, in order: removals first, then new descriptors appended.
    /// A removal of an absent ID, a different descriptor under an existing ID and a result
    /// over the document cap are refused rather than ignored.
    pub fn apply(&self, current: &[Descriptor]) -> Result<Vec<Descriptor>> {
        let mut out: Vec<Descriptor> = current.to_vec();
        for id in self.remove.as_slice() {
            let before = out.len();
            out.retain(|d| d.attachment_id != *id);
            require(out.len() + 1 == before)?;
        }
        for descriptor in self.set.as_slice() {
            match out
                .iter()
                .find(|d| d.attachment_id == descriptor.attachment_id)
            {
                Some(existing) => require(existing == descriptor)?,
                None => out.push(descriptor.clone()),
            }
        }
        validate_attachment_list(&out, DOCUMENT_ATTACHMENTS, None)?;
        Ok(out)
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

/// This record has authority only inside the original creator's authenticated,
/// positive-sequence own stream. Its fields alone are not a creation witness.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentPublication {
    pub version: u8,
    pub kind: String,
    pub space_id: String,
    pub page_id: String,
    pub epoch: String,
    pub sender_device: String,
    pub membership_revision: String,
    pub attachment_id: String,
    pub descriptor_hash: String,
    pub source: Source,
    pub base_revision: String,
}
impl AttachmentPublication {
    pub fn from_json(raw: &[u8]) -> Result<Self> {
        require(raw.len() <= REFERENCE_BYTES)?;
        let value: Self = serde_json::from_slice(raw).map_err(|_| Invalid)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        require(self.version == 1 && self.kind == "attachment-publication")?;
        values::space_id(&self.space_id)?;
        for id in [&self.page_id, &self.sender_device, &self.attachment_id] {
            values::generated_id(id)?;
        }
        values::decimal(&self.epoch, false)?;
        values::decimal(&self.membership_revision, false)?;
        values::object_id(&self.descriptor_hash)?;
        self.source.input()?;
        attachment_revision(&self.base_revision)?;
        require(serde_json::to_vec(self).map_err(|_| Invalid)?.len() <= REFERENCE_BYTES)
    }
    pub fn matches_descriptor(&self, descriptor: &Descriptor) -> Result<()> {
        self.validate()?;
        require(
            self.space_id == descriptor.space
                && self.page_id == descriptor.page
                && self.epoch == descriptor.epoch
                && self.sender_device == descriptor.author_device
                && self.membership_revision == descriptor.membership_revision
                && self.attachment_id == descriptor.attachment_id
                && self.descriptor_hash == hex(&descriptor.hash()?)
                && self.source == descriptor.source,
        )
    }
}

/// A bounded exact-reference selector, never an actor or permission claim.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AttachmentSelector {
    DocumentCurrent {
        #[serde(rename = "attachmentId")]
        attachment_id: String,
        #[serde(rename = "descriptorHash")]
        descriptor_hash: String,
        #[serde(rename = "contentRevision")]
        content_revision: String,
    },
    Message {
        #[serde(rename = "writerId")]
        writer_id: String,
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "messageRevision")]
        message_revision: String,
        #[serde(rename = "attachmentId")]
        attachment_id: String,
        #[serde(rename = "descriptorHash")]
        descriptor_hash: String,
    },
}
impl AttachmentSelector {
    pub fn from_json(raw: &[u8]) -> Result<Self> {
        require(raw.len() <= REFERENCE_BYTES)?;
        let value: Self = serde_json::from_slice(raw).map_err(|_| Invalid)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        let (id, hash) = match self {
            Self::DocumentCurrent {
                attachment_id,
                descriptor_hash,
                content_revision,
            } => {
                attachment_revision(content_revision)?;
                (attachment_id, descriptor_hash)
            }
            Self::Message {
                writer_id,
                message_id,
                message_revision,
                attachment_id,
                descriptor_hash,
            } => {
                values::generated_id(writer_id)?;
                values::generated_id(message_id)?;
                values::decimal(message_revision, false)?;
                (attachment_id, descriptor_hash)
            }
        };
        values::generated_id(id)?;
        values::object_id(hash)?;
        require(serde_json::to_vec(self).map_err(|_| Invalid)?.len() <= REFERENCE_BYTES)
    }
    pub fn attachment(&self) -> (&str, &str) {
        match self {
            Self::DocumentCurrent {
                attachment_id,
                descriptor_hash,
                ..
            }
            | Self::Message {
                attachment_id,
                descriptor_hash,
                ..
            } => (attachment_id, descriptor_hash),
        }
    }
}
fn attachment_revision(value: &str) -> Result<()> {
    values::object_id(value.strip_prefix("v1:").ok_or(Invalid)?)
}
