//! Activity: the one board projection of core's public `session.activity`.
//! Every surface reads this typed value; none re-parses the row.
use serde_json::Value;

/// Core's admitted phase. `waiting` is not a public core phase yet, so it
/// parses as unknown like any other unrecognized token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    Working,
    Idle,
    Ended,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObservedActivity {
    pub phase: Phase,
    /// When core admitted the current phase; only a running phase has one.
    pub since_ms: Option<u64>,
    /// When core admitted the last transition. It is not a heartbeat.
    pub last_ms: Option<u64>,
}

/// The largest integer a JSON producer can carry exactly.
const MAX_SAFE_MS: u64 = (1 << 53) - 1;

impl ObservedActivity {
    /// Fails closed per row: a missing or malformed document is unknown, and a
    /// timestamp that is not a safe integer is absent.
    pub fn of(row: &Value) -> Self {
        let document = &row["session"]["activity"];
        let ms = |key: &str| document[key].as_u64().filter(|ms| *ms <= MAX_SAFE_MS);
        let phase = match document["state"].as_str() {
            Some("working") => Phase::Working,
            Some("idle") => Phase::Idle,
            Some("ended") => Phase::Ended,
            _ => Phase::Unknown,
        };
        Self {
            phase,
            since_ms: ms("sinceMs").filter(|_| matches!(phase, Phase::Working | Phase::Idle)),
            last_ms: ms("lastActivityMs"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn of(activity: Value) -> ObservedActivity {
        ObservedActivity::of(&json!({"id": "a", "session": {"activity": activity}}))
    }

    #[test]
    fn admitted_phases_keep_their_times() {
        for (state, phase) in [("working", Phase::Working), ("idle", Phase::Idle)] {
            assert_eq!(
                of(json!({"state": state, "sinceMs": 10, "lastActivityMs": 12, "providers": {}})),
                ObservedActivity {
                    phase,
                    since_ms: Some(10),
                    last_ms: Some(12)
                }
            );
        }
        // Core gives an ended or unknown phase no start time; one is ignored.
        for (state, phase) in [("ended", Phase::Ended), ("unknown", Phase::Unknown)] {
            assert_eq!(
                of(json!({"state": state, "sinceMs": 10, "lastActivityMs": 12})),
                ObservedActivity {
                    phase,
                    since_ms: None,
                    last_ms: Some(12)
                }
            );
        }
    }

    #[test]
    fn malformed_documents_fail_closed_per_row() {
        let unknown = ObservedActivity::default();
        assert_eq!(ObservedActivity::of(&json!({"id": "a"})), unknown);
        assert_eq!(
            ObservedActivity::of(&json!({"session": "working"})),
            unknown
        );
        assert_eq!(of(json!(null)), unknown);
        assert_eq!(of(json!({"state": 1})), unknown);
        // A provider waiting phase is slice B's; until core publishes it, it is unknown.
        assert_eq!(of(json!({"state": "waiting", "sinceMs": 5})), unknown);
        assert_eq!(of(json!({"state": "Working"})), unknown);
        for bad in [json!(-1), json!(1.5), json!("12"), json!(MAX_SAFE_MS + 1)] {
            assert_eq!(
                of(json!({"state": "idle", "sinceMs": bad, "lastActivityMs": bad})),
                ObservedActivity {
                    phase: Phase::Idle,
                    since_ms: None,
                    last_ms: None
                }
            );
        }
        assert_eq!(
            of(json!({"state": "idle", "sinceMs": MAX_SAFE_MS, "lastActivityMs": 0})).since_ms,
            Some(MAX_SAFE_MS)
        );
    }
}
