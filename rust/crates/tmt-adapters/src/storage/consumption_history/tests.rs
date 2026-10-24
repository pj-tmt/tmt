use super::{details::DriverUsage, *};

// Literal expectations characterize the pre-optimization public projection.
#[test]
fn history_empty_and_missing_keep_complete_wire_shape() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    let response = storage
        .consumption_history(
            &[id.clone(), "missing".into()],
            &[10000],
            120,
            15001,
            false,
            None,
        )
        .unwrap();
    assert_eq!(
        response,
        json!({
            "asOfMs":15001,"throughMs":15000,"retainedFromMs":0,"resolutionMs":5000,
            "identities":[
                {"id":id,"found":true,"reporting":false,"availableFromMs":null,
                 "lastSampleAtMs":null,"latest":null,"windows":[
                    {"windowMs":10000,"fromMs":5000,"toMs":15000,"bucketMs":5000,
                     "buckets":[
                        {"fromMs":5000,"toMs":10000,"inputTokens":0,"outputTokens":0,
                         "cachedInputTokens":0,"coveredMs":0,"complete":false,"gap":true,"discontinuous":false},
                        {"fromMs":10000,"toMs":15000,"inputTokens":0,"outputTokens":0,
                         "cachedInputTokens":0,"coveredMs":0,"complete":false,"gap":true,"discontinuous":false}
                     ]}
                 ],"cause":"no_agent"},
                {"id":"missing","found":false}
            ]
        })
    );
}

/// A client carries the first batch's cut to its later batches (#4176): every
/// window ends there, whatever the read time, exactly as an unpinned read at that
/// moment would end it.
#[test]
fn a_pinned_cut_ends_every_window_there_whatever_the_read_time() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    for (from, input) in [(0, 1), (5000, 2), (10000, 3), (15000, 4), (20000, 99)] {
        Bucket {
            from,
            input,
            covered: 5000,
            ..Bucket::default()
        }
        .write(storage.connection().unwrap(), FINE, &id)
        .unwrap();
    }
    let ask = |storage: &mut Storage, now: u64, pin: Option<u64>| {
        storage
            .consumption_history(
                std::slice::from_ref(&id),
                &[15000, 5000],
                120,
                now,
                false,
                pin,
            )
            .unwrap()
    };
    let first = ask(&mut storage, 22_500, Some(15_000));
    let later = ask(&mut storage, 26_000, Some(15_000));
    assert_eq!(
        (first["asOfMs"].as_u64(), later["asOfMs"].as_u64()),
        (Some(22_500), Some(26_000))
    );
    assert_eq!(
        (&first["throughMs"], &later["throughMs"]),
        (&json!(15_000), &json!(15_000))
    );
    assert_eq!(
        first["identities"], later["identities"],
        "the read time is not part of the cut"
    );
    // The same as a read that happened at the cut, and it excludes what closed later.
    assert_eq!(
        first["identities"],
        ask(&mut storage, 17_500, None)["identities"]
    );
    let windows = first["identities"][0]["windows"].as_array().unwrap();
    assert_eq!(
        windows
            .iter()
            .map(|w| w["toMs"].clone())
            .collect::<Vec<_>>(),
        [json!(15_000), json!(15_000)]
    );
    let total: u64 = windows[0]["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| bucket["inputTokens"].as_u64().unwrap())
        .sum();
    assert_eq!(
        total,
        1 + 2 + 3,
        "the buckets that closed after the cut are not counted"
    );
    // An unpinned read at the later time does see them.
    let unpinned = ask(&mut storage, 26_000, None);
    assert_eq!(unpinned["throughMs"], json!(25_000));
    // A cut that is off the resolution or later than the read is refused here too.
    for pin in [15_001, 25_000] {
        assert!(
            storage
                .consumption_history(
                    std::slice::from_ref(&id),
                    &[5000],
                    120,
                    22_500,
                    false,
                    Some(pin)
                )
                .is_err(),
            "{pin}"
        );
    }
}

#[test]
fn history_populated_windows_preserve_gaps_and_exact_boundaries() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    for (from, input, covered, incomplete, gap, discontinuous) in [
        (0, 1, 5000, false, false, false),
        (5000, 2, 5000, false, false, false),
        (10000, 3, 2000, true, true, false),
        (15000, 4, 5000, false, false, true),
        (20000, 99, 5000, false, false, false), // Open bucket is excluded.
    ] {
        Bucket {
            from,
            input,
            covered,
            incomplete,
            gap,
            discontinuous,
            ..Bucket::default()
        }
        .write(storage.connection().unwrap(), FINE, &id)
        .unwrap();
    }
    let response = storage
        .consumption_history(
            std::slice::from_ref(&id),
            &[15000, 5000],
            2,
            22500,
            false,
            None,
        )
        .unwrap();
    assert_eq!(
        response,
        json!({
            "asOfMs":22500,"throughMs":20000,"retainedFromMs":0,"resolutionMs":5000,
            "identities":[{"id":id,"found":true,"reporting":false,"availableFromMs":null,
            "lastSampleAtMs":null,"latest":null,"windows":[
                {"windowMs":15000,"fromMs":5000,"toMs":20000,"bucketMs":10000,"buckets":[
                    {"fromMs":5000,"toMs":15000,"inputTokens":5,"outputTokens":0,"cachedInputTokens":0,
                     "coveredMs":7000,"complete":false,"gap":true,"discontinuous":false},
                    {"fromMs":15000,"toMs":20000,"inputTokens":4,"outputTokens":0,"cachedInputTokens":0,
                     "coveredMs":5000,"complete":true,"gap":false,"discontinuous":true}
                ]},
                {"windowMs":5000,"fromMs":15000,"toMs":20000,"bucketMs":5000,"buckets":[
                    {"fromMs":15000,"toMs":20000,"inputTokens":4,"outputTokens":0,"cachedInputTokens":0,
                     "coveredMs":5000,"complete":true,"gap":false,"discontinuous":true}
                ]}
            ],"cause":"no_agent"}]
        })
    );
}

