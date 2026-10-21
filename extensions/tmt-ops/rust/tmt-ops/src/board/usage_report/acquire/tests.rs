use super::*;
use crate::test_support::write_ready_executable;

const ID: &str = "10000000-0000-4000-8000-000000000001";
const NOW: u64 = 1_800_000_000_000;

fn histories(range: TokRange) -> (Value, Value) {
    let from = NOW - range.milliseconds();
    let total = json!({"asOfMs":NOW,"throughMs":NOW,"identities":[{"id":ID,"found":true,"availableFromMs":from,
        "windows":[{"windowMs":range.milliseconds(),"fromMs":from,"toMs":NOW,"availableFromMs":from,
            "buckets":[{"fromMs":from,"toMs":NOW,"inputTokens":900,"outputTokens":100,"cachedInputTokens":800,"coveredMs":range.milliseconds()/2,"complete":false}]}]}]});
    let start = NOW - 86_400_000;
    let points:Vec<_> = (0..24).map(|hour|json!({"fromMs":start+hour*3_600_000,"toMs":start+(hour+1)*3_600_000,
        "inputTokens":1,"outputTokens":0,"cachedInputTokens":0,"coveredMs":3_600_000,"complete":true})).collect();
    let series = json!({"asOfMs":NOW,"throughMs":NOW,"identities":[{"id":ID,"found":true,"windows":[{"windowMs":86_400_000,
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
        assert_eq!(
            seat.series.iter().map(|point| point.total()).sum::<u64>(),
            24
        );
        assert_eq!(seat.series.iter().count(), 24);
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
fn echoed_closed_window_bounds_are_not_rederived_from_a_local_resolution() {
    for range in TokRange::ALL {
        let (mut totals, series) = histories(range);
        let total = &mut totals["identities"][0]["windows"][0];
        let from = NOW - range.milliseconds() - 5_000;
        let to = NOW - 5_000;
        total["fromMs"] = json!(from);
        total["toMs"] = json!(to);
        total["buckets"][0]["fromMs"] = json!(from);
        total["buckets"][0]["toMs"] = json!(to);
        let (observed, _) = seat(&totals, Ok(&series), ID, range).unwrap();
        assert_eq!((observed.from_ms, observed.to_ms), (from, to));
        assert_eq!(observed.total(), 1_000);
        let total = &mut totals["identities"][0]["windows"][0];
        total["fromMs"] = json!(from - 1);
        assert!(
            seat(&totals, Ok(&series), ID, range).is_err(),
            "interval length is still checked"
        );
    }
}

#[test]
fn a_changed_or_future_activity_cut_keeps_totals_but_never_admits_its_series() {
    for cut in [NOW - 5_000, NOW + 5_000] {
        let scratch = super::super::tests::Scratch::new();
        let (totals, mut series) = histories(TokRange::Hour);
        series["throughMs"] = json!(cut);
        std::fs::write(scratch.0.join("totals.json"), totals.to_string()).unwrap();
        std::fs::write(scratch.0.join("series.json"), series.to_string()).unwrap();
        let script = scratch.0.join("core");
        write_ready_executable(
            &script,
            &format!(
                "#!/bin/sh\ncase \"$1\" in\nx) printf '%s' '{{\"sinceMs\":0,\"observedAtMs\":{NOW},\"rows\":[]}}';;\napi) payload=$(cat); case \"$payload\" in *'\"maxBuckets\":1'*) cat '{root}/totals.json';; *) cat '{root}/series.json';; esac;;\n*) exit 77;;\nesac\n",
                root = scratch.0.display()
            ),
        );
        let report = read(
            &Core::at(script),
            &Request::new(TokRange::Hour, vec![ID.into()]),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(report.seats.len(), 1);
        assert_eq!(report.seats[0].total(), 1_000);
        assert!(report.seats[0].series.is_empty());
        assert_eq!(report.warnings[0].message, "Mismatched consumption cut.");
    }
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

#[test]
fn skewed_batch_clocks_keep_the_all_row_total_and_activity_on_one_cut() {
    use crate::board::{
        app::workspace::tests::board, shell::OpsTab, usage_report::Delivery, workspace::OpsTarget,
    };
    use ratatui::{Terminal, backend::TestBackend};
    const CUT: u64 = NOW - 5_000; // Cross both a fine and rollup boundary on the next call.
    fn shift(value: &mut Value, by: u64) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if matches!(
                        key.as_str(),
                        "asOfMs" | "throughMs" | "fromMs" | "toMs" | "availableFromMs"
                    ) {
                        *value = json!(value.as_u64().unwrap() - by);
                    } else {
                        shift(value, by);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| shift(item, by)),
            _ => {}
        }
    }
    for range in TokRange::ALL {
        let scratch = super::super::tests::Scratch::new();
        let ids: Vec<_> = (0..35)
            .map(|index| format!("10000000-0000-4000-8000-{index:012}"))
            .collect();
        let (mut totals, mut series) = histories(range);
        // Complete totals make SHARE observable as well as the all-row TOK count.
        totals["identities"][0]["windows"][0]["buckets"][0]["complete"] = json!(true);
        totals["identities"][0]["windows"][0]["buckets"][0]["coveredMs"] =
            json!(range.milliseconds());
        for document in [&mut totals, &mut series] {
            let template = document["identities"][0].clone();
            document["identities"] = json!(
                ids.iter()
                    .map(|id| {
                        let mut row = template.clone();
                        row["id"] = json!(id);
                        row
                    })
                    .collect::<Vec<_>>()
            );
        }
        // Later unpinned reads cross the boundary. A pin must keep the first cut,
        // while asOfMs still advances (it is not the cut, nor a cache clock).
        for (name, mut document) in [
            ("late-total", totals.clone()),
            ("late-series", series.clone()),
        ] {
            document["asOfMs"] = json!(NOW + 10_000);
            std::fs::write(scratch.0.join(format!("{name}.json")), document.to_string()).unwrap();
        }
        shift(
            &mut totals,
            if range == TokRange::Hour {
                5_000
            } else {
                300_000
            },
        );
        shift(&mut series, 300_000);
        for document in [&mut totals, &mut series] {
            document["throughMs"] = json!(CUT);
            document["asOfMs"] = json!(NOW + 10_000);
        }
        let mut first = totals.clone();
        first["asOfMs"] = json!(NOW - 1);
        for (name, document) in [
            ("first", first),
            ("pinned-total", totals),
            ("pinned-series", series),
        ] {
            std::fs::write(scratch.0.join(format!("{name}.json")), document.to_string()).unwrap();
        }
        let script = scratch.0.join("skewed-core");
        write_ready_executable(
            &script,
            &format!(
                "#!/bin/sh\ncase \"$1\" in\nx) printf '%s' '{{\"sinceMs\":0,\"observedAtMs\":{NOW},\"rows\":[]}}';;\napi) payload=$(cat); printf '%s\\n' \"$payload\" >> '{root}/calls'\nif [ ! -f '{root}/first-read' ]; then : > '{root}/first-read'; cat '{root}/first.json'; exit; fi\ncase \"$payload\" in *'\"throughMs\":{CUT}'*) prefix=pinned;; *) prefix=late;; esac\ncase \"$payload\" in *'\"maxBuckets\":1'*) cat '{root}/'\"$prefix\"'-total.json';; *) cat '{root}/'\"$prefix\"'-series.json';; esac;;\n*) exit 77;;\nesac\n",
                root = scratch.0.display()
            ),
        );
        let report = read(
            &Core::at(script),
            &Request::new(range, ids.clone()),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(
            report.seats.len(),
            35,
            "{range:?}: {:?}",
            report.unavailable
        );
        assert_eq!(report.observed_at_ms, NOW + 10_000);
        assert!(
            report
                .seats
                .iter()
                .all(|seat| Some(seat.from_ms) == report.window_from_ms)
        );
        let calls: Vec<Value> = scratch
            .calls()
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(calls.len(), 4);
        assert!(calls[0]["input"].get("throughMs").is_none());
        assert!(
            calls[1..]
                .iter()
                .all(|call| call["input"]["throughMs"] == CUT)
        );

        let mut app = board();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        let template = snapshot.roster.members.values().next().unwrap().clone();
        snapshot.roster.members.clear();
        snapshot.roster.squads.clear();
        snapshot.rows.clear();
        for id in &ids {
            let mut member = template.clone();
            member.id = id.clone();
            member.name = id.clone();
            snapshot.roster.members.insert(id.clone(), member);
            snapshot.rows.insert(
                OpsTarget::Member {
                    room_id: None,
                    identity_id: id.clone(),
                },
                json!({"id":id,"name":id}),
            );
        }
        snapshot.preferences.tok_range = range;
        app.usage_report.apply(Delivery {
            since: range,
            result: Ok(report),
        });
        app.shell.tab = OpsTab::Digest;
        app.workspace.selected = Some(OpsTarget::DigestAll);
        app.workspace_reconcile();
        assert_eq!(app.workspace_rows().len(), 36);
        let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
        let lines: Vec<String> = terminal
            .backend()
            .buffer()
            .content
            .chunks(160)
            .map(|line| line.iter().map(|cell| cell.symbol()).collect())
            .collect();
        let all = lines
            .iter()
            .find(|line| line.contains("all") && line.contains("35.0k"))
            .unwrap_or_else(|| panic!("{range:?}: all-row total is missing: {}", lines.join("\n")));
        assert!(all.contains("100%"), "{all}");
        assert!(
            !all.contains('?'),
            "all-row activity must have a common clock: {all}"
        );
    }
}
