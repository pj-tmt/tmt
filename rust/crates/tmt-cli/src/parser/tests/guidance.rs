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

#[test]
fn workspace_restore_defaults_to_revival_and_has_no_provider_flags() {
    assert_eq!(
        parsed(&[
            "workspace",
            "restore",
            "--layout-only",
            "--socket",
            "/tmp/exact"
        ])
        .invocation,
        Invocation::WorkspaceRestore {
            socket: Some("/tmp/exact".into()),
            layout_only: true
        }
    );
    assert_eq!(
        parsed(&["workspace", "restore", "--layout-only"]).invocation,
        Invocation::WorkspaceRestore {
            socket: None,
            layout_only: true
        }
    );
    assert_eq!(
        parsed(&["workspace", "restore"]).invocation,
        Invocation::WorkspaceRestore {
            socket: None,
            layout_only: false
        }
    );
    for args in [
        vec!["workspace", "show", "--layout-only"],
        vec!["workspace", "restore", "--layout-only", "--retry"],
        vec!["workspace", "restore", "--layout-only", "--force"],
        vec!["workspace", "restore", "--layout-only", "--channel"],
    ] {
        assert_eq!(parse_error(&args).code, "USAGE_ERROR");
    }
}
