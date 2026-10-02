//! Pure decoded-value framing. These builders do not admit wire input or grant authority.
use base64::{
    Engine,
    alphabet::URL_SAFE,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
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
/// The exact loopback door origin `http://127.0.0.1:<port>`, canonical decimal port.
fn door_origin(value: &str) -> bool {
    value.strip_prefix("http://127.0.0.1:").is_some_and(|port| {
        port.parse::<u16>()
            .is_ok_and(|n| n != 0 && n.to_string() == port)
    })
}
fn addon_origin(value: &str) -> bool {
    value
        .strip_prefix("chrome-extension://")
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b)))
}
fn origin(value: &str) -> Result<()> {
    require(value == "cli" || addon_origin(value) || door_origin(value))
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

/// Device enrollment candidate (`tmt-device-pair-v1`). The device proposes no
/// agents, scopes, mode or expiry; the owner's confirmation sets them.
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
}
/// Frames a candidate only: key validity and proof checks are separate pure operations.
pub fn enrollment(value: &Enrollment<'_>) -> Result<Vec<u8>> {
    for id in [value.machine_id, value.window_id, value.offer_id] {
        uuid(id)?;
    }
    origin(value.origin)?;
    require(match value.kind {
        "addon" => addon_origin(value.origin),
        "browser" => door_origin(value.origin),
        "cli" => value.origin == "cli",
        _ => false,
    })?;
    require(
        !value.name.is_empty()
            && value.name.len() <= 64
            && value
                .name
                .chars()
                .any(|c| !c.is_whitespace() && c != '\u{feff}')
            && !value.name.chars().any(char::is_control),
    )?;
    framed(&[
        b"tmt-device-pair-v1",
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
    ])
}
pub fn possession(enrollment: &[u8], mac: &[u8; 32]) -> Result<Vec<u8>> {
    framed(&[b"tmt-device-pair-possession-v1", enrollment, mac])
}

/// An extension name as mounted under `/x/<extension>/`: a lowercase ASCII
/// letter, then lowercase letters, digits or hyphens, at most 32 bytes.
pub fn extension_name(value: &str) -> bool {
    value.len() <= 32
        && value.starts_with(|c: char| c.is_ascii_lowercase())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
/// Extension key certificate (`tmt-ext-cert-v1`): a device key certifies an
/// extension-generated key. It binds the key to the device and grants no
/// remote authority by itself.
pub struct ExtCert<'a> {
    pub extension: &'a str,
    /// `sign` or `enc`.
    pub purpose: &'a str,
    pub public_key: &'a [u8; 32],
    pub issued_at_ms: u64,
}
pub fn ext_cert(value: &ExtCert<'_>) -> Result<Vec<u8>> {
    require(
        extension_name(value.extension)
            && matches!(value.purpose, "sign" | "enc")
            && value.issued_at_ms <= 9_007_199_254_740_991,
    )?;
    framed(&[
        b"tmt-ext-cert-v1",
        value.extension.as_bytes(),
        value.purpose.as_bytes(),
        value.public_key,
        value.issued_at_ms.to_string().as_bytes(),
    ])
}

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
/// The 16-byte pairing code as 26 uppercase RFC 4648 base32 characters,
/// grouped by four with hyphens for copy/paste.
pub fn pairing_code_text(code: &[u8; 16]) -> String {
    let mut text = String::with_capacity(32);
    let (mut buffer, mut bits) = (0u32, 0);
    for byte in code {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            text.push(BASE32[((buffer >> bits) & 31) as usize] as char);
        }
    }
    text.push(BASE32[((buffer << (5 - bits)) & 31) as usize] as char);
    text.as_bytes()
        .chunks(4)
        .map(|group| std::str::from_utf8(group).expect("ASCII"))
        .collect::<Vec<_>>()
        .join("-")
}
/// Decode after removing ASCII spaces and hyphens only; any other character,
/// a wrong length or nonzero unused bits refuses.
pub fn pairing_code(text: &str) -> Result<[u8; 16]> {
    let symbols: Vec<u8> = text.bytes().filter(|b| !matches!(b, b' ' | b'-')).collect();
    require(symbols.len() == 26)?;
    let mut code = [0; 16];
    let (mut buffer, mut bits, mut index) = (0u32, 0, 0);
    for symbol in symbols {
        let value = BASE32
            .iter()
            .position(|c| *c == symbol)
            .ok_or(InvalidBytes)?;
        buffer = (buffer << 5) | value as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            code[index] = (buffer >> bits) as u8;
            index += 1;
        }
    }
    // 130 bits carry 128: the last two must be zero.
    require(index == 16 && buffer & ((1 << bits) - 1) == 0)?;
    Ok(code)
}

/// Pinned BIP-39 English list: bitcoin/bips ce1862ac bip-0039/english.txt,
/// SHA-256 2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda.
const WORDLIST: &str = include_str!("../assets/bip39-english.txt");
/// Four 11-bit indexes from the first 44 bits of the key fingerprint.
pub fn fingerprint_indexes(public_key: &[u8; 32]) -> Result<[u16; 4]> {
    let digest = Sha256::digest(framed(&[b"tmt-local-key-fingerprint-v1", public_key])?);
    let bits = u64::from_be_bytes(digest[..8].try_into().expect("eight bytes")) >> 20;
    Ok([33, 22, 11, 0].map(|shift| ((bits >> shift) & 0x7ff) as u16))
}
/// Comparison text shown on both sides of pairing; not a recovery mnemonic.
pub fn fingerprint_words(public_key: &[u8; 32]) -> Result<[&'static str; 4]> {
    let words: Vec<&'static str> = WORDLIST.lines().collect();
    require(words.len() == 2048)?;
    Ok(fingerprint_indexes(public_key)?.map(|i| words[usize::from(i)]))
}

/// Unpadded RFC 4648 base64url. Decoding refuses padding, other alphabets and
/// nonzero unused bits, so every value has exactly one accepted spelling.
const BASE64URL: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(false),
);
pub fn base64url(bytes: &[u8]) -> String {
    BASE64URL.encode(bytes)
}
/// Decode strict base64url of any length.
pub fn base64url_decode(text: &str) -> Result<Vec<u8>> {
    BASE64URL.decode(text).map_err(|_| InvalidBytes)
}
/// Decode exactly `length` bytes of strict base64url.
pub fn base64url_bytes(text: &str, length: usize) -> Result<Vec<u8>> {
    let bytes = base64url_decode(text)?;
    require(bytes.len() == length)?;
    Ok(bytes)
}
