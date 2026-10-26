use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const T0: u64 = 1_791_600_000_000;
const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "ops-limits-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn store(&self) -> Store {
        Store::load(Some(self.0.clone()))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn reading(at_ms: u64, left: f64) -> Reading {
    Reading {
        observed_at_ms: at_ms,
        weekly: Window {
            left,
            resets_at_ms: T0 + 5 * DAY,
        },
        short: None,
    }
}

fn times(store: &Store, provider: &Provider) -> Vec<u64> {
    store
        .history(provider)
        .iter()
        .map(|r| r.observed_at_ms)
        .collect()
}

#[test]
fn readings_are_kept_per_provider_in_time_order() {
    let mut store = Store::load(None);
    store.record(&Provider::named("codex"), reading(T0, 50.0));
    store.record(&Provider::named("claude"), reading(T0 + HOUR, 60.0));
    store.record(&Provider::named("codex"), reading(T0 + 2 * HOUR, 49.0));
    assert_eq!(
        times(&store, &Provider::named("codex")),
        [T0, T0 + 2 * HOUR]
    );
    assert_eq!(times(&store, &Provider::named("claude")), [T0 + HOUR]);
}

#[test]
fn an_older_or_equal_reading_changes_nothing() {
    let mut store = Store::load(None);
    store.record(&Provider::named("codex"), reading(T0 + HOUR, 50.0));
    store.record(&Provider::named("codex"), reading(T0, 60.0));
    store.record(&Provider::named("codex"), reading(T0 + HOUR, 40.0));
    assert_eq!(
        store.history(&Provider::named("codex")),
        [reading(T0 + HOUR, 50.0)]
    );
}

#[test]
fn a_newer_reading_in_the_same_ten_minutes_replaces_the_last_one() {
    let mut store = Store::load(None);
    // T0 is not on a bucket boundary, so stay inside one explicitly.
    let start = (T0 / BUCKET_MS) * BUCKET_MS;
    store.record(&Provider::named("codex"), reading(start + MINUTE, 50.0));
    store.record(&Provider::named("codex"), reading(start + 2 * MINUTE, 49.5));
    assert_eq!(
        store.history(&Provider::named("codex")),
        [reading(start + 2 * MINUTE, 49.5)]
    );
    store.record(
        &Provider::named("codex"),
        reading(start + BUCKET_MS + MINUTE, 49.0),
    );
    assert_eq!(
        times(&store, &Provider::named("codex")),
        [start + 2 * MINUTE, start + BUCKET_MS + MINUTE]
    );
}

#[test]
fn invalid_readings_are_refused() {
    let mut store = Store::load(None);
    let mut bad = reading(T0, 50.0);
    bad.weekly.left = 100.5;
    store.record(&Provider::named("codex"), bad);
    store.record(&Provider::named("codex"), reading(0, 50.0));
    assert!(store.history(&Provider::named("codex")).is_empty());
}

#[test]
fn history_older_than_a_cycle_and_a_day_is_dropped() {
    let mut store = Store::load(None);
    store.record(&Provider::named("codex"), reading(T0, 90.0));
    store.record(&Provider::named("codex"), reading(T0 + 7 * DAY, 50.0));
    assert_eq!(times(&store, &Provider::named("codex")), [T0, T0 + 7 * DAY]);
    store.record(&Provider::named("codex"), reading(T0 + 9 * DAY, 40.0));
    assert_eq!(
        times(&store, &Provider::named("codex")),
        [T0 + 7 * DAY, T0 + 9 * DAY]
    );
}

#[test]
fn saved_history_loads_back_identically_including_the_short_window() {
    let fixture = Fixture::new();
    let mut with_short = reading(T0 + HOUR, 42.5);
    with_short.short = Some(Window {
        left: 97.0,
        resets_at_ms: T0 + 4 * HOUR,
    });
    let mut store = fixture.store();
    store.record(&Provider::named("claude"), reading(T0, 44.0));
    store.record(&Provider::named("claude"), with_short);
    store.record(&Provider::named("codex"), reading(T0, 70.0));
    store.save_if_due(T0 + HOUR);
    let loaded = fixture.store();
    assert_eq!(
        loaded.history(&Provider::named("claude")),
        [reading(T0, 44.0), with_short]
    );
    assert_eq!(
        loaded.history(&Provider::named("codex")),
        [reading(T0, 70.0)]
    );
}

