use super::*;

#[test]
fn negative_config_values_reach_setting_validation_without_accepting_unknown_flags() {
    let invocation = parsed(&["config", "set", "preambleEvery", "-1", "--json"]);
    assert_eq!(
        invocation.invocation,
        Invocation::Config(crate::invocation::ConfigRequest::Set {
            key: "preambleEvery".into(),
            value: "-1".into(),
            global: false,
        })
    );
    assert!(invocation.mode.json);
    assert_eq!(
        parse_error(&["config", "set", "preambleEvery", "--unknown", "--json"]).code,
        "USAGE_ERROR"
    );
}
