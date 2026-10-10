use super::*;
use serde_json::json;

// Independently computed (Python datetime, UTC) rather than by the code under test.
const LINE_TIME: &str = "2026-10-10T03:48:29.763Z";
const LINE_TIME_MS: u64 = 1_791_604_109_763;

fn line(limits: Value) -> Value {
    json!({
        "timestamp": LINE_TIME,
        "type": "event_msg",
        "payload": {"type": "token_count", "info": {}, "rate_limits": limits},
    })
}

fn weekly() -> Value {
    json!({"used_percent": 78.0, "window_minutes": 10080, "resets_at": 1_791_948_558u64})
}

fn short() -> Value {
    json!({"used_percent": 12.5, "window_minutes": 300, "resets_at": 1_791_620_000u64})
}

#[test]
fn timestamps_convert_to_exact_unix_milliseconds_and_reject_every_other_form() {
    assert_eq!(timestamp_ms(LINE_TIME), Some(LINE_TIME_MS));
    assert_eq!(
        timestamp_ms("2024-02-29T00:00:00Z"),
        Some(1_709_164_800_000)
    );
    assert_eq!(timestamp_ms("1970-01-02T00:00:01Z"), Some(86_401_000));
    // One digit is a tenth of a second; digits beyond milliseconds truncate.
    assert_eq!(timestamp_ms("1970-01-01T00:00:00.5Z"), Some(500));
    assert_eq!(timestamp_ms("1970-01-01T00:00:00.123456789Z"), Some(123));
    for bad in [
        "",
        "2026-10-10T03:48:29",
        "2026-10-10T03:48:29+00:00",
        "2026-10-10 03:48:29Z",
        "2026-10-10T03:48:29.Z",
        "2026-10-10T03:48:29.1234567890Z",
        "2026-10-10T03:48:29ZZ",
        "2026-13-10T03:48:29Z",
        "2026-02-29T00:00:00Z",
        "2026-10-10T24:00:00Z",
        "2026-10-10T03:60:00Z",
        "2026-10-10T03:48:60Z",
        "+026-10-10T03:48:29Z",
        "2026-10-1xT03:48:29Z",
        "２０２６-10-10T03:48:29Z",
    ] {
        assert_eq!(timestamp_ms(bad), None, "{bad:?}");
    }
}

#[test]
fn one_weekly_window_keeps_the_line_time_and_converts_reset_seconds() {
    let limits = codex(&line(json!({"primary": weekly(), "secondary": null}))).unwrap();
    assert_eq!(
        limits,
        RateLimits {
            observed_at_ms: LINE_TIME_MS,
            windows: vec![Window {
                window_minutes: 10_080,
                used_percent: 78.0,
                resets_at_ms: 1_791_948_558_000,
            }],
        }
    );
    assert!(limits.valid());
}

#[test]
fn windows_are_ordered_by_length_whichever_slot_the_provider_used() {
    for (primary, secondary) in [(weekly(), short()), (short(), weekly())] {
        let limits = codex(&line(json!({"primary": primary, "secondary": secondary}))).unwrap();
        let lengths: Vec<_> = limits.windows.iter().map(|w| w.window_minutes).collect();
        assert_eq!(lengths, [300, 10_080]);
        assert_eq!(limits.windows[0].used_percent, 12.5);
    }
}

#[test]
fn a_line_without_usable_limits_reports_none() {
    let mut no_field = line(Value::Null);
    no_field["payload"]
        .as_object_mut()
        .unwrap()
        .remove("rate_limits");
    for entry in [
        no_field,
        line(Value::Null),
        line(json!({})),
        line(json!({"primary": null, "secondary": null})),
        line(json!("high")),
    ] {
        assert_eq!(codex(&entry), None, "{entry}");
    }
}

