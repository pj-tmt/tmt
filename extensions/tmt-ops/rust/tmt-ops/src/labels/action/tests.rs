//! The action against the supplier's frozen vectors.
use super::*;
use serde_json::json;
use std::fs;

const VECTORS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../tmt-digest/contracts/vectors/status-v1.json"
);
const MEMBER: &str = "10000000-0000-4000-8000-000000000003";

fn vectors() -> Vec<Value> {
    let document: Value =
        serde_json::from_slice(&fs::read(VECTORS).expect("the supplier's vectors")).unwrap();
    document["actions"].as_array().unwrap().clone()
}

fn reason(name: &str) -> Refusal {
    match name {
        "kind" => Refusal::Kind,
        "argv-shape" => Refusal::ArgvShape,
        "namespace" => Refusal::Namespace,
        "placeholder" => Refusal::Placeholder,
        other => panic!("unknown refusal {other}"),
    }
}

fn choose(argv: Value) -> Value {
    json!({"kind":"choose","argv":argv,
        "options":[{"label":"Auto","value":"auto"},{"label":"5m","value":"5m"}],"current":"auto"})
}

#[test]
fn every_choose_vector_expands_or_is_refused_as_the_supplier_froze_it() {
    let all = vectors();
    assert_eq!(all.len(), 12);
    for vector in &all {
        let name = vector["name"].as_str().unwrap();
        let action = &vector["action"];
        let expected = &vector["expected"];
        let parsed = Choose::parse("digest", action);
        if action["kind"] != "choose" {
            // Nothing here runs `run` actions, valid or not: such a label stays display-only.
            assert!(parsed.is_err(), "{name}");
            continue;
        }
        match expected["result"].as_str().unwrap() {
            "argv" => {
                let selected = vector["selectedValue"].as_str().unwrap();
                let expanded = parsed.unwrap_or_else(|refusal| panic!("{name}: {refusal:?}"));
                let argv: Vec<String> = expected["argv"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|element| element.as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(expanded.argv(selected).unwrap(), argv, "{name}");
            }
            _ => assert_eq!(
                parsed.unwrap_err(),
                reason(expected["reason"].as_str().unwrap()),
                "{name}"
            ),
        }
    }
}

#[test]
fn the_namespace_comes_from_the_command_run_not_from_the_response() {
    let action = choose(json!(["digest", MEMBER, "{value}"]));
    assert!(Choose::parse("digest", &action).is_ok());
    assert_eq!(Choose::parse("other", &action), Err(Refusal::Namespace));
    assert_eq!(
        Choose::parse("digest", &choose(json!(["tmt", "digest", "{value}"]))),
        Err(Refusal::Namespace)
    );
}

#[test]
fn a_value_placeholder_mixed_into_any_other_element_is_refused() {
    for argv in [
        json!(["digest", "{value}", "x{value}"]),
        json!(["digest", "{value}{value}"]),
        json!(["{value}", "digest"]),
    ] {
        assert!(
            Choose::parse("digest", &choose(argv.clone())).is_err(),
            "{argv}"
        );
    }
}

#[test]
fn a_chosen_value_stays_one_element_and_may_not_smuggle_a_nul() {
    let action = Choose::parse("digest", &choose(json!(["digest", MEMBER, "{value}"]))).unwrap();
    assert_eq!(
        action.argv("1h30m; rm -rf /").unwrap(),
        ["digest", MEMBER, "1h30m; rm -rf /"]
    );
    assert_eq!(action.argv("a\0b"), Err(Refusal::ArgvShape));
}

#[test]
fn options_are_plain_text_and_current_names_an_offered_value() {
    let action = |options: Value, current: &str| json!({"kind":"choose","argv":["digest","{value}"],"options":options,"current":current});
    let one = json!([{"label":"Auto","value":"auto"}]);
    assert!(Choose::parse("digest", &action(one.clone(), "auto")).is_ok());
    assert_eq!(
        Choose::parse("digest", &action(one, "5m")),
        Err(Refusal::Options)
    );
    for options in [
        json!([]),
        json!("auto"),
        json!([{"label":"","value":"auto"}]),
        json!([{"label":"A\u{1b}[31m","value":"auto"}]),
        json!([{"label":"Auto","value":""}]),
        json!([{"label":"Auto","value":7}]),
    ] {
        assert_eq!(
            Choose::parse("digest", &action(options.clone(), "auto")),
            Err(Refusal::Options),
            "{options}"
        );
    }
}
