//! Colab's declared sealed-document mapping, without provider or effect authority.
//! These codecs neither enable cloud sync nor admit a signature, role or epoch.
pub mod originals;

use serde_json::{Map, Value, json};
use tmt_colab_model::{crypto, values};
use tmt_extension_objects::{Sha256Hex, sync};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    Spaces,
    Pages,
    Links,
    Members,
    Log,
    Checkpoints,
    Authority,
}
impl Resource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Spaces => "spaces",
            Self::Pages => "pages",
            Self::Links => "links",
            Self::Members => "members",
            Self::Log => "log",
            Self::Checkpoints => "checkpoints",
            Self::Authority => "authority",
        }
    }
    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Spaces => &["space", "owner"],
            Self::Pages => &["space", "page", "epoch", "state", "expiresAt"],
            Self::Links => &["space", "page", "link", "role", "token"],
            Self::Members => &["space", "page", "uid", "role", "link", "token"],
            Self::Log => &[
                "space",
                "page",
                "epoch",
                "stream",
                "seq",
                "uid",
                "hash",
                "envelope",
                "expiresAt",
            ],
            Self::Checkpoints => &[
                "space",
                "page",
                "epoch",
                "object",
                "index",
                "seq",
                "count",
                "uid",
                "bytes",
                "expiresAt",
            ],
            Self::Authority => &[
                "space",
                "page",
                "epoch",
                "kind",
                "subject",
                "seq",
                "count",
                "uid",
                "recipient",
                "bytes",
                "expiresAt",
            ],
        }
    }
}

/// REST Values stay explicit: a base64url sealed payload is a stringValue, not
/// a bytesValue or an alternate outer envelope. Timestamp syntax is checked by
/// the carrier's provider codec; this mapping never infers authority from expiry.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Field {
    #[serde(rename = "stringValue")]
    String(String),
    #[serde(rename = "integerValue")]
    Integer(String),
    #[serde(rename = "nullValue")]
    Null(()),
    #[serde(rename = "timestampValue")]
    Timestamp(String),
}
impl Field {
    /// RFC 3339 UTC in whole seconds: the shortest exact spelling the carrier accepts.
    pub fn timestamp(unix_ms: u64) -> Result<Self, Invalid> {
        values::time(unix_ms).map_err(|_| Invalid)?;
        let seconds = i64::try_from(unix_ms / 1000).map_err(|_| Invalid)?;
        let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
        // Days since 1970-01-01 to a civil date (proleptic Gregorian).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        Ok(Self::Timestamp(format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            rest / 3_600,
            rest % 3_600 / 60,
            rest % 60
        )))
    }
    pub fn number(value: u64) -> Result<Self, Invalid> {
        values::time(value).map_err(|_| Invalid)?;
        Ok(Self::Integer(value.to_string()))
    }
    fn text(&self) -> Result<&str, Invalid> {
        match self {
            Self::String(value) => Ok(value),
            _ => Err(Invalid),
        }
    }
    fn integer(&self) -> Result<u64, Invalid> {
        match self {
            Self::Integer(value) => {
                let n = values::decimal(value, true).map_err(|_| Invalid)?;
                values::time(n).map_err(|_| Invalid)?;
                Ok(n)
            }
            _ => Err(Invalid),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Invalid;
impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Invalid Colab sealed document mapping")
    }
}
impl std::error::Error for Invalid {}

