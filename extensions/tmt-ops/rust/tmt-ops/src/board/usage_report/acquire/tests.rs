use super::*;
use crate::test_support::write_ready_executable;

const ID: &str = "10000000-0000-4000-8000-000000000001";
const NOW: u64 = 1_800_000_000_000;

fn histories(range: TokRange) -> (Value, Value) {
    let from = NOW - range.milliseconds();
    let total = json!({"asOfMs":NOW,"identities":[{"id":ID,"found":true,"availableFromMs":from,
        "windows":[{"windowMs":range.milliseconds(),"fromMs":from,"toMs":NOW,"availableFromMs":from,
            "buckets":[{"fromMs":from,"toMs":NOW,"inputTokens":900,"outputTokens":100,"cachedInputTokens":800,"coveredMs":range.milliseconds()/2,"complete":false}]}]}]});
    let start = NOW - 86_400_000;
    let points:Vec<_> = (0..24).map(|hour|json!({"fromMs":start+hour*3_600_000,"toMs":start+(hour+1)*3_600_000,
        "inputTokens":1,"outputTokens":0,"cachedInputTokens":0,"coveredMs":3_600_000,"complete":true})).collect();
    let series = json!({"asOfMs":NOW,"identities":[{"id":ID,"found":true,"windows":[{"windowMs":86_400_000,
        "fromMs":start,"toMs":NOW,"bucketMs":3_600_000,"buckets":points}]}]});
    (total, series)
}

#[test]
fn every_shared_range_uses_exact_core_totals_independent_of_hourly_activity() {
    for range in TokRange::ALL {
        let (totals, series) = histories(range);
        let (seat, warning) = seat(&totals, Ok(&series), ID, range).unwrap();
        assert_eq!(
            seat.total(),
            1000,
            "totals are Core's one aggregate, never the 24 hourly tokens"
        );
        assert_eq!(seat.series.iter().map(Point::total).sum::<u64>(), 24);
        assert_eq!(seat.series.len(), 24);
        assert_eq!(seat.window_ms, range.milliseconds());
        assert!(!seat.complete);
        assert_eq!(warning, None);
    }
}

#[test]
fn missing_hourly_series_keeps_valid_totals_but_never_manufactures_zero_activity() {
    let (totals, _) = histories(TokRange::Hour);
    let failure = "history read failed".to_string();
    let (seat, warning) = seat(&totals, Err(&failure), ID, TokRange::Hour).unwrap();
    assert_eq!(seat.total(), 1000);
    assert!(seat.series.is_empty());
    assert_eq!(warning, Some(failure));
}

#[test]
fn mismatched_identity_window_or_coverage_cannot_become_a_known_reading() {
    let (mut totals, series) = histories(TokRange::Hour);
    totals["identities"][0]["id"] = json!("another");
    assert!(seat(&totals, Ok(&series), ID, TokRange::Hour).is_err());
    let (mut totals, series) = histories(TokRange::Hour);
    totals["identities"][0]["windows"][0]["toMs"] = json!(NOW + 1);
    assert!(seat(&totals, Ok(&series), ID, TokRange::Hour).is_err());
    let (mut totals, series) = histories(TokRange::Hour);
    totals["identities"][0]["windows"][0]["buckets"][0]["coveredMs"] = json!(3_600_001);
    assert!(seat(&totals, Ok(&series), ID, TokRange::Hour).is_err());
    let (mut totals, series) = histories(TokRange::Hour);
    totals["identities"][0]["availableFromMs"] = Value::Null;
    assert!(seat(&totals, Ok(&series), ID, TokRange::Hour).is_err());
}

