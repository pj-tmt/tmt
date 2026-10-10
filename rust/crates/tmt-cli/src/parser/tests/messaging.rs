use super::*;

#[test]
fn talk_focus_bypass_and_purpose_are_command_local_and_result_is_reserved() {
    let Invocation::Talk { options, .. } =
        parsed(&["talk", "peer", "hello", "--urgent", "--kind", "review"]).invocation
    else {
        panic!("talk invocation")
    };
    assert!(options.urgent);
    assert_eq!(
        options.focus_kind,
        tmt_core::request::focus::FocusKind::Review
    );
    for value in ["result", "urgent", "decision "] {
        assert_eq!(
            parse_error(&["talk", "peer", "hello", "--kind", value]).code,
            "USAGE_ERROR"
        );
    }
    assert_eq!(
        parse_error(&["check", "peer", "--urgent"]).code,
        "USAGE_ERROR"
    );
}

#[test]
fn withdrawal_requires_reason_and_keeps_originator_selection_command_local() {
    assert_eq!(
        parsed(&[
            "x",
            "withdraw",
            "req-owned",
            "--reason",
            "Already resolved",
            "--identity",
            "Owner",
            "--json"
        ])
        .invocation,
        Invocation::Exchange {
            identity: Some("Owner".into()),
            operation: ExchangeOperation::Withdraw {
                request_id: "req-owned".into(),
                reason: "Already resolved".into()
            },
        }
    );
    assert_eq!(
        parse_error(&["x", "withdraw", "req-owned"]).code,
        "USAGE_ERROR"
    );
    assert_eq!(
        parse_error(&[
            "x",
            "withdraw",
            "req-owned",
            "--reason",
            "obsolete",
            "--incoming"
        ])
        .code,
        "USAGE_ERROR"
    );
}

#[test]
fn timing_values_accept_exact_boundaries_and_reject_invalid_values() {
    let timeout = parsed(&["talk", "peer", "hello", "--timeout", "86400s"]);
    assert_eq!(
        timeout.invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "hello".into(),
            originator: None,
            options: TalkOptions {
                urgent: false,
                focus_kind: tmt_core::request::focus::FocusKind::Fyi,
                room: None,
                inbox: false,
                force: false,
                detach: false,
                delay_seconds: None,
                timeout_seconds: Some(86_400.0),
                no_preamble: false,
            },
        }
    );
    let delay = parsed(&["talk", "peer", "hello", "--delay", "2147483647ms"]);
    assert_eq!(
        delay.invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "hello".into(),
            originator: None,
            options: TalkOptions {
                urgent: false,
                focus_kind: tmt_core::request::focus::FocusKind::Fyi,
                room: None,
                inbox: false,
                force: false,
                detach: false,
                delay_seconds: Some(2_147_483.647),
                timeout_seconds: None,
                no_preamble: false,
            },
        }
    );

    for (argv, text) in [
        (
            &["talk", "peer", "hello", "--timeout", "86400.001s"][..],
            "24 hours",
        ),
        (
            &["talk", "peer", "hello", "--delay", "2147483648ms"][..],
            "timer limit",
        ),
        (
            &["talk", "peer", "hello", "--timeout", "-1"][..],
            "Invalid time format",
        ),
        (
            &["talk", "peer", "hello", "--timeout", "NaN"][..],
            "Invalid time format",
        ),
    ] {
        assert_usage_error(argv, text, OutputMode::default());
    }

    let detached = parsed(&["talk", "peer", "hello", "--detach"]);
    assert_eq!(
        detached.invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "hello".into(),
            originator: None,
            options: TalkOptions {
                urgent: false,
                focus_kind: tmt_core::request::focus::FocusKind::Fyi,
                room: None,
                inbox: false,
                force: false,
                detach: true,
                delay_seconds: None,
                timeout_seconds: None,
                no_preamble: false,
            },
        }
    );
    assert_usage_error(
        &["talk", "peer", "hello", "--detach", "--timeout", "1s"],
        "either --timeout or --detach",
        OutputMode::default(),
    );
}

