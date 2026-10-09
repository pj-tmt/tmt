//! Layered Firestore readiness (#2179, slice 1). The fixture source produces every
//! prerequisite state; goldens are written by hand from the documented shape.
#[allow(dead_code)]
#[path = "support/door.rs"]
mod door;
#[path = "support/readiness.rs"]
mod fixture;

use door::Harness;
use fixture::{ALL_ENABLED, FixtureEvidence, scenarios};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
};
use tmt_remote::{
    control::{self, StatusProjection},
    pairing::Timing,
    readiness::{
        FirestoreEvidence, FirestoreEvidenceSource, FirestoreItem, FirestoreLayer,
        FirestoreReason as R, FirestoreTier, LAYERS, LayerSpec, NotConfigured, Observed,
        human_lines, project, validate,
    },
};

fn projected(evidence: FirestoreEvidence) -> Value {
    project(Some(&evidence), &LAYERS)
}
fn layer<'a>(value: &'a Value, name: &str) -> &'a Value {
    value
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["layer"] == name)
        .expect(name)
}
const DEPLOY: &str = "tmt remote deploy firestore";

#[test]
fn nothing_is_projected_without_evidence() {
    assert_eq!(project(None, &LAYERS), json!([]));
    assert!(validate(&json!([]), &LAYERS));
    assert_eq!(NotConfigured.evidence(), None);
    // Honesty: with no deployment nothing names the deploy command, in JSON or in words.
    let lines = human_lines(&json!([]), &LAYERS);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        (lines[0].what.as_str(), lines[0].hint.as_deref()),
        ("Firestore is not configured.", None)
    );
    assert!(!project(None, &LAYERS).to_string().contains("deploy"));
}

#[test]
fn free_plan_golden_marks_device_operations_paid_and_keeps_the_others() {
    let free = FirestoreEvidence {
        tier: FirestoreTier::Free,
        ..ALL_ENABLED
    };
    assert_eq!(
        projected(free),
        json!([
            {"layer": "sharing", "state": "enabled", "prerequisites": [
                {"item": "project", "state": "enabled"},
                {"item": "sign-in", "state": "enabled"},
                {"item": "rules", "state": "enabled"},
                {"item": "plan-tier", "state": "enabled"},
                {"item": "quota", "state": "enabled"}]},
            {"layer": "operations", "state": "not-enabled", "prerequisites": [
                {"item": "plan-tier", "state": "not-enabled", "reason": "paid-plan-required"},
                {"item": "support", "state": "not-enabled", "reason": "not-implemented"}]},
            {"layer": "attachments", "state": "not-enabled", "prerequisites": [
                {"item": "support", "state": "not-enabled", "reason": "not-implemented"},
                {"item": "project", "state": "enabled"},
                {"item": "rules", "state": "enabled"},
                {"item": "quota", "state": "enabled"}]}
        ])
    );
}

#[test]
fn rules_out_of_date_golden_names_the_next_command_and_the_sentence() {
    let stale = FirestoreEvidence {
        rules: Observed::Off(R::OutOfDate),
        ..ALL_ENABLED
    };
    let value = projected(stale);
    assert_eq!(
        layer(&value, "sharing"),
        &json!({"layer": "sharing", "state": "not-enabled", "prerequisites": [
            {"item": "project", "state": "enabled"},
            {"item": "sign-in", "state": "enabled"},
            {"item": "rules", "state": "not-enabled", "reason": "out-of-date", "next": DEPLOY},
            {"item": "plan-tier", "state": "enabled"},
            {"item": "quota", "state": "enabled"}]})
    );
    let lines = human_lines(&value, &LAYERS);
    assert_eq!(
        lines[0].what,
        "Page sharing is not enabled. Firestore rules are out of date."
    );
    assert_eq!(lines[0].hint.as_deref(), Some(DEPLOY));
    assert_eq!(
        lines[1].what,
        "Device operations is not enabled. Not available in this release."
    );
    assert_eq!(
        lines[2].what,
        "Attachments is not enabled. Not available in this release."
    );
    assert!(lines.iter().skip(1).all(|l| l.hint.is_none()));
}

