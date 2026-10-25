//! Ordered conversation pages have their own opaque cursor, never the recent
//! response cursor. Reads do not acknowledge or refresh anything.
use super::{Fault, Request, invalid};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    identity_id: String,
    #[serde(default = "default_limit")]
    limit: u64,
    #[serde(default, deserialize_with = "present")]
    before: Option<String>,
}
fn default_limit() -> u64 {
    50
}
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
pub(super) fn decode(bytes: &[u8]) -> Result<Request, Fault> {
    let input: Input = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let before = input
        .before
        .map(|key| {
            let digits = key
                .strip_prefix("ce_")
                .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(invalid)?;
            let n = digits.parse::<u64>().map_err(|_| invalid())?;
            if n == 0 || n > tmt_core::limits::MAX_JS_SAFE_INTEGER || format!("ce_{n}") != key {
                return Err(invalid());
            }
            Ok(n)
        })
        .transpose()?;
    if !tmt_core::dispatch::canonical_id(&input.identity_id) || !(1..=50).contains(&input.limit) {
        return Err(invalid());
    }
    Ok(Request::ConversationTimeline {
        identity_id: input.identity_id,
        limit: input.limit,
        before,
    })
}
pub(super) fn read(
    storage: &mut crate::storage::Storage,
    id: &str,
    limit: u64,
    before: Option<u64>,
) -> Result<Vec<u8>, Fault> {
    let mut value = storage
        .conversation_timeline(id, limit, before, crate::request_runtime::wall_time_ms())
        .map_err(|_| Fault::unavailable())?;
    if let Some(capture) = super::responses::capture(
        storage
            .remembered_driver(id)
            .map_err(|_| Fault::unavailable())?
            .as_deref(),
        crate::setup::stop_hook_installed,
    ) {
        value["capture"] = capture;
    }
    serde_json::to_vec(&value).map_err(|_| Fault::unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_pages_require_canonical_cursor_and_bounded_limit() {
        let identity = "01234567-89ab-4def-8123-456789abcdef";
        for extra in [
            serde_json::json!({}),
            serde_json::json!({"limit":1,"before":"ce_1"}),
            serde_json::json!({"limit":50,"before":"ce_9007199254740991"}),
        ] {
            let mut input = serde_json::json!({"identityId":identity});
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(
                decode(&serde_json::to_vec(&input).unwrap()).is_ok(),
                "{input}"
            );
        }
        for extra in [
            serde_json::json!({"limit":0}),
            serde_json::json!({"limit":51}),
            serde_json::json!({"before":null}),
            serde_json::json!({"before":1}),
            serde_json::json!({"before":"ce_01"}),
            serde_json::json!({"before":"ce_0"}),
            serde_json::json!({"before":"ce_9007199254740992"}),
            serde_json::json!({"after":"ce_1"}),
        ] {
            let mut input = serde_json::json!({"identityId":identity});
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(
                decode(&serde_json::to_vec(&input).unwrap()).is_err(),
                "{input}"
            );
        }
    }
}