#[test]
fn inbox_and_listener_options_have_typed_defaults_and_minutes() {
    let talk = parsed(&["talk", "Receiver", "hello", "--inbox", "--detach"]);
    let Invocation::Talk { options, .. } = talk.invocation else {
        panic!("talk invocation")
    };
    assert!(options.inbox);

    assert_eq!(
        parsed(&["x", "listen", "--identity", "Receiver"]).invocation,
        Invocation::Exchange {
            identity: Some("Receiver".into()),
            operation: ExchangeOperation::Listen {
                room: None,
                timeout_seconds: 900.0,
                debounce_seconds: 10.0
            },
        }
    );
    assert_eq!(
        parsed(&["x", "listen", "--timeout", "1.5m", "--debounce", "250MS"]).invocation,
        Invocation::Exchange {
            identity: None,
            operation: ExchangeOperation::Listen {
                room: None,
                timeout_seconds: 90.0,
                debounce_seconds: 0.25
            },
        }
    );
    for value in ["0", "86400.1s"] {
        assert_usage_error(
            &["x", "listen", "--timeout", value],
            "Listen timeout",
            OutputMode::default(),
        );
    }
    for value in ["NaN", "1h"] {
        assert_usage_error(
            &["x", "listen", "--timeout", value],
            "Invalid time format",
            OutputMode::default(),
        );
    }
}

#[test]
fn capture_and_exchange_integers_accept_exact_boundaries() {
    assert_eq!(
        parsed(&["check", "peer", "0"]).invocation,
        Invocation::Check {
            target: "peer".into(),
            lines: Some(0),
            capture_only: false,
        }
    );
    assert_eq!(
        parsed(&["check", "peer", "--lines", "2147483647"]).invocation,
        Invocation::Check {
            target: "peer".into(),
            lines: Some(2_147_483_647),
            capture_only: false,
        }
    );
    assert_eq!(
        parsed(&["x", "list", "--limit", "200", "--after", "9007199254740991"]).invocation,
        Invocation::Exchange {
            identity: None,
            operation: ExchangeOperation::List {
                limit: Some(200),
                after: Some(9_007_199_254_740_991),
            },
        }
    );
    assert_eq!(
        parsed(&["x", "ack", "request-1", "--revision", "9007199254740991"]).invocation,
        Invocation::Exchange {
            identity: None,
            operation: ExchangeOperation::Ack {
                request_id: "request-1".into(),
                revision: 9_007_199_254_740_991,
                incoming: false,
            },
        }
    );

    for argv in [
        &["check", "peer", "2147483648"][..],
        &["check", "peer", "--lines", "2147483648"][..],
        &["check", "peer", "--lines", "1.5"][..],
        &["check", "peer", "--lines", "-1"][..],
        &["--lines", "2147483648", "check", "peer", "0"][..],
        &["check", "peer", "0", "--lines", "1.5"][..],
        &["check", "peer", "2147483648", "--lines", "0"][..],
        &["x", "list", "--after", "9007199254740992"][..],
        &["x", "ack", "request-1", "--revision", "0"][..],
    ] {
        assert_eq!(parse_error(argv).code, "USAGE_ERROR", "arguments: {argv:?}");
    }
    assert_eq!(
        parsed(&["--lines", "12", "check", "peer", "0"]).invocation,
        Invocation::Check {
            target: "peer".into(),
            lines: Some(0),
            capture_only: false,
        }
    );
}

#[test]
fn capture_only_is_an_opt_in_flag_that_composes_with_the_line_count() {
    for argv in [
        &["check", "peer", "--capture-only"][..],
        &["read", "peer", "--capture-only"][..],
    ] {
        assert_eq!(
            parsed(argv).invocation,
            Invocation::Check {
                target: "peer".into(),
                lines: None,
                capture_only: true,
            },
            "arguments: {argv:?}"
        );
    }
    assert_eq!(
        parsed(&["check", "peer", "40", "--capture-only"]).invocation,
        Invocation::Check {
            target: "peer".into(),
            lines: Some(40),
            capture_only: true,
        }
    );
    // Only `check` owns the flag; it is not a global option.
    for argv in [
        &["--capture-only", "check", "peer"][..],
        &["talk", "peer", "hi", "--capture-only"][..],
        &["check", "peer", "--capture-only=1"][..],
    ] {
        assert_eq!(parse_error(argv).code, "USAGE_ERROR", "arguments: {argv:?}");
    }
}

