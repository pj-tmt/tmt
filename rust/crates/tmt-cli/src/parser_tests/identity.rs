use super::*;

#[test]
fn whoami_context_is_a_read_mode_not_a_new_command_or_global_option() {
    assert_eq!(parsed(&["whoami"]).invocation, Invocation::Whoami);
    assert_eq!(
        parsed(&["whoami", "--context"]).invocation,
        Invocation::WhoamiContext
    );
    let context = parsed(&["whoami", "--context", "--json"]);
    assert_eq!(context.invocation, Invocation::WhoamiContext);
    assert!(context.mode.json);
    assert!(!crate::skill_reminder::eligible_for_drift(&parsed(&[
        "whoami",
        "--context"
    ])));
    assert!(parse(&args(&["list", "--context"])).is_err());
}

#[test]
fn identity_status_uses_shared_duration_grammar_and_scoped_options() {
    for (duration, ttl_ms) in [
        ("1s", 1000),
        ("1500ms", 1500),
        ("1.5m", 90000),
        ("1440m", 86400000),
    ] {
        assert_eq!(
            parsed(&[
                "identity",
                "status",
                "set",
                "Reviewing",
                "--mood",
                "focused",
                "--for",
                duration,
                "--identity",
                "Alice"
            ])
            .invocation,
            Invocation::Identity(IdentityRequest::Status {
                identity: Some("Alice".into()),
                operation: IdentityStatusRequest::Set {
                    activity: "Reviewing".into(),
                    mood: Some("focused".into()),
                    ttl_ms
                }
            })
        );
    }
    assert_eq!(
        parsed(&["identity", "status", "set", "Reviewing"]).invocation,
        Invocation::Identity(IdentityRequest::Status {
            identity: None,
            operation: IdentityStatusRequest::Set {
                activity: "Reviewing".into(),
                mood: None,
                ttl_ms: 3600000
            }
        })
    );
    for duration in ["0", "999ms", "1441m", "NaN", "1h"] {
        assert_eq!(
            parse_error(&["identity", "status", "set", "Reviewing", "--for", duration]).code,
            "USAGE_ERROR"
        );
    }
    for verb in ["show", "clear"] {
        assert_eq!(
            parse_error(&["identity", "status", verb, "--mood", "happy"]).code,
            "USAGE_ERROR"
        );
    }
}

#[test]
fn identity_show_selects_a_name_or_the_verified_caller() {
    assert_eq!(
        parsed(&["identity", "show"]).invocation,
        Invocation::Identity(IdentityRequest::Show(None))
    );
    assert_eq!(
        parsed(&["identity", "show", "Alice"]).invocation,
        Invocation::Identity(IdentityRequest::Show(Some("Alice".into())))
    );
}

#[test]
fn identity_metadata_and_repeated_filters_have_typed_requests() {
    assert_eq!(
        parsed(&[
            "identity",
            "meta",
            "set",
            "--identity",
            "alice",
            "project",
            "tmt",
        ])
        .invocation,
        Invocation::Identity(IdentityRequest::Metadata {
            identity: Some("alice".into()),
            operation: IdentityMetadataRequest::Set {
                key: "project".into(),
                value: "tmt".into(),
            },
        })
    );
    assert_eq!(
        parsed(&[
            "identity",
            "list",
            "--where",
            "project=tmt=alpha",
            "--has",
            "capability.review",
            "--where",
            "department=engineering",
        ])
        .invocation,
        Invocation::Identity(IdentityRequest::List(vec![
            IdentityFilterRequest::Equals {
                key: "project".into(),
                value: "tmt=alpha".into(),
            },
            IdentityFilterRequest::Equals {
                key: "department".into(),
                value: "engineering".into(),
            },
            IdentityFilterRequest::Has("capability.review".into()),
        ]))
    );
    assert_usage_error(
        &["identity", "list", "--where", "project"],
        "KEY=VALUE",
        OutputMode::default(),
    );
    let Invocation::Identity(IdentityRequest::Metadata {
        operation: IdentityMetadataRequest::Set { value, .. },
        ..
    }) = parsed(&[
        "identity",
        "meta",
        "set",
        "--identity",
        "alice",
        "key",
        "--",
        "--literal-value",
    ])
    .invocation
    else {
        panic!("expected metadata set request")
    };
    assert_eq!(value, "--literal-value");
}

#[test]
fn focus_takes_one_identity_or_pane_target() {
    for target in ["auth-fix", "%2"] {
        assert_eq!(
            parsed(&["focus", target, "--json"]).invocation,
            Invocation::Focus {
                target: target.into()
            }
        );
    }
    assert_eq!(parse_error(&["focus"]).code, "USAGE_ERROR");
    assert_eq!(
        parsed(&["focus", "--client", "--json"]).invocation,
        Invocation::FocusClient
    );
    assert_eq!(
        parse_error(&["focus", "auth-fix", "--client"]).code,
        "USAGE_ERROR"
    );
}

#[test]
fn rename_takes_two_names_at_the_top_level_and_under_identity() {
    let expected = Invocation::Rename {
        old: "opus-tmt-peer-2".into(),
        new: "tmt-peer-2".into(),
    };
    assert_eq!(
        parsed(&["rename", "opus-tmt-peer-2", "tmt-peer-2"]).invocation,
        expected
    );
    assert_eq!(
        parsed(&[
            "identity",
            "rename",
            "opus-tmt-peer-2",
            "tmt-peer-2",
            "--json"
        ])
        .invocation,
        expected
    );
    parse_error(&["rename", "only-one"]);
    parse_error(&["identity", "rename"]);
}
