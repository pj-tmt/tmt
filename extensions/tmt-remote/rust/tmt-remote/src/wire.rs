//! Strict wire admission preserves the exact decoded payload used by signatures.
use crate::canonical::{self, Envelope};
use serde::{
    Deserialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::{collections::BTreeSet, fmt};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MessageWire {
    version: u64,
    profile: String,
    kind: String,
    id: String,
    correlation_id: Value,
    machine_id: String,
    window_id: String,
    client_id: String,
    session_id: String,
    sequence: String,
    timestamp_ms: u64,
    origin: String,
    operation: String,
    payload: String,
    signature: String,
}
/// Syntax establishes no authority. All callers must verify the device and grant.
pub struct SignedMessage {
    wire: MessageWire,
    pub payload: Vec<u8>,
    pub input: Value,
    pub signature: Vec<u8>,
}
impl SignedMessage {
    pub fn decode(bytes: &[u8], payload_limit: usize) -> Option<Self> {
        if bytes.len() > 4 * payload_limit.div_ceil(3) + crate::limits::METADATA_BYTES {
            return None;
        }
        let wire: MessageWire = serde_json::from_slice(bytes).ok()?;
        if wire.version != 1
            || wire.profile != "local-v1"
            || !wire.correlation_id.is_null()
            || !matches!(wire.kind.as_str(), "request" | "control")
            || wire.payload.len() > 4 * payload_limit.div_ceil(3)
        {
            return None;
        }
        let payload = canonical::base64url_decode(&wire.payload).ok()?;
        if payload.len() > payload_limit {
            return None;
        }
        let input = strict_json(&payload)?;
        let signature = canonical::base64url_bytes(&wire.signature, 64).ok()?;
        let message = Self {
            wire,
            payload,
            input,
            signature,
        };
        canonical::envelope(&message.envelope()).ok()?;
        Some(message)
    }
    pub fn envelope(&self) -> Envelope<'_> {
        Envelope {
            kind: &self.wire.kind,
            id: &self.wire.id,
            correlation_id: None,
            machine_id: &self.wire.machine_id,
            window_id: &self.wire.window_id,
            client_id: &self.wire.client_id,
            session_id: &self.wire.session_id,
            sequence: &self.wire.sequence,
            timestamp_ms: self.wire.timestamp_ms,
            origin: &self.wire.origin,
            operation: &self.wire.operation,
            payload: &self.payload,
        }
    }
}
/// Reject duplicate members at every depth, using serde_json's bounded parser.
/// This is admission only; signatures always hash the original bytes.
pub fn strict_json(bytes: &[u8]) -> Option<Value> {
    // Validation does not reconstruct numbers: the workspace enables
    // arbitrary_precision, whose numeric deserializer has a private map form.
    serde_json::from_slice::<UniqueJson>(bytes).ok()?;
    serde_json::from_slice(bytes).ok()
}
struct UniqueJson;
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        decoder.deserialize_any(UniqueJsonVisitor)
    }
}
struct UniqueJsonVisitor;
impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJson;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON with unique object members")
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(UniqueJson)
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(UniqueJson)
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(UniqueJson)
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        if value.is_finite() {
            Ok(UniqueJson)
        } else {
            Err(E::custom("non-finite number"))
        }
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(UniqueJson)
    }
    fn visit_string<E: de::Error>(self, _: String) -> Result<Self::Value, E> {
        Ok(UniqueJson)
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueJson)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
        while input.next_element::<UniqueJson>()?.is_some() {}
        Ok(UniqueJson)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
        let mut keys = BTreeSet::new();
        while let Some(key) = input.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(de::Error::custom("duplicate member"));
            }
            input.next_value::<UniqueJson>()?;
        }
        Ok(UniqueJson)
    }
}
