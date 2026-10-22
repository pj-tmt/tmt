use super::*;

#[test]
fn hidden_subcommand_suggestions_are_filtered_without_hiding_public_candidates() {
    for json in [false, true] {
        let mut argv = args(&["extension install"]);
        if json {
            argv.insert(0, "--json".into());
        }
        let error = super::super::parse_core(&argv).unwrap_err();
        assert_eq!(error.mode.json, json);
        assert!(error.message.contains("extension install"));
        assert!(error.message.contains("'extension'"));
        assert!(!error.message.contains("__native-install"));
        assert_eq!(error.hint, None);

        let error = super::super::parse_core(&args(if json {
            &["--json", "__native-instal"]
        } else {
            &["__native-instal"]
        }))
        .unwrap_err();
        assert_eq!(error.mode.json, json);
        assert!(error.message.contains("'__native-instal'"));
        for hidden in [
            "__native-install",
            "__native-upgrade-extensions",
            "__native-refresh-skills",
            "__native-schema",
        ] {
            assert!(!error.message.contains(&format!("'{hidden}'")));
        }
        assert_eq!(
            error.hint.as_deref(),
            Some("Run tmt help to see the commands and options.")
        );
        let public = parse_error(if json {
            &["extension", "--json", "uninstal"]
        } else {
            &["extension", "uninstal"]
        });
        assert!(public.message.contains("'install'"));
        assert!(public.message.contains("'uninstall'"));
        assert_eq!(public.hint, None);
    }
}

#[test]
fn command_aliases_preserve_typed_invocations() {
    let cases = [
        (
            "ls alias",
            vec!["ls"],
            Invocation::List {
                target: None,
                room: None,
                scope: Default::default(),
            },
        ),
        (
            "this alias",
            vec!["this", "Alice"],
            Invocation::Bind {
                pane: None,
                name: "Alice".into(),
                save: false,
            },
        ),
        (
            "remove alias",
            vec!["remove", "Alice"],
            Invocation::Remove {
                name: "Alice".into(),
                force: false,
            },
        ),
        (
            "send alias",
            vec!["send", "peer", "hello"],
            Invocation::Talk {
                target: "peer".into(),
                message: "hello".into(),
                originator: None,
                options: TalkOptions {
                    urgent: false,
                    passthrough: false,
                    digest_kind: tmt_core::request::digest::DigestKind::Fyi,
                    room: None,
                    force: false,
                    detach: false,
                    delay_seconds: None,
                    timeout_seconds: None,
                    no_preamble: false,
                },
            },
        ),
        (
            "read alias",
            vec!["read", "peer", "0"],
            Invocation::Check {
                target: "peer".into(),
                lines: Some(0),
                capture_only: false,
            },
        ),
    ];

    for (name, argv, invocation) in cases {
        let actual = parsed(&argv);
        assert_eq!(actual.invocation, invocation, "{name}");
        assert_eq!(actual.mode, OutputMode::default(), "{name} mode");
    }
}

#[test]
fn save_and_force_flags_have_false_defaults_and_short_aliases() {
    assert_eq!(
        parsed(&["add", "%1", "Alice"]).invocation,
        Invocation::Bind {
            pane: Some("%1".into()),
            name: "Alice".into(),
            save: false,
        }
    );
    assert_eq!(
        parsed(&["add", "%1", "Alice", "-s"]).invocation,
        Invocation::Bind {
            pane: Some("%1".into()),
            name: "Alice".into(),
            save: true,
        }
    );
    assert_eq!(
        parsed(&["name", "Alice", "--save"]).invocation,
        Invocation::Bind {
            pane: None,
            name: "Alice".into(),
            save: true,
        }
    );
    assert_eq!(
        parsed(&["marked", "Alice"]).invocation,
        Invocation::BindMarked {
            name: "Alice".into(),
            save: false,
        }
    );
    assert_eq!(
        parsed(&["marked", "Alice", "-s"]).invocation,
        Invocation::BindMarked {
            name: "Alice".into(),
            save: true,
        }
    );
    assert_eq!(
        parsed(&["rm", "Alice"]).invocation,
        Invocation::Remove {
            name: "Alice".into(),
            force: false,
        }
    );
    assert_eq!(
        parsed(&["remove", "Alice", "-f"]).invocation,
        Invocation::Remove {
            name: "Alice".into(),
            force: true,
        }
    );
}