#[test]
fn history_corrupt_retained_rows_outside_requested_window_still_fail() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    Bucket {
        from: 0,
        sampled: Some(1000),
        snapshot: Some("invalid json".into()),
        ..Bucket::default()
    }
    .write(storage.connection().unwrap(), FINE, &id)
    .unwrap();
    let error = storage
        .consumption_history(std::slice::from_ref(&id), &[5000], 120, 20000, false, None)
        .unwrap_err();
    assert_eq!(error.code, StorageErrorCode::Corrupt);
    assert_eq!(error.message, "Invalid consumption history evidence");
}

#[test]
fn shared_seed_fixture_excludes_open_counter() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/consumption-history-v1.json"
    ))
    .unwrap();
    let directory = crate::test_support::TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("history.sqlite")).unwrap();
    let id = fixture["request"]["input"]["identityIds"][0]
        .as_str()
        .unwrap();
    storage.connection().unwrap().execute("INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES(?,'Meter','meter','t','t','saved')",[id]).unwrap();
    // Reducer fixture uses real schema constraints; caller admission is tested separately.
    let schema = storage.connection().unwrap();
    insert_binding(schema, id);
    schema.execute("INSERT INTO consumption_sources(identity_id,binding_id,driver,session) VALUES(?,'fixture-binding','claude','fixture-session')",[id]).unwrap();
    for reading in fixture["observations"].as_array().unwrap() {
        let latest: ConsumptionLatest = serde_json::from_value(json!({"driver":reading["driver"],"session":reading["session"],"consumption":reading["consumption"]})).unwrap();
        record_sample(
            schema,
            id,
            Some(latest),
            reading["sampleAtMs"].as_u64().unwrap(),
        )
        .unwrap();
    }
    let response = storage
        .consumption_history(&[id.to_owned()], &[15000], 120, 22500, false, None)
        .unwrap();
    assert_eq!(response, fixture["response"]);
    assert_eq!(
        response["identities"][0]["latest"]["consumption"]["inputTokens"],
        114
    );
    storage.close().unwrap();
}

pub(super) fn insert_binding(connection: &Connection, id: &str) {
    connection.execute("INSERT INTO bindings(id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at) VALUES('fixture-binding',?,'tmux','%1','fixture-server','/tmp/fixture-socket',10,'start',11,'t','t')",[id]).unwrap();
}
pub(super) fn fixture() -> (
    crate::test_support::TestDirectory,
    std::path::PathBuf,
    String,
) {
    let directory = crate::test_support::TestDirectory::new();
    let path = directory.path.join("history.db");
    let mut storage = Storage::open(&path).unwrap();
    let identity = tmt_core::identity::create_or_resolve(
        &mut storage,
        "Meter",
        tmt_core::identity::Lifetime::Saved,
    )
    .unwrap()
    .identity;
    insert_binding(storage.connection().unwrap(), &identity.id);
    storage.connection().unwrap().execute("INSERT INTO identity_session_preferences(identity_id,preferred_harness,remembered_harness,runtime_mode,provider_session_id) VALUES(?,'claude','claude','plain','fixture-session')",[&identity.id]).unwrap();
    storage.close().unwrap();
    (directory, path, identity.id)
}
pub(super) fn latest(input: u64, output: u64, sequence: u64, observed: u64) -> ConsumptionLatest {
    ConsumptionLatest {
        driver: "claude".into(),
        session: "fixture-session".into(),
        consumption: Consumption {
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: 0,
            cache_write_tokens: None,
            model_id: None,
            delta_by_model: None,
            rate_limits: None,
            credits: None,
            service_tier: None,
            epoch: "33333333-3333-4333-8333-333333333333".into(),
            sequence,
            observed_at_ms: observed,
            complete: true,
            gap: false,
        },
    }
}
pub(super) fn source(storage: &Storage, id: &str) {
    storage.connection().unwrap().execute("INSERT INTO consumption_sources(identity_id,binding_id,driver,session) VALUES(?,'fixture-binding','claude','fixture-session')",[id]).unwrap();
}
#[test]
fn duplicate_read_is_zero_and_failure_never_bridges_coverage() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let connection = storage.connection().unwrap();
    record_sample(connection, &id, Some(latest(100, 50, 1, 6000)), 6000).unwrap();
    let second = latest(110, 55, 2, 11000);
    record_sample(connection, &id, Some(second.clone()), 11000).unwrap();
    record_sample(connection, &id, Some(second), 12000).unwrap();
    record_sample(connection, &id, None, 13000).unwrap();
    let failed = storage
        .consumption_history(std::slice::from_ref(&id), &[10000], 120, 15000, false, None)
        .unwrap();
    assert_eq!(failed["identities"][0]["latest"], Value::Null);
    assert_eq!(failed["identities"][0]["lastSampleAtMs"], Value::Null);
    assert_eq!(
        failed["identities"][0]["windows"][0]["buckets"][1]["inputTokens"],
        10
    );
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(latest(114, 57, 3, 16000)),
        16000,
    )
    .unwrap();
    let history = storage
        .consumption_history(std::slice::from_ref(&id), &[15000], 120, 20000, false, None)
        .unwrap();
    let buckets = history["identities"][0]["windows"][0]["buckets"]
        .as_array()
        .unwrap();
    assert_eq!(
        buckets
            .iter()
            .map(|row| row["inputTokens"].as_u64().unwrap())
            .sum::<u64>(),
        10
    );
    assert_eq!(buckets[2]["coveredMs"], 0);
    assert_eq!(buckets[2]["gap"], true);
    storage.close().unwrap();
}
#[test]
fn rollback_discontinuity_and_expiry_are_fail_closed_and_bounded() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(latest(100, 50, 1, 6000)),
        6000,
    )
    .unwrap();
    assert!(
        record_sample(
            storage.connection().unwrap(),
            &id,
            Some(latest(100, 50, 1, 6000)),
            5999
        )
        .is_err()
    );
    let mut boundary = latest(90, 40, 1, 11000);
    boundary.session = "other-session".into();
    record_sample(storage.connection().unwrap(), &id, Some(boundary), 11000).unwrap();
    let history = storage
        .consumption_history(std::slice::from_ref(&id), &[10000], 120, 15000, false, None)
        .unwrap();
    let row = &history["identities"][0]["windows"][0]["buckets"][1];
    assert_eq!(row["discontinuous"], true);
    assert_eq!(row["inputTokens"], 0);
    for seq in 1..=1443 {
        let now = 15000 + seq * BUCKET_MS;
        record_sample(
            storage.connection().unwrap(),
            &id,
            Some(latest(seq, seq, seq, now)),
            now,
        )
        .unwrap();
    }
    let count: i64 = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM consumption_buckets WHERE identity_id=?",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1441);
    let now = 15000 + 1443 * BUCKET_MS + HISTORY_MS + BUCKET_MS;
    let cursor = storage.change_cursor().unwrap();
    let expired = storage
        .consumption_history(std::slice::from_ref(&id), &[3600000], 120, now, false, None)
        .unwrap();
    assert_eq!(expired["identities"][0]["reporting"], false);
    assert_eq!(expired["identities"][0]["latest"], Value::Null);
    assert_eq!(
        expired["identities"][0]["windows"][0]["buckets"]
            .as_array()
            .unwrap()
            .len(),
        120
    );
    assert_eq!(
        storage.change_cursor().unwrap(),
        cursor,
        "reads never prune or renew"
    );
    assert_eq!(
        storage
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM consumption_buckets", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1441
    );
    storage.close().unwrap();
}
#[test]
fn binding_and_preferences_cas_commits_counter_and_history_once() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    let original = Storage::context_by_identity(&path, &id, 6000)
        .unwrap()
        .unwrap();
    let commit = |storage: &mut Storage,
                  expected: &IdentityContextSnapshot,
                  preferences: &SessionPreferences,
                  now,
                  consumption| {
        storage
            .commit_runtime_observation(RuntimeObservation {
                boundary: None,
                expected,
                preferences,
                response: None,
                family: None,
                remember_source: true,
                locator: Some("project/session.jsonl"),
                sampled: true,
                consumption: Some(consumption),
                now_ms: now,
                deadline: Instant::now() + std::time::Duration::from_secs(1),
            })
            .unwrap()
    };
    assert!(commit(
        &mut storage,
        &original,
        &original.preferences,
        6000,
        latest(100, 50, 1, 6000).consumption
    ));
    let expected = Storage::context_by_identity(&path, &id, 11000)
        .unwrap()
        .unwrap();
    let mut changed = expected.preferences.clone();
    changed.remembered.as_mut().unwrap().state =
        Some(tmt_core::binding::session::DriverState::new(1, "{\"model\":\"sampled\"}").unwrap());
    assert!(commit(
        &mut storage,
        &expected,
        &changed,
        11000,
        latest(110, 55, 2, 11000).consumption
    ));
    assert!(
        !commit(
            &mut storage,
            &expected,
            &changed,
            12000,
            latest(110, 55, 2, 11000).consumption
        ),
        "racing Stop loses stale cursor CAS"
    );
    let fresh = Storage::context_by_identity(&path, &id, 16000)
        .unwrap()
        .unwrap();
    assert!(commit(
        &mut storage,
        &fresh,
        &fresh.preferences,
        16000,
        latest(110, 55, 2, 11000).consumption
    ));
    let history = storage
        .consumption_history(std::slice::from_ref(&id), &[15000], 120, 20000, false, None)
        .unwrap();
    let sum: u64 = history["identities"][0]["windows"][0]["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["inputTokens"].as_u64().unwrap())
        .sum();
    assert_eq!(sum, 10);
    storage
        .connection()
        .unwrap()
        .execute("UPDATE bindings SET pane_pid=12", [])
        .unwrap();
    assert!(
        !commit(
            &mut storage,
            &fresh,
            &fresh.preferences,
            17000,
            latest(120, 60, 3, 17000).consumption
        ),
        "rebound pane loses full binding CAS"
    );
    storage.close().unwrap();
}