#[test]
fn every_prerequisite_state_is_produced_valid_and_seen() {
    let mut seen = std::collections::BTreeSet::new();
    for (label, evidence) in scenarios() {
        let value = projected(evidence);
        assert!(validate(&value, &LAYERS), "{label}: {value}");
        for layer in value.as_array().unwrap() {
            for p in layer["prerequisites"].as_array().unwrap() {
                let reason = p["reason"].as_str().unwrap_or("-");
                seen.insert((
                    p["item"].as_str().unwrap().to_owned(),
                    p["state"].as_str().unwrap().to_owned(),
                    reason.to_owned(),
                ));
            }
        }
    }
    let expected = [
        ("project", "enabled", "-"),
        ("project", "not-enabled", "not-configured"),
        ("project", "not-enabled", "access-lost"),
        ("project", "unknown", "not-checked"),
        ("sign-in", "enabled", "-"),
        ("sign-in", "not-enabled", "provider-disabled"),
        ("sign-in", "not-enabled", "permission-missing"),
        ("sign-in", "unknown", "not-checked"),
        ("rules", "enabled", "-"),
        ("rules", "not-enabled", "not-deployed"),
        ("rules", "not-enabled", "out-of-date"),
        ("rules", "not-enabled", "partial"),
        ("rules", "unknown", "not-checked"),
        ("plan-tier", "enabled", "-"),
        ("plan-tier", "not-enabled", "paid-plan-required"),
        ("plan-tier", "unknown", "not-checked"),
        ("quota", "enabled", "-"),
        ("quota", "not-enabled", "headroom-low"),
        ("quota", "not-enabled", "exhausted"),
        ("quota", "unknown", "not-checked"),
        ("support", "not-enabled", "not-implemented"),
    ];
    let expected: std::collections::BTreeSet<_> = expected
        .iter()
        .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
        .collect();
    assert_eq!(seen, expected);
}

#[test]
fn layers_are_checked_independently() {
    let base = projected(ALL_ENABLED);
    let changed = |evidence: FirestoreEvidence| {
        let value = projected(evidence);
        ["sharing", "operations", "attachments"]
            .into_iter()
            .filter(|name| layer(&value, name) != layer(&base, name))
            .collect::<Vec<_>>()
    };
    let on = |f: fn(&mut FirestoreEvidence)| {
        let mut evidence = ALL_ENABLED;
        f(&mut evidence);
        evidence
    };
    assert_eq!(
        changed(on(|e| e.rules = Observed::Off(R::NotDeployed))),
        ["sharing", "attachments"]
    );
    assert_eq!(
        changed(on(|e| e.project = Observed::Off(R::AccessLost))),
        ["sharing", "attachments"]
    );
    assert_eq!(
        changed(on(|e| e.quota = Observed::Off(R::Exhausted))),
        ["sharing", "attachments"]
    );
    assert_eq!(
        changed(on(|e| e.sign_in = Observed::Off(R::ProviderDisabled))),
        ["sharing"]
    );
    assert_eq!(
        changed(on(|e| e.tier = FirestoreTier::Free)),
        ["operations"]
    );
    assert_eq!(
        changed(on(|e| e.tier = FirestoreTier::Unknown)),
        ["sharing", "operations"]
    );
    // Device operations and attachments are not offered by this release whatever is recorded.
    for (label, evidence) in scenarios() {
        let value = projected(evidence);
        for name in ["operations", "attachments"] {
            assert_eq!(
                layer(&value, name)["state"],
                "not-enabled",
                "{label} {name}"
            );
        }
    }
}

#[test]
fn paid_plan_required_is_one_table_row() {
    let free = FirestoreEvidence {
        tier: FirestoreTier::Free,
        ..ALL_ENABLED
    };
    let spec = |requires_paid_plan| {
        [LayerSpec {
            layer: FirestoreLayer::Operations,
            requires_paid_plan,
            items: &[FirestoreItem::PlanTier],
        }]
    };
    let paid = project(Some(&free), &spec(true));
    assert_eq!(
        paid[0]["prerequisites"][0],
        json!({"item": "plan-tier", "state": "not-enabled", "reason": "paid-plan-required"})
    );
    assert_eq!(project(Some(&free), &spec(false))[0]["state"], "enabled");
    assert!(validate(&paid, &spec(true)));
    // The production table's only paid layer is device operations.
    let paying: Vec<_> = LAYERS
        .iter()
        .filter(|l| l.requires_paid_plan)
        .map(|l| l.layer)
        .collect();
    assert_eq!(paying, [FirestoreLayer::Operations]);
}

