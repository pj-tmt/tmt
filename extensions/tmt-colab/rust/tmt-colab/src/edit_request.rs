//! The typed `edit-request` record of the `intents` root: what a link holder who cannot seal
//! content asks the page's owner to save. It is data for the isolated projection, no dispatch:
//! the decoder checks syntax, size and digest; the owner's serve decides what it may do.
use serde::Deserialize;
use serde_json::Value;
use tmt_colab_model::{Invalid, Result, crypto, values};

/// The most source a request may carry. A request is one sealed own update of at most
/// `decoder::UPDATE_BYTES`; this leaves the Yjs and JSON overhead inside it.
pub const SOURCE_BYTES: usize = 128 * 1024;
/// The most anchored-hunk text (old and new together) a request may carry.
pub const HUNK_BYTES: usize = 64 * 1024;
/// The most hunks, as for a browser Save.
pub const HUNKS: usize = 64;

// The largest request still fits the update the decoder admits, with room for Yjs and JSON framing.
const _: () = assert!(SOURCE_BYTES + HUNK_BYTES + 8 * 1024 <= crate::decoder::UPDATE_BYTES);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditRequest {
    version: u8,
    #[allow(dead_code)]
    kind: String,
    operation_id: String,
    base_sha256: String,
    source_sha256: String,
    source: String,
    hunks: Option<Vec<Hunk>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Hunk {
    old_start: u32,
    old: String,
    new: String,
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(Invalid) }
}
/// Checks an `edit-request` record and ignores every other kind. The request is a whole proposed
/// source and the base it was edited from, with the editor's change as anchored hunks so that
/// the owner can merge it into a page that has moved since.
pub(crate) fn validate_record(root: &str, key: &str, value: &Value) -> Result<()> {
    if value.get("kind").and_then(Value::as_str) != Some("edit-request") {
        return Ok(());
    }
    // A request without hunks carries no `hunks` key, never a null one.
    require(value.get("hunks").is_none_or(|hunks| !hunks.is_null()))?;
    let v: EditRequest = serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
    require(root == "intents" && v.version == 1 && v.operation_id == key)?;
    values::generated_id(&v.operation_id)?;
    require(values::binary(&v.base_sha256, 32)?.len() == 32)?;
    require(
        v.source.len() <= SOURCE_BYTES
            && values::binary(&v.source_sha256, 32)? == crypto::digest(v.source.as_bytes()),
    )?;
    if let Some(hunks) = &v.hunks {
        require(!hunks.is_empty() && hunks.len() <= HUNKS)?;
        let mut text = 0usize;
        for hunk in hunks {
            require(hunk.old_start > 0 && !hunk.old.is_empty())?;
            text += hunk.old.len() + hunk.new.len();
        }
        require(text <= HUNK_BYTES)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Expands `{"fill": text, "count": n}` to the text repeated n times, anywhere in the value.
    pub(crate) fn expand(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if map.len() == 2
                    && let (Some(Value::String(text)), Some(Value::Number(count))) =
                        (map.get("fill"), map.get("count"))
                {
                    *value = Value::String(text.repeat(count.as_u64().unwrap() as usize));
                    return;
                }
                map.values_mut().for_each(expand);
            }
            Value::Array(items) => items.iter_mut().for_each(expand),
            _ => {}
        }
    }
    fn vectors() -> Value {
        serde_json::from_str(include_str!(
            "../../../contracts/vectors/edit-request-v1.json"
        ))
        .unwrap()
    }
    #[test]
    fn the_frozen_vectors_are_accepted_and_refused_as_listed() {
        let v = vectors();
        assert_eq!(v["sourceBytes"], SOURCE_BYTES);
        assert_eq!(v["hunkBytes"], HUNK_BYTES);
        assert_eq!(v["hunks"], HUNKS);
        for case in v["cases"].as_array().unwrap() {
            let mut record = case["record"].clone();
            expand(&mut record);
            let checked = validate_record(
                case["root"].as_str().unwrap(),
                case["key"].as_str().unwrap(),
                &record,
            );
            assert_eq!(
                checked.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
    #[test]
    fn a_record_of_another_kind_is_not_this_validators_to_judge() {
        let other = json!({"kind": "ask", "anything": [1, 2]});
        assert!(validate_record("intents", "k", &other).is_ok());
        assert!(validate_record("messages", "k", &json!({"no": "kind"})).is_ok());
    }
}
