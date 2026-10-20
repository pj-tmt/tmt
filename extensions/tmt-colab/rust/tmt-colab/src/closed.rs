//! Closed JSON value checks shared by the structured page values (`shell`, `work`). No Yjs,
//! storage, target lookup or authority: a value is valid or the whole action is refused.
use serde_json::{Map, Value};
use tmt_colab_model::values;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalid;
pub type Result<T> = std::result::Result<T, Invalid>;

pub fn require(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Invalid) }
}
pub fn object(value: &Value) -> Result<&Map<String, Value>> {
    value.as_object().ok_or(Invalid)
}
/// Exactly the required keys and some of the optional ones; an optional key is never `null`.
pub fn keys(value: &Value, required: &[&str], optional: &[&str]) -> Result<()> {
    let map = object(value)?;
    require(required.iter().all(|key| map.contains_key(*key)))?;
    require(map.iter().all(|(key, value)| {
        (required.contains(&key.as_str()) || optional.contains(&key.as_str())) && !value.is_null()
    }))
}
pub fn text(value: &Value) -> Result<&str> {
    value.as_str().ok_or(Invalid)
}
/// A canonical generated UUID.
pub fn id(value: &Value) -> Result<&str> {
    let value = text(value)?;
    values::generated_id(value).map_err(|_| Invalid)?;
    Ok(value)
}
/// Inert text of at most `bytes` UTF-8 bytes: no control characters.
pub fn plain(value: &Value, bytes: usize) -> Result<&str> {
    let value = text(value)?;
    require(value.len() <= bytes && !value.chars().any(char::is_control))?;
    Ok(value)
}
/// Like `plain`, but a line feed is allowed: a spec reads as paragraphs.
pub fn multiline(value: &Value, bytes: usize) -> Result<&str> {
    let value = text(value)?;
    require(value.len() <= bytes && !value.chars().any(|c| c.is_control() && c != '\n'))?;
    Ok(value)
}
/// Non-empty inert text of at most `bytes` UTF-8 bytes.
pub fn named(value: &Value, bytes: usize) -> Result<&str> {
    let value = plain(value, bytes)?;
    require(!value.is_empty())?;
    Ok(value)
}
/// An integral JSON number within the range browsers keep exact.
pub fn integer(value: &Value) -> Result<u64> {
    let number = value.as_f64().ok_or(Invalid)?;
    require((0.0..=9_007_199_254_740_991.0).contains(&number) && number.fract() == 0.0)?;
    Ok(number as u64)
}
/// Compact sorted ASCII field/UUID keys; integral JSON numbers normalize like browser JSON.
pub fn canonical(value: &Value) -> Result<Vec<u8>> {
    fn normalize(value: &mut Value) -> Result<()> {
        match value {
            Value::Object(map) => {
                map.sort_keys();
                for value in map.values_mut() {
                    normalize(value)?;
                }
            }
            Value::Array(array) => {
                for value in array {
                    normalize(value)?;
                }
            }
            Value::Number(_) => *value = Value::from(integer(value)?),
            _ => {}
        }
        Ok(())
    }
    let mut value = value.clone();
    normalize(&mut value)?;
    serde_json::to_vec(&value).map_err(|_| Invalid)
}
