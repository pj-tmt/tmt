//! The report's joins against its frozen vector, then the edges the vector does
//! not show, then the Core reads through a fake `tmt`.

use super::{
    Since, Table, collect, csv, execute,
    model::{self, Label, Report, Usage, line_of},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const T: u64 = 1_800_000_000_000;
const AS_OF: u64 = T + 60_000;
const DAY: u64 = 86_400_000;
const A: &str = "10000000-0000-4000-8000-000000000001";
const B: &str = "10000000-0000-4000-8000-000000000002";
const F: &str = "10000000-0000-4000-8000-000000000006";

fn vector() -> Value {
    let all: Value = serde_json::from_str(
        &fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/vectors/usage-report-v1.json"
        ))
        .expect("the supplier's frozen usage report vector"),
    )
    .unwrap();
    all["cases"][0].clone()
}

fn labels(case: &Value) -> BTreeMap<String, Label> {
    case["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let id = row["id"].as_str().unwrap().to_owned();
            (
                id.clone(),
                Label {
                    id,
                    name: row["name"].as_str().map(str::to_owned),
                    retired: row["retired"] == true,
                },
            )
        })
        .collect()
}

fn ids(case: &Value) -> Vec<String> {
    case["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect()
}

fn report(case: &Value) -> Report {
    let labels = labels(case);
    Report {
        as_of_ms: case["asOfMs"].as_u64().unwrap(),
        since: Since::parse(case["since"].as_str().unwrap())
            .unwrap()
            .text(),
        usage: model::usage(&case["totals"], &case["series"], &ids(case), &labels).unwrap(),
        exchange: Some(model::exchange(&case["flow"], &labels).unwrap()),
        warnings: vec![],
    }
}

#[test]
fn the_frozen_vector_joins_to_the_documented_report_and_csv() {
    let case = vector();
    let document = model::document(&report(&case)).unwrap();
    assert_eq!(document, case["expected"]);
    assert_eq!(
        csv::table(&document, Table::Usage).unwrap(),
        case["csv"]["usage"].as_str().unwrap()
    );
    assert_eq!(
        csv::table(&document, Table::Credits).unwrap(),
        case["csv"]["credits"].as_str().unwrap()
    );
    assert_eq!(
        csv::table(&document, Table::Exchange).unwrap(),
        case["csv"]["exchange"].as_str().unwrap()
    );
}

#[test]
fn csv_has_one_row_per_json_row() {
    let document = vector()["expected"].clone();
    let usage = &document["usage"];
    let expected: usize = usage["seats"]
        .as_array()
        .unwrap()
        .iter()
        .map(|seat| {
            seat["byModel"]
                .as_array()
                .map_or(1, |models| models.len().max(1))
        })
        .sum::<usize>()
        + usage["unavailable"].as_array().unwrap().len();
    let table = csv::table(&document, Table::Usage).unwrap();
    assert_eq!(table.lines().count() - 1, expected);
    let table = csv::table(&document, Table::Exchange).unwrap();
    assert_eq!(
        table.lines().count() - 1,
        document["exchange"]["pairs"].as_array().unwrap().len()
    );
}

#[test]
fn csv_cells_quote_and_never_read_as_formulas() {
    assert_eq!(csv::cell("plain"), "plain");
    assert_eq!(csv::cell("a,b"), "\"a,b\"");
    assert_eq!(csv::cell("say \"hi\""), "\"say \"\"hi\"\"\"");
    assert_eq!(csv::cell("two\nlines"), "\"two\nlines\"");
    for dangerous in ["=SUM(A1)", "+1", "-1", "@cmd", "\tx", "\rx"] {
        assert!(csv::cell(dangerous).starts_with('\'') || csv::cell(dangerous).starts_with("\"'"));
    }
    assert_eq!(csv::cell("=1,2"), "\"'=1,2\"");
    assert_eq!(csv::cell("a=b"), "a=b");
}

#[test]
fn a_hostile_name_stays_one_csv_cell() {
    let mut document = vector()["expected"].clone();
    document["usage"]["seats"][0]["name"] = json!("=HYPERLINK(\"x\",\"y\"),\n+evil");
    let table = csv::table(&document, Table::Usage).unwrap();
    assert!(table.contains(",\"'=HYPERLINK(\"\"x\"\",\"\"y\"\"),\n+evil\","));
}

#[test]
fn model_families_are_presentation_only() {
    assert_eq!(line_of("claude-opus-5-5"), "claude");
    assert_eq!(line_of("gpt-6.1-sol"), "gpt");
    assert_eq!(line_of("gemini-3"), "other");
    assert_eq!(line_of(""), "other");
    assert_eq!(line_of("my-claude-fork"), "other");
}

/// One found seat with a single bucket and the given limits.
fn one_seat(limits: Option<Value>) -> (Value, Value, Vec<String>, BTreeMap<String, Label>) {
    let bucket = |extra: Option<Value>| {
        let mut bucket = json!({
            "fromMs": T - DAY, "toMs": T, "inputTokens": 10, "outputTokens": 2, "cachedInputTokens": 4,
            "coveredMs": DAY, "complete": true, "gap": false, "discontinuous": false,
            "byModel": [{"modelId": "gpt-6.1-sol", "inputTokens": 10, "outputTokens": 2, "cachedInputTokens": 4}],
        });
        if let Some(limits) = extra {
            bucket["rateLimits"] = limits;
        }
        bucket
    };
    let document = |bucket: Value| {
        json!({"identities": [{"id": A, "found": true, "windows": [{
            "windowMs": DAY, "fromMs": T - DAY, "toMs": T, "bucketMs": DAY,
            "availableFromMs": T - DAY, "buckets": [bucket]}]}]})
    };
    (
        document(bucket(None)),
        document(bucket(limits)),
        vec![A.to_owned()],
        BTreeMap::from([(
            A.to_owned(),
            Label {
                id: A.to_owned(),
                name: Some("seat".into()),
                retired: false,
            },
        )]),
    )
}

fn limits_of(five: Value) -> Value {
    let (totals, series, ids, labels) = one_seat(Some(json!({
        "observedAtMs": T, "windows": [five]
    })));
    let report = Report {
        as_of_ms: AS_OF,
        since: "24h",
        usage: model::usage(&totals, &series, &ids, &labels).unwrap(),
        exchange: None,
        warnings: vec![],
    };
    model::document(&report).unwrap()["limits"].clone()
}

/// The report for `one_seat` after Core says its evidence begins at `available`.
fn report_with_available(available: Value) -> Result<Value, crate::core::Error> {
    let (mut totals, series, ids, labels) = one_seat(None);
    totals["identities"][0]["windows"][0]["availableFromMs"] = available;
    let report = Report {
        as_of_ms: AS_OF,
        since: "24h",
        usage: model::usage(&totals, &series, &ids, &labels)?,
        exchange: None,
        warnings: vec![],
    };
    model::document(&report)
}

#[test]
fn coverage_starts_where_core_first_recorded_the_seat_but_never_before_the_window() {
    let start = |available: u64| {
        let document = report_with_available(json!(available)).unwrap();
        document["usage"]["seats"][0]["coverage"]["startsAtMs"].clone()
    };
    // Core kept history from before the window: evidence starts with the window.
    assert_eq!(start(T - 3 * DAY), json!(T - DAY));
    assert_eq!(start(T - DAY), json!(T - DAY));
    // The seat was first recorded inside the window: that is where it starts.
    assert_eq!(start(T - DAY / 4), json!(T - DAY / 4));
    // A start that is not a number is a contradiction, not a guess.
    assert!(report_with_available(json!("yesterday")).is_err());
}

/// `one_seat` after its series bucket gains `fields`.
fn credited(fields: Value) -> Result<Value, crate::core::Error> {
    let (totals, mut series, ids, labels) = one_seat(None);
    let bucket = &mut series["identities"][0]["windows"][0]["buckets"][0];
    for (key, value) in fields.as_object().unwrap() {
        bucket[key] = value.clone();
    }
    let report = Report {
        as_of_ms: AS_OF,
        since: "24h",
        usage: model::usage(&totals, &series, &ids, &labels)?,
        exchange: None,
        warnings: vec![],
    };
    model::document(&report)
}

fn credit_fields(observed: u64, balance: &str) -> Value {
    json!({"observedAtMs": observed, "balance": balance, "hasCredits": true, "unlimited": false})
}

#[test]
fn credits_are_one_series_by_newest_observation_and_absent_where_nothing_was_seen() {
    // No seat observed credits: nothing is reported, not a zero balance.
    let none = credited(json!({})).unwrap();
    assert_eq!(none["credits"], Value::Null);
    assert!(
        none["usage"]["seats"][0]["series"][0]
            .get("serviceTier")
            .is_none()
    );
    assert_eq!(
        csv::table(&none, Table::Credits).unwrap().lines().count(),
        1
    );

    let one =
        credited(json!({"credits": credit_fields(T - 1000, "7.50"), "serviceTier": "priority"}))
            .unwrap();
    assert_eq!(
        one["credits"],
        json!({"points": [{"fromMs": T - DAY, "toMs": T, "balance": "7.50",
            "observedAtMs": T - 1000, "hasCredits": true, "unlimited": false,
            "serviceTiers": ["priority"]}]})
    );
    assert_eq!(
        one["usage"]["seats"][0]["series"][0]["serviceTier"],
        "priority"
    );
    // A tier without a balance names no point, and a balance without a tier names no tier.
    let tier_only = credited(json!({"serviceTier": "default"})).unwrap();
    assert_eq!(tier_only["credits"], Value::Null);
    let credit_only = credited(json!({"credits": credit_fields(T - 1000, "7")})).unwrap();
    assert_eq!(
        credit_only["credits"]["points"][0]["serviceTiers"],
        json!([])
    );
}

#[test]
fn malformed_credits_or_tier_make_the_read_fail_rather_than_guess() {
    for fields in [
        json!({"credits": credit_fields(T, "1e3")}),
        json!({"credits": credit_fields(T, "")}),
        json!({"credits": credit_fields(T, ".5")}),
        json!({"credits": credit_fields(T, "1.2.3")}),
        json!({"credits": {"observedAtMs": 0, "balance": "1", "hasCredits": true, "unlimited": false}}),
        json!({"credits": {"observedAtMs": T, "balance": 1, "hasCredits": true, "unlimited": false}}),
        json!({"credits": {"observedAtMs": T, "balance": "1", "hasCredits": "yes", "unlimited": false}}),
        json!({"credits": {"observedAtMs": T, "balance": "1", "unlimited": false}}),
        json!({"serviceTier": "Priority"}),
        json!({"serviceTier": 7}),
        json!({"serviceTier": ""}),
    ] {
        assert!(credited(fields.clone()).is_err(), "{fields}");
    }
}

#[test]
fn a_no_evidence_identity_carries_core_s_cause_and_unknown_causes_pass_through() {
    let unavailable = |cause: Value| {
        let (mut totals, series, ids, labels) = one_seat(None);
        totals["identities"][0]["windows"][0]["availableFromMs"] = Value::Null;
        if !cause.is_null() {
            totals["identities"][0]["cause"] = cause;
        }
        let report = Report {
            as_of_ms: AS_OF,
            since: "24h",
            usage: model::usage(&totals, &series, &ids, &labels).unwrap(),
            exchange: None,
            warnings: vec![],
        };
        model::document(&report).unwrap()
    };
    let known = unavailable(json!("no_sampler"));
    assert_eq!(known["usage"]["unavailable"][0]["reason"], "no_evidence");
    assert_eq!(known["usage"]["unavailable"][0]["cause"], "no_sampler");
    assert!(
        csv::table(&known, Table::Usage)
            .unwrap()
            .ends_with("no_evidence,no_sampler\n")
    );
    // A cause a later host adds is passed through; one that is not a plain id is dropped.
    assert_eq!(
        unavailable(json!("no_muse_session"))["usage"]["unavailable"][0]["cause"],
        "no_muse_session"
    );
    for bad in [
        json!("No Agent"),
        json!(7),
        json!(""),
        json!("x".repeat(25)),
    ] {
        assert!(
            unavailable(bad.clone())["usage"]["unavailable"][0]
                .get("cause")
                .is_none(),
            "{bad}"
        );
    }
    // Core gave none: the row stays as it was.
    assert!(
        unavailable(Value::Null)["usage"]["unavailable"][0]
            .get("cause")
            .is_none()
    );
    // Not found and unreadable identities never have one.
    let (mut totals, series, ids, labels) = one_seat(None);
    totals["identities"][0] = json!({"id": A, "found": false, "cause": "no_agent"});
    let usage = model::usage(&totals, &series, &ids, &labels).unwrap();
    let report = Report {
        as_of_ms: AS_OF,
        since: "24h",
        usage,
        exchange: None,
        warnings: vec![],
    };
    assert!(
        model::document(&report).unwrap()["usage"]["unavailable"][0]
            .get("cause")
            .is_none()
    );
}

#[test]
fn pace_is_exact_at_the_edges_of_a_window() {
    let length = 300 * 60_000;
    // Resets exactly now: already over, so nothing about the current pace.
    let over = limits_of(json!({"windowMinutes": 300, "usedPercent": 12.5, "resetsAtMs": AS_OF}));
    assert_eq!(over[0]["stale"], true);
    assert_eq!(over[0]["elapsedPercent"], Value::Null);
    assert_eq!(over[0]["paceDeltaPoints"], Value::Null);
    // The window starts exactly now: nothing elapsed, pace is the whole usage.
    let begun =
        limits_of(json!({"windowMinutes": 300, "usedPercent": 12.5, "resetsAtMs": AS_OF + length}));
    assert_eq!(begun[0]["stale"], false);
    assert_eq!(begun[0]["elapsedPercent"], json!(0.0));
    assert_eq!(begun[0]["paceDeltaPoints"], json!(12.5));
    // One millisecond left: elapsed rounds to a full window.
    let last =
        limits_of(json!({"windowMinutes": 300, "usedPercent": 99.0, "resetsAtMs": AS_OF + 1}));
    assert_eq!(last[0]["elapsedPercent"], json!(100.0));
    assert_eq!(last[0]["paceDeltaPoints"], json!(-1.0));
    // A reset farther away than the window length is clamped, never negative.
    let early = limits_of(
        json!({"windowMinutes": 300, "usedPercent": 5.0, "resetsAtMs": AS_OF + 2 * length}),
    );
    assert_eq!(early[0]["elapsedPercent"], json!(0.0));
}

#[test]
fn invalid_limits_are_ignored_not_guessed() {
    for bad in [
        json!({"windowMinutes": 0, "usedPercent": 1.0, "resetsAtMs": AS_OF + 1}),
        json!({"windowMinutes": 300, "usedPercent": 101.0, "resetsAtMs": AS_OF + 1}),
        json!({"windowMinutes": 300, "usedPercent": -1.0, "resetsAtMs": AS_OF + 1}),
        json!({"windowMinutes": 300, "usedPercent": 1.0}),
    ] {
        assert_eq!(limits_of(bad), json!([]));
    }
}

#[test]
fn missing_evidence_is_unavailable_never_a_zero_seat() {
    let (mut totals, mut series, ids, labels) = one_seat(None);
    for document in [&mut totals, &mut series] {
        document["identities"][0]["windows"][0]["availableFromMs"] = Value::Null;
    }
    let usage = model::usage(&totals, &series, &ids, &labels).unwrap();
    let document = model::document(&Report {
        as_of_ms: AS_OF,
        since: "24h",
        usage,
        exchange: None,
        warnings: vec![],
    })
    .unwrap();
    assert_eq!(document["usage"]["seats"], json!([]));
    assert_eq!(document["usage"]["lines"], json!([]));
    assert_eq!(document["usage"]["unavailable"][0]["reason"], "no_evidence");
    // The window is still stated, so a reader knows what was asked.
    assert_eq!(document["window"]["toMs"], T);
}

#[test]
fn unreadable_batches_are_unavailable_with_their_labels() {
    let (_, _, ids, labels) = one_seat(None);
    let usage = Usage::unreadable(&ids, &labels);
    let document = model::document(&Report {
        as_of_ms: AS_OF,
        since: "24h",
        usage,
        exchange: None,
        warnings: vec![],
    })
    .unwrap();
    assert_eq!(
        document["usage"]["unavailable"],
        json!([{"id": A, "name": "seat", "retired": false, "reason": "read_failed"}])
    );
    assert_eq!(document["window"], Value::Null);
}

#[test]
fn a_response_that_contradicts_itself_is_refused() {
    // Attribution that does not add up to the total.
    let (mut totals, series, ids, labels) = one_seat(None);
    totals["identities"][0]["windows"][0]["buckets"][0]["byModel"][0]["inputTokens"] = json!(9);
    assert_eq!(
        model::usage(&totals, &series, &ids, &labels)
            .unwrap_err()
            .code,
        "CORE_RESPONSE_INVALID"
    );
    // Identities in another order than asked.
    let (mut totals, series, ids, labels) = one_seat(None);
    totals["identities"][0]["id"] = json!(B);
    assert_eq!(
        model::usage(&totals, &series, &ids, &labels)
            .unwrap_err()
            .code,
        "CORE_RESPONSE_INVALID"
    );
    // A total beyond the JavaScript-safe integer bound.
    let (mut totals, series, ids, labels) = one_seat(None);
    totals["identities"][0]["windows"][0]["buckets"][0]["inputTokens"] =
        json!(9_007_199_254_740_992u64);
    assert!(model::usage(&totals, &series, &ids, &labels).is_err());
}

#[test]
fn flow_totals_past_the_safe_bound_are_refused() {
    let big = 9_007_199_254_740_991u64;
    let row = |count| {
        json!({"senderIdentityId": A, "recipientIdentityId": B, "kind": "talk",
        "bucketStartMs": T, "count": count, "bytes": 1})
    };
    let flow = json!({"sinceMs": T, "observedAtMs": AS_OF, "rows": [row(big), row(1)]});
    assert_eq!(
        model::exchange(&flow, &BTreeMap::new()).unwrap_err().code,
        "CORE_RESPONSE_INVALID"
    );
    let flow = json!({"sinceMs": T, "observedAtMs": AS_OF, "rows": [{"senderIdentityId": A,
        "recipientIdentityId": B, "kind": "shout", "bucketStartMs": T, "count": 1, "bytes": 1}]});
    assert!(model::exchange(&flow, &BTreeMap::new()).is_err());
}

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A directory holding canned Core documents and a `tmt` that serves them.
struct FakeCore(PathBuf);
impl FakeCore {
    fn new(files: &[(&str, Value)]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "digest-usage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        for (name, document) in files {
            fs::write(path.join(name), serde_json::to_vec(document).unwrap()).unwrap();
        }
        let script = format!(
            "#!/bin/sh
cd '{dir}' || exit 9
printf '%s\\n' \"$*\" >> args
case \"$1\" in
  identity) cat roster.json ;;
  x) if [ -f flow.fail ]; then cat flow.fail; exit 1; fi; cat flow.json ;;
  api)
    input=$(cat)
    printf '%s\\n' \"$input\" >> calls
    case \"$input\" in
      *references.resolve*) cat resolve.json ;;
      *consumption.history*)
        if [ -f history.fail ]; then cat history.fail; exit 1; fi
        case \"$input\" in *'\"maxBuckets\":1,'*) cat totals.json ;; *) cat series.json ;; esac ;;
      *) exit 9 ;;
    esac ;;
  *) exit 9 ;;
