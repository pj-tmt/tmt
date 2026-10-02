//! Value syntax from colab-v1; no identity or role lookup.
use crate::{Invalid, Result, require};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

pub fn space_id(value: &str) -> Result<()> {
    require(
        value.len() == 32
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)),
    )
}
pub fn core_id(value: &str) -> Result<()> {
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
pub fn generated_id(value: &str) -> Result<()> {
    core_id(value)?;
    require(
        value.as_bytes()[14] == b'4' && matches!(value.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
    )
}
pub fn object_id(value: &str) -> Result<()> {
    require(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
    )
}
pub fn decimal(value: &str, zero: bool) -> Result<u64> {
    require(!value.is_empty() && value.len() <= 20 && value.bytes().all(|b| b.is_ascii_digit()))?;
    let n: u64 = value.parse().map_err(|_| Invalid)?;
    require(n.to_string() == value && (zero || n > 0))?;
    Ok(n)
}
pub fn time(value: u64) -> Result<()> {
    require(value <= 9_007_199_254_740_991)
}
pub fn namespace(value: &str) -> Result<()> {
    require(matches!(value, "content" | "own"))
}
pub fn binary(value: &str, max: usize) -> Result<Vec<u8>> {
    require(value.len() <= max.div_ceil(3).checked_mul(4).ok_or(Invalid)?)?;
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| Invalid)?;
    require(bytes.len() <= max && URL_SAFE_NO_PAD.encode(&bytes) == value)?;
    Ok(bytes)
}
pub fn encode_binary(value: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(value)
}
