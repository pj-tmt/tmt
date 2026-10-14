//! One status projection for CLI and extension transport adapters.

use serde_json::{Value, json};
use tmt_core::identity_status::IdentityStatus;

pub fn status_value(status: Option<&IdentityStatus>, now_ms: u64) -> Value {
    match status {
        None => Value::Null,
        Some(status) => json!({
            "activity": status.activity,
            "mood": status.mood,
            "updatedAtMs": status.updated_at_ms,
            "expiresAtMs": status.expires_at_ms,
            "stale": status.is_stale(now_ms),
        }),
    }
}
