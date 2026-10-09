//! Strict transport DTOs; cryptographic envelope syntax stays in the model.
use super::Code;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tmt_colab_model::{bounded::List, values};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SyncScope {
    pub space: String,
    pub page: String,
    pub epoch: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SyncCursor {
    pub stream_id: String,
    pub namespace: String,
    pub seq: String,
    pub envelope_hash: String,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Payload {
    Inline(String),
    Reference(Reference),
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Reference {
    pub object_id: String,
}
// Repeat common fields in each typed variant: flatten/Value would lose duplicate-field rejection.
macro_rules! frames {
    ($($name:ident { $($field:ident: $ty:ty),* $(,)? }),* $(,)?) => {
        #[derive(Debug, Deserialize)]
        #[serde(tag = "type", rename_all = "lowercase", rename_all_fields = "camelCase", deny_unknown_fields)]
        pub enum Frame {
            $($name { version: u8, space: String, page: String, epoch: String, $($field: $ty),* }),*
        }
        impl Frame {
            pub fn scope(&self) -> Result<SyncScope, Code> {
                let (version, space, page, epoch) = match self {
                    $(Self::$name { version, space, page, epoch, .. } => (version, space, page, epoch)),*
                };
                if *version != 1 { return Err(Code::Invalid); }
                values::space_id(space)?;
                values::generated_id(page)?;
                values::decimal(epoch, false)?;
                Ok(SyncScope { space: space.clone(), page: page.clone(), epoch: epoch.clone() })
            }
        }
    };
}
frames! {
    Hello { device: String, membership_revision: String, cursors: List<SyncCursor, 256> },
    Subscribe { cursors: List<SyncCursor, 256> },
    Append { stream_id: String, seq: String, envelope_hash: String, envelope: Payload },
    Chunk { object_id: String, envelope_hash: String, index: usize, count: usize, bytes: String },
    Save { operation_id: String, base_sha256: String, source_sha256: String, source: Payload },
    SaveStatus { operation_id: String },
    Object { request_id: String, request: Box<crate::object_channel::ObjectRequest> },
    Ack { cursors: List<SyncCursor, 256> },
    Awareness { device: String, data: String },
}
pub fn cursors(list: &List<SyncCursor, 256>) -> Result<(), Code> {
    let mut seen = BTreeSet::new();
    for c in list.as_slice() {
        values::generated_id(&c.stream_id)?;
        values::namespace(&c.namespace)?;
        let n = values::decimal(&c.seq, true)?;
        let hash = hash(&c.envelope_hash)?;
        if (n == 0 && hash != [0; 32]) || !seen.insert((&c.stream_id, &c.namespace)) {
            return Err(Code::Invalid);
        }
    }
    Ok(())
}
pub fn hash(text: &str) -> Result<[u8; 32], Code> {
    values::binary(text, 32)?
        .try_into()
        .map_err(|_| Code::Invalid)
}
pub fn output(scope: &SyncScope, kind: &str, fields: serde_json::Value) -> Result<String, Code> {
    let mut out = serde_json::json!({"version":1,"type":kind,"space":scope.space,"page":scope.page,"epoch":scope.epoch});
    out.as_object_mut()
        .ok_or(Code::Invalid)?
        .extend(fields.as_object().ok_or(Code::Invalid)?.clone());
    let text = serde_json::to_string(&out).map_err(|_| Code::Invalid)?;
    if text.len() > crate::limits::WS_FRAME_BYTES {
        return Err(Code::Capacity);
    }
    Ok(text)
}
