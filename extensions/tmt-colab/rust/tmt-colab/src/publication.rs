//! Pure content-publication codecs. Exact bytes and signatures confer no effect authority.
//! No route, persistence, key selection, plaintext decoding or terminal-outcome creation.
use crate::{decoder, limits};
use serde::{Deserialize, Deserializer, Serialize};
use tmt_colab_model::{
    Invalid, Result,
    bounded::List,
    crypto,
    framing::frame,
    object::{Envelope, Header},
    values,
};

pub const JSON_BYTES: usize = 64 * 1024;
pub const CHAIN_BYTES: usize = 16 * 1024;
pub const MAX_PACKET_BYTES: usize = {
    let envelopes = decoder::WRITE_TAIL_UPDATES * limits::UPDATE_BYTES;
    let geometry = (decoder::WRITE_TAIL_BYTES + decoder::WRITE_TAIL_UPDATES * 2048) * 4 / 3
        + decoder::WRITE_TAIL_UPDATES * 2048;
    if envelopes < geometry {
        envelopes
    } else {
        geometry
    }
};
pub const LOCAL_WRITE_BYTES: usize =
    MAX_PACKET_BYTES.div_ceil(3) * 4 + JSON_BYTES + CHAIN_BYTES.div_ceil(3) * 4 + 2048;
