//! Narrow provider document codec. Only output-only createTime establishes ingress time.
use crate::{canonical, error::RemoteError, limits};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

pub(crate) struct Document {
    peer: String,
    id: String,
    created_ms: u64,
    bytes: Vec<u8>,
    cursor: Value,
}
impl Document {
    pub(crate) fn cursor(&self) -> &Value {
        &self.cursor
    }
    pub(crate) fn peer(&self) -> &str {
        &self.peer
    }
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn created_ms(&self) -> u64 {
        self.created_ms
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn read(
        value: &Value,
        root: &str,
        machine: &str,
        stream: &str,
    ) -> Result<Self, RemoteError> {
        let name = value["name"].as_str().ok_or_else(invalid)?;
        let suffix = name
            .strip_prefix(&format!("{root}/m/{machine}/{stream}/"))
            .ok_or_else(invalid)?;
        canonical::uuid(suffix).map_err(|_| invalid())?;
        let fields = value["fields"].as_object().ok_or_else(invalid)?;
        let peer = fields
            .get("peer")
            .and_then(Value::as_object)
            .filter(|field| field.len() == 1)
            .and_then(|field| field.get("stringValue"))
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        if fields.len() != 3 {
            return Err(invalid());
        }
        canonical::uuid(peer).map_err(|_| invalid())?;
        let created_ms = timestamp(value["createTime"].as_str().ok_or_else(invalid)?)?;
        if fields
            .get("createdAt")
            .and_then(Value::as_object)
            .is_none_or(|field| field.len() != 1)
        {
            return Err(invalid());
        }
        let indexed = timestamp(
            value["fields"]["createdAt"]["timestampValue"]
                .as_str()
                .ok_or_else(invalid)?,
        )?;
        if indexed.abs_diff(created_ms) > 1 {
            return Err(invalid());
        }
        let encoded = fields
            .get("carrier")
            .and_then(|v| v.as_object())
            .filter(|v| v.len() == 1)
            .and_then(|v| v.get("bytesValue"))
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        if encoded.len() > limits::CLOUD_CARRIER_BYTES.div_ceil(3) * 4 {
            return Err(invalid());
        }
        let bytes = STANDARD.decode(encoded).map_err(|_| invalid())?;
        if bytes.is_empty()
            || bytes.len() > limits::CLOUD_CARRIER_BYTES
            || STANDARD.encode(&bytes) != encoded
        {
            return Err(invalid());
        }
        Ok(Self {
            peer: peer.into(),
            id: suffix.into(),
            created_ms,
            bytes,
            cursor: json!({"values":[fields["createdAt"].clone(),{"referenceValue":name}],"before":false}),
        })
    }
}
pub(crate) fn create_carrier(name: &str, peer: &str, carrier: &[u8]) -> Result<Value, RemoteError> {
    if carrier.is_empty() || carrier.len() > limits::CLOUD_CARRIER_BYTES {
        return Err(invalid());
    }
    canonical::uuid(peer).map_err(|_| invalid())?;
    Ok(
        json!({"update":{"name":name,"fields":{"peer":{"stringValue":peer},"carrier":{"bytesValue":STANDARD.encode(carrier)}}},"updateTransforms":[{"fieldPath":"createdAt","setToServerValue":"REQUEST_TIME"}],"currentDocument":{"exists":false}}),
    )
}
fn invalid() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_DOCUMENT_INVALID",
        "Invalid cloud carrier document.",
    )
}
/// Firestore emits normalized UTC with 0, 3, 6 or 9 fractional digits. Floor to ms,
/// preserving the conservative 60 s fence; no sender-provided timestamp is substituted.
pub(crate) fn timestamp(s: &str) -> Result<u64, RemoteError> {
    let b = s.as_bytes();
    if !b.is_ascii()
        || ![20, 24, 27, 30].contains(&b.len())
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[b.len() - 1] != b'Z'
    {
        return Err(invalid());
    }
    let num = |a, z| {
        s.get(a..z)
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(invalid)
    };
    let (y, m, d, h, min, sec) = (
        num(0, 4)?,
        num(5, 7)?,
        num(8, 10)?,
        num(11, 13)?,
        num(14, 16)?,
        num(17, 19)?,
    );
    let days = crate::calendar::days_from_civil(y, m, d);
    let date = crate::calendar::civil_from_days(days);
    if y < 1970
        || date.year != y
        || date.month != m
        || date.day != d
        || h > 23
        || min > 59
        || sec > 59
    {
        return Err(invalid());
    }
    let fraction = if b.len() == 20 {
        0
    } else {
        if b[19] != b'.' || !b[20..b.len() - 1].iter().all(u8::is_ascii_digit) {
            return Err(invalid());
        }
        num(20, 23)?
    };
    Ok((((days * 24 + h) * 60 + min) * 60 + sec) as u64 * 1000 + fraction as u64)
}
