use serde_json::{Value, json};
use tmt_core::endpoint::{BindingMarker, valid_process_id};

pub(super) fn decode(text: &str) -> Value {
    crate::json_document::parse(text)
        .ok()
        .filter(|value| {
            value.is_object() && value.get("version").and_then(Value::as_u64) == Some(1)
        })
        .unwrap_or_else(|| json!({"version": 1}))
}

pub(super) fn marker(document: &Value) -> Option<BindingMarker> {
    let value = document.get("globalIdentity")?.as_object()?;
    let text = |name| {
        value
            .get(name)?
            .as_str()
            // Decode shape only. The shared core name validator owns Unicode
            // whitespace/normalization when evaluating a binding marker.
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let pane_pid = value
        .get("panePid")?
        .as_u64()
        .filter(|pid| valid_process_id(*pid))?;
    Some(BindingMarker {
        name: text("name")?,
        canonical_name: text("canonicalName")?,
        identity_id: text("identityId")?,
        binding_id: text("bindingId")?,
        server_id: text("serverId")?,
        pane_pid,
    })
}

/// A pre-UUID identity marker has names but no binding ID. It is not routing authority.
pub(super) fn is_legacy_marker(document: &Value) -> bool {
    document.get("version").and_then(Value::as_u64) == Some(1)
        && document
            .get("globalIdentity")
            .and_then(Value::as_object)
            .is_some_and(|marker| {
                !marker.contains_key("bindingId")
                    && ["name", "canonicalName"].iter().all(|key| {
                        marker
                            .get(*key)
                            .and_then(Value::as_str)
                            .is_some_and(|value| !value.is_empty())
                    })
            })
}

pub(super) fn replace(document: &mut Value, marker: &BindingMarker) {
    document["globalIdentity"] = json!({
        "name": marker.name,
        "canonicalName": marker.canonical_name,
        "identityId": marker.identity_id,
        "bindingId": marker.binding_id,
        "serverId": marker.server_id,
        "panePid": marker.pane_pid,
    });
}

pub(super) fn clear(document: &mut Value, binding_id: Option<&str>) -> bool {
    let Some(marker) = document.get("globalIdentity") else {
        return false;
    };
    // Preserve the reference clear contract for malformed false-like markers;
    // arrays/objects still count as present, but never match a supplied ID.
    let present = match marker {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        _ => true,
    };
    if !present
        || binding_id.is_some_and(|id| marker.get("bindingId").and_then(Value::as_str) != Some(id))
    {
        return false;
    }
    document
        .as_object_mut()
        .expect("decoded metadata object")
        .remove("globalIdentity");
    true
}

pub(super) fn has_fields(document: &Value) -> bool {
    document
        .as_object()
        .expect("decoded metadata object")
        .keys()
        .any(|name| name != "version")
}