#[test]
fn first_gap_baseline_retains_known_following_delta_without_claiming_coverage() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let mut baseline = latest(0, 0, 1, 6000);
    baseline.consumption.gap = true;
    baseline.consumption.complete = false;
    record_sample(storage.connection().unwrap(), &id, Some(baseline), 6000).unwrap();
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(latest(10, 5, 2, 11000)),
        11000,
    )
    .unwrap();
    let result = storage
        .consumption_history(std::slice::from_ref(&id), &[10000], 120, 15000, false, None)
        .unwrap();
    let row = &result["identities"][0]["windows"][0]["buckets"][1];
    assert_eq!(row["inputTokens"], 10);
    assert_eq!(row["outputTokens"], 5);
    assert_eq!(row["coveredMs"], 0);
    assert_eq!(row["complete"], false);
    storage.close().unwrap();
}

#[test]
fn invalid_cached_delta_creates_gap_instead_of_freezing_future_reads() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(latest(10, 5, 1, 6000)),
        6000,
    )
    .unwrap();
    let mut adjusted = latest(10, 6, 2, 11000);
    adjusted.consumption.cached_input_tokens = 1;
    record_sample(storage.connection().unwrap(), &id, Some(adjusted), 11000).unwrap();
    let mut next = latest(14, 8, 3, 16000);
    next.consumption.cached_input_tokens = 1;
    record_sample(storage.connection().unwrap(), &id, Some(next), 16000).unwrap();
    let history = storage
        .consumption_history(std::slice::from_ref(&id), &[15000], 120, 20000, false, None)
        .unwrap();
    let rows = history["identities"][0]["windows"][0]["buckets"]
        .as_array()
        .unwrap();
    assert_eq!(rows[1]["gap"], true);
    // The correction establishes a new baseline at 11s; the next valid read
    // proves 11..15s, while 10..11s remains unknown in this closed bucket.
    assert_eq!(rows[1]["coveredMs"], 4000);
    assert_eq!(rows[1]["complete"], false);
    assert_eq!(rows[2]["inputTokens"], 4);
    assert_eq!(rows[2]["outputTokens"], 2);
    storage.close().unwrap();
}

