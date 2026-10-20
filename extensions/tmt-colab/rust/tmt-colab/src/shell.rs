//! Structured shell values and trial actions. No Yjs, storage, target lookup or authority.
//! The admitted writer supplies attribution; references never grant access to their targets.
use crate::closed::{Invalid, canonical, id, integer, keys, object, plain, require, text};
use crate::summary_lines;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use tmt_colab_model::values;

pub use crate::limits::{SHELL_BYTES as DOCUMENT_BYTES, SHELL_ITEMS as ITEM_LIMIT};

type Result<T> = crate::closed::Result<T>;

pub fn valid_anchor(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn optional_text(value: &Value) -> Result<()> {
    for (key, bytes) in [
        ("note", crate::limits::SHELL_NOTE_BYTES),
        ("audience", crate::limits::SHELL_AUDIENCE_BYTES),
    ] {
        if let Some(value) = value.get(key) {
            plain(value, bytes)?;
        }
    }
    Ok(())
}
pub fn validate_item(value: &Value) -> Result<()> {
    let kind = text(&value["kind"])?;
    let required: &[&str] = match kind {
        "page" => &["kind", "pageId"],
        "anchor" => &["kind", "pageId", "anchor"],
        "reading" => &["kind", "pageId", "readingId"],
        _ => return Err(Invalid),
    };
    keys(value, required, &["note", "audience"])?;
    id(&value["pageId"])?;
    if kind == "anchor" {
        require(valid_anchor(text(&value["anchor"])?))?;
    }
    if kind == "reading" {
        id(&value["readingId"])?;
    }
    optional_text(value)
}
/// Measures the admitted value, without retained Yjs history or transport framing.
pub fn validate(value: &Value) -> Result<()> {
    keys(value, &["version", "items", "order"], &["summary"])?;
    require(integer(&value["version"])? == 1)?;
    let items = object(&value["items"])?;
    let order = value["order"].as_array().ok_or(Invalid)?;
    require(items.len() <= ITEM_LIMIT && order.len() == items.len())?;
    for (key, item) in items {
        values::generated_id(key).map_err(|_| Invalid)?;
        validate_item(item)?;
    }
    let mut seen = BTreeSet::new();
    for entry in order {
        let key = id(entry)?;
        require(items.contains_key(key) && seen.insert(key))?;
    }
    if let Some(value) = value.get("summary") {
        summary_lines::validate(value)?;
    }
    require(canonical(value)?.len() <= DOCUMENT_BYTES)
}
pub fn empty() -> Value {
    json!({"version":1,"items":{},"order":[]})
}
fn placement(body: &Value, order: &[Value], item_id: &str) -> Result<usize> {
    let Some(before) = body.get("beforeId") else {
        return Ok(order.len());
    };
    let before = id(before)?;
    require(before != item_id)?;
    order
        .iter()
        .position(|v| v.as_str() == Some(before))
        .ok_or(Invalid)
}
/// Produces a detached result or refuses the whole action. Does not mutate the supplied base.
/// `attribution` comes from the write owner, never from the action body.
pub fn apply(base: &Value, action: &str, body: &Value, attribution: &Value) -> Result<Value> {
    validate(base)?;
    let mut next = base.clone();
    match action {
        "summary.set" => {
            next["summary"] = summary_lines::set(body, attribution)?;
        }
        "summary.clear" => {
            keys(body, &[], &[])?;
            next.as_object_mut().ok_or(Invalid)?.remove("summary");
        }
        "item.add" | "reading.suggest" | "item.remove" | "item.move" | "item.set" => {
            let (required, optional): (&[&str], &[&str]) = match action {
                "item.add" => (&["itemId", "item"], &["beforeId"]),
                "reading.suggest" => (
                    &["itemId", "pageId", "readingId"],
                    &["beforeId", "note", "audience"],
                ),
                "item.remove" => (&["itemId"], &[]),
                "item.move" => (&["itemId"], &["beforeId"]),
                _ => (&["itemId", "item"], &[]),
            };
            keys(body, required, optional)?;
            let key = id(&body["itemId"])?;
            let mut items = object(&base["items"])?.clone();
            let mut order = base["order"].as_array().ok_or(Invalid)?.clone();
            if matches!(action, "item.add" | "reading.suggest") {
                require(!items.contains_key(key))?;
                let item = if action == "reading.suggest" {
                    let mut item = json!({"kind":"reading","pageId":body["pageId"],"readingId":body["readingId"]});
                    for field in ["note", "audience"] {
                        if let Some(value) = body.get(field) {
                            item[field] = value.clone();
                        }
                    }
                    item
                } else {
                    body["item"].clone()
                };
                validate_item(&item)?;
                let index = placement(body, &order, key)?;
                items.insert(key.into(), item);
                order.insert(index, Value::String(key.into()));
            } else {
                require(items.contains_key(key))?;
                if action == "item.set" {
                    validate_item(&body["item"])?;
                    items.insert(key.into(), body["item"].clone());
                } else {
                    order.retain(|v| v.as_str() != Some(key));
                    if action == "item.remove" {
                        items.remove(key);
                    } else {
                        let index = placement(body, &order, key)?;
                        order.insert(index, Value::String(key.into()));
                    }
                }
            }
            next["items"] = Value::Object(items);
            next["order"] = Value::Array(order);
        }
        _ => return Err(Invalid),
    }
    validate(&next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_shell_vectors() {
        let vectors: Value =
            serde_json::from_str(include_str!("../../../contracts/vectors/shell-v1.json")).unwrap();
        for case in vectors["cases"].as_array().unwrap() {
            let base = case["document"].clone();
            let result = if let Some(action) = case.get("action") {
                apply(
                    &base,
                    action.as_str().unwrap(),
                    &case["body"],
                    &case["attribution"],
                )
            } else {
                validate(&base).map(|()| base.clone())
            };
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}: {result:?}",
                case["name"]
            );
            if let Some(expected) = case.get("expected") {
                assert_eq!(result.as_ref().unwrap(), expected, "{}", case["name"]);
            }
            if case["valid"] == true {
                assert_eq!(
                    String::from_utf8(canonical(result.as_ref().unwrap()).unwrap()).unwrap(),
                    case["canonical"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
            }
            assert_eq!(base, case["document"]);
        }
    }

    #[test]
    fn exact_document_and_item_bounds() {
        let mut value = empty();
        for n in 0..ITEM_LIMIT {
            let id = format!("10000000-0000-4000-8000-{n:012x}");
            value["items"][&id] =
                json!({"kind":"page","pageId":"20000000-0000-4000-8000-000000000001"});
            value["order"]
                .as_array_mut()
                .unwrap()
                .push(Value::String(id));
        }
        validate(&value).unwrap();
        let mut too_many = value.clone();
        let extra = "10000000-0000-4000-8000-ffffffffffff";
        too_many["items"][extra] = value["items"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone();
        too_many["order"].as_array_mut().unwrap().push(json!(extra));
        assert_eq!(validate(&too_many), Err(Invalid));
        let ids = value["order"].as_array().unwrap().clone();
        let mut last = String::new();
        for id in ids {
            let id = id.as_str().unwrap();
            value["items"][id]["note"] = json!("");
            let remaining = DOCUMENT_BYTES - canonical(&value).unwrap().len();
            value["items"][id]["note"] = json!("x".repeat(remaining.min(512)));
            if remaining <= 512 {
                last = id.into();
                break;
            }
        }
        assert_eq!(canonical(&value).unwrap().len(), DOCUMENT_BYTES);
        validate(&value).unwrap();
        value["items"][&last]["note"] = json!(format!(
            "{}x",
            value["items"][&last]["note"].as_str().unwrap()
        ));
        assert_eq!(validate(&value), Err(Invalid));
    }
}
