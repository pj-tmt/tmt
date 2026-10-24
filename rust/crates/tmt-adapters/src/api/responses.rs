//! Exact-identity local pane output; no provider access or request attribution.
use super::{Fault, Request, invalid};
use crate::storage::Storage;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    identity_id: String,
    #[serde(default = "default_limit")]
    limit: u64,
    #[serde(default, deserialize_with = "present")]
    before: Option<u64>,
}
/// An absent `before` is none; a present one must be an integer (a JSON null is refused).
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(deserializer).map(Some)
}
fn default_limit() -> u64 {
    tmt_core::responses::DEFAULT_PAGE
}
pub(super) fn decode(input: &[u8]) -> Result<Request, Fault> {
    let input: Input = serde_json::from_slice(input).map_err(|_| invalid())?;
    if !tmt_core::dispatch::canonical_id(&input.identity_id)
        || !(1..=tmt_core::responses::MAX_PAGE).contains(&input.limit)
        || input
            .before
            .is_some_and(|before| before > tmt_core::limits::MAX_JS_SAFE_INTEGER)
    {
        return Err(invalid());
    }
    Ok(Request::RecentResponses {
        identity_id: input.identity_id,
        limit: input.limit,
        before: input.before,
    })
}
pub(super) fn recent(
    storage: &mut Storage,
    id: &str,
    limit: u64,
    before: Option<u64>,
) -> Result<Vec<u8>, Fault> {
    let value = storage
        .recent_responses(id, limit, before, crate::request_runtime::wall_time_ms())
        .map_err(|_| Fault::unavailable())?;
    serde_json::to_vec(&value).map_err(|_| Fault::unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_request_uses_default_ten_without_a_new_envelope_field() {
        let vector: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../contracts/recent-responses-v1.json"
        ))
        .unwrap();
        assert!(matches!(
            decode(vector["request"]["input"].to_string().as_bytes()).unwrap(),
            Request::RecentResponses { limit: 10, .. }
        ));
    }
}