/// The `LocalWrite` body version this build speaks and posts. A serve that receives another
/// version refuses it by name (`COLAB_SERVER_MISMATCH`) instead of reading it as this one.
pub const LOCAL_WRITE_VERSION: u8 = 2;
/// True when the body is JSON whose `version` is not [`LOCAL_WRITE_VERSION`]. Other fields are not
/// read, so a body of a later shape is still recognised as a later build.
pub fn names_other_write_version(body: &[u8]) -> bool {
    #[derive(Deserialize)]
    struct Versioned {
        version: serde_json::Value,
    }
    serde_json::from_slice::<Versioned>(body)
        .is_ok_and(|v| v.version != serde_json::json!(LOCAL_WRITE_VERSION))
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(Invalid) }
}
/// Checked even for a caller-supplied count outside the admitted entry range.
pub fn packet_limit(count: usize) -> Result<usize> {
    let reserve = count.checked_mul(2048).ok_or(Invalid)?;
    let geometry = decoder::WRITE_TAIL_BYTES
        .checked_add(reserve)
        .and_then(|n| n.checked_mul(4))
        .map(|n| n / 3)
        .and_then(|n| n.checked_add(reserve))
        .ok_or(Invalid)?;
    Ok(count
        .checked_mul(limits::UPDATE_BYTES)
        .ok_or(Invalid)?
        .min(geometry))
}
fn binary<const N: usize>(value: &str) -> Result<[u8; N]> {
    values::binary(value, N)?.try_into().map_err(|_| Invalid)
}
fn revision(value: &str) -> Result<()> {
    values::object_id(value.strip_prefix("v1:").ok_or(Invalid)?)
}
/// Whether `value` is a page revision token.
pub fn valid_revision(value: &str) -> bool {
    revision(value).is_ok()
}
fn json<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>> {
    let out = serde_json::to_vec(value).map_err(|_| Invalid)?;
    require(out.len() <= max)?;
    Ok(out)
}
fn parse<T: serde::de::DeserializeOwned>(input: &[u8], max: usize) -> Result<T> {
    require(input.len() <= max)?;
    serde_json::from_slice(input).map_err(|_| Invalid)
}
fn entries<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<PublicationEntry>, D::Error> {
    Ok(
        List::<PublicationEntry, { decoder::WRITE_TAIL_UPDATES }>::deserialize(d)?
            .as_slice()
            .to_vec(),
    )
}
fn present_evidence<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<NativeEvidence>, D::Error> {
    // Missing uses default None; a present field must be a complete object, never null.
    NativeEvidence::deserialize(d).map(Some)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PublicationKind {
    Content,
    Own,
}
impl PublicationKind {
    /// The stream namespace every entry of a publication of this kind belongs to.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Own => "own",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MembershipHead {
    pub revision: String,
    pub statement_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationEntry {
    pub namespace: PublicationKind,
    pub seq: String,
    pub envelope_hash: String,
    pub envelope_bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeEvidence {
    pub source_sha256: String,
    pub memory_limit: decoder::MemoryLimit,
    pub chain_hash: String,
}
impl NativeEvidence {
    fn validate(&self) -> Result<()> {
        values::object_id(&self.source_sha256)?;
        binary::<32>(&self.chain_hash)?;
        Ok(())
    }
    fn frame(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let memory = match self.memory_limit {
            decoder::MemoryLimit::Enforced => "512 MiB address-space limit",
            decoder::MemoryLimit::Unavailable => "memory limit unavailable",
        };
        let mut out = vec![1];
        out.extend(frame(&[
            self.source_sha256.as_bytes(),
            memory.as_bytes(),
            &binary::<32>(&self.chain_hash)?,
        ])?);
        Ok(out)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub version: u8,
    pub operation_id: String,
    pub space_id: String,
    pub page_id: String,
    pub epoch: String,
    pub stream_id: String,
    pub kind: PublicationKind,
    pub membership_head: MembershipHead,
    pub base_revision: String,
    #[serde(deserialize_with = "entries")]
    pub entries: Vec<PublicationEntry>,
    pub packet_bytes: usize,
    pub packet_hash: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_evidence"
    )]
    pub native_evidence: Option<NativeEvidence>,
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        require(
            self.version == 1
                && !self.entries.is_empty()
                && self.entries.len() <= decoder::WRITE_TAIL_UPDATES,
        )?;
        values::generated_id(&self.operation_id)?;
        values::space_id(&self.space_id)?;
        values::generated_id(&self.page_id)?;
        values::generated_id(&self.stream_id)?;
        values::decimal(&self.epoch, false)?;
        values::decimal(&self.membership_head.revision, false)?;
        values::object_id(&self.membership_head.statement_hash)?;
        revision(&self.base_revision)?;
        binary::<32>(&self.packet_hash)?;
        if let Some(evidence) = &self.native_evidence {
            evidence.validate()?;
        }
        let mut total = 0usize;
        let mut previous: Option<u64> = None;
        for entry in &self.entries {
            let seq = values::decimal(&entry.seq, false)?;
            require(previous.is_none_or(|p| p.checked_add(1) == Some(seq)))?;
            previous = Some(seq);
            binary::<32>(&entry.envelope_hash)?;
            require(entry.namespace == self.kind)?;
            require(entry.envelope_bytes > 0 && entry.envelope_bytes <= limits::UPDATE_BYTES)?;
            total = total.checked_add(entry.envelope_bytes).ok_or(Invalid)?;
        }
        require(total == self.packet_bytes && total <= packet_limit(self.entries.len())?)
    }
    pub fn signature_input(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut entries = u32::try_from(self.entries.len())
            .map_err(|_| Invalid)?
            .to_be_bytes()
            .to_vec();
        for entry in &self.entries {
            entries.extend(frame(&[
                entry.namespace.name().as_bytes(),
                entry.seq.as_bytes(),
                &binary::<32>(&entry.envelope_hash)?,
                entry.envelope_bytes.to_string().as_bytes(),
            ])?);
        }
        let evidence = self
            .native_evidence
            .as_ref()
            .map(NativeEvidence::frame)
            .transpose()?
            .unwrap_or_else(|| vec![0]);
        frame(&[
            b"tmt-colab-publication-v1",
            b"1",
            self.operation_id.as_bytes(),
            self.space_id.as_bytes(),
            self.page_id.as_bytes(),
            self.epoch.as_bytes(),
            self.stream_id.as_bytes(),
            self.kind.name().as_bytes(),
            self.membership_head.revision.as_bytes(),
            self.membership_head.statement_hash.as_bytes(),
            self.base_revision.as_bytes(),
            &entries,
            self.packet_bytes.to_string().as_bytes(),
            &binary::<32>(&self.packet_hash)?,
            &evidence,
        ])
    }
    pub fn job_digest(&self) -> Result<String> {
        Ok(values::encode_binary(&crypto::digest(
            &self.signature_input()?,
        )))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedJob {
    pub manifest: Manifest,
    pub signature: String,
}
/// Borrows the original JSON slice: downstream persistence must not substitute to_json().
pub struct VerifiedEntry<'a> {
    pub bytes: &'a [u8],
    pub envelope: Envelope,
    pub header: Header,
}
impl SignedJob {
    fn validate(&self) -> Result<()> {
        self.manifest.validate()?;
        binary::<64>(&self.signature)?;
        json(self, JSON_BYTES)?;
        Ok(())
    }
    pub fn from_json(input: &[u8]) -> Result<Self> {
        let out: Self = parse(input, JSON_BYTES)?;
        out.validate()?;
        Ok(out)
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        json(self, JSON_BYTES)
    }
    pub fn key(&self) -> Result<JobKey> {
        self.validate()?;
        let m = &self.manifest;
        Ok(JobKey {
            operation_id: m.operation_id.clone(),
            job_digest: m.job_digest()?,
            space_id: m.space_id.clone(),
            page_id: m.page_id.clone(),
            original_epoch: m.epoch.clone(),
            stream_id: m.stream_id.clone(),
        })
    }
    /// Authenticates syntax and exact bytes under the supplied key, never current effect authority.
    pub fn verify_packet<'a>(
        &self,
        packet: &'a [u8],
        author_key: &[u8],
    ) -> Result<Vec<VerifiedEntry<'a>>> {
        self.validate()?;
        crypto::public_key(author_key)?;
        let m = &self.manifest;
        require(
            packet.len() == m.packet_bytes
                && packet.len() <= packet_limit(m.entries.len())?
                && crypto::digest(packet) == binary::<32>(&m.packet_hash)?,
        )?;
        crypto::verify_signature(
            author_key,
            &m.signature_input()?,
            &binary::<64>(&self.signature)?,
        )?;
        let mut out = Vec::with_capacity(m.entries.len());
        let mut offset = 0usize;
        let mut previous = None;
        let mut payload_bytes = 0usize;
        for entry in &m.entries {
            let end = offset.checked_add(entry.envelope_bytes).ok_or(Invalid)?;
            let bytes = packet.get(offset..end).ok_or(Invalid)?;
            let envelope = Envelope::from_json(bytes)?;
            let header = Header::decode(envelope.header())?;
            let c = &header.context;
            require(
                c.kind == "update"
                    && c.namespace == m.kind.name()
                    && c.space == m.space_id
                    && c.page == m.page_id
                    && c.epoch == m.epoch
                    && c.author_device == m.stream_id
                    && c.membership_revision == m.membership_head.revision
                    && c.stream_seq == entry.seq
                    && previous.is_none_or(|hash| c.prev_hash == hash),
            )?;
            let hash = envelope.hash()?;
            require(hash == binary::<32>(&entry.envelope_hash)?)?;
            previous = Some(hash);
            let payload = envelope.ciphertext().len().checked_sub(16).ok_or(Invalid)?;
            require(payload <= decoder::UPDATE_BYTES)?;
            payload_bytes = payload_bytes.checked_add(payload).ok_or(Invalid)?;
            require(payload_bytes <= decoder::WRITE_TAIL_BYTES)?;
            crypto::verify_signature(
                author_key,
                &envelope.signature_input()?,
                envelope.signature(),
            )?;
            out.push(VerifiedEntry {
                bytes,
                envelope,
                header,
            });
            offset = end;
        }
        require(offset == packet.len())?;
        Ok(out)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobKey {
    pub operation_id: String,
    pub job_digest: String,
    pub space_id: String,
    pub page_id: String,
    pub original_epoch: String,
    pub stream_id: String,
}
impl JobKey {
    pub fn validate(&self) -> Result<()> {
        values::generated_id(&self.operation_id)?;
        binary::<32>(&self.job_digest)?;
        values::space_id(&self.space_id)?;
        values::generated_id(&self.page_id)?;
        values::decimal(&self.original_epoch, false)?;
        values::generated_id(&self.stream_id)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Position {
    pub seq: String,
    pub envelope_hash: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rejection {
    #[serde(rename = "COLAB_STALE_BASE")]
    StaleBase,
    #[serde(rename = "COLAB_CAPACITY")]
    Capacity,
    #[serde(rename = "COLAB_PAGE_INACTIVE")]
    PageInactive,
    #[serde(rename = "COLAB_STATE_MISSING")]
    StateMissing,
    #[serde(rename = "COLAB_STREAM_GAP")]
    StreamGap,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Outcome {
    Committed {
        key: JobKey,
        count: usize,
        final_position: Position,
        committed_revision: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present_evidence"
        )]
        native_evidence: Option<NativeEvidence>,
    },
    Rejected {
        key: JobKey,
        code: Rejection,
    },
    Unknown {
        key: JobKey,
    },
}
impl Outcome {
    pub fn key(&self) -> &JobKey {
        match self {
            Self::Committed { key, .. } | Self::Rejected { key, .. } | Self::Unknown { key } => key,
        }
    }
    pub fn validate(&self, expected: &JobKey, job: Option<&SignedJob>) -> Result<()> {
        self.key().validate()?;
        expected.validate()?;
        require(self.key() == expected)?;
        if let Some(job) = job {
            require(job.key()? == *expected)?;
        }
        if let Self::Committed {
            count,
            final_position,
            committed_revision,
            native_evidence,
            ..
        } = self
        {
            require(*count > 0 && *count <= decoder::WRITE_TAIL_UPDATES)?;
            values::decimal(&final_position.seq, false)?;
            binary::<32>(&final_position.envelope_hash)?;
            revision(committed_revision)?;
            if let Some(evidence) = native_evidence {
                evidence.validate()?;
            }
            if let Some(job) = job {
                let last = job.manifest.entries.last().ok_or(Invalid)?;
                require(
                    *count == job.manifest.entries.len()
                        && final_position.seq == last.seq
                        && final_position.envelope_hash == last.envelope_hash
                        && native_evidence == &job.manifest.native_evidence,
                )?;
            }
        }
        Ok(())
    }
    pub fn from_json(input: &[u8], expected: &JobKey, job: Option<&SignedJob>) -> Result<Self> {
        let out: Self = parse(input, JSON_BYTES)?;
        out.validate(expected, job)?;
        Ok(out)
    }
    pub fn to_json(&self, expected: &JobKey, job: Option<&SignedJob>) -> Result<Vec<u8>> {
        self.validate(expected, job)?;
        json(self, JSON_BYTES)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteAction {
    Write,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusAction {
    Status,
}
fn raw_signed_job<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<SignedJob, D::Error> {
    // Borrow the original nested object: reserialization would discard bytes inside its cap.
    let raw = <&'de serde_json::value::RawValue>::deserialize(d)?;
    SignedJob::from_json(raw.get().as_bytes())
        .map_err(|_| serde::de::Error::custom("invalid or oversized signed job"))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalWrite {
    pub version: u8,
    pub action: WriteAction,
    #[serde(deserialize_with = "raw_signed_job")]
    pub signed_job: SignedJob,
    pub packet: String,
    pub chain: String,
}
impl LocalWrite {
    fn validate(&self, author_key: &[u8]) -> Result<()> {
        require(self.version == LOCAL_WRITE_VERSION)?;
        self.signed_job.validate()?;
        let evidence = self
            .signed_job
            .manifest
            .native_evidence
            .as_ref()
            .ok_or(Invalid)?;
        let chain = values::binary(&self.chain, CHAIN_BYTES)?;
        require(crypto::digest(&chain) == binary::<32>(&evidence.chain_hash)?)?;
        let packet = values::binary(
            &self.packet,
            packet_limit(self.signed_job.manifest.entries.len())?,
        )?;
        self.signed_job.verify_packet(&packet, author_key)?;
        Ok(())
    }
    pub fn from_json(input: &[u8], author_key: &[u8]) -> Result<Self> {
        let out: Self = parse(input, LOCAL_WRITE_BYTES)?;
        out.validate(author_key)?;
        Ok(out)
    }
    pub fn to_json(&self, author_key: &[u8]) -> Result<Vec<u8>> {
        self.validate(author_key)?;
        json(self, LOCAL_WRITE_BYTES)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalStatus {
    pub version: u8,
    pub action: StatusAction,
    pub key: JobKey,
}
impl LocalStatus {
    fn validate(&self) -> Result<()> {
        require(self.version == 2)?;
        self.key.validate()
    }
    pub fn from_json(input: &[u8]) -> Result<Self> {
        let out: Self = parse(input, JSON_BYTES)?;
        out.validate()?;
        Ok(out)
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        json(self, JSON_BYTES)
    }
}
#[cfg(test)]
mod tests;
