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
    for name in ["tmt", "tmt-inbox"] {
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
            name: None,
            expect: None,
            socket: Some("/tmp/exact".into()),
            layout_only: true
        }
    );
    assert_eq!(
        parsed(&["workspace", "restore", "--layout-only"]).invocation,
        Invocation::WorkspaceRestore {
            name: None,
            expect: None,
            socket: None,
            layout_only: true
        }
    );
    assert_eq!(
        parsed(&["workspace", "restore"]).invocation,
        Invocation::WorkspaceRestore {
            name: None,
            expect: None,
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

#[test]
fn workspace_snapshot_selection_and_digest_are_parsed_before_effects() {
    let name = "work-1".parse().unwrap();
    assert_eq!(
        parsed(&["workspace", "save", "work-1"]).invocation,
        Invocation::WorkspaceSave { socket: None, name }
    );
    assert_eq!(
        parsed(&["workspace", "ls"]).invocation,
        Invocation::WorkspaceList { socket: None }
    );
    assert_eq!(
        parsed(&["workspace", "list"]).invocation,
        parsed(&["workspace", "ls"]).invocation
    );
    assert_eq!(
        parsed(&["workspace", "show", "work-1"]).invocation,
        Invocation::WorkspaceShow {
            socket: None,
            name: Some("work-1".parse().unwrap())
        }
    );
    let digest = "a".repeat(64);
    assert_eq!(
        parsed(&["workspace", "restore", "work-1", "--expect", &digest]).invocation,
        Invocation::WorkspaceRestore {
            socket: None,
            name: Some("work-1".parse().unwrap()),
            expect: Some(digest),
            layout_only: false
        }
    );
    for name in [
        "",
        "latest",
        "../escape",
        "/absolute",
        "Uppercase",
        "雪",
        "bad.name",
        "-bad",
        "bad\nname",
    ] {
        for command in ["save", "rm", "show", "restore"] {
            assert_eq!(
                parse_error(&["workspace", command, name, "--json"]).code,
                "USAGE_ERROR"
            );
        }
    }
    let oversized = "a".repeat(65);
    assert_eq!(
        parse_error(&["workspace", "save", &oversized]).code,
        "USAGE_ERROR"
    );
    for digest in ["abc".to_owned(), "A".repeat(64), "g".repeat(64)] {
        assert_eq!(
            parse_error(&["workspace", "restore", "--expect", &digest]).code,
            "USAGE_ERROR"
        );
    }
    for args in [
        vec!["workspace", "save"],
        vec!["workspace", "rm", "latest"],
        vec!["workspace", "show", "--expect", &"a".repeat(64)],
    ] {
        assert_eq!(parse_error(&args).code, "USAGE_ERROR");
    }
}