#[test]
fn literal_option_words_remain_data_when_the_grammar_requires_values() {
    assert_eq!(
        parsed(&["talk", "peer", "--", "--json --debug"]).invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "--json --debug".into(),
            originator: None,
            options: TalkOptions {
                urgent: false,
                passthrough: false,
                digest_kind: tmt_core::request::digest::DigestKind::Fyi,
                room: None,
                force: false,
                detach: false,
                delay_seconds: None,
                timeout_seconds: None,
                no_preamble: false,
            },
        }
    );
    assert_eq!(
        parsed(&[
            "reply",
            "request-1",
            "--receipt",
            "receipt",
            "--message=--json"
        ])
        .invocation,
        Invocation::Reply {
            request_id: "request-1".into(),
            receipt: "receipt".into(),
            input: ContentInput::Inline("--json".into()),
        }
    );
}

#[test]
fn root_and_command_local_options_work_before_and_after_the_command() {
    assert_eq!(
        parsed(&["--json", "talk", "peer", "hello", "--detach"]).mode,
        OutputMode { json: true }
    );
    assert_eq!(
        parsed(&["talk", "peer", "hello", "--json", "--no-preamble"]),
        Parsed {
            legacy_hook: false,
            invocation: Invocation::Talk {
                target: "peer".into(),
                message: "hello".into(),
                originator: None,
                options: TalkOptions {
                    urgent: false,
                    passthrough: false,
                    digest_kind: tmt_core::request::digest::DigestKind::Fyi,
                    room: None,
                    force: false,
                    detach: false,
                    delay_seconds: None,
                    timeout_seconds: None,
                    no_preamble: true,
                },
            },
            mode: OutputMode { json: true },
        }
    );
    assert_eq!(
        parsed(&["--timeout", "2", "send", "peer", "hello"]).invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "hello".into(),
            originator: None,
            options: TalkOptions {
                urgent: false,
                passthrough: false,
                digest_kind: tmt_core::request::digest::DigestKind::Fyi,
                room: None,
                force: false,
                detach: false,
                delay_seconds: None,
                timeout_seconds: Some(2.0),
                no_preamble: false,
            },
        }
    );
}

#[test]
fn removed_output_flags_are_rejected_but_remain_literal_payload_data() {
    for option in ["--verbose", "-v", "--debug"] {
        for argv in [
            vec![option, "list", "--json"],
            vec!["--json", "list", option],
            vec!["identity", "create", "Agent", option, "--json"],
            vec!["--json", "--version", option],
        ] {
            assert_usage_error(&argv, option, OutputMode { json: true });
        }
    }
    assert_eq!(
        parsed(&["talk", "peer", "--", "--verbose --debug -v"]).invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "--verbose --debug -v".into(),
            originator: None,
            options: TalkOptions {
                urgent: false,
                passthrough: false,
                digest_kind: tmt_core::request::digest::DigestKind::Fyi,
                room: None,
                force: false,
                detach: false,
                delay_seconds: None,
                timeout_seconds: None,
                no_preamble: false,
            },
        }
    );
}

#[test]
fn command_local_options_are_rejected_at_the_wrong_boundary() {
    for (argv, text) in [
        (
            &["list", "--force"][..],
            "Unknown option or argument 'force'",
        ),
        (
            &["role", "show", "--force"][..],
            "Unknown option or argument 'force'",
        ),
        (
            &["list", "--delay", "1"][..],
            "Unknown option or argument 'delay'",
        ),
        (
            &["role", "show", "--no-preamble"][..],
            "Unknown option or argument 'no-preamble'",
        ),
        (
            &["read", "peer", "--timeout", "2"][..],
            "Unknown option or argument 'timeout'",
        ),
    ] {
        assert_usage_error(argv, text, OutputMode::default());
    }
}

#[test]
fn missing_arguments_keep_json_mode_before_or_after_the_command() {
    for argv in [
        &["--json", "name"][..],
        &["name", "--json"][..],
        &["--json", "talk", "peer"][..],
        &["talk", "peer", "--json"][..],
    ] {
        assert_usage_error(argv, "required", OutputMode { json: true });
    }
}