#[test]
fn saving_waits_five_minutes_and_only_when_something_changed() {
    let fixture = Fixture::new();
    let path = fixture.0.join(FILE);
    let mut store = fixture.store();
    store.save_if_due(T0);
    assert!(!path.exists(), "nothing recorded, nothing written");
    store.record(&Provider::named("codex"), reading(T0, 70.0));
    store.save_if_due(T0 + SAVE_EVERY_MS);
    assert!(path.exists());
    fs::remove_file(&path).unwrap();
    store.save_if_due(T0 + 2 * SAVE_EVERY_MS);
    assert!(!path.exists(), "unchanged since the last write");
    store.record(&Provider::named("codex"), reading(T0 + HOUR, 69.0));
    store.save_if_due(T0 + SAVE_EVERY_MS + MINUTE);
    assert!(!path.exists(), "the last write is too recent");
    store.save_if_due(T0 + 2 * SAVE_EVERY_MS);
    assert!(path.exists());
}

#[test]
fn a_cache_file_that_cannot_be_trusted_starts_empty() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.0).unwrap();
    let path = fixture.0.join(FILE);
    for contents in [
        "".to_owned(),
        "not json".to_owned(),
        r#"{"version":2,"providers":{}}"#.to_owned(),
        r#"{"providers":{}}"#.to_owned(),
        " ".repeat(READ_LIMIT as usize + 1),
    ] {
        fs::write(&path, contents).unwrap();
        let store = fixture.store();
        assert_eq!(store.providers().count(), 0);
    }
}

#[test]
fn a_bad_row_drops_itself_and_order_is_restored() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.0).unwrap();
    let good = |at: u64, left: f64| json!([at, left, T0 + DAY, null, null]);
    let document = json!({
        "version": VERSION,
        "providers": {"codex": [
            good(T0 + 2 * HOUR, 60.0),
            good(T0, 70.0),
            [T0 + 3 * HOUR, 101.0, T0 + DAY, null, null],
            [T0 + 4 * HOUR, 55.0, T0 + DAY, 90.0, null],
            "junk",
            [1, 2],
            good(T0 + 5 * HOUR, 50.0),
        ]},
    });
    fs::write(fixture.0.join(FILE), document.to_string()).unwrap();
    let store = fixture.store();
    assert_eq!(
        times(&store, &Provider::named("codex")),
        [T0 + 2 * HOUR, T0 + 5 * HOUR]
    );
    assert!(store.history(&Provider::named("claude")).is_empty());
}

#[test]
fn only_plain_driver_ids_and_a_few_providers_are_kept() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.0).unwrap();
    let good = json!([[T0, 60.0, T0 + DAY, null, null]]);
    let mut providers = serde_json::Map::new();
    for key in [
        "",
        "has space",
        "../escape",
        "a-very-long-driver-id-over-the-cap",
    ] {
        providers.insert(key.into(), good.clone());
    }
    for n in 0..MAX_DRIVERS + 2 {
        providers.insert(format!("driver-{n}"), good.clone());
    }
    let document = json!({"version": VERSION, "providers": providers});
    fs::write(fixture.0.join(FILE), document.to_string()).unwrap();
    let store = fixture.store();
    assert_eq!(store.providers().count(), MAX_DRIVERS);
    assert!(store.providers().all(|p| p.id().starts_with("driver-")));
    let mut store = store;
    store.record(&Provider::named("extra"), reading(T0, 50.0));
    assert!(store.history(&Provider::named("extra")).is_empty());
}

#[test]
fn the_saved_file_names_providers_by_their_driver_id() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.record(&Provider::named("codex"), reading(T0, 70.0));
    store.save_if_due(T0 + HOUR);
    let saved: Value =
        serde_json::from_str(&fs::read_to_string(fixture.0.join(FILE)).unwrap()).unwrap();
    assert_eq!(saved["providers"]["codex"][0][0], T0);
}

const WORK: &str = "0123456789ab";

