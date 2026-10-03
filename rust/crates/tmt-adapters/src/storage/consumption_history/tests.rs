use super::*;

#[test]
fn shared_seed_fixture_excludes_open_counter() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/consumption-history-v1.json"
    ))
    .unwrap();
    let directory = crate::test_support::TestDirectory::new();
    let mut storage = Storage::open(&directory.path.join("history.sqlite")).unwrap();
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
        .consumption_history(&[id.to_owned()], &[15000], 120, 22500)
        .unwrap();
    assert_eq!(response, fixture["response"]);
    assert_eq!(
        response["identities"][0]["latest"]["consumption"]["inputTokens"],
        114
    );
    storage.close().unwrap();
}

fn insert_binding(connection: &Connection, id: &str) {
    connection.execute("INSERT INTO bindings(id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at) VALUES('fixture-binding',?,'tmux','%1','fixture-server','/tmp/fixture-socket',10,'start',11,'t','t')",[id]).unwrap();
}
fn fixture() -> (
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
fn latest(input: u64, output: u64, sequence: u64, observed: u64) -> ConsumptionLatest {
    ConsumptionLatest {
        driver: "claude".into(),
        session: "fixture-session".into(),
        consumption: Consumption {
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: 0,
            epoch: "33333333-3333-4333-8333-333333333333".into(),
            sequence,
            observed_at_ms: observed,
            complete: true,
            gap: false,
        },
    }
}
fn source(storage: &Storage, id: &str) {
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
        .consumption_history(std::slice::from_ref(&id), &[10000], 120, 15000)
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
        .consumption_history(std::slice::from_ref(&id), &[15000], 120, 20000)
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
        .consumption_history(std::slice::from_ref(&id), &[10000], 120, 15000)
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
        .consumption_history(std::slice::from_ref(&id), &[3600000], 120, now)
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
        .consumption_history(std::slice::from_ref(&id), &[15000], 120, 20000)
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
        .consumption_history(std::slice::from_ref(&id), &[10000], 120, 15000)
        .unwrap();
    let row = &result["identities"][0]["windows"][0]["buckets"][1];
    assert_eq!(row["inputTokens"], 10);
    assert_eq!(row["outputTokens"], 5);
    assert_eq!(row["coveredMs"], 0);
    assert_eq!(row["complete"], false);
    storage.close().unwrap();
}
