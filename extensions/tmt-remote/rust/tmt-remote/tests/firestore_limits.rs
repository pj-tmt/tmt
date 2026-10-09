//! The free-plan limits member (#2180): the table matches the numbers hand-copied from the
//! official pages into a fixture, the validator accepts only that shape, and the human lines
//! say what the numbers are and are not.
use serde_json::{Value, json};
use std::fs;
use tmt_remote::firestore_limits::{LIMITS, human_lines, member, validate};

fn fixture() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/firestore_budget/limits-member.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn the_member_equals_the_numbers_read_from_the_official_pages() {
    assert_eq!(member(), fixture());
    assert!(validate(&member()));
    assert_eq!(
        member()["limits"].as_object().unwrap().len(),
        LIMITS.len(),
        "every table row is in the member"
    );
}

#[test]
fn the_validator_accepts_only_the_member_shape() {
    let good = fixture();
    // A door of another build may carry other values, not another shape.
    let mut other_values = good.clone();
    other_values["limits"]["readsPerDay"] = json!(1);
    other_values["readOn"] = json!("2027-01-02");
    assert!(validate(&other_values));
    let mut cases: Vec<(&str, Value)> = vec![("not an object", json!([])), ("null", Value::Null)];
    let mutate = |change: &dyn Fn(&mut Value)| {
        let mut value = good.clone();
        change(&mut value);
        value
    };
    cases.push(("extra top key", mutate(&|v| v["seen"] = json!(1))));
    cases.push((
        "missing top key",
        mutate(&|v| {
            v.as_object_mut().unwrap().remove("guard");
        }),
    ));
    cases.push(("extra limit", mutate(&|v| v["limits"]["extra"] = json!(1))));
    cases.push((
        "missing limit",
        mutate(&|v| {
            v["limits"].as_object_mut().unwrap().remove("databases");
        }),
    ));
    cases.push((
        "float limit",
        mutate(&|v| v["limits"]["readsPerDay"] = json!(1.5)),
    ));
    cases.push((
        "zero limit",
        mutate(&|v| v["limits"]["readsPerDay"] = json!(0)),
    ));
    cases.push((
        "string limit",
        mutate(&|v| v["limits"]["readsPerDay"] = json!("50000")),
    ));
    cases.push((
        "negative limit",
        mutate(&|v| v["limits"]["readsPerDay"] = json!(-1)),
    ));
    cases.push((
        "other plan",
        mutate(&|v| v["plan"] = json!("pay-as-you-go")),
    ));
    cases.push((
        "free-form reset",
        mutate(&|v| v["resetsAt"] = json!("whenever")),
    ));
    cases.push(("bad date", mutate(&|v| v["readOn"] = json!("yesterday"))));
    cases.push(("date shape", mutate(&|v| v["readOn"] = json!("2026-1-9x"))));
    cases.push((
        "inverted guard",
        mutate(&|v| v["guard"]["warnPercent"] = json!(95)),
    ));
    cases.push((
        "guard over 100",
        mutate(&|v| v["guard"]["refusePercent"] = json!(101)),
    ));
    cases.push(("guard extra", mutate(&|v| v["guard"]["more"] = json!(1))));
    for (name, case) in cases {
        assert!(!validate(&case), "{name} must be refused");
    }
}

#[test]
fn the_human_lines_name_each_limit_and_what_they_are_not() {
    let lines = human_lines(&member());
    assert_eq!(
        lines,
        [
            "Firestore free-plan limits (read 2026-10-09; Google changes them, so recheck before relying on them):",
            "  Document reads                      50,000 per day",
            "  Document writes                     20,000 per day",
            "  Document deletes                    20,000 per day",
            "  Stored data                         1 GiB",
            "  Data sent out                       10 GiB per month",
            "  Free databases per project          1",
            "  Composite indexes                   200",
            "  Single-field index configs          200",
            "  Document size                       1 MiB",
            "  Rules document lookups per request  10",
            "The whole Firebase project shares these limits and the daily ones reset around midnight Pacific. Remote cannot see the project's real usage; only the Firebase console shows it.",
            "A client warns at 70% and refuses at 90% of its own share before it reaches a limit.",
        ]
    );
    // Values come from the validated answer, so an older door's numbers are shown as read.
    let mut older = member();
    older["readOn"] = json!("2026-09-01");
    older["limits"]["readsPerDay"] = json!(30000);
    let shown = human_lines(&older);
    assert!(shown[0].contains("read 2026-09-01"), "{}", shown[0]);
    assert!(shown[1].ends_with("30,000 per day"), "{}", shown[1]);
}
