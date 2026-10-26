//! The kept weekly-limit samples as one document, for the read-only `limits` command.
//! Loading never writes: a missing or damaged cache is an empty history.
use super::store::Store;
use serde_json::{Value, json};
use std::path::PathBuf;

/// The weekly window every kept reading belongs to (`QuotaWindow::Weekly`).
const WEEKLY_MINUTES: u64 = 7 * 24 * 60;

/// `{version:1, limitHistory:[{provider, account?, windowMinutes, points:[[atMs, percentLeft, resetsAtMs]]}]}`,
/// providers by ID (and account key) and points oldest first; `account` is `{key,label}`, present only
/// for an account Core named. Percent is rounded to two decimals; the reset time rides
/// with each point because the kept eight days can span two windows.
pub(crate) fn document(directory: Option<PathBuf>) -> Value {
    let store = Store::load(directory);
    let kept: Vec<super::Provider> = store.providers().collect();
    // A driver with exactly one account is that account alone: the readings kept before accounts
    // were named fold into it, as on the board.
    let one_account = |driver: &str| {
        kept.iter()
            .filter(|other| other.id() == driver && other.account_key().is_some())
            .count()
            == 1
    };
    let history: Vec<Value> = kept
        .iter()
        .filter(|provider| provider.account_key().is_some() || !one_account(provider.id()))
        .map(|provider| {
            let readings = if provider.account_key().is_some() && one_account(provider.id()) {
                super::folded(store.history(&provider.plain()), store.history(provider))
            } else {
                store.history(provider).to_vec()
            };
            let points: Vec<Value> = readings
                .iter()
                .map(|reading| {
                    json!([
                        reading.observed_at_ms,
                        (reading.weekly.left * 100.0).round() / 100.0,
                        reading.weekly.resets_at_ms,
                    ])
                })
                .collect();
            let mut entry =
                json!({"provider": provider.id(), "windowMinutes": WEEKLY_MINUTES, "points": points});
            // Named only for an account Core named, so a plain provider's entry is unchanged.
            if let (Some(key), Some(label)) = (provider.account_key(), provider.account_label()) {
                entry["account"] = json!({"key": key, "label": label});
            }
            entry
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
    fn an_account_is_named_only_where_core_named_one_and_a_single_account_absorbs_the_plain_driver()
    {
        let directory =
            std::env::temp_dir().join(format!("ops-history-acc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let mut store = Store::load(Some(directory.clone()));
        let key = "0123456789ab";
        let work = Provider::from_key(&format!("claude.{key}"))
            .unwrap()
            .labelled("work");
        store.record(
            &Provider::named("claude"),
            reading(1_000_000, 90.0, 5_000_000),
        );
        store.record(&work, reading(2_000_000, 80.0, 5_000_000));
        store.record(
            &Provider::named("codex"),
            reading(1_000_000, 70.0, 5_000_000),
        );
        store.save_if_due(10_000_000_000);
        assert_eq!(
            document(Some(directory.clone())),
            json!({"version": 1, "limitHistory": [
                {"provider": "claude", "windowMinutes": 10080,
                 "points": [[1_000_000, 90.0, 5_000_000], [2_000_000, 80.0, 5_000_000]],
                 "account": {"key": key, "label": "work"}},
                {"provider": "codex", "windowMinutes": 10080, "points": [[1_000_000, 70.0, 5_000_000]]},
            ]})
        );
        let home = Provider::from_key("claude.ba9876543210")
            .unwrap()
            .labelled("home");
        store.record(&home, reading(3_000_000, 60.0, 5_000_000));
        store.save_if_due(20_000_000_000);
        let two = document(Some(directory.clone()));
        let entries: Vec<_> = two["limitHistory"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry["provider"].as_str().unwrap(),
                    entry["account"]["label"].as_str(),
                )
            })
            .collect();
        assert_eq!(
            entries,
            [
                ("claude", None),
                ("claude", Some("work")),
                ("claude", Some("home")),
                ("codex", None)
            ],
            "two accounts keep the plain driver beside them, unmerged"
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
