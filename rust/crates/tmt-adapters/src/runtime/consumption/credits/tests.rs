use super::*;
use serde_json::json;

// Independently computed (Python datetime, UTC) rather than by the code under test.
const LINE_TIME: &str = "2026-10-10T03:48:29.763Z";
const LINE_TIME_MS: u64 = 1_791_604_109_763;

fn line(credits: Value) -> Value {
    json!({
        "timestamp": LINE_TIME,
        "type": "event_msg",
        "payload": {"type": "token_count", "info": {}, "rate_limits": {"primary": null, "credits": credits}},
    })
}

fn credits(balance: Value) -> Value {
    json!({"balance": balance, "has_credits": true, "unlimited": false})
}

#[test]
fn credits_keep_the_line_time_and_the_provider_decimal_text() {
    let found = codex(&line(credits(json!("1234.5678901234")))).unwrap();
    assert_eq!(
        found,
        Credits {
            observed_at_ms: LINE_TIME_MS,
            balance: "1234.5678901234".into(),
            has_credits: true,
            unlimited: false,
        }
    );
    assert_eq!(
        serde_json::to_value(&found).unwrap(),
        json!({"observedAtMs": LINE_TIME_MS, "balance": "1234.5678901234", "hasCredits": true, "unlimited": false})
    );
}

#[test]
fn credits_do_not_depend_on_the_rate_limit_windows() {
    // The line above has no window at all; an unlimited account still reports.
    let unlimited = json!({"balance": "0", "has_credits": false, "unlimited": true});
    let found = codex(&line(unlimited)).unwrap();
    assert!(found.unlimited && !found.has_credits);
    assert_eq!(found.balance, "0");
}

#[test]
fn the_balance_is_a_bounded_decimal_and_anything_else_is_absent() {
    let whole = "9".repeat(15);
    let fraction = "9".repeat(12);
    for good in [
        "0".to_owned(),
        "12.5".to_owned(),
        "0.0000000001".to_owned(),
        whole.clone(),
        format!("{whole}.{fraction}"),
    ] {
        assert!(codex(&line(credits(json!(good)))).is_some(), "{good:?}");
    }
    for bad in [
        json!(""),
        json!("."),
        json!("1."),
        json!(".5"),
        json!("-1"),
        json!("+1"),
        json!("1e3"),
        json!("1,5"),
        json!("1 "),
        json!("１２"),
        json!(format!("{whole}9")),
        json!(format!("1.{fraction}9")),
        json!(12.5),
        json!(12),
        json!(null),
    ] {
        assert!(codex(&line(credits(bad.clone()))).is_none(), "{bad:?}");
    }
}

#[test]
fn a_line_with_any_missing_or_mistyped_field_has_no_credits() {
    let mut cases = Vec::new();
    for field in ["balance", "has_credits", "unlimited"] {
        let mut object = credits(json!("1"));
        object.as_object_mut().unwrap().remove(field);
        cases.push(object);
    }
    cases.push(json!({"balance": "1", "has_credits": "yes", "unlimited": false}));
    cases.push(json!({"balance": "1", "has_credits": true, "unlimited": 0}));
    cases.push(json!(null));
    cases.push(json!("1"));
    for case in cases {
        assert!(codex(&line(case.clone())).is_none(), "{case}");
    }
    // No credits object, no rate_limits object, or an unusable line time.
    let mut bare = line(credits(json!("1")));
    bare["payload"]["rate_limits"]
        .as_object_mut()
        .unwrap()
        .remove("credits");
    assert!(codex(&bare).is_none());
    let mut none = line(credits(json!("1")));
    none["payload"]
        .as_object_mut()
        .unwrap()
        .remove("rate_limits");
    assert!(codex(&none).is_none());
    let mut late = line(credits(json!("1")));
    late["timestamp"] = json!("yesterday");
    assert!(codex(&late).is_none());
}
