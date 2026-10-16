//! The 5-minute rollup is the sum of the 5-second buckets it covers, served for
//! windows above an hour, and ages out by time alone (#3248).
use super::tests::{attributed, fixture, latest, model_row, source};
use super::*;

const VECTOR: &str = include_str!("../../../../../../contracts/consumption-history-v1.json");

/// One identity fed the shared vector, then asked about a rollup window.
fn vector_history(now: u64, windows: &[u64]) -> Value {
    let fixture: Value = serde_json::from_str(VECTOR).unwrap();
    let directory = crate::test_support::TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("history.sqlite")).unwrap();
    let id = fixture["request"]["input"]["identityIds"][0]
        .as_str()
        .unwrap();
    let connection = storage.connection().unwrap();
    connection.execute("INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES(?,'Meter','meter','t','t','saved')",[id]).unwrap();
    super::tests::insert_binding(connection, id);
    connection.execute("INSERT INTO consumption_sources(identity_id,binding_id,driver,session) VALUES(?,'fixture-binding','claude','fixture-session')",[id]).unwrap();
    for reading in fixture["observations"].as_array().unwrap() {
        let latest: ConsumptionLatest = serde_json::from_value(json!({"driver":reading["driver"],"session":reading["session"],"consumption":reading["consumption"]})).unwrap();
        record_sample(
            connection,
            id,
            Some(latest),
            reading["sampleAtMs"].as_u64().unwrap(),
        )
        .unwrap();
    }
    storage
        .consumption_history(&[id.to_owned()], windows, 120, now, false)
        .unwrap()
}

#[test]
fn the_shared_vector_rolls_up_into_one_closed_bucket_with_the_newest_limits() {
    let history = vector_history(322_500, &[3_900_000]);
    let identity = &history["identities"][0];
    // The identity-level fields still describe the 5-second tier.
    assert_eq!(identity["reporting"], true);
    let window = &identity["windows"][0];
    // The shared vector is the contract's expectation for this window.
    let fixture: Value = serde_json::from_str(VECTOR).unwrap();
    assert_eq!(fixture["rollup"]["asOfMs"], 322_500);
    assert_eq!(window, &fixture["rollup"]["window"]);
    // 10 + 4 + 6 input, 5 + 2 + 3 output, 2 + 1 + 1 cached across the three
    // continuous reads; the first read is a baseline, so the bucket has a gap.
    assert_eq!(window["buckets"][0]["inputTokens"], 20);
    assert_eq!(window["buckets"][0]["gap"], true);
}

#[test]
fn fine_windows_and_the_open_rollup_bucket_are_unchanged() {
    let fixture: Value = serde_json::from_str(VECTOR).unwrap();
    let history = vector_history(22_500, &[15_000, 3_900_000]);
    // The shared vector's fine-tier response is byte-identical.
    assert_eq!(history["identities"][0]["windows"][0], {
        fixture["response"]["identities"][0]["windows"][0].clone()
    });
    // The first five minutes are still open at 22.5 s: nothing closed to report.
    let window = &history["identities"][0]["windows"][1];
    assert_eq!(window["toMs"], 0);
    assert_eq!(window["buckets"], json!([]));
    assert_eq!(window["availableFromMs"], Value::Null);
}

/// A varied hour of reads: model changes, a failed read, a session boundary and
/// reads too far apart to establish coverage.
fn feed(storage: &mut Storage, id: &str) -> u64 {
    source(storage, id);
    let connection = storage.connection().unwrap();
    let (mut input, mut output, mut sequence) = (0u64, 0u64, 1u64);
    record_sample(connection, id, Some(latest(0, 0, 1, 1000)), 1000).unwrap();
    let mut now = 1000;
    for step in 1..=660u64 {
        now += if step % 97 == 0 { 40_000 } else { BUCKET_MS };
        if step % 41 == 0 {
            record_sample(connection, id, None, now).unwrap();
            continue;
        }
        input += step % 13;
        output += step % 5;
        sequence += 1;
        let rows = if step % 7 == 0 {
            vec![
                model_row("a", step % 13 / 2, step % 5, Some(0)),
                model_row("b", step % 13 - step % 13 / 2, 0, Some(0)),
            ]
        } else {
            vec![model_row("a", step % 13, step % 5, Some(0))]
        };
        let mut reading = attributed(input, output, 0, sequence, now, rows);
        if step % 211 == 0 {
            reading.session = "other-session".into();
        }
        if step % 3 == 0 {
            reading.consumption.rate_limits = Some(
                serde_json::from_value(json!({"observedAtMs":now,"windows":[
                    {"windowMinutes":300,"usedPercent":(step % 100) as f64,"resetsAtMs":now + 1000}]}))
                .unwrap(),
            );
        }
        record_sample(connection, id, Some(reading), now).unwrap();
    }
    now
}

