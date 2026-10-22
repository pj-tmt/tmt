use super::*;

const ID: &str = "10000000-0000-4000-8000-000000000001";

#[test]
fn every_frozen_status_response_matches_the_same_projection() {
    let vectors: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/vectors/status-v1.json"
        ))
        .expect("the supplier's frozen status vectors"),
    )
    .unwrap();
    for vector in vectors["status"].as_array().unwrap() {
        let expected = &vector["response"];
        let mut settings = format!(
            "default = '{}'\nflushCount = 10\n",
            vector["given"]["globalDefault"].as_str().unwrap()
        );
        let mut identities = vec![];
        let mut stats = vec![];
        for row in expected["members"].as_array().unwrap() {
            let id = row["identityId"].as_str().unwrap();
            identities.push((id.to_owned(), row["name"].as_str().unwrap().to_owned()));
            // Inputs are independently fixed for these named examples; expected timing/labels
            // are never fed back into the projection.
            let (mode, held, age, attributed) = match row["name"].as_str().unwrap() {
                "Inherited auto" => (None, Some(0), None, false),
                "Explicit auto" => (Some("auto"), Some(3), Some(1000), true),
                "Custom interval" => (Some("20s"), Some(3), Some(0), true),
                "Due interval" => (Some("5m"), Some(4), Some(300000), true),
                "Off with held" => (Some("off"), Some(2), Some(1000), true),
                "Unavailable stats" => (Some("5m"), None, None, true),
                "Unknown setter" => (Some("5m"), Some(4), Some(279000), false),
                "Early flush" => (Some("5m"), Some(10), Some(279000), true),
                "Inherited interval" => (None, Some(2), Some(279000), false),
                "Explicit auto over interval default" => (Some("auto"), Some(3), Some(1000), false),
                name => panic!("Add independent inputs for new vector member {name}"),
            };
            if let Some(mode) = mode {
                settings.push_str(&format!(
                    "[members.{id}]\nmode = '{mode}'\nsetAtMs = 1791629940000\n"
                ));
                if attributed {
                    settings.push_str("setByIdentityId = '20000000-0000-4000-8000-000000000001'\n");
                }
            }
            stats.push(held.map(|held| Observation {
                held,
                delivered: None,
                observed: 1791629999000,
                age,
            }));
        }
        let settings = DigestConfig::parse(settings.as_bytes()).unwrap();
        let mut actual = project(&settings, &identities, &stats, 1791630000000).unwrap();
        for row in actual["members"].as_array_mut().unwrap() {
            assert!(row.get("oldestHeldAgeMs").is_some());
            assert!(row.get("ownerBypass").is_some());
            assert_eq!(row["deliveredDigests"], Value::Null);
            row.as_object_mut().unwrap().remove("oldestHeldAgeMs");
            row.as_object_mut().unwrap().remove("deliveredDigests");
            row.as_object_mut().unwrap().remove("ownerBypass");
        }
        assert_eq!(actual, *expected, "{}", vector["name"]);
    }
}

fn row(mode: &str, observation: Option<Observation>, now: u64) -> Value {
    let settings =
        DigestConfig::parse(format!("[members.{ID}]\nmode = '{mode}'\n").as_bytes()).unwrap();
    member(&settings, ID, "worker", observation, now).unwrap()
}

#[test]
fn zero_held_leaves_only_the_mode_choice_and_unknown_evidence_stays_visible() {
    for (mode, text) in [("auto", "Auto"), ("5m", "Every 5m"), ("off", "Off")] {
        let observation = Observation {
            held: 0,
            delivered: None,
            observed: 1000,
            age: None,
        };
        let idle = row(mode, Some(observation), 1000);
        assert_eq!(idle["labels"].as_array().unwrap().len(), 1, "{mode}");
        assert_eq!(idle["labels"][0]["text"], text);
        assert_eq!(idle["labels"][0]["action"]["current"], mode);
        assert_eq!(idle["nextSendAtMs"], Value::Null);
        assert_eq!(idle["timeToSendMs"], Value::Null);

        let held = row(
            mode,
            Some(Observation {
                held: 3,
                ..observation
            }),
            1000,
        );
        assert_eq!(held["labels"][1]["text"], "3 held");
        let drained = row(mode, Some(observation), 2000);
        assert_eq!(drained["labels"], idle["labels"]);

        let unknown = row(mode, None, 1000);
        assert_eq!(unknown["heldCount"], Value::Null);
        if mode != "off" {
            assert_eq!(unknown["labels"][1]["text"], "Unknown");
        }
    }
}

