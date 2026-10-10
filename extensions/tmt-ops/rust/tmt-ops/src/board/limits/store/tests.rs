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
    for n in 0..MAX_PROVIDERS + 2 {
        providers.insert(format!("driver-{n}"), good.clone());
    }
    let document = json!({"version": VERSION, "providers": providers});
    fs::write(fixture.0.join(FILE), document.to_string()).unwrap();
    let store = fixture.store();
    assert_eq!(store.providers().count(), MAX_PROVIDERS);
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