#[test]
fn every_rollup_bucket_equals_the_sum_of_the_fine_buckets_it_covers() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    let end = feed(&mut storage, &id);
    // Read at the start of a five-minute slot so both tiers end at the same instant.
    let now = (end / ROLLUP_BUCKET_MS + 1) * ROLLUP_BUCKET_MS + 2500;
    // 12 buckets make the fine hour 5-minute wide; 13 keep the rollup 5-minute wide.
    let ask = |storage: &mut Storage, window: u64, buckets: u64| {
        let history = storage
            .consumption_history(std::slice::from_ref(&id), &[window], buckets, now, false)
            .unwrap();
        history["identities"][0]["windows"][0].clone()
    };
    let fine_window = ask(&mut storage, FINE_WINDOW_MS, 12);
    let rolled_window = ask(&mut storage, 3_900_000, 13);
    let fine = fine_window["buckets"].as_array().unwrap();
    let rolled = rolled_window["buckets"].as_array().unwrap();
    assert_eq!(fine.len(), 12);
    assert_eq!(fine_window["bucketMs"], 300_000);
    assert_eq!(rolled_window["bucketMs"], 300_000);
    let strip = |bucket: &Value| {
        let mut bucket = bucket.clone();
        bucket.as_object_mut().unwrap().remove("rateLimits");
        bucket
    };
    let mut busy = 0;
    for (fine, rolled) in fine.iter().zip(&rolled[rolled.len() - 12..]) {
        assert_eq!(strip(fine), strip(rolled));
        busy += usize::from(fine["inputTokens"] != 0);
    }
    assert!(
        busy >= 8,
        "the hour exercised the tiers: {busy} busy buckets"
    );
    // Some bucket kept its newest rate limits, and attribution survived the sum.
    assert!(
        rolled
            .iter()
            .any(|bucket| bucket.get("rateLimits").is_some())
    );
    assert!(rolled.iter().any(|bucket| bucket.get("byModel").is_some()));
    assert!(fine.iter().any(|bucket| bucket["gap"] == true));
    assert!(fine.iter().any(|bucket| bucket["discontinuous"] == true));
}

#[test]
fn later_coverage_and_a_failed_read_reach_the_rollup_like_the_fine_tier() {
    let (_directory, path, id) = fixture();
    let storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let connection = storage.connection().unwrap();
    record_sample(connection, &id, Some(latest(100, 50, 1, 6000)), 6000).unwrap();
    record_sample(connection, &id, Some(latest(110, 55, 2, 11000)), 11000).unwrap();
    // A failed read adds no successful observation to the rollup and no coverage.
    record_sample(connection, &id, None, 13000).unwrap();
    let row: (i64, i64, Option<i64>, bool) = connection
        .query_row(
            "SELECT input_tokens,covered_ms,sampled_at_ms,gap FROM consumption_tier5m WHERE identity_id=? AND from_ms=0",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row, (10, 5000, Some(11000), true));
    // The rollup never keeps the per-read observation the fine tier does.
    let kept: Option<String> = connection
        .query_row(
            "SELECT limits FROM consumption_tier5m WHERE identity_id=? AND from_ms=0",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kept, None);
}

