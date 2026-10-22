//! The kept weekly-limit samples as one document, for the read-only `limits` command.
//! Loading never writes: a missing or damaged cache is an empty history.
use super::store::Store;
use serde_json::{Value, json};
use std::path::PathBuf;

/// The weekly window every kept reading belongs to (`QuotaWindow::Weekly`).
const WEEKLY_MINUTES: u64 = 7 * 24 * 60;

/// `{version:1, limitHistory:[{provider, windowMinutes, points:[[atMs, percentLeft, resetsAtMs]]}]}`,
/// providers by ID and points oldest first. Percent is rounded to two decimals; the reset time rides
/// with each point because the kept eight days can span two windows.
pub(crate) fn document(directory: Option<PathBuf>) -> Value {
    let store = Store::load(directory);
    let history: Vec<Value> = store
        .providers()
        .map(|provider| {
            let points: Vec<Value> = store
                .history(&provider)
                .iter()
                .map(|reading| {
                    json!([
                        reading.observed_at_ms,
                        (reading.weekly.left * 100.0).round() / 100.0,
                        reading.weekly.resets_at_ms,
                    ])
                })
                .collect();
            json!({"provider": provider.id(), "windowMinutes": WEEKLY_MINUTES, "points": points})
        })
        .collect();
    json!({"version": 1, "limitHistory": history})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::limits::{Provider, Reading, Window};

    fn reading(at: u64, left: f64, resets: u64) -> Reading {
        Reading {
            observed_at_ms: at,
            weekly: Window {
                left,
                resets_at_ms: resets,
            },
            short: Some(Window {
                left: 1.0,
                resets_at_ms: 9,
            }),
        }
    }

    #[test]
    fn the_document_lists_weekly_points_oldest_first_and_never_the_short_window() {
        let directory = std::env::temp_dir().join(format!("ops-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let mut store = Store::load(Some(directory.clone()));
        let codex = Provider::named("codex");
        store.record(&codex, reading(1_000_000, 80.126, 5_000_000));
        store.record(&codex, reading(2_000_000, 79.0, 5_000_000));
        store.record(
            &Provider::named("claude"),
            reading(1_500_000, 50.0, 6_000_000),
        );
        store.save_if_due(10_000_000_000);
        let document = document(Some(directory.clone()));
        assert_eq!(
            document,
            json!({"version": 1, "limitHistory": [
                {"provider": "claude", "windowMinutes": 10080, "points": [[1_500_000, 50.0, 6_000_000]]},
                {"provider": "codex", "windowMinutes": 10080,
                 "points": [[1_000_000, 80.13, 5_000_000], [2_000_000, 79.0, 5_000_000]]},
            ]})
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_missing_or_unreadable_cache_is_an_empty_history_and_nothing_is_created() {
        let directory =
            std::env::temp_dir().join(format!("ops-history-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        assert_eq!(
            document(Some(directory.clone())),
            json!({"version": 1, "limitHistory": []})
        );
        assert!(!directory.exists(), "a read created the cache directory");
        assert_eq!(document(None), json!({"version": 1, "limitHistory": []}));
    }
}