#[test]
fn missing_unsafe_or_impossible_deadlines_are_unknown_and_threshold_stays_explicit() {
    for observation in [
        Observation {
            held: 3,
            delivered: None,
            observed: 1000,
            age: None,
        },
        Observation {
            held: 3,
            delivered: None,
            observed: 1000,
            age: Some(1001),
        },
        Observation {
            held: 3,
            delivered: None,
            observed: MAX_NUMBER,
            age: Some(0),
        },
    ] {
        let document = row("5m", Some(observation), 1000);
        assert_eq!(document["heldCount"], 3);
        assert_eq!(document["nextSendAtMs"], Value::Null);
        assert_eq!(document["timeToSendMs"], Value::Null);
        assert_eq!(document["labels"][2]["text"], "Unknown");
    }
    let document = row(
        "5m",
        Some(Observation {
            held: 10,
            delivered: None,
            observed: 1000,
            age: None,
        }),
        1000,
    );
    assert_eq!(document["nextSendAtMs"], Value::Null);
    assert_eq!(document["timeToSendMs"], 0);
    assert_eq!(document["labels"][2]["text"], "Due now");
    let document = row(
        "off",
        Some(Observation {
            held: 10,
            delivered: None,
            observed: 1000,
            age: Some(0),
        }),
        1000,
    );
    assert_eq!(document["labels"].as_array().unwrap().len(), 2);
    assert_eq!(document["timeToSendMs"], Value::Null);
}

#[test]
fn duration_normalization_and_countdown_rounding_do_not_overflow_or_duplicate_presets() {
    for (mode, interval, text) in [
        ("0.025m", 1500, "1500ms"),
        ("300s", 300000, "5m"),
        ("48h", 172800000, "2d"),
    ] {
        let document = row(
            mode,
            Some(Observation {
                held: 1,
                delivered: None,
                observed: 1000,
                age: Some(0),
            }),
            1000,
        );
        assert_eq!(document["intervalMs"], interval);
        assert_eq!(document["settingValue"], text);
        let options = document["labels"][0]["action"]["options"]
            .as_array()
            .unwrap();
        assert_eq!(
            options.iter().filter(|item| item["value"] == text).count(),
            1
        );
        assert_eq!(
            document["labels"][0]["action"]["argv"],
            json!(["digest", ID, "{value}"])
        );
    }
    assert_eq!(countdown(1), "1s");
    assert_eq!(countdown(59999), "1m");
    assert_eq!(countdown(60001), "61s");
    assert_eq!(countdown(MAX_NUMBER), "9007199254741s");
}

#[test]
fn malformed_stats_never_become_zero_and_the_clock_is_js_safe() {
    for stats in [
        json!({"heldCount":0,"observedAtMs":1000}),
        json!({"heldCount":-1,"observedAtMs":1000,"oldestHeldAgeMs":null}),
        json!({"heldCount":0,"observedAtMs":MAX_NUMBER+1,"oldestHeldAgeMs":null}),
        json!({"heldCount":1,"observedAtMs":1000,"oldestHeldAgeMs":1.5}),
    ] {
        assert!(Observation::parse(&stats).is_none());
    }
    let settings = DigestConfig::parse(b"").unwrap();
    assert_eq!(
        project(&settings, &[], &[], MAX_NUMBER + 1)
            .unwrap_err()
            .code,
        "DIGEST_CLOCK_INVALID"
    );
}