#[test]
fn the_rollup_ages_out_by_time_for_active_and_idle_identities() {
    let (_directory, path, id) = fixture();
    let storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let connection = storage.connection().unwrap();
    record_sample(connection, &id, Some(latest(1, 1, 1, 6000)), 6000).unwrap();
    record_sample(connection, &id, Some(latest(2, 2, 2, 11000)), 11000).unwrap();
    let count = |connection: &Connection| -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM consumption_tier5m", [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(count(connection), 1);
    // A second identity writing just inside 30 days leaves the first bucket alone;
    // one more five-minute slot past it expires the idle identity's bucket too.
    let other = "22222222-2222-4222-8222-222222222222";
    connection.execute("INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES(?,'Other','other','t','t','saved')",[other]).unwrap();
    connection.execute("INSERT INTO bindings(id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at) VALUES('other-binding',?,'tmux','%2','fixture-server','/tmp/fixture-socket',10,'start',12,'t','t')",[other]).unwrap();
    connection.execute("INSERT INTO consumption_sources(identity_id,binding_id,driver,session) VALUES(?,'other-binding','claude','fixture-session')",[other]).unwrap();
    let inside = ROLLUP_MS + 6000;
    record_sample(connection, other, Some(latest(1, 1, 1, inside)), inside).unwrap();
    assert_eq!(count(connection), 2, "still inside the 30 days");
    let past = ROLLUP_MS + ROLLUP_BUCKET_MS + 6000;
    record_sample(connection, other, Some(latest(2, 2, 2, past)), past).unwrap();
    let remaining: Vec<String> = {
        let mut statement = connection
            .prepare("SELECT identity_id FROM consumption_tier5m ORDER BY from_ms")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(remaining, vec![other.to_owned(), other.to_owned()]);
}

#[test]
fn retired_identities_are_answered_only_when_asked() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let connection = storage.connection().unwrap();
    record_sample(connection, &id, Some(latest(1, 1, 1, 6000)), 6000).unwrap();
    record_sample(connection, &id, Some(latest(5, 3, 2, 11000)), 11000).unwrap();
    connection
        .execute(
            "UPDATE identities SET retired_at_ms=? WHERE id=?",
            params![12_000, &id],
        )
        .unwrap();
    let now = 322_500;
    let hidden = storage
        .consumption_history(std::slice::from_ref(&id), &[3_900_000], 120, now, false)
        .unwrap();
    assert_eq!(hidden["identities"][0], json!({"id":id,"found":false}));
    let shown = storage
        .consumption_history(std::slice::from_ref(&id), &[3_900_000], 120, now, true)
        .unwrap();
    let identity = &shown["identities"][0];
    assert_eq!(identity["found"], true);
    assert_eq!(identity["retired"], true);
    // Labels come from `references.resolve`, not from this read.
    assert!(identity.get("name").is_none());
    assert_eq!(identity["windows"][0]["buckets"][0]["inputTokens"], 4);
    // An active identity carries no `retired` key, so the default shape is unchanged.
    connection_clear_retirement(&mut storage, &id);
    let active = storage
        .consumption_history(std::slice::from_ref(&id), &[3_900_000], 120, now, true)
        .unwrap();
    assert!(active["identities"][0].get("retired").is_none());
}

fn connection_clear_retirement(storage: &mut Storage, id: &str) {
    storage
        .connection()
        .unwrap()
        .execute("UPDATE identities SET retired_at_ms=NULL WHERE id=?", [id])
        .unwrap();
}

#[test]
fn the_window_boundaries_and_corrupt_rollup_rows_are_refused() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    let ask = |storage: &mut Storage, window: u64, max: u64| {
        storage.consumption_history(std::slice::from_ref(&id), &[window], max, 400_000, false)
    };
    for (window, max) in [
        (3_650_000, 120),
        (ROLLUP_MS + ROLLUP_BUCKET_MS, 120),
        (FINE_WINDOW_MS, 0),
        (FINE_WINDOW_MS, MAX_BUCKETS + 1),
    ] {
        assert!(ask(&mut storage, window, max).is_err(), "{window} {max}");
    }
    for (window, max) in [
        (3_900_000, 1),
        (ROLLUP_MS, MAX_BUCKETS),
        (FINE_WINDOW_MS, 720),
    ] {
        assert!(ask(&mut storage, window, max).is_ok(), "{window} {max}");
    }
    // A rollup row whose attribution disagrees with its totals fails closed.
    storage.connection().unwrap().execute("INSERT INTO consumption_tier5m(identity_id,from_ms,input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous,details) VALUES(?,0,5,0,0,0,1,0,0,'{\"byModel\":[{\"modelId\":\"a\",\"inputTokens\":9,\"outputTokens\":0,\"cachedInputTokens\":0}]}')",[&id]).unwrap();
    assert!(ask(&mut storage, 3_900_000, 120).is_err());
}

