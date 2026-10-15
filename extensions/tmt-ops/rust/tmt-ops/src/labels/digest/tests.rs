use super::*;
use serde_json::json;

fn document() -> Value {
    super::super::tests::vectors()["status"][0]["response"].clone()
}

fn parse(document: &Value) -> Option<Status> {
    let labels = super::super::parse("digest", document)?;
    Status::from_source("digest", document, &labels)
}

#[test]
fn supplier_vectors_preserve_settings_null_evidence_and_source_times() {
    for vector in super::super::tests::vectors()["status"].as_array().unwrap() {
        let document = &vector["response"];
        let status = parse(document).expect("supplier status vector");
        assert_eq!(
            status.observed_at_ms,
            document["observedAtMs"].as_u64().unwrap()
        );
        assert_eq!(
            status.members.len(),
            document["members"].as_array().unwrap().len()
        );
        for supplied in document["members"].as_array().unwrap() {
            let member = &status.members[supplied["identityId"].as_str().unwrap()];
            assert_eq!(member.name, supplied["name"]);
            assert_eq!(member.held_count, supplied["heldCount"].as_u64());
            assert_eq!(
                member.stats_observed_at_ms,
                supplied["statsObservedAtMs"].as_u64()
            );
            assert_eq!(member.next_send_at_ms, supplied["nextSendAtMs"].as_u64());
            assert_eq!(member.time_to_send_ms, supplied["timeToSendMs"].as_u64());
            assert_eq!(member.setting_value, supplied["settingValue"]);
            assert_eq!(member.set_at_ms, supplied["setAtMs"].as_u64());
            assert_eq!(
                member.set_by_identity_id.as_deref(),
                supplied["setByIdentityId"].as_str()
            );
        }
    }
}

#[test]
fn auto_default_and_override_are_distinct_without_invented_deadlines() {
    let status = parse(&document()).unwrap();
    let inherited = &status.members["10000000-0000-4000-8000-000000000001"];
    let explicit = &status.members["10000000-0000-4000-8000-000000000002"];
    assert_eq!(inherited.mode, Mode::Auto);
    assert_eq!(inherited.setting_source, SettingSource::Default);
    assert_eq!(inherited.held_count, Some(0));
    assert_eq!(explicit.mode, Mode::Auto);
    assert_eq!(explicit.setting_source, SettingSource::Member);
    assert_eq!(explicit.held_count, Some(3));
    assert_eq!(explicit.next_send_at_ms, None);
    assert_eq!(explicit.time_to_send_ms, None);
}

