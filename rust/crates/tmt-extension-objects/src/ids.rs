//! Canonical identifiers and fixed-width binary values. Every constructor accepts
//! exactly one spelling of a value, so a parsed value re-encodes to the same text.
use crate::error::ErrorClass;
use base64::{
    Engine,
    alphabet::URL_SAFE,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use std::fmt;

/// Unpadded RFC 4648 base64url. Decoding refuses padding, other alphabets and
/// nonzero unused bits.
const BASE64URL: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(false),
);
/// Room the engine needs to decode the 43 characters of 32 bytes, with no spare for more.
const BYTES32_BUFFER: usize = 33;

fn lower_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
/// Decode `text` as lowercase hex into `out`; the text must be exactly `2 * out.len()` digits.
fn hex_into(text: &str, out: &mut [u8]) -> Result<(), ErrorClass> {
    let bytes = text.as_bytes();
    if bytes.len() != out.len() * 2 {
        return Err(ErrorClass::Value);
    }
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        let (Some(high), Some(low)) = (lower_hex(pair[0]), lower_hex(pair[1])) else {
            return Err(ErrorClass::Value);
        };
        *slot = (high << 4) | low;
    }
    Ok(())
}
fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    bytes.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
}

/// A lowercase hyphenated UUID of version 4 and RFC 4122 variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Uuid4([u8; 16]);
impl Uuid4 {
    pub fn parse(text: &str) -> Result<Self, ErrorClass> {
        let bytes = text.as_bytes();
        let hyphens = [8, 13, 18, 23];
        if bytes.len() != 36 || hyphens.iter().any(|&at| bytes[at] != b'-') {
            return Err(ErrorClass::Value);
        }
        // Version nibble 4, variant nibble 8, 9, a or b.
        if bytes[14] != b'4' || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b') {
            return Err(ErrorClass::Value);
        }
        let digits: String = text.chars().filter(|&c| c != '-').collect();
        let mut value = [0; 16];
        hex_into(&digits, &mut value)?;
        Ok(Self(value))
    }
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}
impl fmt::Display for Uuid4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// A positive correlation counter, spelled as a canonical decimal string that
/// spans the whole `u64` range (it is never a JSON number).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Counter(u64);
impl Counter {
    pub fn new(value: u64) -> Result<Self, ErrorClass> {
        if value == 0 {
            return Err(ErrorClass::Value);
        }
        Ok(Self(value))
    }
    /// `^[1-9][0-9]{0,19}$` that fits in a `u64`.
    pub fn parse(text: &str) -> Result<Self, ErrorClass> {
        let bytes = text.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 20
            || bytes[0] == b'0'
            || !bytes.iter().all(u8::is_ascii_digit)
        {
            return Err(ErrorClass::Value);
        }
        text.parse()
            .map_err(|_| ErrorClass::Value)
            .and_then(Self::new)
    }
    pub fn get(self) -> u64 {
        self.0
    }
}
impl fmt::Display for Counter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Exactly 32 opaque bytes, spelled as 43 characters of unpadded base64url.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bytes32([u8; 32]);
impl Bytes32 {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn parse(text: &str) -> Result<Self, ErrorClass> {
        // A fixed buffer bounds the work: longer input cannot be decoded into it.
        let mut bytes = [0u8; BYTES32_BUFFER];
        let width = BASE64URL
            .decode_slice(text, &mut bytes)
            .map_err(|_| ErrorClass::Value)?;
        if width != 32 {
            return Err(ErrorClass::Value);
        }
        let mut exact = [0u8; 32];
        exact.copy_from_slice(&bytes[..32]);
        Ok(Self(exact))
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl fmt::Display for Bytes32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&BASE64URL.encode(self.0))
    }
}

/// A SHA-256 digest, spelled as 64 lowercase hexadecimal digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sha256Hex([u8; 32]);
impl Sha256Hex {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn parse(text: &str) -> Result<Self, ErrorClass> {
        let mut value = [0; 32];
        hex_into(text, &mut value)?;
        Ok(Self(value))
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(f, &self.0)
    }
}

#[cfg(test)]
mod tests;