pub(super) fn attributed(
    input: u64,
    output: u64,
    write: u64,
    sequence: u64,
    observed: u64,
    rows: Vec<ModelUsage>,
) -> ConsumptionLatest {
    let mut reading = latest(input, output, sequence, observed);
    reading.consumption.cache_write_tokens = Some(write);
    reading.consumption.model_id = rows.last().map(|row| row.model_id.clone());
    reading.consumption.delta_by_model = Some(rows);
    reading
}
pub(super) fn model_row(model: &str, input: u64, output: u64, write: Option<u64>) -> ModelUsage {
    ModelUsage {
        model_id: model.into(),
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: 0,
        cache_write_tokens: write,
    }
}

#[test]
fn model_changes_in_one_sample_and_bucket_survive_reopen_aggregation_and_replay() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(attributed(0, 0, 0, 1, 6000, vec![])),
        6000,
    )
    .unwrap();
    let batch = attributed(
        30,
        5,
        9,
        2,
        11000,
        vec![
            model_row("a", 10, 2, Some(3)),
            model_row("b", 20, 3, Some(6)),
        ],
    );
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(batch.clone()),
        11000,
    )
    .unwrap();
    record_sample(storage.connection().unwrap(), &id, Some(batch), 11500).unwrap();
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(attributed(
            40,
            7,
            12,
            3,
            12000,
            vec![model_row("a", 10, 2, Some(3))],
        )),
        12000,
    )
    .unwrap();
    storage.close().unwrap();
    let mut storage = Storage::open(&path).unwrap();
    let history = storage
        .consumption_history(std::slice::from_ref(&id), &[10000], 1, 15000, false, None)
        .unwrap();
    let bucket = &history["identities"][0]["windows"][0]["buckets"][0];
    assert_eq!(bucket["inputTokens"], 40);
    assert_eq!(bucket["outputTokens"], 7);
    assert_eq!(bucket["cacheWriteTokens"], 12);
    assert_eq!(
        bucket["byModel"],
        json!([
            {"modelId":"a","inputTokens":20,"outputTokens":4,"cachedInputTokens":0,"cacheWriteTokens":6},
            {"modelId":"b","inputTokens":20,"outputTokens":3,"cachedInputTokens":0,"cacheWriteTokens":6}
        ])
    );
    assert_eq!(
        history["identities"][0]["latest"]["consumption"]["modelId"],
        "a"
    );
}

#[test]
fn unknown_attribution_never_labels_the_whole_mixed_bucket_with_the_last_model() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(latest(0, 0, 1, 6000)),
        6000,
    )
    .unwrap();
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(latest(10, 2, 2, 11000)),
        11000,
    )
    .unwrap();
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(attributed(
            20,
            4,
            3,
            3,
            12000,
            vec![model_row("a", 10, 2, Some(3))],
        )),
        12000,
    )
    .unwrap();
    let history = storage
        .consumption_history(std::slice::from_ref(&id), &[5000], 120, 15000, false, None)
        .unwrap();
    let bucket = &history["identities"][0]["windows"][0]["buckets"][0];
    assert_eq!(bucket["inputTokens"], 20);
    assert_eq!(bucket["outputTokens"], 4);
    // Only the attributed part carries a model; the other 10 + 2 stay unattributed.
    assert_eq!(
        bucket["byModel"],
        json!([{"modelId":"a","inputTokens":10,"outputTokens":2,"cachedInputTokens":0,"cacheWriteTokens":3}])
    );
    assert!(bucket.get("cacheWriteTokens").is_none());
    assert_eq!(
        history["identities"][0]["latest"]["consumption"]["modelId"],
        "a"
    );
}

/// The same fixture counters as `latest`, read from another driver's session.
fn from_driver(
    driver: &str,
    input: u64,
    output: u64,
    sequence: u64,
    observed: u64,
) -> ConsumptionLatest {
    let mut reading = latest(input, output, sequence, observed);
    reading.driver = driver.into();
    reading.session = format!("{driver}-session");
    reading
}

fn only_bucket(storage: &mut Storage, id: &str, window: u64, now: u64) -> Value {
    let history = storage
        .consumption_history(&[id.to_owned()], &[window], 120, now, false, None)
        .unwrap();
    history["identities"][0]["windows"][0]["buckets"][0].clone()
}

#[test]
fn a_driver_switch_inside_one_bucket_splits_its_deltas_by_driver() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let connection = storage.connection().unwrap();
    // One 5-second bucket: claude counts, the seat switches to codex (a boundary
    // that counts nothing), then codex counts.
    record_sample(
        connection,
        &id,
        Some(from_driver("claude", 0, 0, 1, 5100)),
        5100,
    )
    .unwrap();
    record_sample(
        connection,
        &id,
        Some(from_driver("claude", 10, 2, 2, 6000)),
        6000,
    )
    .unwrap();
    record_sample(
        connection,
        &id,
        Some(from_driver("codex", 500, 90, 1, 7000)),
        7000,
    )
    .unwrap();
    record_sample(
        connection,
        &id,
        Some(from_driver("codex", 530, 96, 2, 8000)),
        8000,
    )
    .unwrap();
    let bucket = only_bucket(&mut storage, &id, 5000, 10_500);
    assert_eq!(bucket["inputTokens"], 40);
    assert_eq!(bucket["outputTokens"], 8);
    assert_eq!(bucket["discontinuous"], true);
    assert_eq!(
        bucket["byDriver"],
        json!([
            {"driver":"claude","inputTokens":10,"outputTokens":2,"cachedInputTokens":0},
            {"driver":"codex","inputTokens":30,"outputTokens":6,"cachedInputTokens":0}
        ])
    );
}