/// The size of the rollup on a synthetic full fleet, so growth stays visible: 32
/// identities for 30 days of 5-minute buckets, half Claude-shaped (one model, no
/// limits) and half Codex-shaped (one model, both limit windows on a quarter of the buckets). The ceiling is the
/// budget the plan gave the tier, with a margin over the measured size.
#[test]
fn the_rollup_fits_its_budget_on_a_full_thirty_day_fleet() {
    let directory = crate::test_support::TestDirectory::new();
    let storage = Storage::open(directory.path.join("budget.sqlite")).unwrap();
    let connection = storage.connection().unwrap();
    let rows_per_identity = ROLLUP_MS / ROLLUP_BUCKET_MS;
    let claude = r#"{"byModel":[{"modelId":"claude-opus-5-5","inputTokens":62000,"outputTokens":4900,"cachedInputTokens":38000}]}"#;
    let codex = r#"{"byModel":[{"modelId":"gpt-6.1-sol","inputTokens":62000,"outputTokens":4900,"cachedInputTokens":38000}]}"#;
    let limits = r#"{"observedAtMs":1791517290230,"windows":[{"windowMinutes":300,"usedPercent":41.5,"resetsAtMs":1791530000000},{"windowMinutes":10080,"usedPercent":63.25,"resetsAtMs":1792000000000}]}"#;
    connection.execute_batch("BEGIN").unwrap();
    for n in 0..32u64 {
        let id = format!("00000000-0000-4000-8000-{n:012}");
        connection.execute("INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES(?,?,?,'t','t','saved')",params![id,format!("m{n}"),format!("m{n}")]).unwrap();
        let mut insert = connection.prepare("INSERT INTO consumption_tier5m(identity_id,from_ms,input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous,sampled_at_ms,limits,details) VALUES(?,?,62000,4900,38000,300000,1,0,0,?,?,?)").unwrap();
        let (limits, details) = if n % 2 == 0 {
            (None, claude)
        } else {
            (Some(limits), codex)
        };
        for row in 0..rows_per_identity {
            let from = row * ROLLUP_BUCKET_MS;
            // Only a bucket that saw a fresh provider line keeps limits: 28% of the
            // Codex buckets on the measured roster, a quarter here.
            let limits = limits.filter(|_| row % 4 == 0);
            insert
                .execute(params![
                    id,
                    from as i64,
                    (from + 1000) as i64,
                    limits,
                    details
                ])
                .unwrap();
        }
    }
    connection.execute_batch("COMMIT").unwrap();
    let (rows, bytes): (i64, i64) = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM consumption_tier5m), SUM(pgsize) FROM dbstat WHERE name IN ('consumption_tier5m','consumption_tier5m_expiry') OR name LIKE 'sqlite_autoindex_consumption_tier5m%'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(rows, 32 * 8640);
    eprintln!(
        "rollup: {rows} rows, {bytes} bytes, {} bytes per row",
        bytes / rows
    );
    assert!(bytes < 90 * 1024 * 1024, "{bytes} bytes for {rows} rows");
}

#[test]
fn only_limits_observed_inside_the_bucket_are_kept_and_the_newest_wins() {
    let (_directory, path, id) = fixture();
    let storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let connection = storage.connection().unwrap();
    let limited = |observed: u64, percent: f64, sequence: u64, at: u64| {
        let mut reading = latest(sequence, sequence, sequence, at);
        reading.consumption.rate_limits = Some(
            serde_json::from_value(json!({"observedAtMs":observed,"windows":[
                {"windowMinutes":10_080,"usedPercent":percent,"resetsAtMs":9_000_000}]}))
            .unwrap(),
        );
        reading
    };
    let kept = |connection: &Connection, from: u64| -> Option<String> {
        connection
            .query_row(
                "SELECT limits FROM consumption_tier5m WHERE identity_id=? AND from_ms=?",
                params![&id, from as i64],
                |row| row.get(0),
            )
            .unwrap()
    };
    // Limits seen in the first bucket are stored there, newest first.
    record_sample(connection, &id, Some(limited(5_000, 10.0, 1, 6_000)), 6_000).unwrap();
    record_sample(
        connection,
        &id,
        Some(limited(9_000, 11.0, 2, 11_000)),
        11_000,
    )
    .unwrap();
    // An older line repeated later by an idle session does not replace it.
    record_sample(
        connection,
        &id,
        Some(limited(5_000, 10.0, 3, 16_000)),
        16_000,
    )
    .unwrap();
    let first = kept(connection, 0).unwrap();
    assert!(
        first.contains(r#""observedAtMs":9000"#) && first.contains("11.0"),
        "{first}"
    );
    // The same stale limits repeated into the next five minutes are not stored there.
    let later = ROLLUP_BUCKET_MS + 6_000;
    record_sample(connection, &id, Some(limited(9_000, 11.0, 4, later)), later).unwrap();
    assert_eq!(kept(connection, ROLLUP_BUCKET_MS), None);
    // A fresh line in that bucket is stored.
    let fresh = ROLLUP_BUCKET_MS + 11_000;
    record_sample(
        connection,
        &id,
        Some(limited(fresh - 500, 12.5, 5, fresh)),
        fresh,
    )
    .unwrap();
    assert!(kept(connection, ROLLUP_BUCKET_MS).unwrap().contains("12.5"));
}