pub type Fields = std::collections::BTreeMap<String, Field>;
fn text<'a>(fields: &'a Fields, name: &str) -> Result<&'a str, Invalid> {
    fields.get(name).ok_or(Invalid)?.text()
}
fn integer(fields: &Fields, name: &str) -> Result<u64, Invalid> {
    fields.get(name).ok_or(Invalid)?.integer()
}
fn positive(fields: &Fields, name: &str) -> Result<u64, Invalid> {
    let n = integer(fields, name)?;
    if n == 0 {
        return Err(Invalid);
    }
    Ok(n)
}
/// Compute the page-bound ID from the declared fields, never from a server path.
fn id(resource: Resource, fields: &Fields) -> Result<String, Invalid> {
    let space = text(fields, "space")?;
    values::space_id(space).map_err(|_| Invalid)?;
    if resource == Resource::Spaces {
        return Ok(space.into());
    }
    let page = text(fields, "page")?;
    values::generated_id(page).map_err(|_| Invalid)?;
    let base = format!("{space}_{page}");
    Ok(match resource {
        Resource::Spaces => unreachable!(),
        Resource::Pages => base,
        Resource::Links => {
            let link = text(fields, "link")?;
            values::generated_id(link).map_err(|_| Invalid)?;
            format!("{base}_{link}")
        }
        Resource::Members => {
            let uid = text(fields, "uid")?;
            wire_name(uid)?;
            format!("{base}_{uid}")
        }
        Resource::Log => {
            let epoch = positive(fields, "epoch")?;
            let stream = text(fields, "stream")?;
            values::generated_id(stream).map_err(|_| Invalid)?;
            format!("{base}_{epoch}_{stream}_{}", positive(fields, "seq")?)
        }
        Resource::Checkpoints => {
            let epoch = positive(fields, "epoch")?;
            let object = text(fields, "object")?;
            values::object_id(object).map_err(|_| Invalid)?;
            let index = integer(fields, "index")?;
            if positive(fields, "seq")? != index.checked_add(1).ok_or(Invalid)?
                || positive(fields, "count")? <= index
            {
                return Err(Invalid);
            }
            format!("{base}_{epoch}_{object}_{index}")
        }
        Resource::Authority => {
            let epoch = positive(fields, "epoch")?;
            let kind = text(fields, "kind")?;
            let recipient = text(fields, "recipient")?;
            match kind {
                "membership" | "chain" if recipient.is_empty() => {}
                "wrap" => wire_name(recipient)?,
                _ => return Err(Invalid),
            }
            let subject = text(fields, "subject")?;
            values::object_id(subject).map_err(|_| Invalid)?;
            let seq = positive(fields, "seq")?;
            let count = positive(fields, "count")?;
            if seq > count || count > if kind == "membership" { 11 } else { 1 } {
                return Err(Invalid);
            }
            format!("{base}_{epoch}_{kind}_{subject}_{seq}")
        }
    })
}
fn wire_name(value: &str) -> Result<(), Invalid> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
    {
        return Err(Invalid);
    }
    Ok(())
}
fn check(resource: Resource, fields: &Fields) -> Result<(), Invalid> {
    if fields.len() != resource.fields().len()
        || resource
            .fields()
            .iter()
            .any(|key| !fields.contains_key(*key))
    {
        return Err(Invalid);
    }
    for (key, value) in fields {
        match key.as_str() {
            "epoch" | "seq" | "index" | "count" => {
                value.integer()?;
            }
            "expiresAt" if matches!(value, Field::Timestamp(_)) => {}
            "expiresAt" => return Err(Invalid),
            "link" | "token"
                if resource == Resource::Members && matches!(value, Field::Null(())) => {}
            _ => {
                value.text()?;
            }
        }
    }
    if resource == Resource::Members {
        let null_link = matches!(fields.get("link"), Some(Field::Null(())));
        let null_token = matches!(fields.get("token"), Some(Field::Null(())));
        if null_link != null_token {
            return Err(Invalid);
        }
    }
    match resource {
        Resource::Log => {
            if values::binary(text(fields, "hash")?, 32)
                .map_err(|_| Invalid)?
                .len()
                != 32
            {
                return Err(Invalid);
            }
            let envelope = text(fields, "envelope")?;
            if !envelope.is_ascii() || envelope.len() > 354_304 {
                return Err(Invalid);
            }
        }
        Resource::Checkpoints | Resource::Authority => {
            let bytes = values::binary(text(fields, "bytes")?, 32_768).map_err(|_| Invalid)?;
            if bytes.is_empty() {
                return Err(Invalid);
            }
        }
        _ => {}
    }
    Ok(())
}
/// Canonical fields and checksum are transport integrity only. The caller must
/// separately verify the exact signed/encrypted payload before Store admission.
pub fn encode(resource: Resource, fields: Fields) -> Result<sync::Document, Invalid> {
    check(resource, &fields)?;
    let target = sync::Target {
        resource: resource.name().into(),
        id: id(resource, &fields)?,
    };
    let Value::Object(fields) = serde_json::to_value(fields).map_err(|_| Invalid)? else {
        return Err(Invalid);
    };
    let digest = Sha256Hex::from_bytes(crypto::digest(
        &serde_json::to_vec(&fields).map_err(|_| Invalid)?,
    ))
    .to_string();
    let document = sync::Document {
        target,
        fields,
        digest,
    };
    if serde_json::to_vec(&document.fields)
        .map_err(|_| Invalid)?
        .len()
        > tmt_extension_objects::limits::SYNC_DOCUMENT_BYTES
    {
        return Err(Invalid);
    }
    Ok(document)
}
pub fn decode(resource: Resource, document: &sync::Document) -> Result<Fields, Invalid> {
    if serde_json::to_vec(&document.fields)
        .map_err(|_| Invalid)?
        .len()
        > tmt_extension_objects::limits::SYNC_DOCUMENT_BYTES
    {
        return Err(Invalid);
    }
    if document.target.resource != resource.name() {
        return Err(Invalid);
    }
    let fields: Fields =
        serde_json::from_value(Value::Object(document.fields.clone())).map_err(|_| Invalid)?;
    let expected = encode(resource, fields.clone())?;
    if expected != *document {
        return Err(Invalid);
    }
    Ok(fields)
}
/// Page interests and discovery are equality-only scopes. Stream progress is a
/// separate verified cursor, never inserted into an interest.
pub fn page_scope(space: &str, page: &str, epoch: u64) -> Result<Map<String, Value>, Invalid> {
    values::space_id(space).map_err(|_| Invalid)?;
    values::generated_id(page).map_err(|_| Invalid)?;
    if epoch == 0 {
        return Err(Invalid);
    }
    values::time(epoch).map_err(|_| Invalid)?;
    Ok(Map::from_iter([
        ("space".into(), json!({"stringValue": space})),
        ("page".into(), json!({"stringValue": page})),
        ("epoch".into(), json!({"integerValue": epoch.to_string()})),
    ]))
}
/// The scope of one stream's log query: the page's scope plus the stream, as the declaration's
/// `log` query lists its scope fields.
pub fn stream_scope(
    space: &str,
    page: &str,
    epoch: u64,
    stream: &str,
) -> Result<Map<String, Value>, Invalid> {
    values::generated_id(stream).map_err(|_| Invalid)?;
    let mut scope = page_scope(space, page, epoch)?;
    scope.insert("stream".into(), json!({"stringValue": stream}));
    Ok(scope)
}
pub fn authority_scope(
    space: &str,
    page: &str,
    epoch: u64,
    kind: &str,
    recipient: &str,
) -> Result<Map<String, Value>, Invalid> {
    match kind {
        "membership" | "chain" if recipient.is_empty() => {}
        "wrap" => wire_name(recipient)?,
        _ => return Err(Invalid),
    }
    let mut scope = page_scope(space, page, epoch)?;
    scope.insert("kind".into(), json!({"stringValue":kind}));
    scope.insert("recipient".into(), json!({"stringValue":recipient}));
    Ok(scope)
}

#[cfg(test)]
mod tests;