#[test]
fn flow_keeps_unknown_participants_and_counts_messages_without_reads_or_acknowledgements() {
    let document = json!({"sinceMs":0,"observedAtMs":NOW,"rows":[
        {"senderIdentityId":ID,"recipientIdentityId":null,"kind":"talk","bucketStartMs":0,"count":3},
        {"senderIdentityId":ID,"recipientIdentityId":null,"kind":"reply","bucketStartMs":0,"count":4},
        {"senderIdentityId":ID,"recipientIdentityId":null,"kind":"result","bucketStartMs":0,"count":100},
        {"senderIdentityId":ID,"recipientIdentityId":null,"kind":"ack","bucketStartMs":0,"count":100}]});
    let flow = traffic(&document).unwrap();
    assert_eq!(flow.messages[&Some(ID.into())][&None], 7);
    assert_eq!(flow.maximum, 7);
    let mut malformed = document;
    malformed["rows"][0]["bucketStartMs"] = json!(NOW + 1);
    assert!(traffic(&malformed).is_err());
}

#[test]
fn acquired_roster_drives_batched_public_reads_without_digest_or_per_row_processes() {
    let scratch = super::super::tests::Scratch::new();
    let ids: Vec<_> = (0..35)
        .map(|index| format!("10000000-0000-4000-8000-{index:012}"))
        .collect();
    let (mut totals, mut series) = histories(TokRange::Hour);
    let total = totals["identities"][0].clone();
    let point = series["identities"][0].clone();
    totals["identities"] = Value::Array(
        ids.iter()
            .map(|id| {
                let mut row = total.clone();
                row["id"] = json!(id);
                row
            })
            .collect(),
    );
    series["identities"] = Value::Array(
        ids.iter()
            .map(|id| {
                let mut row = point.clone();
                row["id"] = json!(id);
                row
            })
            .collect(),
    );
    std::fs::write(scratch.0.join("totals.json"), totals.to_string()).unwrap();
    std::fs::write(scratch.0.join("series.json"), series.to_string()).unwrap();
    let script = scratch.0.join("core");
    write_ready_executable(
        &script,
        &format!(
            "#!/bin/sh\ncase \"$1\" in\nx) printf '%s\\n' 'flow' >> '{root}/calls'; printf '%s' '{{\"sinceMs\":0,\"observedAtMs\":{NOW},\"rows\":[]}}';;\napi) payload=$(cat); printf '%s\\n' \"$payload\" >> '{root}/calls'; case \"$payload\" in *'\"maxBuckets\":1'*) cat '{root}/totals.json';; *) cat '{root}/series.json';; esac;;\n*) exit 77;;\nesac\n",
            root = scratch.0.display()
        ),
    );
    let request = Request::new(TokRange::Hour, ids);
    let report = read(&Core::at(script), &request, Duration::from_secs(10)).unwrap();
    assert_eq!(
        report.seats.len(),
        35,
        "first unavailable: {:?}; calls: {:?}",
        report.unavailable.first(),
        scratch.calls()
    );
    assert!(report.unavailable.is_empty());
    let calls = scratch.calls();
    assert_eq!(
        calls.len(),
        5,
        "one flow plus two history calls for each of two batches"
    );
    for call in &calls[1..] {
        let payload: Value = serde_json::from_str(call).unwrap();
        assert_eq!(payload["operation"], "consumption.history");
        assert!(payload["input"]["identityIds"].as_array().unwrap().len() <= 32);
    }
    assert!(calls.iter().all(|call| !call.contains("digest")));
}

#[test]
fn input_price_attribution_requires_the_whole_input_split_and_valid_distinct_models() {
    let mut bucket = json!({"inputTokens":100,"outputTokens":10,"cachedInputTokens":20,
        "byModel":[{"modelId":"model-a","inputTokens":60,"outputTokens":5,"cachedInputTokens":10},
        {"modelId":"model-b","inputTokens":40,"outputTokens":5,"cachedInputTokens":10}]});
    assert_eq!(
        model_inputs(&bucket),
        Some(vec![("model-a".into(), 60), ("model-b".into(), 40)])
    );
    let complete = bucket.clone();
    bucket["byModel"][1]["inputTokens"] = json!(39);
    assert_eq!(model_inputs(&bucket), None);
    bucket = complete.clone();
    bucket["byModel"][1]["modelId"] = json!("model-a");
    assert_eq!(model_inputs(&bucket), None);
    bucket = complete;
    bucket["byModel"][0]["cachedInputTokens"] = json!(61);
    assert_eq!(model_inputs(&bucket), None);
}