#[test]
fn input_sources_are_mutually_exclusive_and_required() {
    assert_eq!(
        parsed(&["reply", "request-1", "--receipt", "receipt", "--stdin",]).invocation,
        Invocation::Reply {
            request_id: "request-1".into(),
            receipt: "receipt".into(),
            input: ContentInput::Stdin,
        }
    );
    assert_eq!(
        parsed(&["role", "set", "profile text", "--identity", "Alice",]).invocation,
        Invocation::Role {
            identity: Some("Alice".into()),
            operation: RoleOperation::Set(ContentInput::Inline("profile text".into())),
        }
    );

    for (argv, text) in [
        (
            &["reply", "request-1", "--receipt", "receipt"][..],
            "exactly one inline content",
        ),
        (
            &[
                "reply",
                "request-1",
                "--receipt",
                "receipt",
                "--message",
                "inline",
                "--file",
                "/tmp/response.txt",
            ][..],
            "exactly one inline content",
        ),
        (
            &["role", "set", "inline", "--file", "/tmp/profile.txt"][..],
            "exactly one inline content",
        ),
    ] {
        assert_usage_error(argv, text, OutputMode::default());
    }
}

#[test]
fn nested_exchange_and_role_options_stay_at_their_own_boundaries() {
    assert_eq!(
        parsed(&["x", "--identity", "Alice", "list", "--limit", "2"]).invocation,
        Invocation::Exchange {
            identity: Some("Alice".into()),
            operation: ExchangeOperation::List {
                limit: Some(2),
                after: None,
            },
        }
    );
    assert_eq!(
        parsed(&["x", "list", "--identity", "Alice", "--after", "0"]).invocation,
        Invocation::Exchange {
            identity: Some("Alice".into()),
            operation: ExchangeOperation::List {
                limit: None,
                after: Some(0),
            },
        }
    );
    assert_eq!(
        parsed(&["role", "--identity", "Alice", "show"]).invocation,
        Invocation::Role {
            identity: Some("Alice".into()),
            operation: RoleOperation::Show,
        }
    );
    assert_eq!(
        parsed(&["role", "show", "--identity", "Alice"]).invocation,
        Invocation::Role {
            identity: Some("Alice".into()),
            operation: RoleOperation::Show,
        }
    );

    for (argv, text) in [
        (
            &["x", "list", "--force"][..],
            "Unknown option or argument 'force'",
        ),
        (
            &["role", "show", "--limit", "2"][..],
            "unexpected argument '--limit'",
        ),
        (
            &["role", "show", "--file", "/tmp/profile.txt"][..],
            "unexpected argument '--file'",
        ),
    ] {
        assert_usage_error(argv, text, OutputMode::default());
    }
}

#[test]
fn inbox_and_answer_select_an_identity_and_one_body_source() {
    assert_eq!(
        parsed(&[
            "inbox",
            "--from",
            "alice",
            "--limit",
            "5",
            "--identity",
            "ben"
        ])
        .invocation,
        Invocation::Inbox {
            identity: Some("ben".into()),
            from: Some("alice".into()),
            limit: Some(5),
        }
    );
    assert_eq!(
        parsed(&["answer", "alice", "yes", "--request", "req_1"]).invocation,
        Invocation::Answer {
            identity: None,
            from: Some("alice".into()),
            request: Some("req_1".into()),
            input: ContentInput::Inline("yes".into()),
        }
    );
    assert_eq!(
        parsed(&["answer", "alice", "--stdin"]).invocation,
        Invocation::Answer {
            identity: None,
            from: Some("alice".into()),
            request: None,
            input: ContentInput::Stdin,
        }
    );
    // With --request the sender is optional: one operand is the text unless
    // --file or --stdin supplies the body.
    for (argv, from, input) in [
        (
            &["answer", "--request", "req_1", "done"][..],
            None,
            ContentInput::Inline("done".into()),
        ),
        (
            &["answer", "--request", "req_1", "alice", "--stdin"][..],
            Some("alice"),
            ContentInput::Stdin,
        ),
        (
            &["answer", "--request", "req_1", "--file", "/tmp/a.md"][..],
            None,
            ContentInput::File("/tmp/a.md".into()),
        ),
    ] {
        assert_eq!(
            parsed(argv).invocation,
            Invocation::Answer {
                identity: None,
                from: from.map(str::to_owned),
                request: Some("req_1".into()),
                input,
            },
            "arguments: {argv:?}"
        );
    }
    assert_usage_error(&["answer"], "choose --request", OutputMode::default());
    for argv in [
        &["answer", "alice"][..],
        &["answer", "alice", "yes", "--file", "/tmp/a.md"][..],
    ] {
        assert_usage_error(argv, "exactly one inline content", OutputMode::default());
    }
    for argv in [
        &["inbox", "--limit", "0"][..],
        &["inbox", "--limit", "201"][..],
    ] {
        assert_eq!(parse_error(argv).code, "USAGE_ERROR", "arguments: {argv:?}");
    }
}