#[test]
fn missing_or_impossible_evidence_is_unknown_never_guessed() {
    let value = projected(FirestoreEvidence {
        project: Observed::Unknown,
        ..ALL_ENABLED
    });
    assert_eq!(layer(&value, "sharing")["state"], "unknown");
    assert_eq!(
        layer(&value, "sharing")["prerequisites"][0],
        json!({"item": "project", "state": "unknown", "reason": "not-checked"})
    );
    // A reason the item cannot report is no evidence.
    let odd = projected(FirestoreEvidence {
        project: Observed::Off(R::PaidPlanRequired),
        ..ALL_ENABLED
    });
    assert_eq!(
        layer(&odd, "sharing")["prerequisites"][0],
        json!({"item": "project", "state": "unknown", "reason": "not-checked"})
    );
    // Unknown never hides a definite `not-enabled` in the same layer.
    let mixed = projected(FirestoreEvidence {
        project: Observed::Unknown,
        rules: Observed::Off(R::Partial),
        ..ALL_ENABLED
    });
    assert_eq!(layer(&mixed, "sharing")["state"], "not-enabled");
    let lines = human_lines(&value, &LAYERS);
    assert_eq!(
        lines[0].what,
        "Page sharing is not checked. The Firebase project has not been checked."
    );
    assert_eq!(lines[0].hint, None);
}

#[test]
fn the_human_line_names_the_prerequisite_behind_the_layer_state() {
    let sharing =
        |evidence: FirestoreEvidence| human_lines(&projected(evidence), &LAYERS)[0].clone();
    // A definite not-enabled outranks an unknown, whichever comes first in the layer.
    let unknown_first = sharing(FirestoreEvidence {
        project: Observed::Unknown,
        rules: Observed::Off(R::NotDeployed),
        ..ALL_ENABLED
    });
    assert_eq!(
        unknown_first.what,
        "Page sharing is not enabled. Firestore rules are not deployed."
    );
    assert_eq!(unknown_first.hint.as_deref(), Some(DEPLOY));
    let off_first = sharing(FirestoreEvidence {
        project: Observed::Off(R::AccessLost),
        rules: Observed::Unknown,
        ..ALL_ENABLED
    });
    assert_eq!(
        off_first.what,
        "Page sharing is not enabled. Access to the Firebase project was lost."
    );
    // With nothing definite, the first unknown names the layer.
    let unknowns = sharing(FirestoreEvidence {
        sign_in: Observed::Unknown,
        rules: Observed::Unknown,
        ..ALL_ENABLED
    });
    assert_eq!(
        unknowns.what,
        "Page sharing is not checked. Sign-in has not been checked."
    );
}

