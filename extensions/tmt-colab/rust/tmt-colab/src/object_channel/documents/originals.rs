//! Exact original bytes behind the declared transport projection. Syntax and
//! scope checks here never replace certificate, membership or epoch admission.
use super::{Field, Fields, Invalid, Resource, decode, encode, integer, text};
use tmt_colab_model::{object, values};
use tmt_extension_objects::sync;

/// A parsed original still needs the existing Colab signature/admission verifier.
pub struct Original {
    pub bytes: Vec<u8>,
    pub header: object::Header,
    pub hash: [u8; 32],
}
fn original(bytes: Vec<u8>, fields: &Fields, kind: &str) -> Result<Original, Invalid> {
    let envelope = object::Envelope::from_json(&bytes).map_err(|_| Invalid)?;
    let header = object::Header::decode(envelope.header()).map_err(|_| Invalid)?;
    let context = &header.context;
    if context.space != text(fields, "space")?
        || context.page != text(fields, "page")?
        || context.epoch != integer(fields, "epoch")?.to_string()
        || context.kind != kind
    {
        return Err(Invalid);
    }
    let hash = envelope.hash().map_err(|_| Invalid)?;
    Ok(Original {
        bytes,
        header,
        hash,
    })
}
/// Compare unsigned routing fields with the sealed header/hash before admission.
/// The returned bytes are exactly those received, including JSON whitespace.
pub fn log(document: &sync::Document) -> Result<Original, Invalid> {
    let fields = decode(Resource::Log, document)?;
    let result = original(
        text(&fields, "envelope")?.as_bytes().to_vec(),
        &fields,
        "update",
    )?;
    if result.header.context.author_device != text(&fields, "stream")?
        || result.header.context.stream_seq != integer(&fields, "seq")?.to_string()
        || values::binary(text(&fields, "hash")?, 32).map_err(|_| Invalid)? != result.hash
    {
        return Err(Invalid);
    }
    Ok(result)
}
/// Split one already sealed checkpoint. No new seal, identity or payload store.
pub fn checkpoint_documents(
    mut fields: Fields,
    bytes: &[u8],
) -> Result<Vec<sync::Document>, Invalid> {
    let parsed = original(bytes.to_vec(), &fields, "checkpoint")?;
    if parsed.header.object_id != text(&fields, "object")? || bytes.is_empty() {
        return Err(Invalid);
    }
    let count = bytes.len().div_ceil(32_768);
    if count > 513 {
        return Err(Invalid);
    }
    fields.insert("count".into(), Field::number(count as u64)?);
    bytes
        .chunks(32_768)
        .enumerate()
        .map(|(index, chunk)| {
            let mut row = fields.clone();
            row.insert("index".into(), Field::number(index as u64)?);
            row.insert("seq".into(), Field::number(index as u64 + 1)?);
            row.insert("bytes".into(), Field::String(values::encode_binary(chunk)));
            encode(Resource::Checkpoints, row)
        })
        .collect()
}
/// A complete ordered object query is required. Partial, duplicate, mixed or
/// reordered chunks cannot become a checkpoint or advance a verified position.
pub fn checkpoint(documents: &[sync::Document]) -> Result<Original, Invalid> {
    let first = decode(Resource::Checkpoints, documents.first().ok_or(Invalid)?)?;
    let count = integer(&first, "count")?;
    if count == 0 || count > 513 || count != documents.len() as u64 {
        return Err(Invalid);
    }
    let mut bytes = Vec::new();
    for (index, document) in documents.iter().enumerate() {
        let fields = decode(Resource::Checkpoints, document)?;
        for key in ["space", "page", "epoch", "object", "count", "uid"] {
            if fields.get(key) != first.get(key) {
                return Err(Invalid);
            }
        }
        if integer(&fields, "index")? != index as u64 {
            return Err(Invalid);
        }
        let chunk = values::binary(text(&fields, "bytes")?, 32_768).map_err(|_| Invalid)?;
        // Only the final chunk may be short; the producer's partition is fixed.
        if index + 1 < documents.len() && chunk.len() != 32_768 {
            return Err(Invalid);
        }
        bytes.extend_from_slice(&chunk);
    }
    let result = original(bytes, &first, "checkpoint")?;
    if result.header.object_id != text(&first, "object")? {
        return Err(Invalid);
    }
    Ok(result)
}
