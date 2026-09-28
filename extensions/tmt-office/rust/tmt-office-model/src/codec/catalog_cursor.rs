//! Fixed binary cursor codec shared by independent local content catalogs.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use tmt_core::limits::MAX_JS_SAFE_INTEGER;

pub const ENCODED_MAX_BYTES: usize = 55;

pub fn encode(domain: u8, revision: u64, digest: &str) -> Option<String> {
    if revision > MAX_JS_SAFE_INTEGER {
        return None;
    }
    let hex = digest.strip_prefix("sha256:")?;
    if !tmt_core::content_digest::is_sha256(hex) {
        return None;
    }
    let mut bytes = Vec::with_capacity(41);
    bytes.push(domain);
    bytes.extend_from_slice(&revision.to_be_bytes());
    for pair in hex.as_bytes().chunks_exact(2) {
        bytes.push(u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?);
    }
    let encoded = URL_SAFE_NO_PAD.encode(bytes);
    debug_assert_eq!(encoded.len(), ENCODED_MAX_BYTES);
    Some(encoded)
}

pub fn decode(domain: u8, value: &str) -> Option<(u64, String)> {
    if value.contains('=') {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
    if bytes.len() != 41 || bytes[0] != domain {
        return None;
    }
    let revision = u64::from_be_bytes(bytes[1..9].try_into().ok()?);
    if revision > MAX_JS_SAFE_INTEGER {
        return None;
    }
    Some((
        revision,
        format!(
            "sha256:{}",
            bytes[9..]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
    ))
}
