//! Strict whole-number value decoding shared by browser-authored documents.

use serde::Deserialize;

// JSON numbers have value semantics: 1, 1.0 and 1e0 are the same browser integer.
// Decode individual fields so serde still rejects duplicate/unknown members.
pub fn whole_value<T: TryFrom<i64>>(number: serde_json::Number) -> Result<T, &'static str> {
    let invalid = "Expected a representable whole JSON number.";
    let value = number.as_f64().ok_or(invalid)?;
    let limit = tmt_core::limits::MAX_JS_SAFE_INTEGER as f64;
    if !value.is_finite() || value.fract() != 0.0 || !(-limit..=limit).contains(&value) {
        return Err(invalid);
    }
    T::try_from(value as i64).map_err(|_| invalid)
}

pub fn whole<'de, D: serde::Deserializer<'de>, T: TryFrom<i64>>(
    deserializer: D,
) -> Result<T, D::Error> {
    whole_value(serde_json::Number::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}
