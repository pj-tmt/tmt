//! Native sealing of one staged file into a document attachment (#2291). The browser seals in
//! its Writer; this is the same descriptor over the same object, authored by the root-local
//! writer under the page's current epoch, so every later read, export and publication check
//! treats it like any other attachment.
use crate::{Result, attachments, keyring::Keyring, page};
use std::time::Instant;
use tmt_colab_model::{
    attachment::{Descriptor, Source},
    crypto, object, values,
};

/// One sealed original: the exact ciphertext to upload, its descriptor and the page revision
/// the upload is fenced against.
pub(crate) struct Sealed {
    pub ciphertext: Vec<u8>,
    pub descriptor: Descriptor,
    pub base: String,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
/// Seal `plaintext` as a document attachment of `page`, bound to the page source as it is now.
/// An archived, deleted or read-only page refuses before anything is sealed.
pub(crate) fn seal_document(
    source: &mut page::save::Source,
    page: &str,
    plaintext: &[u8],
    filename: &str,
    media_type: &str,
    deadline: Instant,
) -> Result<Sealed> {
    let key: &Keyring = &source.keyring;
    let snapshot = page::snapshot(&source.store, key, page, true)?;
    let view = snapshot.materialize_until(key, page, &mut source.decoder, deadline)?;
    let (author, _, _) = key.local_writer()?;
    let epoch = snapshot.epoch.to_string();
    let membership = snapshot.authority.head.revision.to_string();
    let envelope = key.seal_content(
        &object::Context {
            space: key.space_id.clone(),
            page: page.into(),
            epoch: epoch.clone(),
            kind: "asset".into(),
            namespace: "content".into(),
            author_device: author.clone(),
            membership_revision: membership.clone(),
            stream_seq: "0".into(),
            prev_hash: [0; 32],
        },
        &snapshot.secret,
        plaintext,
    )?;
    let ciphertext = envelope.to_json()?;
    let descriptor = Descriptor {
        version: 1,
        attachment_id: page::fresh_id()?,
        space: key.space_id.clone(),
        page: page.into(),
        epoch,
        namespace: "content".into(),
        object_id: object::Header::decode(envelope.header())?.object_id,
        author_device: author,
        membership_revision: membership,
        source: Source::Document {
            source_digest: hex(&crypto::digest(view.source.as_bytes())),
        },
        envelope_hash: hex(&envelope.hash()?),
        signature: values::encode_binary(envelope.signature()),
        payload_sha256: hex(&crypto::digest(&ciphertext)),
        payload_bytes: ciphertext.len().to_string(),
        plaintext_bytes: plaintext.len().to_string(),
        filename: filename.into(),
        media_type: media_type.into(),
    };
    descriptor.validate()?;
    let base = attachments::current_base(key, &descriptor, &snapshot)?;
    Ok(Sealed {
        ciphertext,
        descriptor,
        base,
    })
}