#[test]
fn a_row_says_whether_a_recorded_owner_passes_its_hold() {
    const SETTER: &str = "20000000-0000-4000-8000-000000000001";
    let bypass = |toml: String| {
        let settings = DigestConfig::parse(toml.as_bytes()).unwrap();
        member(&settings, ID, "worker", None, 0).unwrap()["ownerBypass"].clone()
    };
    let row = |mode: &str, setter: &str| format!("[members.{ID}]\nmode = '{mode}'\n{setter}");
    let own = format!("setByIdentityId = '{SETTER}'\n");
    // A member that inherits auto follows the global default's recorded setter...
    assert_eq!(
        bypass(format!("defaultSetByIdentityId = '{SETTER}'\n")),
        json!(true)
    );
    // ...and with none recorded anywhere it owns its hold, which the row flags.
    assert_eq!(bypass(String::new()), json!(false));
    assert_eq!(bypass("default = '10m'\n".into()), json!(false));
    // An override stands on its own setter: it neither borrows the default's nor loses its own.
    assert_eq!(bypass(row("auto", &own)), json!(true));
    assert_eq!(bypass(row("5m", &own)), json!(true));
    assert_eq!(bypass(row("auto", "")), json!(false));
    assert_eq!(
        bypass(format!(
            "defaultSetByIdentityId = '{SETTER}'\n{}",
            row("auto", "")
        )),
        json!(false)
    );
    // The member being its own setter is no one else to pass, and `off` holds nothing.
    assert_eq!(
        bypass(row("5m", &format!("setByIdentityId = '{ID}'\n"))),
        json!(false)
    );
    assert_eq!(bypass(row("off", &own)), Value::Null);
    assert_eq!(
        bypass(format!(
            "default = 'off'\ndefaultSetByIdentityId = '{SETTER}'\n"
        )),
        Value::Null
    );
}

#[test]
fn the_global_switch_is_reported_with_the_default_it_would_restore() {
    let parse = |source: &str| DigestConfig::parse(source.as_bytes()).unwrap();
    assert_eq!(
        global(&parse("")),
        json!({"enabled":true,"default":"auto","text":"Digest: on (auto)"})
    );
    assert_eq!(
        global(&parse("default = '10m'\nenabled = false\n")),
        json!({"enabled":false,"default":"10m","text":"Digest: off"})
    );
    assert_eq!(
        global(&parse("default = '10m'\n")),
        json!({"enabled":true,"default":"10m","text":"Digest: on (10m)"})
    );
}

#[test]
fn an_auto_row_counts_down_to_its_own_caps_and_is_due_at_the_count_cap() {
    let auto = |source: &str, held: u64, age: u64| {
        let settings = DigestConfig::parse(source.as_bytes()).unwrap();
        let observation = Observation {
            held,
            delivered: None,
            observed: 100_000_000,
            age: Some(age),
        };
        member(&settings, ID, "worker", Some(observation), 100_000_000).unwrap()
    };
    let text = |row: &Value| row["labels"].as_array().unwrap().last().unwrap().clone();
    let member_row = |keys: &str| format!("[members.{ID}]\nmode = 'auto'\n{keys}");

    // The defaults: five messages or five minutes, 62 s into the hold.
    let row = auto(&member_row(""), 2, 62_000);
    assert_eq!(row["autoCaps"], json!({"maxWaitMs":300_000,"maxCount":5}));
    assert_eq!(row["timeToSendMs"], 238_000);
    assert_eq!(
        text(&row),
        json!({"text":"Latest in 3:58","colorClass":"muted","action":null})
    );

    // A member's caps win, and a global cap fills in the rest.
    let row = auto(
        &format!(
            "maxWait = '1h'\nmaxCount = 9\n{}",
            member_row("maxCount = 3\n")
        ),
        1,
        0,
    );
    assert_eq!(row["autoCaps"], json!({"maxWaitMs":3_600_000,"maxCount":3}));
    assert_eq!(text(&row)["text"], "Latest in 1:00:00");

    // The count cap makes it due now, whatever the wait left.
    let row = auto(&member_row("maxCount = 3\n"), 3, 1000);
    assert_eq!(row["timeToSendMs"], 0);
    assert_eq!(
        text(&row),
        json!({"text":"Due now","colorClass":"waiting","action":null})
    );

    // Only auto rows carry caps.
    assert!(row_without_caps().get("autoCaps").is_none());
}

fn row_without_caps() -> Value {
    row("5m", None, 1000)
}
