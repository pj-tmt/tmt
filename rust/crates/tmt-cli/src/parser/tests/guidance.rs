use super::*;
use crate::invocation::LearnReference;

#[test]
fn learn_selects_exact_bundled_guidance_without_breaking_the_core_flag() {
    assert_eq!(
        parsed(&["learn"]).invocation,
        Invocation::Learn {
            skill: None,
            reference: None
        }
    );
    for args in [vec!["learn", "--skill"], vec!["learn", "--skill=tmt"]] {
        assert_eq!(
            parsed(&args).invocation,
            Invocation::Learn {
                skill: Some("tmt".into()),
                reference: None
            }
        );
    }
    for name in ["tmt", "tmt-inbox"] {
        assert_eq!(
            parsed(&["learn", "--skill", name]).invocation,
            Invocation::Learn {
                skill: Some(name.into()),
                reference: None
            }
        );
    }
    assert_eq!(
        parse_error(&["learn", "--skill", "unknown"]).code,
        "USAGE_ERROR"
    );
}

#[test]
fn learn_selects_a_catalog_reference_topic_or_the_topic_list() {
    for skill in &tmt_core::skill_catalog::BUNDLED {
        let name = skill.name;
        for topic in skill.topics {
            let expected = Invocation::Learn {
                skill: Some(name.into()),
                reference: Some(LearnReference::Topic((*topic).into())),
            };
            assert_eq!(
                parsed(&["learn", "--skill", name, topic]).invocation,
                expected
            );
            assert_eq!(
                parsed(&["learn", topic, "--skill", name]).invocation,
                expected
            );
        }
        let list = ["learn", "--skill", name, "--topics"];
        if skill.topics.is_empty() {
            let error = parse_error(&list);
            assert_eq!(error.code, "USAGE_ERROR");
            assert_eq!(
                error.message,
                format!("The {name} skill has no reference topics.")
            );
            assert_eq!(
                parse_error(&["learn", "--skill", name, "topic"]).message,
                error.message
            );
        } else {
            assert_eq!(
                parsed(&list).invocation,
                Invocation::Learn {
                    skill: Some(name.into()),
                    reference: Some(LearnReference::Topics),
                }
            );
            let error = parse_error(&["learn", "--skill", name, "unknown"]);
            assert_eq!(error.code, "USAGE_ERROR");
            assert_eq!(
                error.message,
                format!(
                    "Unknown {name} reference topic 'unknown'. Topics: {}.",
                    skill.topics.join(", ")
                )
            );
        }
    }
    // A bare --skill means tmt for the list as well as the entry.
    let outcome = |values: &[&str]| {
        parse(&args(values))
            .map(|parsed| parsed.invocation)
            .map_err(|error| error.message)
    };
    assert_eq!(
        outcome(&["learn", "--topics", "--skill"]),
        outcome(&["learn", "--skill", "tmt", "--topics"])
    );
}

#[test]
fn learn_reference_needs_a_selected_skill_and_one_form() {
    for args in [
        vec!["learn", "run"],
        vec!["learn", "--topics"],
        // `run` is read as the --skill value, which no bundled skill has.
        vec!["learn", "--skill", "run"],
        vec!["learn", "--skill", "tmt", "run", "--topics"],
        vec!["learn", "--skill", "tmt", "../SKILL"],
        vec!["learn", "--skill", "tmt", "SKILL"],
        vec!["learn", "--skill", "tmt", "a", "b"],
    ] {
        assert_eq!(parse_error(&args).code, "USAGE_ERROR", "{args:?}");
    }
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
