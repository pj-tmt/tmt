use super::*;

#[test]
fn learn_selects_exact_bundled_guidance_without_breaking_the_core_flag() {
    assert_eq!(
        parsed(&["learn"]).invocation,
        Invocation::Learn { skill: None }
    );
    assert_eq!(
        parsed(&["learn", "--skill"]).invocation,
        Invocation::Learn {
            skill: Some("tmt".into())
        }
    );
    for name in [
        "tmt",
        "tmt-inbox",
        "tmt-office",
        "tmt-prop-create",
        "tmt-avatar-create",
    ] {
        assert_eq!(
            parsed(&["learn", "--skill", name]).invocation,
            Invocation::Learn {
                skill: Some(name.into())
            }
        );
    }
    assert_eq!(
        parse_error(&["learn", "--skill", "unknown"]).code,
        "USAGE_ERROR"
    );
}