#[test]
fn a_restart_gap_keeps_the_older_tokens_under_their_driver() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let connection = storage.connection().unwrap();
    record_sample(
        connection,
        &id,
        Some(from_driver("claude", 0, 0, 1, 6000)),
        6000,
    )
    .unwrap();
    record_sample(
        connection,
        &id,
        Some(from_driver("claude", 10, 2, 2, 11000)),
        11000,
    )
    .unwrap();
    // A failed read, then a new epoch (a restart): a gap and a boundary, no delta.
    record_sample(connection, &id, None, 16000).unwrap();
    let mut restarted = from_driver("claude", 3, 1, 1, 21000);
    restarted.consumption.epoch = "44444444-4444-4444-8444-444444444444".into();
    record_sample(connection, &id, Some(restarted), 21000).unwrap();
    let mut counted = from_driver("claude", 8, 2, 2, 26000);
    counted.consumption.epoch = "44444444-4444-4444-8444-444444444444".into();
    record_sample(connection, &id, Some(counted), 26000).unwrap();
    let history = storage
        .consumption_history(
            std::slice::from_ref(&id),
            &[25000],
            120,
            30_500,
            false,
            None,
        )
        .unwrap();
    let buckets = history["identities"][0]["windows"][0]["buckets"]
        .as_array()
        .unwrap();
    let share = |from: u64| {
        buckets
            .iter()
            .find(|bucket| bucket["fromMs"] == from)
            .unwrap()
            .clone()
    };
    let claude = |input: u64, output: u64| json!([{"driver":"claude","inputTokens":input,"outputTokens":output,"cachedInputTokens":0}]);
    assert_eq!(share(10_000)["byDriver"], claude(10, 2));
    assert_eq!(share(25_000)["byDriver"], claude(5, 1));
    // The restart itself counts nothing and carries no attribution to invent.
    assert!(share(20_000).get("byDriver").is_none());
    assert_eq!(share(20_000)["discontinuous"], false);
    assert!(
        buckets
            .iter()
            .any(|bucket| bucket["gap"] == true && bucket.get("byDriver").is_none())
    );
}

#[test]
fn rows_counted_before_drivers_were_recorded_stay_unattributed() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    // A bucket written before this field existed holds 100 tokens with no split.
    Bucket {
        from: 5000,
        input: 100,
        ..Bucket::default()
    }
    .write(storage.connection().unwrap(), FINE, &id)
    .unwrap();
    let connection = storage.connection().unwrap();
    record_sample(
        connection,
        &id,
        Some(from_driver("claude", 0, 0, 1, 6000)),
        6000,
    )
    .unwrap();
    record_sample(
        connection,
        &id,
        Some(from_driver("claude", 10, 2, 2, 7000)),
        7000,
    )
    .unwrap();
    let bucket = only_bucket(&mut storage, &id, 5000, 10_500);
    assert_eq!(bucket["inputTokens"], 110);
    assert_eq!(
        bucket["byDriver"],
        json!([{"driver":"claude","inputTokens":10,"outputTokens":2,"cachedInputTokens":0}])
    );
}

#[test]
fn a_stored_split_that_overcounts_or_is_not_canonical_is_corrupt() {
    let row = |driver: &str, input: u64| DriverUsage {
        driver: driver.into(),
        input_tokens: input,
        output_tokens: 0,
        cached_input_tokens: 0,
    };
    let write = |rows: Vec<DriverUsage>| {
        let (_directory, path, id) = fixture();
        let storage = Storage::open(&path).unwrap();
        Bucket {
            from: 5000,
            input: 10,
            details: Details {
                by_driver: Some(rows),
                ..Details::default()
            },
            ..Bucket::default()
        }
        .write(storage.connection().unwrap(), FINE, &id)
    };
    assert!(write(vec![row("claude", 4), row("codex", 6)]).is_ok());
    assert!(write(vec![row("claude", 4)]).is_ok());
    assert!(write(vec![row("claude", 11)]).is_err());
    assert!(write(vec![row("claude", 6), row("codex", 6)]).is_err());
    assert!(write(vec![row("codex", 1), row("claude", 1)]).is_err());
    assert!(write(vec![row("claude", 1), row("claude", 1)]).is_err());
    assert!(write(vec![row("Claude", 1)]).is_err());
    assert!(write(Vec::new()).is_err());
}