#[test]
fn one_invalid_field_rejects_the_whole_object_not_just_that_window() {
    let with = |mutate: &dyn Fn(&mut Value)| {
        let mut primary = weekly();
        mutate(&mut primary);
        line(json!({"primary": primary, "secondary": short()}))
    };
    let cases: Vec<(&str, Value)> = vec![
        (
            "percent above 100",
            with(&|w| w["used_percent"] = json!(100.1)),
        ),
        (
            "negative percent",
            with(&|w| w["used_percent"] = json!(-0.1)),
        ),
        (
            "percent as text",
            with(&|w| w["used_percent"] = json!("78")),
        ),
        (
            "missing percent",
            with(&|w| {
                w.as_object_mut().unwrap().remove("used_percent");
            }),
        ),
        ("zero window", with(&|w| w["window_minutes"] = json!(0))),
        (
            "fractional window",
            with(&|w| w["window_minutes"] = json!(10080.5)),
        ),
        (
            "negative window",
            with(&|w| w["window_minutes"] = json!(-10080)),
        ),
        ("missing reset", with(&|w| w["resets_at"] = Value::Null)),
        ("negative reset", with(&|w| w["resets_at"] = json!(-1))),
        ("zero reset", with(&|w| w["resets_at"] = json!(0))),
        (
            "reset beyond the safe millisecond range",
            with(&|w| w["resets_at"] = json!(u64::MAX / 1000)),
        ),
        (
            "window that is not an object",
            line(json!({"primary": weekly(), "secondary": 5})),
        ),
        (
            "repeated window length",
            line(json!({"primary": weekly(), "secondary": weekly()})),
        ),
    ];
    for (name, entry) in cases {
        assert_eq!(codex(&entry), None, "{name}");
    }
    let mut no_time = line(json!({"primary": weekly()}));
    no_time.as_object_mut().unwrap().remove("timestamp");
    assert_eq!(codex(&no_time), None, "line without a timestamp");
    let mut bad_time = line(json!({"primary": weekly()}));
    bad_time["timestamp"] = json!("yesterday");
    assert_eq!(codex(&bad_time), None, "line with an unparsable timestamp");
}

#[test]
fn the_boundary_percentages_are_measurements() {
    for percent in [0.0, 100.0] {
        let mut window = weekly();
        window["used_percent"] = json!(percent);
        let limits = codex(&line(json!({"primary": window}))).unwrap();
        assert_eq!(limits.windows[0].used_percent, percent);
    }
}

#[test]
fn the_public_document_uses_camel_case_and_rejects_unknown_fields() {
    let limits = codex(&line(json!({"primary": weekly()}))).unwrap();
    let document = serde_json::to_value(&limits).unwrap();
    assert_eq!(
        document,
        json!({
            "observedAtMs": LINE_TIME_MS,
            "windows": [{"windowMinutes": 10080, "usedPercent": 78.0, "resetsAtMs": 1_791_948_558_000u64}],
        })
    );
    assert_eq!(
        serde_json::from_value::<RateLimits>(document.clone()).unwrap(),
        limits
    );
    let mut extra = document;
    extra["planType"] = json!("prolite");
    assert!(serde_json::from_value::<RateLimits>(extra).is_err());
}

#[test]
fn validity_requires_one_to_two_strictly_ascending_windows() {
    let window = |minutes| Window {
        window_minutes: minutes,
        used_percent: 1.0,
        resets_at_ms: 1,
    };
    let limits = |windows| RateLimits {
        observed_at_ms: 1,
        windows,
    };
    assert!(limits(vec![window(300), window(10_080)]).valid());
    for windows in [
        vec![],
        vec![window(10_080), window(300)],
        vec![window(300), window(300)],
        vec![window(1), window(2), window(3)],
    ] {
        assert!(!limits(windows.clone()).valid(), "{windows:?}");
    }
    let mut stale = limits(vec![window(300)]);
    stale.observed_at_ms = 0;
    assert!(!stale.valid());
    let mut nan = limits(vec![window(300)]);
    nan.windows[0].used_percent = f64::NAN;
    assert!(!nan.valid());
}
