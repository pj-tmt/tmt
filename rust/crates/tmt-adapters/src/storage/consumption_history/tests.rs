use super::*;

// Literal expectations characterize the pre-optimization public projection.
#[test]
fn history_empty_and_missing_keep_complete_wire_shape() {
    let (_directory, path, id) = fixture();
    let mut storage = Storage::open(&path).unwrap();
    let response = storage
        .consumption_history(&[id.clone(), "missing".into()], &[10000], 120, 15001, false)
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
                 ]},
                {"id":"missing","found":false}
            ]
        })
    );
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
        .consumption_history(std::slice::from_ref(&id), &[15000, 5000], 2, 22500, false)
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
            ]}]
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
        .consumption_history(std::slice::from_ref(&id), &[5000], 120, 20000, false)
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
        .consumption_history(&[id.to_owned()], &[15000], 120, 22500, false)
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
        .consumption_history(std::slice::from_ref(&id), &[10000], 120, 15000, false)
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
        .consumption_history(std::slice::from_ref(&id), &[15000], 120, 20000, false)
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
        .consumption_history(std::slice::from_ref(&id), &[10000], 120, 15000, false)
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
        .consumption_history(std::slice::from_ref(&id), &[3600000], 120, now, false)
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
                expected,
                preferences,
                response: None,
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
        .consumption_history(std::slice::from_ref(&id), &[15000], 120, 20000, false)
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
        .consumption_history(std::slice::from_ref(&id), &[10000], 120, 15000, false)
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
        .consumption_history(std::slice::from_ref(&id), &[15000], 120, 20000, false)
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
        .consumption_history(std::slice::from_ref(&id), &[10000], 1, 15000, false)
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
        .consumption_history(std::slice::from_ref(&id), &[5000], 120, 15000, false)
        .unwrap();
    let bucket = &history["identities"][0]["windows"][0]["buckets"][0];
    assert_eq!(bucket["inputTokens"], 20);
    assert_eq!(bucket["outputTokens"], 4);
    assert!(bucket.get("byModel").is_none());
    assert!(bucket.get("cacheWriteTokens").is_none());
    assert_eq!(
        history["identities"][0]["latest"]["consumption"]["modelId"],
        "a"
    );
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
                expected,
                preferences: &preferences,
                remember_source: false,
                locator: None,
                sampled: false,
                consumption: None,
                now_ms: now,
                deadline: Instant::now() + std::time::Duration::from_secs(1),
                response: text.map(|text| ResponseCapture {
                    text: FinalText::new(text).unwrap(),
                    retention_days: 1,
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
    let value = storage.recent_responses(&id, 10, 9000).unwrap();
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
            .recent_responses(&id, 10, 6000 + 86_400_000)
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
fn recent_responses_keep_ten_in_capture_order_and_retirement_removes_them() {
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
                    expected: &expected,
                    preferences: &preferences,
                    remember_source: false,
                    locator: None,
                    sampled: false,
                    consumption: None,
                    now_ms: 20000 - sequence,
                    deadline: Instant::now() + std::time::Duration::from_secs(1),
                    response: Some(ResponseCapture {
                        text: FinalText::new(&format!("response-{sequence}")).unwrap(),
                        retention_days: if sequence == 11 { 2 } else { 1 }
                    }),
                })
                .unwrap()
        );
    }
    let value = storage.recent_responses(&id, 10, 20001).unwrap();
    assert_eq!(value["items"].as_array().unwrap().len(), 10);
    assert_eq!(value["items"][0]["text"], "response-11");
    assert_eq!(value["items"][9]["text"], "response-2");
    assert_eq!(
        storage.recent_responses(&id, 1, 20001).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        storage
            .recent_responses(&id, 10, 20000 + 86_400_000)
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "each capture freezes its own retention"
    );
    let unknown = "11111111-1111-4111-8111-111111111111";
    assert_eq!(
        storage.recent_responses(unknown, 10, 20001).unwrap()["items"],
        serde_json::json!([])
    );
    let identity = storage.find_identity_by_id(&id).unwrap().unwrap();
    use tmt_core::binding::BindingRecords;
    BindingRows(storage.connection().unwrap())
        .retire_identity(&identity, false)
        .unwrap();
    assert_eq!(
        storage.recent_responses(&id, 10, 20001).unwrap()["items"],
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
        expected: &expected,
        preferences: &preferences,
        remember_source: true,
        locator: Some("project/session.jsonl"),
        sampled: false,
        consumption: None,
        now_ms: 6000,
        deadline: Instant::now() + std::time::Duration::from_secs(1),
        response: Some(crate::storage::ResponseCapture {
            text: tmt_core::responses::FinalText::new("must roll back").unwrap(),
            retention_days: 3651,
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
        storage.recent_responses(&id, 10, 6000).unwrap()["items"],
        json!([])
    );
    storage.close().unwrap();
}