#[test]
fn local_missing_values_do_not_swallow_root_presentation_flags() {
    for argv in [
        &["talk", "peer", "--identity", "--json", "hello"][..],
        &["talk", "peer", "--identity", "--json", "hello", "--nope"][..],
        &[
            "no-command",
            "talk",
            "peer",
            "--identity",
            "--json",
            "--nope",
        ][..],
    ] {
        let error = if argv[0] == "no-command" {
            // A missing external executable falls back to this core diagnostic;
            // an installed extension owns its entire tail instead.
            crate::parser::parse_core(&args(argv)).unwrap_err()
        } else {
            parse_error(argv)
        };
        assert_eq!(error.code, "USAGE_ERROR", "{argv:?}");
        assert!(error.mode.json, "{argv:?}: {error:?}");
    }
    let result = parsed(&["talk", "peer", "--identity=--json", "hello"]);
    assert!(!result.mode.json);
    let Invocation::Talk {
        originator,
        message,
        ..
    } = result.invocation
    else {
        panic!("expected talk request");
    };
    assert_eq!(originator.as_deref(), Some("--json"));
    assert_eq!(message, "hello");
    let error = parse_error(&["learn", "--config", "--json"]);
    assert!(!error.mode.json);
    assert!(error.message.contains("config"));
}

#[test]
fn retired_wait_and_team_paths_have_distinct_errors() {
    let wait = parse_error(&["talk", "peer", "hello", "--wait"]);
    assert_eq!(wait.code, "USAGE_ERROR");
    assert_eq!(wait.mode, OutputMode::default());
    assert!(wait.message.contains("--wait option is retired"));

    let team = parse_error(&["team", "legacy"]);
    assert_eq!(team.code, "UNSUPPORTED_TEAM");
    assert_eq!(team.mode, OutputMode::default());
    assert_eq!(team.message, "Team workflows are not supported.");

    let scoped = parse_error(&["--json", "--team", "legacy", "list"]);
    assert_eq!(scoped.code, "UNSUPPORTED_TEAM");
    assert_eq!(scoped.mode, OutputMode { json: true });
}

#[test]
fn rejected_placement_option_reports_public_usage() {
    let rejected_option = parse_error(&["role", "show", "--timeout", "1s"]);
    assert_eq!(rejected_option.code, "USAGE_ERROR");
    assert!(rejected_option.message.contains("Usage: tmt role show"));
    assert!(rejected_option.message.contains("tmt help role show"));
}

#[test]
fn unknown_argument_hints_select_only_the_canonical_public_command() {
    for (argv, topic) in [
        (vec!["--bogus-flag"], ""),
        (vec!["ls", "--bogus-flag"], " ls"),
        (
            vec!["identity", "status", "show", "--bogus-flag"],
            " identity status show",
        ),
        (vec!["read", "peer", "--bogus-flag"], " check"),
        (
            vec!["learn", "--config", "identity", "--bogus-flag"],
            " learn",
        ),
        (vec!["ls", "--", "identity", "status"], " ls"),
        (
            vec!["run", "--bogus-flag", "worker", "identity", "status"],
            " run",
        ),
        (vec!["__hook", "--provider", "claude", "--bogus-flag"], ""),
    ] {
        let error = parse_error(&argv);
        assert_eq!(error.code, "USAGE_ERROR", "{argv:?}");
        let expected = if topic.is_empty() {
            "Run tmt help to see the commands and options.".to_owned()
        } else {
            format!("Run tmt help{topic} to see its options.")
        };
        assert_eq!(error.hint.as_deref(), Some(expected.as_str()), "{argv:?}");
        assert!(!error.message.contains("Run tmt help"), "{argv:?}");
    }
}

#[test]
fn unknown_argument_json_keeps_the_original_clap_message() {
    for argv in [
        vec!["--json", "identity", "status", "show", "--bogus-flag"],
        vec!["identity", "status", "show", "--bogus-flag", "--json"],
    ] {
        let error = parse_error(&argv);
        let original = crate::grammar::grammar()
            .try_get_matches_from(std::iter::once(OsString::from("tmt")).chain(args(&argv)))
            .unwrap_err();
        assert_eq!(error.message, original.to_string());
        assert!(error.mode.json);
        assert_eq!(
            error.hint.as_deref(),
            Some("Run tmt help identity status show to see its options.")
        );
    }
    for argv in [vec!["name"], vec!["check", "peer", "--lines", "bad"]] {
        assert_eq!(parse_error(&argv).hint, None, "{argv:?}");
    }
    let Invocation::Run { command, .. } =
        parsed(&["run", "worker", "identity", "--bogus-flag"]).invocation
    else {
        panic!("expected opaque launch arguments");
    };
    assert_eq!(command, args(&["identity", "--bogus-flag"]));
}