fn account(label: &str) -> Provider {
    Provider::from_key(&format!("claude.{WORK}"))
        .unwrap()
        .labelled(label)
}

#[test]
fn an_accounts_label_is_saved_beside_its_history_and_a_plain_cache_has_no_account_section() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.record(&Provider::named("codex"), reading(T0, 50.0));
    store.save_if_due(T0 + HOUR);
    let plain: Value = serde_json::from_slice(&fs::read(fixture.0.join(FILE)).unwrap()).unwrap();
    assert!(
        plain.get("accounts").is_none(),
        "an account-free cache keeps its old shape"
    );

    store.record(&account("work"), reading(T0 + MINUTE, 40.0));
    store.save_if_due(T0 + 2 * HOUR);
    let saved: Value = serde_json::from_slice(&fs::read(fixture.0.join(FILE)).unwrap()).unwrap();
    assert_eq!(
        saved["providers"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["claude.0123456789ab", "codex"]
    );
    assert_eq!(
        saved["accounts"],
        serde_json::json!({"claude.0123456789ab": "work"})
    );

    let loaded = fixture.store();
    let labels: Vec<_> = loaded
        .providers()
        .map(|provider| (provider.key(), provider.account_label().map(str::to_owned)))
        .collect();
    assert_eq!(
        labels,
        [
            ("claude.0123456789ab".to_owned(), Some("work".to_owned())),
            ("codex".to_owned(), None)
        ]
    );
}

#[test]
fn a_new_label_replaces_the_old_one_and_keeps_the_readings() {
    let mut store = Store::load(None);
    store.record(&account("work"), reading(T0, 50.0));
    store.record(&account("renamed"), reading(T0 + HOUR, 49.0));
    let kept: Vec<_> = store.providers().collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].account_label(), Some("renamed"));
    assert_eq!(times(&store, &account("anything")), [T0, T0 + HOUR]);
}

#[test]
fn a_saved_label_that_is_not_display_safe_is_not_trusted() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.0).unwrap();
    let row = serde_json::json!([T0, 50.0, T0 + 5 * DAY, null, null]);
    let document = serde_json::json!({
        "version": VERSION,
        "providers": {"claude.0123456789ab": [row]},
        "accounts": {"claude.0123456789ab": "two words\nand a newline"},
    });
    fs::write(fixture.0.join(FILE), document.to_string()).unwrap();
    let store = fixture.store();
    let kept: Vec<_> = store.providers().collect();
    assert_eq!(kept.len(), 1, "the readings are kept");
    assert_eq!(
        kept[0].account_label(),
        Some("unnamed"),
        "the hostile label is not"
    );
}

#[test]
fn accounts_are_capped_per_driver_and_never_displace_another_drivers_history() {
    let key = |n: usize| format!("claude.{n:012x}");
    let mut store = Store::load(None);
    store.record(&Provider::named("claude"), reading(T0, 50.0));
    for n in 0..MAX_ACCOUNTS + 2 {
        store.record(&Provider::from_key(&key(n)).unwrap(), reading(T0, 50.0));
    }
    store.record(&Provider::named("codex"), reading(T0, 70.0));
    let claude = store.providers().filter(|p| p.id() == "claude").count();
    assert_eq!(
        claude, MAX_ACCOUNTS,
        "the plain driver counts as one entry of the driver"
    );
    assert!(
        !store.history(&Provider::named("codex")).is_empty(),
        "accounts of one driver leave room for the others"
    );
    // The same limits apply when the file is read.
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.0).unwrap();
    let rows = json!([[T0, 60.0, T0 + DAY, null, null]]);
    let mut providers = serde_json::Map::new();
    for n in 0..MAX_ACCOUNTS + 2 {
        providers.insert(key(n), rows.clone());
    }
    providers.insert("codex".into(), rows);
    fs::write(
        fixture.0.join(FILE),
        json!({"version": VERSION, "providers": providers}).to_string(),
    )
    .unwrap();
    let loaded = fixture.store();
    assert_eq!(
        loaded.providers().filter(|p| p.id() == "claude").count(),
        MAX_ACCOUNTS
    );
    assert!(!loaded.history(&Provider::named("codex")).is_empty());
}