esac
",
            dir = path.display()
        );
        fs::write(path.join("core"), script).unwrap();
        fs::set_permissions(path.join("core"), fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn core(&self) -> crate::core::Core {
        crate::core::Core::fixture(self.0.join("core"))
    }
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.0.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn args(&self) -> Vec<String> {
        fs::read_to_string(self.0.join("args"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}
impl Drop for FakeCore {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn error(code: &str) -> Value {
    json!({"error": {"code": code, "message": format!("{code} happened")}})
}

fn seat_rows(ids: &[&str]) -> Vec<Value> {
    ids.iter()
        .map(|id| {
            let window = |buckets: Value| {
                json!([{"windowMs": DAY, "fromMs": T - DAY, "toMs": T, "bucketMs": DAY,
                    "availableFromMs": T - DAY, "buckets": buckets}])
            };
            let bucket = json!({"fromMs": T - DAY, "toMs": T, "inputTokens": 7, "outputTokens": 1,
                "cachedInputTokens": 3, "coveredMs": DAY, "complete": true, "gap": false,
                "discontinuous": false});
            json!({"id": id, "found": true, "windows": window(json!([bucket]))})
        })
        .collect()
}

#[test]
fn core_is_read_in_the_documented_shape() {
    let flow_row = |s: &str, r: &str| {
        json!({"senderIdentityId": s, "recipientIdentityId": r,
        "kind": "talk", "bucketStartMs": T - DAY, "count": 2, "bytes": 10})
    };
    let fake = FakeCore::new(&[
        (
            "roster.json",
            json!({"identities": [{"id": B, "name": "zed"}, {"id": A, "name": "amy"}]}),
        ),
        (
            "flow.json",
            json!({"sinceMs": T - DAY, "observedAtMs": AS_OF, "bucketMs": DAY,
            "rows": [flow_row(A, F), flow_row(F, B)]}),
        ),
        (
            "resolve.json",
            json!({"identities": [{"id": F, "found": true, "name": "gone", "lifetime": "session", "retired": true}]}),
        ),
        // Core answers in request order, which is by name: amy, gone, zed.
        ("totals.json", json!({"identities": seat_rows(&[A, F, B])})),
        ("series.json", json!({"identities": seat_rows(&[A, F, B])})),
    ]);
    let document = execute(&fake.core(), Since::Day, AS_OF).unwrap();
    // Seats are ordered by name; the retired flow participant is read too.
    let names: Vec<_> = document["usage"]["seats"]
        .as_array()
        .unwrap()
        .iter()
        .map(|seat| {
            (
                seat["name"].as_str().unwrap(),
                seat["retired"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(names, [("amy", false), ("gone", true), ("zed", false)]);
    assert_eq!(document["warnings"], json!([]));
    assert_eq!(
        fake.args(),
        [
            "identity ls --json",
            "x flow --since 24h --bucket 24h --json",
            "api",
            "api",
            "api"
        ]
    );
    let calls = fake.calls();
    // Only the participant off the roster is resolved, then two reads per batch.
    assert_eq!(calls[0]["operation"], "references.resolve");
    assert_eq!(calls[0]["input"], json!({"identityIds": [F]}));
    let reads: Vec<_> = calls[1..].iter().collect();
    assert_eq!(reads.len(), 2);
    for (read, buckets) in reads.iter().zip([1, 24]) {
        assert_eq!(read["operation"], "consumption.history");
        assert_eq!(
            read["input"],
            json!({"identityIds": [A, F, B], "windowsMs": [DAY], "maxBuckets": buckets,
                "includeRetired": true})
        );
    }
}

#[test]
fn a_failed_flow_degrades_the_exchange_and_nothing_else() {
    let fake = FakeCore::new(&[
        (
            "roster.json",
            json!({"identities": [{"id": A, "name": "amy"}]}),
        ),
        ("flow.fail", error("X_ERROR")),
        ("totals.json", json!({"identities": seat_rows(&[A])})),
        ("series.json", json!({"identities": seat_rows(&[A])})),
    ]);
    let document = execute(&fake.core(), Since::Day, AS_OF).unwrap();
    assert_eq!(document["exchange"], Value::Null);
    assert_eq!(document["warnings"][0]["section"], "exchange");
    assert_eq!(document["warnings"][0]["code"], "X_ERROR");
    assert_eq!(
        document["usage"]["seats"][0]["totals"],
        json!({"input": 7, "output": 1, "cached": 3})
    );
    assert!(csv::table(&document, Table::Usage).is_ok());
    let refused = csv::table(&document, Table::Exchange).unwrap_err();
    assert_eq!(refused.code, "EXCHANGE_UNAVAILABLE");
    assert_eq!(refused.message, "X_ERROR happened");
}

#[test]
fn only_the_roster_is_fatal() {
    let fake = FakeCore::new(&[]);
    assert!(collect(&fake.core(), Since::Day, AS_OF).is_err());
}

#[test]
fn failed_history_batches_leave_every_seat_unavailable() {
    let roster: Vec<Value> = (1..=17)
        .map(|n| json!({"id": format!("10000000-0000-4000-8000-{n:012}"), "name": format!("m{n:02}")}))
        .collect();
    let fake = FakeCore::new(&[
        ("roster.json", json!({"identities": roster})),
        (
            "flow.json",
            json!({"sinceMs": T - DAY, "observedAtMs": AS_OF, "bucketMs": DAY, "rows": []}),
        ),
        ("history.fail", error("CORE_FAILED")),
    ]);
    let document = execute(&fake.core(), Since::Day, AS_OF).unwrap();
    assert_eq!(document["usage"]["seats"], json!([]));
    let unavailable = document["usage"]["unavailable"].as_array().unwrap();
    assert_eq!(unavailable.len(), 17);
    assert!(
        unavailable
            .iter()
            .all(|seat| seat["reason"] == "read_failed")
    );
    assert_eq!(document["usage"]["lines"], json!([]));
    assert_eq!(document["warnings"].as_array().unwrap().len(), 2);
    // Batches of 16 and 1.
    let sizes: Vec<_> = fake
        .calls()
        .iter()
        .map(|call| call["input"]["identityIds"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [16, 1]);
}

#[test]
fn windows_and_tables_parse_only_the_documented_values() {
    assert_eq!(Since::parse("24h").unwrap(), Since::Day);
    assert_eq!(Since::parse("7d").unwrap(), Since::Week);
    assert_eq!(Since::parse("30d").unwrap(), Since::Month);
    assert!(Since::parse("1h").is_err() && Since::parse("lifetime").is_err());
    assert_eq!(Table::parse("usage").unwrap(), Table::Usage);
    assert!(Table::parse("limits").is_err());
    // Each window is a whole number of its series buckets, and rollup-sized.
    for since in [Since::Day, Since::Week, Since::Month] {
        assert_eq!(since.window_ms() % 300_000, 0);
        assert!(since.window_ms() > 3_600_000 && since.window_ms() <= 2_592_000_000);
        assert_eq!(since.window_ms() % since.buckets(), 0);
    }
}