#[test]
fn recent_responses_share_the_binding_cas_without_text_deduplication() {
    use crate::storage::ResponseCapture;
    use tmt_core::responses::FinalText;
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    let first = Storage::context_by_identity(&path, &id, 6000)
        .unwrap()
        .unwrap();
    let commit = |storage: &mut Storage,
                  expected: &IdentityContextSnapshot,
                  state: &str,
                  now,
                  text: Option<&str>| {
        let mut preferences = expected.preferences.clone();
        preferences.remembered.as_mut().unwrap().state =
            Some(tmt_core::binding::session::DriverState::new(1, state).unwrap());
        storage
            .commit_runtime_observation(RuntimeObservation {
                boundary: None,
                expected,
                preferences: &preferences,
                remember_source: false,
                locator: None,
                sampled: false,
                consumption: None,
                now_ms: now,
                family: None,
                deadline: Instant::now() + std::time::Duration::from_secs(1),
                response: text.map(|text| ResponseCapture {
                    text: FinalText::new(text).unwrap(),
                }),
            })
            .unwrap()
    };
    assert!(commit(
        &mut storage,
        &first,
        r#"{"model":"turn-one"}"#,
        6000,
        Some("identical final text")
    ));
    assert!(
        !commit(
            &mut storage,
            &first,
            r#"{"model":"turn-one"}"#,
            6000,
            Some("identical final text")
        ),
        "duplicate stale Stop loses the full CAS"
    );
    let second = Storage::context_by_identity(&path, &id, 7000)
        .unwrap()
        .unwrap();
    // A fresh unchanged observation remains valid for sampling, but never captures.
    assert!(commit(
        &mut storage,
        &second,
        r#"{"model":"turn-one"}"#,
        7000,
        Some("duplicate Stop text")
    ));
    assert!(commit(
        &mut storage,
        &second,
        r#"{"model":"turn-two"}"#,
        8000,
        Some("identical final text")
    ));
    let before = storage
        .connection()
        .unwrap()
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    let value = storage.recent_responses(&id, 10, None, 9000).unwrap();
    assert_eq!(value["items"].as_array().unwrap().len(), 2);
    assert_eq!(value["items"][0]["text"], "identical final text");
    assert_eq!(value["items"][1]["text"], "identical final text");
    assert_eq!(value["items"][0]["endedMs"], 8000);
    assert_eq!(value["items"][1]["endedMs"], 6000);
    assert_eq!(value["items"][0]["provider"], "claude");
    assert_eq!(value["items"][0]["sessionId"], "fixture-session");
    assert_eq!(
        before,
        storage
            .connection()
            .unwrap()
            .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        "reads do not advance the cursor"
    );
    assert_eq!(
        storage
            .recent_responses(&id, 10, None, 6000 + tmt_core::responses::RETENTION_MS)
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "expiry is hidden at equality without cleanup"
    );
    assert_eq!(
        storage
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM recent_responses", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let old_binding = Storage::context_by_identity(&path, &id, 10000)
        .unwrap()
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute("UPDATE bindings SET pane_pid=12", [])
        .unwrap();
    assert!(!commit(
        &mut storage,
        &old_binding,
        r#"{"model":"replaced"}"#,
        10000,
        Some("must not commit")
    ));
    storage.close().unwrap();
}

#[test]
fn recent_responses_keep_by_age_in_capture_order_and_retirement_removes_them() {
    use crate::storage::ResponseCapture;
    use tmt_core::responses::FinalText;
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    for sequence in 0..12 {
        let expected = Storage::context_by_identity(&path, &id, 20000)
            .unwrap()
            .unwrap();
        let mut preferences = expected.preferences.clone();
        preferences.remembered.as_mut().unwrap().state = Some(
            tmt_core::binding::session::DriverState::new(
                1,
                &format!(r#"{{"model":"turn-{sequence}"}}"#),
            )
            .unwrap(),
        );
        assert!(
            storage
                .commit_runtime_observation(RuntimeObservation {
                    boundary: None,
                    expected: &expected,
                    preferences: &preferences,
                    remember_source: false,
                    locator: None,
                    sampled: false,
                    consumption: None,
                    now_ms: 20000 - sequence,
                    family: None,
                    deadline: Instant::now() + std::time::Duration::from_secs(1),
                    response: Some(ResponseCapture {
                        text: FinalText::new(&format!("response-{sequence}")).unwrap(),
                    }),
                })
                .unwrap()
        );
    }
    // A page of ten is the newest ten in capture order; the cursor reads the rest.
    let value = storage.recent_responses(&id, 10, None, 20001).unwrap();
    assert_eq!(value["items"].as_array().unwrap().len(), 10);
    assert_eq!(value["items"][0]["text"], "response-11");
    assert_eq!(value["items"][9]["text"], "response-2");
    assert_eq!(value["coverage"], json!({"olderDropped": false}));
    let rest = storage
        .recent_responses(&id, 10, value["nextBefore"].as_u64(), 20001)
        .unwrap();
    assert_eq!(rest["items"].as_array().unwrap().len(), 2);
    assert_eq!(rest["items"][1]["text"], "response-0");
    assert!(
        rest.get("nextBefore").is_none(),
        "the last page has no cursor"
    );
    assert_eq!(
        storage.recent_responses(&id, 1, None, 20001).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Every answer lives the same 7 days, so age alone drops the oldest and the read says so.
    let aged = storage
        .recent_responses(&id, 50, None, 19995 + tmt_core::responses::RETENTION_MS)
        .unwrap();
    assert_eq!(aged["items"].as_array().unwrap().len(), 5);
    assert_eq!(
        aged["coverage"],
        json!({"olderDropped": true, "droppedThroughMs": 19995})
    );
    let unknown = "11111111-1111-4111-8111-111111111111";
    assert_eq!(
        storage.recent_responses(unknown, 10, None, 20001).unwrap()["items"],
        serde_json::json!([])
    );
    let identity = storage.find_identity_by_id(&id).unwrap().unwrap();
    use tmt_core::binding::BindingRecords;
    BindingRows(storage.connection().unwrap())
        .retire_identity(&identity, false)
        .unwrap();
    assert_eq!(
        storage.recent_responses(&id, 10, None, 20001).unwrap()["items"],
        serde_json::json!([])
    );
    assert_eq!(
        storage
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM recent_responses", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    storage.close().unwrap();
}

#[test]
fn recent_response_failure_rolls_back_preferences_and_the_change_cursor() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    let expected = Storage::context_by_identity(&path, &id, 6000)
        .unwrap()
        .unwrap();
    let mut preferences = expected.preferences.clone();
    preferences.remembered.as_mut().unwrap().state = Some(
        tmt_core::binding::session::DriverState::new(1, r#"{"model":"must roll back"}"#).unwrap(),
    );
    let cursor = || {
        storage
            .connection()
            .unwrap()
            .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    let before = cursor();
    let result = storage.commit_runtime_observation(RuntimeObservation {
        boundary: None,
        expected: &expected,
        preferences: &preferences,
        remember_source: true,
        locator: Some("project/session.jsonl"),
        sampled: false,
        consumption: None,
        now_ms: tmt_core::limits::MAX_JS_SAFE_INTEGER,
        family: None,
        deadline: Instant::now() + std::time::Duration::from_secs(1),
        response: Some(crate::storage::ResponseCapture {
            text: tmt_core::responses::FinalText::new("must roll back").unwrap(),
        }),
    });
    assert!(result.is_err());
    let actual = Storage::context_by_identity(&path, &id, 6000)
        .unwrap()
        .unwrap();
    assert_eq!(actual.preferences, expected.preferences);
    let after = storage
        .connection()
        .unwrap()
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    assert_eq!(after, before);
    assert_eq!(
        storage.recent_responses(&id, 10, None, 6000).unwrap()["items"],
        json!([])
    );
    storage.close().unwrap();
}

#[test]
fn an_identity_that_reports_nothing_says_why_from_its_binding_alone() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    let cause = |storage: &mut Storage| {
        let history = storage
            .consumption_history(std::slice::from_ref(&id), &[5000], 120, 20000, false, None)
            .unwrap();
        let entry = history["identities"][0].clone();
        assert_eq!(entry["found"], true);
        assert_eq!(entry["reporting"], false);
        entry.get("cause").cloned()
    };
    let update = |storage: &Storage, sql: &str| {
        storage.connection().unwrap().execute(sql, [&id]).unwrap();
    };
    // A bound pane with no admitted agent session: a shell, a Herdr pane, or an
    // agent whose start hook never admitted its session.
    assert_eq!(cause(&mut storage), Some(json!("no_agent")));
    // Running, but only observed as a plain pane process: still no admitted session.
    update(
        &storage,
        "UPDATE bindings SET runtime_state='running', runtime_pid=12, runtime_start_identity='s' WHERE identity_id=?",
    );
    assert_eq!(cause(&mut storage), Some(json!("no_agent")));
    // An admitted running session with no foreground owner is Stop-only.
    update(
        &storage,
        "UPDATE bindings SET observed_provider_session_id='session' WHERE identity_id=?",
    );
    assert_eq!(cause(&mut storage), Some(json!("no_sampler")));
    // Owned and admitted, yet nothing sampled: hook missing, wrapper too old, or too new.
    update(
        &storage,
        "UPDATE bindings SET launch_owner_pid=13, launch_owner_start_identity='o' WHERE identity_id=?",
    );
    assert_eq!(cause(&mut storage), Some(json!("not_sampled")));
    // No binding at all: offline or never bound.
    update(&storage, "DELETE FROM bindings WHERE identity_id=?");
    assert_eq!(cause(&mut storage), Some(json!("no_binding")));
}

#[test]
fn a_reporting_or_unknown_identity_has_no_cause() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    record_sample(
        storage.connection().unwrap(),
        &id,
        Some(latest(10, 5, 1, 6000)),
        6000,
    )
    .unwrap();
    let missing = "ffffffff-ffff-4fff-8fff-ffffffffffff".to_owned();
    let history = storage
        .consumption_history(&[id.clone(), missing], &[5000], 120, 20000, false, None)
        .unwrap();
    assert_eq!(history["identities"][0]["reporting"], true);
    assert!(history["identities"][0].get("cause").is_none());
    assert_eq!(history["identities"][1]["found"], false);
    assert!(history["identities"][1].get("cause").is_none());
}

#[test]
fn a_skipped_sequence_attributes_only_its_last_step_and_overcounting_rows_attribute_nothing() {
    for (rows, expected) in [
        // Sequence 1 to 3: the delta is (30, 6) but the rows cover the last step.
        (vec![model_row("a", 10, 2, None)], Some((10, 2))),
        (vec![model_row("a", 50, 5, None)], None),
        (vec![model_row("a", 10, 9, None)], None),
        (vec![], None),
    ] {
        let (_directory, path, id) = fixture();
        let mut storage = Storage::open(&path).unwrap();
        source(&storage, &id);
        let connection = storage.connection().unwrap();
        record_sample(connection, &id, Some(latest(100, 20, 1, 6000)), 6000).unwrap();
        let mut read = latest(130, 26, 3, 7000);
        read.consumption.delta_by_model = Some(rows.clone());
        record_sample(connection, &id, Some(read), 7000).unwrap();
        let bucket = only_bucket(&mut storage, &id, 5000, 10_500);
        assert_eq!(bucket["inputTokens"], 30, "{rows:?}");
        match expected {
            Some((input, output)) => assert_eq!(
                bucket["byModel"],
                json!([{"modelId":"a","inputTokens":input,"outputTokens":output,"cachedInputTokens":0}])
            ),
            None => assert!(bucket.get("byModel").is_none(), "{rows:?}"),
        }
    }
}

#[test]
fn a_window_keeps_the_attributed_models_and_leaves_the_rest_unattributed() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    source(&storage, &id);
    let connection = storage.connection().unwrap();
    record_sample(connection, &id, Some(latest(0, 0, 1, 6000)), 6000).unwrap();
    // Three 5-second buckets: model a, no attribution, model b.
    let mut read = latest(10, 2, 2, 11000);
    read.consumption.delta_by_model = Some(vec![model_row("a", 10, 2, None)]);
    record_sample(connection, &id, Some(read), 11000).unwrap();
    record_sample(connection, &id, Some(latest(30, 6, 3, 16000)), 16000).unwrap();
    let mut read = latest(35, 7, 4, 21000);
    read.consumption.delta_by_model = Some(vec![model_row("b", 5, 1, None)]);
    record_sample(connection, &id, Some(read), 21000).unwrap();
    let history = storage
        .consumption_history(std::slice::from_ref(&id), &[15000], 1, 25_500, false, None)
        .unwrap();
    let bucket = &history["identities"][0]["windows"][0]["buckets"][0];
    assert_eq!(bucket["inputTokens"], 35);
    assert_eq!(bucket["outputTokens"], 7);
    assert_eq!(
        bucket["byModel"],
        json!([
            {"modelId":"a","inputTokens":10,"outputTokens":2,"cachedInputTokens":0},
            {"modelId":"b","inputTokens":5,"outputTokens":1,"cachedInputTokens":0}
        ])
    );
}

#[test]
fn a_fifth_model_drops_the_split_built_so_far_and_a_later_one_starts_again() {
    use crate::storage::consumption_history::details::Details;
    let split = |ids: &[&str]| Details {
        by_model: Some(ids.iter().map(|id| model_row(id, 1, 0, None)).collect()),
        ..Details::default()
    };
    let mut held = split(&["a", "b", "c", "d"]);
    held.merge(&split(&["a"])).unwrap();
    assert_eq!(held.by_model.as_ref().unwrap()[0].input_tokens, 2);
    held.merge(&split(&["e"])).unwrap();
    assert!(held.by_model.is_none());
    // The at-most rule still holds, so a later split is a partial one.
    held.merge(&split(&["e"])).unwrap();
    assert_eq!(held.by_model.as_ref().unwrap().len(), 1);
    // A side without models leaves the other's share as it was.
    held.merge(&Details::default()).unwrap();
    assert_eq!(held.by_model.as_ref().unwrap()[0].model_id, "e");
    assert!(held.valid(1, 0, 0));
    assert!(!held.valid(0, 0, 0), "rows above the bucket are corrupt");
}

mod family {
    use super::*;
    use crate::runtime::consumption::{
        State,
        family::{Header, Member, Stored, Write},
    };

    fn file(n: u32) -> String {
        format!(
            "2026/10/19/rollout-2026-10-19T09-10-{n:02}-00000000-0000-4000-8000-0000000000{n:02}.jsonl"
        )
    }

    fn header(session: &str) -> Header {
        Header {
            session: session.into(),
            scan: Some("2026-10-19T09-10-00".into()),
            edge: vec![file(0)],
            closed: serde_json::from_value(json!({"input":5,"output":1,"cached":0})).unwrap(),
            warm: false,
        }
    }

    fn member(n: u32, input: u64) -> Member {
        let child = format!("00000000-0000-4000-8000-0000000000{n:02}");
        Member {
            file: file(n),
            child,
            parent: "00000000-0000-4000-8000-0000000000ff".into(),
            base: serde_json::from_value(json!({"input":1,"output":0,"cached":0})).unwrap(),
            state: State::read(&json!({
                "value":{"inputTokens":input,"outputTokens":1,"cachedInputTokens":0,
                    "epoch":"33333333-3333-4333-8333-333333333333","sequence":2,
                    "observedAtMs":7000,"complete":true,"gap":false},
                "cursor":{"dev":1,"ino":2,"offset":900}
            }))
            .unwrap(),
            touched_ms: 7000,
        }
    }

    fn write(
        storage: &Storage,
        id: &str,
        reset: bool,
        header: Option<Header>,
        upserts: Vec<Member>,
        deletes: Vec<String>,
    ) {
        super::super::super::consumption_family::apply(
            storage.connection().unwrap(),
            id,
            &Write {
                reset,
                header,
                upserts,
                deletes,
            },
        )
        .unwrap();
    }

    #[test]
    fn a_family_round_trips_updates_in_place_and_is_replaced_by_a_reset() {
        let (_directory, path, id) = fixture();
        let mut storage = Storage::open(&path).unwrap();
        assert_eq!(storage.consumption_family(&id).unwrap(), None);
        source(&storage, &id);
        write(
            &storage,
            &id,
            false,
            Some(header("fixture-session")),
            vec![member(1, 100), member(2, 200)],
            vec![],
        );
        let stored = storage.consumption_family(&id).unwrap().unwrap();
        assert_eq!(stored.header, header("fixture-session"));
        assert_eq!(
            stored
                .members
                .iter()
                .map(|m| m.file.as_str())
                .collect::<Vec<_>>(),
            [file(1), file(2)]
        );
        // One row moves, one is folded away; the unchanged header is not rewritten.
        let cursor = storage.change_cursor().unwrap();
        write(
            &storage,
            &id,
            false,
            None,
            vec![member(1, 150)],
            vec![file(2)],
        );
        assert!(storage.change_cursor().unwrap() > cursor);
        let stored = storage.consumption_family(&id).unwrap().unwrap();
        assert_eq!(stored.members, [member(1, 150)]);
        let cursor = storage.change_cursor().unwrap();
        write(
            &storage,
            &id,
            false,
            Some(header("fixture-session")),
            vec![member(1, 150)],
            vec![],
        );
        assert_eq!(
            storage.change_cursor().unwrap(),
            cursor,
            "an identical write changes nothing"
        );
        // Another session replaces the whole family, rows included.
        write(
            &storage,
            &id,
            true,
            Some(header("next-session")),
            vec![member(3, 10)],
            vec![],
        );
        let stored: Stored = storage.consumption_family(&id).unwrap().unwrap();
        assert_eq!(stored.header.session, "next-session");
        assert_eq!(stored.members, [member(3, 10)]);
        storage.close().unwrap();
    }

    #[test]
    fn a_family_goes_with_its_source_and_a_row_that_does_not_read_back_is_corrupt() {
        let (_directory, path, id) = fixture();
        let mut storage = Storage::open(&path).unwrap();
        source(&storage, &id);
        write(
            &storage,
            &id,
            false,
            Some(header("fixture-session")),
            vec![member(1, 100)],
            vec![],
        );
        let connection = storage.connection().unwrap();
        connection
            .execute(
                "UPDATE consumption_family_files SET state='{\"value\":1}' WHERE identity_id=?",
                [&id],
            )
            .unwrap();
        assert_eq!(
            storage.consumption_family(&id).unwrap_err().code,
            StorageErrorCode::Corrupt
        );
        connection
            .execute(
                "UPDATE consumption_family SET edge='[\"../escape\"]' WHERE identity_id=?",
                [&id],
            )
            .unwrap();
        assert_eq!(
            storage.consumption_family(&id).unwrap_err().code,
            StorageErrorCode::Corrupt
        );
        // The source is the owner: its removal takes the family and its rows.
        connection
            .execute("DELETE FROM consumption_sources WHERE identity_id=?", [&id])
            .unwrap();
        for table in ["consumption_family", "consumption_family_files"] {
            let rows: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0, "{table}");
        }
        storage.close().unwrap();
    }

    #[test]
    fn an_invalid_write_is_refused_and_the_other_seats_sessions_exclude_the_own() {
        let (_directory, path, id) = fixture();
        let mut storage = Storage::open(&path).unwrap();
        source(&storage, &id);
        let mut bad = member(1, 100);
        bad.file = "../escape.jsonl".into();
        let connection = storage.connection().unwrap();
        assert!(
            super::super::super::consumption_family::apply(
                connection,
                &id,
                &Write {
                    reset: false,
                    header: Some(header("fixture-session")),
                    upserts: vec![bad],
                    deletes: vec![],
                },
            )
            .is_err()
        );
        assert_eq!(
            storage
                .other_bound_sessions("someone-else", "claude")
                .unwrap(),
            ["fixture-session".to_owned()].into_iter().collect()
        );
        assert!(
            storage
                .other_bound_sessions(&id, "claude")
                .unwrap()
                .is_empty()
        );
        assert!(
            storage
                .other_bound_sessions("someone-else", "codex")
                .unwrap()
                .is_empty()
        );
        storage.close().unwrap();
    }
}