#[test]
fn the_validator_accepts_only_what_the_table_derives() {
    let good = projected(FirestoreEvidence {
        rules: Observed::Off(R::OutOfDate),
        quota: Observed::Unknown,
        ..ALL_ENABLED
    });
    assert!(validate(&good, &LAYERS));
    let refuses = |label: &str, change: &dyn Fn(&mut Value)| {
        let mut value = good.clone();
        change(&mut value);
        assert!(!validate(&value, &LAYERS), "{label}: {value}");
    };
    let rules = |v: &mut Value| v[0]["prerequisites"][2].take();
    let _ = rules;
    refuses("not an array", &|v| *v = json!({}));
    refuses("null", &|v| *v = Value::Null);
    refuses("one layer only", &|v| {
        v.as_array_mut().unwrap().truncate(1);
    });
    refuses("layers reordered", &|v| {
        v.as_array_mut().unwrap().swap(0, 1)
    });
    refuses("duplicate layer", &|v| v[1] = v[0].clone());
    refuses("unknown layer", &|v| v[0]["layer"] = json!("device"));
    refuses("extra layer member", &|v| v[0]["note"] = json!("x"));
    refuses("layer state not derived", &|v| {
        v[0]["state"] = json!("enabled")
    });
    refuses("layer state unknown word", &|v| {
        v[0]["state"] = json!("off")
    });
    refuses("prerequisites reordered", &|v| {
        v[0]["prerequisites"].as_array_mut().unwrap().swap(0, 1)
    });
    refuses("prerequisite dropped", &|v| {
        v[0]["prerequisites"].as_array_mut().unwrap().pop();
    });
    refuses("extra prerequisite member", &|v| {
        v[0]["prerequisites"][0]["detail"] = json!("x")
    });
    refuses("extra member on a not-enabled prerequisite", &|v| {
        v[0]["prerequisites"][2]["detail"] = json!("x")
    });
    refuses("extra member on an unknown prerequisite", &|v| {
        v[0]["prerequisites"][4]["detail"] = json!("x")
    });
    refuses("paid plan required outside a paid-plan layer", &|v| {
        v[0]["prerequisites"][3] =
            json!({"item": "plan-tier", "state": "not-enabled", "reason": "paid-plan-required"});
        v[0]["state"] = json!("not-enabled");
    });
    refuses("reason on enabled", &|v| {
        v[0]["prerequisites"][0]["reason"] = json!("not-checked")
    });
    refuses("missing reason", &|v| {
        v[0]["prerequisites"][2]
            .as_object_mut()
            .unwrap()
            .remove("reason");
    });
    refuses("reason of another item", &|v| {
        v[0]["prerequisites"][2]["reason"] = json!("exhausted")
    });
    refuses("free-text reason", &|v| {
        v[0]["prerequisites"][2]["reason"] = json!("quota exceeded for project my-app")
    });
    refuses("next missing", &|v| {
        v[0]["prerequisites"][2]
            .as_object_mut()
            .unwrap()
            .remove("next");
    });
    refuses("next not from the table", &|v| {
        v[0]["prerequisites"][2]["next"] = json!("tmt remote deploy cloudflare")
    });
    refuses("next where none exists", &|v| {
        v[0]["prerequisites"][4]["next"] = json!(DEPLOY)
    });
    refuses("unknown with a definite reason", &|v| {
        v[0]["prerequisites"][4]["reason"] = json!("exhausted")
    });
    refuses("not-enabled with not-checked", &|v| {
        v[0]["prerequisites"][2]["reason"] = json!("not-checked")
    });
    refuses("state not a string", &|v| {
        v[0]["prerequisites"][0]["state"] = json!(true)
    });
}

fn raw_request(h: &Harness, request: &Value) -> Value {
    let mut stream = UnixStream::connect(h.root.join("remote/control.sock")).unwrap();
    writeln!(stream, "{request}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn the_control_socket_answers_layers_and_leaves_every_other_projection_alone() {
    let plain = Harness::new(Timing::CONTRACT);
    let stale = FirestoreEvidence {
        rules: Observed::Off(R::OutOfDate),
        ..ALL_ENABLED
    };
    let configured = Harness::with_evidence(Timing::CONTRACT, FixtureEvidence::shared(stale));
    for (h, expected) in [(&plain, json!([])), (&configured, projected(stale))] {
        let answer = raw_request(h, &json!({"op": "status", "layers": true}));
        assert_eq!(answer["firestoreLayers"], expected);
        assert_eq!(answer.as_object().unwrap().len(), 4);
        // The client validates exactly this shape.
        let checked = control::status_projection(&h.root.join("remote"), StatusProjection::Layers)
            .unwrap()
            .unwrap();
        assert_eq!(checked["firestoreLayers"], expected);
        // Ordinary, machine and objects answers keep their exact shapes.
        let ordinary = raw_request(h, &json!({"op": "status"}));
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&ordinary), ["running", "origin", "path"]);
        assert_eq!(
            keys(&raw_request(h, &json!({"op": "status", "machine": true}))),
            ["running", "origin", "path", "machineId"]
        );
        assert_eq!(
            keys(&raw_request(h, &json!({"op": "status", "objects": true}))),
            ["running", "origin", "path", "objectChannels"]
        );
        assert!(!ordinary.to_string().contains("firestore"));
        // Extra members are not a projection request.
        let refused = raw_request(h, &json!({"op": "status", "layers": true, "machine": true}));
        assert_eq!(refused["error"]["code"], "REMOTE_CONTROL_UNSUPPORTED");
    }
    // Same-shaped answers: configuring Firestore changes only the optional projection.
    assert_eq!(
        raw_request(&plain, &json!({"op": "status"}))
            .as_object()
            .unwrap()
            .len(),
        raw_request(&configured, &json!({"op": "status"}))
            .as_object()
            .unwrap()
            .len()
    );
}