#[test]
fn zero_count_and_unavailable_count_remain_distinct() {
    let mut document = document();
    let id = document["members"][0]["identityId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(parse(&document).unwrap().members[&id].held_count, Some(0));
    document["members"][0]["heldCount"] = Value::Null;
    document["members"][0]["statsObservedAtMs"] = Value::Null;
    let unknown = parse(&document).unwrap();
    assert_eq!(unknown.members[&id].held_count, None);
    assert_eq!(unknown.members[&id].stats_observed_at_ms, None);
    document["members"][0]
        .as_object_mut()
        .unwrap()
        .remove("heldCount");
    assert!(
        parse(&document).is_none(),
        "missing required evidence is not null or zero"
    );
}

#[test]
fn required_shapes_safe_numbers_and_canonical_identity_fail_closed() {
    for (field, value) in [
        ("identityId", json!("alias")),
        ("name", json!(false)),
        ("identityId", json!("AAAAAAAA-0000-4000-8000-000000000001")),
        ("mode", json!("idle")),
        ("intervalMs", json!(0)),
        ("settingSource", json!("global")),
        ("settingValue", json!("auto")),
        ("heldCount", json!(-1)),
        ("heldCount", json!(1.5)),
        ("statsObservedAtMs", json!(SAFE + 1)),
        ("nextSendAtMs", json!(1)),
        ("timeToSendMs", json!("0")),
        ("setAtMs", json!(0)),
        ("setByIdentityId", json!("not-an-identity")),
    ] {
        let mut document = document();
        document["members"][0][field] = value;
        assert!(
            parse(&document).is_none(),
            "accepted {field}: {}",
            document["members"][0][field]
        );
    }
    for field in ["version", "observedAtMs", "members"] {
        let mut document = document();
        document.as_object_mut().unwrap().remove(field);
        assert!(parse(&document).is_none());
    }
    let mut document = document();
    document["version"] = json!(2);
    assert!(parse(&document).is_none());
}

#[test]
fn duplicate_identity_is_not_silently_overwritten_and_namespace_is_external() {
    let mut document = document();
    let labels = super::super::parse("digest", &document).unwrap();
    assert!(Status::from_source("other", &document, &labels).is_none());
    let duplicate = document["members"][0].clone();
    document["members"].as_array_mut().unwrap().push(duplicate);
    assert!(parse(&document).is_none());
}

#[test]
fn threshold_zero_retains_scheduled_deadline_and_additive_fields_are_ignored() {
    let mut document = document();
    let id = document["members"][2]["identityId"]
        .as_str()
        .unwrap()
        .to_owned();
    document["members"][2]["timeToSendMs"] = json!(0);
    let status = parse(&document).unwrap();
    assert_eq!(status.members[&id].mode, Mode::Interval);
    assert_eq!(status.members[&id].time_to_send_ms, Some(0));
    assert!(status.members[&id].next_send_at_ms.unwrap() > status.observed_at_ms);
    document["future"] = json!({"unknown": true});
    document["members"][2]["future"] = json!([1, 2]);
    // Unsupported additive savings stays unavailable without hiding valid policy.
    document["members"][2]["savings"] = json!({"future": true});
    assert_eq!(parse(&document), Some(status));
}

#[test]
fn published_savings_keep_the_supplied_window_coverage_and_known_zero() {
    for (window, tokens) in [("24h", 2700), ("7d", 0), ("lifetime", 18_200)] {
        let mut document = document();
        let id = document["members"][0]["identityId"]
            .as_str()
            .unwrap()
            .to_owned();
        document["members"][0]["savings"] = json!({
            "method":"context-input-avoided-v1", "window":window,
            "fromMs":100, "throughMs":1000,
            "estimatedInputTokensAvoided":tokens, "avoidedSeparateDeliveries":2,
            "complete":true, "gap":null, "cost":null, "percent":null
        });
        let parsed = parse(&document).expect("valid existing labels and status");
        assert_eq!(
            parsed.members[&id].savings,
            Some(Savings {
                method: "context-input-avoided-v1".into(),
                window: crate::config::DigestWindow::parse(window).unwrap(),
                from_ms: 100,
                through_ms: 1000,
                tokens: Some(tokens),
                complete: true,
                gap: None,
            })
        );
    }
}

#[test]
fn optional_savings_do_not_remove_valid_policy_or_labels() {
    let mut document = document();
    let id = document["members"][0]["identityId"]
        .as_str()
        .unwrap()
        .to_owned();
    let baseline = parse(&document).unwrap();
    for savings in [Value::Null, json!({"future":true}), json!("not evidence")] {
        document["members"][0]["savings"] = savings;
        let parsed = parse(&document).expect("optional estimate cannot erase valid policy");
        assert_eq!(parsed.members[&id].savings, None);
        assert_eq!(
            parsed.members[&id].held_count,
            baseline.members[&id].held_count
        );
        assert!(!super::super::parse("digest", &document).unwrap()[&id].is_empty());
    }
    document["members"][0]
        .as_object_mut()
        .unwrap()
        .remove("savings");
    assert_eq!(parse(&document).unwrap().members[&id].savings, None);
}

#[test]
fn digest_window_uses_supplier_arguments_and_honest_display_names() {
    use crate::config::DigestWindow;
    assert_eq!(DigestWindow::default(), DigestWindow::Day);
    for (window, argument, label) in [
        (DigestWindow::Day, "24h", "24h"),
        (DigestWindow::Week, "7d", "7d"),
        (DigestWindow::AllTime, "lifetime", "all time"),
    ] {
        assert_eq!(DigestWindow::parse(argument), Some(window));
        assert_eq!(window.argument(), argument);
        assert_eq!(window.label(), label);
    }
    assert_eq!(DigestWindow::parse("1h"), None);
}

fn with_savings(window: &str, values: &[(u64, u64, u64)]) -> Value {
    let mut document = document();
    for (row, (from, through, tokens)) in document["members"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(values)
    {
        row["savings"] = json!({"method":"context-input-avoided-v1", "window":window,
            "fromMs":from, "throughMs":through, "estimatedInputTokensAvoided":tokens,
            "complete":true, "gap":null});
    }
    document
}

#[test]
fn independently_dated_estimates_sum_once_with_explicit_unavailable_members() {
    let mut document = with_savings(
        "lifetime",
        &[(100, 1000, 2700), (200, 2000, 0), (300, 3000, 99)],
    );
    document["members"][2]["savings"]["window"] = json!("7d");
    let status = parse(&document).unwrap();
    let ids: Vec<_> = document["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["identityId"].as_str().unwrap())
        .collect();
    let missing = "10000000-0000-4000-8000-000000000099";
    let supplied = [ids[0], ids[1], ids[2], ids[0], missing];
    let summary = summarize_savings(
        Some((&status, false)),
        supplied,
        crate::config::DigestWindow::AllTime,
    );
    assert_eq!(
        summary,
        SavingsSummary {
            tokens: Some(2700),
            included: 2,
            total: 4,
            earliest_from_ms: Some(100),
            latest_from_ms: Some(200)
        }
    );
    for evidence in [None, Some((&status, true))] {
        assert_eq!(
            summarize_savings(evidence, supplied, crate::config::DigestWindow::AllTime),
            SavingsSummary {
                total: 4,
                ..Default::default()
            }
        );
    }
    assert_eq!(
        summarize_savings(
            Some((&status, false)),
            [ids[1]],
            crate::config::DigestWindow::AllTime
        )
        .tokens,
        Some(0)
    );
    assert_eq!(
        summarize_savings(
            Some((&status, false)),
            supplied,
            crate::config::DigestWindow::Day
        )
        .tokens,
        None
    );
}

#[test]
fn malformed_optional_estimates_never_poison_other_status_evidence() {
    let baseline = with_savings("24h", &[(100, 1000, 20)]);
    for (field, value) in [
        ("fromMs", json!(1001)),
        ("throughMs", json!(SAFE + 1)),
        ("estimatedInputTokensAvoided", json!(-1)),
        ("estimatedInputTokensAvoided", json!(1.5)),
        ("estimatedInputTokensAvoided", json!(SAFE + 1)),
        ("method", json!(false)),
        ("window", json!("1h")),
        ("complete", json!("true")),
        ("gap", json!(42)),
        ("gap", json!("missing_context")),
    ] {
        let mut document = baseline.clone();
        document["members"][0]["savings"][field] = value;
        let parsed = parse(&document).expect("optional estimate cannot poison policy");
        assert!(
            parsed.members.values().next().unwrap().savings.is_none(),
            "{field}"
        );
    }
}

#[test]
fn unknown_methods_and_gaps_are_unavailable_while_sum_overflow_never_wraps() {
    let mut document = with_savings("7d", &[(100, 1000, SAFE), (200, 2000, 1), (300, 3000, 2)]);
    let ids: Vec<_> = document["members"]
        .as_array()
        .unwrap()
        .iter()
        .take(3) // Only this squad's members contribute, not every supplied identity.
        .map(|row| row["identityId"].as_str().unwrap().to_owned())
        .collect();
    document["members"][2]["savings"]["method"] = json!("future-estimate");
    let status = parse(&document).unwrap();
    let summary = summarize_savings(
        Some((&status, false)),
        ids.iter().map(String::as_str),
        crate::config::DigestWindow::Week,
    );
    assert_eq!(
        (summary.tokens, summary.included, summary.total),
        (None, 2, 3)
    );
    document["members"][0]["savings"]["estimatedInputTokensAvoided"] = Value::Null;
    document["members"][0]["savings"]["complete"] = json!(false);
    document["members"][0]["savings"]["gap"] = json!("missing_context");
    let status = parse(&document).unwrap();
    assert_eq!(
        status.members[&ids[0]]
            .savings
            .as_ref()
            .unwrap()
            .gap
            .as_deref(),
        Some("missing_context")
    );
    let summary = summarize_savings(
        Some((&status, false)),
        ids.iter().map(String::as_str),
        crate::config::DigestWindow::Week,
    );
    assert_eq!(
        (summary.tokens, summary.included, summary.total),
        (Some(1), 1, 3)
    );
}

#[test]
fn source_window_argv_is_supplier_owned_and_other_sources_remain_generic() {
    use crate::config::DigestWindow;
    assert_eq!(
        status_arguments("digest", DigestWindow::Day),
        ["digest", "status"]
    );
    assert_eq!(
        status_arguments("digest", DigestWindow::Week),
        ["digest", "status", "--window", "7d"]
    );
    assert_eq!(
        status_arguments("digest", DigestWindow::AllTime),
        ["digest", "status", "--window", "lifetime"]
    );
    for window in [DigestWindow::Day, DigestWindow::Week, DigestWindow::AllTime] {
        assert_eq!(
            status_arguments("other-supplier", window),
            ["other-supplier", "status"]
        );
    }
}
