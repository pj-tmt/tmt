//! Pure decoded-value framing. These builders do not admit wire input or grant authority.
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidBytes;
impl fmt::Display for InvalidBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid local-v1 bytes")
    }
}
impl std::error::Error for InvalidBytes {}
pub type Result<T> = std::result::Result<T, InvalidBytes>;

pub(crate) fn require(valid: bool) -> Result<()> {
    if valid { Ok(()) } else { Err(InvalidBytes) }
}
pub(crate) fn lp(output: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let length = u32::try_from(value.len()).map_err(|_| InvalidBytes)?;
    let additional = value.len().checked_add(4).ok_or(InvalidBytes)?;
    output.try_reserve(additional).map_err(|_| InvalidBytes)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}
pub(crate) fn framed(fields: &[&[u8]]) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    for field in fields {
        lp(&mut result, field)?;
    }
    Ok(result)
}
fn core_id(value: &str) -> Result<()> {
    require(
        value.len() == 36
            && value.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
                }
            })
            && value.bytes().any(|b| b != b'0' && b != b'-'),
    )
}
fn uuid(value: &str) -> Result<()> {
    core_id(value)?;
    require(
        value.as_bytes()[14] == b'4' && matches!(value.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
    )
}
fn origin(value: &str) -> Result<()> {
    require(
        value == "cli"
            || value
                .strip_prefix("chrome-extension://")
                .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))),
    )
}

/// Strings are valid UTF-8 by construction and are never normalized. Payload is exact bytes.
pub struct Envelope<'a> {
    pub kind: &'a str,
    pub id: &'a str,
    pub correlation_id: Option<&'a str>,
    pub machine_id: &'a str,
    pub window_id: &'a str,
    pub client_id: &'a str,
    pub session_id: &'a str,
    pub sequence: &'a str,
    pub timestamp_ms: u64,
    pub origin: &'a str,
    pub operation: &'a str,
    pub payload: &'a [u8],
}
/// Fixed version 1 / local-v1; no algorithm or profile negotiation.
pub fn envelope(value: &Envelope<'_>) -> Result<Vec<u8>> {
    require(matches!(value.kind, "request" | "response" | "control"))?;
    for id in [value.id, value.machine_id, value.window_id, value.client_id] {
        uuid(id)?;
    }
    if value.kind == "response" {
        uuid(value.correlation_id.ok_or(InvalidBytes)?)?;
    } else {
        require(value.correlation_id.is_none())?;
    }
    let sequence: u64 = value.sequence.parse().map_err(|_| InvalidBytes)?;
    require(sequence.to_string() == value.sequence)?;
    if value.kind == "control" {
        require(matches!(
            value.operation,
            "session.open" | "subscribe" | "ack"
        ))?;
    }
    if value.kind == "control" && value.operation == "session.open" {
        require(value.session_id == "new" && sequence == 0)?;
    } else {
        uuid(value.session_id)?;
        require(sequence > 0)?;
    }
    require(value.timestamp_ms <= 9_007_199_254_740_991 && !value.operation.is_empty())?;
    origin(value.origin)?;
    let timestamp = value.timestamp_ms.to_string();
    let digest = Sha256::digest(value.payload);
    framed(&[
        b"tmt-message-v1",
        b"1",
        b"local-v1",
        value.kind.as_bytes(),
        value.id.as_bytes(),
        value.correlation_id.unwrap_or("").as_bytes(),
        value.machine_id.as_bytes(),
        value.window_id.as_bytes(),
        value.client_id.as_bytes(),
        value.session_id.as_bytes(),
        value.sequence.as_bytes(),
        timestamp.as_bytes(),
        value.origin.as_bytes(),
        value.operation.as_bytes(),
        &digest,
    ])
}

pub struct Enrollment<'a> {
    pub machine_id: &'a str,
    pub window_id: &'a str,
    pub offer_id: &'a str,
    pub server_challenge: &'a [u8; 16],
    pub client_nonce: &'a [u8; 16],
    pub kind: &'a str,
    pub origin: &'a str,
    pub name: &'a str,
    pub public_key: &'a [u8; 32],
    pub agent_ids: &'a [&'a str],
    pub scopes: &'a [&'a str],
}
fn append_sorted_list(
    output: &mut Vec<u8>,
    values: &[&str],
    validate: fn(&str) -> Result<()>,
) -> Result<()> {
    let count = u32::try_from(values.len()).map_err(|_| InvalidBytes)?;
    output.extend_from_slice(&count.to_be_bytes());
    let mut previous = None;
    for value in values {
        validate(value)?;
        require(previous.is_none_or(|p| p < *value))?;
        lp(output, value.as_bytes())?;
        previous = Some(*value);
    }
    Ok(())
}
/// Frames a candidate only: key validity and proof checks are separate pure operations.
pub fn enrollment(value: &Enrollment<'_>) -> Result<Vec<u8>> {
    for id in [value.machine_id, value.window_id, value.offer_id] {
        uuid(id)?;
    }
    require(matches!(value.kind, "addon" | "cli"))?;
    origin(value.origin)?;
    require((value.kind == "cli") == (value.origin == "cli"))?;
    require(
        !value.name.is_empty()
            && value.name.len() <= 64
            && value
                .name
                .chars()
                .any(|c| !c.is_whitespace() && c != '\u{feff}')
            && !value.name.chars().any(char::is_control)
            && value.agent_ids.len() <= 256,
    )?;
    let mut result = framed(&[
        b"tmt-local-pair-v1",
        b"local-v1",
        value.machine_id.as_bytes(),
        value.window_id.as_bytes(),
        value.offer_id.as_bytes(),
        value.server_challenge,
        value.client_nonce,
        value.kind.as_bytes(),
        value.origin.as_bytes(),
        value.name.as_bytes(),
        value.public_key,
    ])?;
    append_sorted_list(&mut result, value.agent_ids, core_id)?;
    append_sorted_list(&mut result, value.scopes, |s| {
        require(matches!(
            s,
            "agents.read" | "results.own" | "status.read" | "talk.hold"
        ))
    })?;
    lp(&mut result, b"hold")?;
    Ok(result)
}
pub fn possession(enrollment: &[u8], mac: &[u8; 32]) -> Result<Vec<u8>> {
    framed(&[b"tmt-local-pair-possession-v1", enrollment, mac])
}
