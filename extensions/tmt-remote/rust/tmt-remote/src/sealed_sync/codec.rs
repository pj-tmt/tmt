//! Restricted REST Document.fields codec. No provider call, timestamp rounding or coercion.
use super::{Document, Query, SyncContext, SyncError};
use crate::{deploy_plan::sha256_hex, wire};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentAccess {
    Read,
    Publish,
}
const SAFE: i64 = (1i64 << 53) - 1;
fn invalid<T>() -> Result<T, SyncError> {
    Err(SyncError::Invalid)
}
fn integer(value: &str) -> bool {
    value
        .parse::<i64>()
        .is_ok_and(|n| (-SAFE..=SAFE).contains(&n) && n.to_string() == value)
}
fn timestamp(value: &str) -> bool {
    let b = value.as_bytes();
    if !b.is_ascii()
        || ![20, 24, 27].contains(&b.len())
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[b.len() - 1] != b'Z'
    {
        return false;
    }
    let number = |start, end| {
        value
            .get(start..end)
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<u32>().ok())
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0, 4),
        number(5, 7),
        number(8, 10),
        number(11, 13),
        number(14, 16),
        number(17, 19),
    ) else {
        return false;
    };
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + u32::from(year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)),
        _ => return false,
    };
    if year == 0 || day == 0 || day > days || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    if b.len() > 20
        && (b[19] != b'.'
            || !b[20..b.len() - 1].iter().all(u8::is_ascii_digit)
            || &value[b.len() - 4..b.len() - 1] == "000")
    {
        return false;
    }
    true
}
fn field(value: &Value) -> bool {
    let Some(row) = value.as_object() else {
        return false;
    };
    if row.len() != 1 {
        return false;
    }
    let (kind, value) = row.iter().next().expect("one value");
    match kind.as_str() {
        "stringValue" => value.is_string(),
        "integerValue" => value.as_str().is_some_and(integer),
        "booleanValue" => value.is_boolean(),
        "nullValue" => value.is_null(),
        "timestampValue" => value.as_str().is_some_and(timestamp),
        "bytesValue" => value.as_str().is_some_and(|s| {
            STANDARD
                .decode(s)
                .is_ok_and(|bytes| STANDARD.encode(bytes) == s)
        }),
        _ => false,
    }
}
fn text_is(value: Option<&Value>, expected: &str) -> bool {
    value.is_some_and(|v| {
        v.as_object().is_some_and(|o| {
            o.len() == 1 && o.get("stringValue").and_then(Value::as_str) == Some(expected)
        })
    })
}
/// Exact canonical bytes and declared fields; visible metadata is not a second outer envelope.
pub fn validate_document(
    context: &SyncContext,
    document: &Document,
    access: DocumentAccess,
) -> Result<(), SyncError> {
    context.document_path(&document.target)?;
    let resource = context
        .resources
        .iter()
        .find(|r| r.name == document.target.resource)
        .ok_or(SyncError::Invalid)?;
    let mapping = resource.sync.as_ref().ok_or(SyncError::Unavailable)?;
    if document.bytes.len() as u64 > resource.bounds.max_object_bytes
        || sha256_hex(&document.bytes) != document.digest
    {
        return invalid();
    }
    let row = validate_fields(&document.bytes)?;
    if row.len() != mapping.fields.len()
        || !mapping
            .fields
            .iter()
            .all(|key| row.get(key).is_some_and(field))
    {
        return invalid();
    }
    if let Some(query) = &mapping.query {
        let stream = row[&query.stream_field]
            .get("stringValue")
            .and_then(Value::as_str);
        let seq = row[&query.seq_field]
            .get("integerValue")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok());
        if !stream.is_some_and(|s| super::id(s, 128)) || !seq.is_some_and(|n| n > 0) {
            return invalid();
        }
    }
    if !text_is(row.get(&mapping.space_field), &context.space) {
        return Err(SyncError::Denied);
    }
    if access == DocumentAccess::Publish
        && mapping
            .uid_field
            .as_ref()
            .is_some_and(|key| !text_is(row.get(key), &context.identity.uid))
    {
        return Err(SyncError::Denied);
    }
    Ok(())
}
/// Exact scope filters and logical contiguous cursor, not an arbitrary REST structured query.
pub fn validate_query(context: &SyncContext, query: &Query) -> Result<(), SyncError> {
    let resource = context
        .resources
        .iter()
        .find(|r| r.name == query.resource)
        .ok_or(SyncError::Invalid)?;
    let mapping = resource.sync.as_ref().ok_or(SyncError::Unavailable)?;
    let spec = mapping.query.as_ref().ok_or(SyncError::Unavailable)?;
    if query.scope.len() != spec.scope_fields.len()
        || !spec
            .scope_fields
            .iter()
            .all(|key| query.scope.get(key).is_some_and(field))
        || !super::id(&query.stream, 128)
    {
        return invalid();
    }
    if !text_is(query.scope.get(&mapping.space_field), &context.space)
        || !text_is(query.scope.get(&spec.stream_field), &query.stream)
    {
        return Err(SyncError::Denied);
    }
    if query
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.stream != query.stream || cursor.seq > SAFE as u64)
    {
        return invalid();
    }
    Ok(())
}
/// Validate a standalone fields map for codec vectors without fabricating a provider response.
pub fn validate_fields(bytes: &[u8]) -> Result<Map<String, Value>, SyncError> {
    let mut value = wire::strict_json(bytes).ok_or(SyncError::Invalid)?;
    value.sort_all_objects();
    if serde_json::to_vec(&value).map_err(|_| SyncError::Invalid)? != bytes {
        return invalid();
    }
    let row = value.as_object().ok_or(SyncError::Invalid)?;
    if row.len() > 64
        || !row
            .iter()
            .all(|(key, value)| crate::declaration::sync_field_name(key) && field(value))
    {
        return invalid();
    }
    Ok(row.clone())
}
