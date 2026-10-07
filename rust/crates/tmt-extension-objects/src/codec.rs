//! Length-prefix decoding and the strict JSON admission every typed frame will
//! pass through. Admission checks bytes and JSON structure only: it knows nothing
//! about frame kinds, fields or authority.
//!
//! The typed frame slices are the callers of `strict_value`; until they land it is
//! reachable only from this crate's tests.
#![cfg_attr(not(test), allow(dead_code))]
use crate::{
    ErrorClass,
    limits::{FRAME_BYTES, JSON_DEPTH, MAX_SAFE_INTEGER, MIN_FRAME_BYTES},
};
use serde::{
    Deserialize, Deserializer,
    de::{self, DeserializeSeed, MapAccess, Visitor},
};
use serde_json::{Value, value::RawValue};
use std::{cell::Cell, collections::BTreeSet, fmt};

/// The body length a big-endian prefix announces. Nothing is allocated from it:
/// callers read exactly this many bytes only after it passed.
pub fn decode_length(prefix: [u8; 4]) -> Result<usize, ErrorClass> {
    let announced = u32::from_be_bytes(prefix);
    usize::try_from(announced)
        .ok()
        .filter(|length| (MIN_FRAME_BYTES..=FRAME_BYTES).contains(length))
        .ok_or(ErrorClass::Length)
}

/// Admit one JSON body and return its parsed value. A body is refused when it is
/// oversized, not UTF-8, not one complete JSON value, nested deeper than
/// [`JSON_DEPTH`], repeats a member name after decoding escapes, or contains a
/// number token that is not a canonical unsigned integer within the JSON-safe range.
pub(crate) fn strict_value(body: &[u8]) -> Result<Value, ErrorClass> {
    if body.len() > FRAME_BYTES {
        return Err(ErrorClass::Length);
    }
    let text = std::str::from_utf8(body).map_err(|_| ErrorClass::Utf8)?;
    let mut parser = serde_json::Deserializer::from_str(text);
    let root = <&RawValue>::deserialize(&mut parser).map_err(|_| ErrorClass::Syntax)?;
    parser.end().map_err(|_| ErrorClass::Trailing)?;
    admit(root, 1)?;
    serde_json::from_str(text).map_err(|_| ErrorClass::Syntax)
}

/// Walk serde's parsed structure. Scalars are checked from the original token text
/// that `RawValue` keeps: the pinned parser normalizes a literal such as `-0` before
/// any `Value` or `Number` sees it.
fn admit(value: &RawValue, depth: usize) -> Result<(), ErrorClass> {
    let token = value.get();
    match token.as_bytes().first() {
        Some(b'{') => {
            if depth > JSON_DEPTH {
                return Err(ErrorClass::Shape);
            }
            for (name, member) in members(token)? {
                // These are serde_json's private markers; a member with either name
                // must never be mistaken for the synthesized numeric/raw forms.
                if name.starts_with("$serde_json::private::") {
                    return Err(ErrorClass::Shape);
                }
                admit(member, depth + 1)?;
            }
            Ok(())
        }
        Some(b'[') => {
            if depth > JSON_DEPTH {
                return Err(ErrorClass::Shape);
            }
            let elements: Vec<&RawValue> =
                serde_json::from_str(token).map_err(|_| ErrorClass::Syntax)?;
            elements
                .into_iter()
                .try_for_each(|element| admit(element, depth + 1))
        }
        Some(b'"' | b't' | b'f' | b'n') => Ok(()),
        _ => unsigned_integer(token),
    }
}

/// `0` or a nonzero-leading digit string no larger than [`MAX_SAFE_INTEGER`].
/// serde has already accepted `token` as a JSON number, so a leading zero cannot occur.
fn unsigned_integer(token: &str) -> Result<(), ErrorClass> {
    if !token.bytes().all(|byte| byte.is_ascii_digit()) {
        // A sign (including `-0`), fraction or exponent: not an unsigned integer.
        return Err(ErrorClass::Shape);
    }
    match token.parse::<u64>() {
        Ok(number) if number <= MAX_SAFE_INTEGER => Ok(()),
        _ => Err(ErrorClass::Value),
    }
}

/// The members of one JSON object in order, with names decoded and checked unique.
fn members(object: &str) -> Result<Vec<(String, &RawValue)>, ErrorClass> {
    let repeated = Cell::new(false);
    let mut parser = serde_json::Deserializer::from_str(object);
    let entries = Members(&repeated).deserialize(&mut parser);
    match entries {
        Ok(entries) => Ok(entries),
        Err(_) if repeated.get() => Err(ErrorClass::Duplicate),
        Err(_) => Err(ErrorClass::Syntax),
    }
}
struct Members<'s>(&'s Cell<bool>);
impl<'de> DeserializeSeed<'de> for Members<'_> {
    type Value = Vec<(String, &'de RawValue)>;
    fn deserialize<D: Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        decoder.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Members<'_> {
    type Value = Vec<(String, &'de RawValue)>;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
        let mut names = BTreeSet::new();
        let mut entries = Vec::new();
        while let Some(name) = input.next_key::<String>()? {
            if !names.insert(name.clone()) {
                self.0.set(true);
                return Err(de::Error::custom("repeated member"));
            }
            entries.push((name, input.next_value::<&RawValue>()?));
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests;
