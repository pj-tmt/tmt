use super::*;
use crate::invocation::ChannelMode;

#[test]
fn run_identity_starts_an_opaque_command_tail() {
    use std::os::unix::ffi::OsStringExt;
    let command = vec![
        OsString::from("claude"),
        "--help".into(),
        "--json".into(),
        "--resume".into(),
        "--config".into(),
        "--".into(),
        "".into(),
        OsString::from_vec(vec![0xff, b'a']),
    ];
    let mut argv = args(&["run", "Alice"]);
    argv.extend(command.clone());
    assert_eq!(
        parse(&argv).unwrap(),
        Parsed {
            legacy_hook: false,
            invocation: Invocation::Run {
                name: "Alice".into(),
                command,
                resume: false,
                save: false,
                channel: ChannelMode::Default,
            },
            mode: OutputMode::default(),
        }
    );
    for misplaced in ["-s", "--save", "--resume", "--help", "-", "--"] {
        let error = parse(&args(&["run", "Alice", misplaced])).unwrap_err();
        assert_eq!(error.code, "USAGE_ERROR");
        assert!(error.message.contains("TMT options go before the name"));
    }
    assert_eq!(
        parsed(&["run", "opus", "claude", "--resume", "x", "--help"]).invocation,
        Invocation::Run {
            name: "opus".into(),
            command: ["claude", "--resume", "x", "--help"]
                .map(OsString::from)
                .to_vec(),
            resume: false,
            save: false,
            channel: ChannelMode::Default,
        }
    );
    assert_eq!(
        parsed(&["run", "--resume", "Alice"]).invocation,
        Invocation::Run {
            name: "Alice".into(),
            command: vec![],
            resume: true,
            save: false,
            channel: ChannelMode::Default,
        }
    );
    assert_eq!(
        parsed(&["run", "--help"]).invocation,
        Invocation::Help(vec!["run".into()])
    );
    assert!(parse(&args(&["run", "--resume", "Alice", "claude"])).is_err());
    assert!(parse(&args(&["run", "--json", "Alice", "claude"])).is_err());
}

#[test]
fn run_save_belongs_only_before_the_identity() {
    for option in ["-s", "--save"] {
        assert_eq!(
            parsed(&["run", option, "Alice", "claude", "-s"]).invocation,
            Invocation::Run {
                name: "Alice".into(),
                command: vec!["claude".into(), "-s".into()],
                resume: false,
                save: true,
                channel: ChannelMode::Default,
            }
        );
        assert_eq!(
            parsed(&["run", option, "--resume", "Alice"]).invocation,
            Invocation::Run {
                name: "Alice".into(),
                command: vec![],
                resume: true,
                save: true,
                channel: ChannelMode::Default,
            }
        );
    }
}

#[test]
fn run_channel_is_an_option_before_the_identity_and_combines_with_exact_resume() {
    assert_eq!(
        parsed(&["run", "--channel", "Alice", "claude", "--channel"]).invocation,
        Invocation::Run {
            name: "Alice".into(),
            command: vec!["claude".into(), "--channel".into()],
            resume: false,
            save: false,
            channel: ChannelMode::Required,
        },
        "an option after the identity belongs to the command"
    );
    assert!(parse(&args(&["run", "--channel", "--resume", "Alice"])).is_ok());
    let error = parse(&args(&["run", "Alice", "--channel"])).unwrap_err();
    assert!(error.message.contains("TMT options go before the name"));
}

#[test]
fn the_hidden_channel_server_takes_four_operands_and_is_not_public() {
    assert_eq!(
        parsed(&["__channel-server", "claude", "b", "g", "/abs/channels"]).invocation,
        Invocation::ChannelServer {
            harness: "claude".into(),
            binding_id: "b".into(),
            generation: "g".into(),
            directory: "/abs/channels".into(),
        }
    );
    assert!(parse(&args(&["__channel-server", "claude"])).is_err());
}

#[test]
fn a_known_agent_tail_preserves_provider_flags_without_a_name() {
    assert_eq!(
        parsed(&["run", "--save", "claude", "--model", "opus", "--help"]).invocation,
        Invocation::Run {
            name: "claude".into(),
            command: ["--model", "opus", "--help"].map(OsString::from).to_vec(),
            resume: false,
            save: true,
            channel: ChannelMode::Default
        }
    );
}

#[test]
fn both_foreground_commands_choose_one_channel_mode_and_reject_conflicts() {
    for (flags, mode) in [
        (vec![], ChannelMode::Default),
        (vec!["--channel"], ChannelMode::Required),
        (vec!["--no-channel"], ChannelMode::Disabled),
    ] {
        for command in ["run", "resume"] {
            let mut words = vec![command];
            words.extend(flags.iter().copied());
            if command == "run" {
                words.push("--resume");
            }
            words.push("Alice");
            match parsed(&words).invocation {
                Invocation::Run {
                    channel,
                    resume: true,
                    ..
                }
                | Invocation::Resume { channel, .. } => assert_eq!(channel, mode),
                other => panic!("unexpected launch: {other:?}"),
            }
        }
    }
    for command in ["run", "resume"] {
        let error = parse(&args(&[command, "--channel", "--no-channel", "Alice"])).unwrap_err();
        assert_eq!(error.code, "USAGE_ERROR");
        assert!(error.message.contains("cannot be used with"));
    }
    for flag in ["--channel", "--no-channel"] {
        assert!(parse(&args(&["resume", "--forget", flag, "Alice"])).is_err());
    }
}

/// The resume list action prints `tmt resume -- <name>` for a name that starts with a dash,
/// so the grammar must read everything after `--` as the one name operand.
#[test]
fn resume_reads_the_operand_after_an_option_terminator_as_the_name() {
    for name in ["-dash", "--forget", "-a b", "plain"] {
        match parsed(&["resume", "--", name]).invocation {
            Invocation::Resume {
                name: got,
                forget,
                retry,
                show,
                ..
            } => {
                assert_eq!(got.as_deref(), Some(name));
                assert!(!forget && !retry && !show, "{name} must stay a name");
            }
            other => panic!("unexpected resume: {other:?}"),
        }
    }
}

#[test]
fn resume_uses_an_optional_name_and_bounded_explicit_overrides() {
    match parsed(&["resume", "--model", "sonnet", "--effort", "high"]).invocation {
        Invocation::Resume {
            name,
            model,
            effort,
            show,
            forget_launch,
            ..
        } => {
            assert_eq!(name, None);
            assert_eq!(model.as_deref(), Some("sonnet"));
            assert_eq!(effort.as_deref(), Some("high"));
            assert!(!show && !forget_launch);
        }
        other => panic!("unexpected resume: {other:?}"),
    }
    for flag in ["--show", "--forget-launch"] {
        assert!(parse(&args(&["resume", flag, "Alice"])).is_ok());
        for conflict in ["--forget", "--retry", "--channel", "--no-channel"] {
            assert!(parse(&args(&["resume", flag, conflict, "Alice"])).is_err());
        }
        assert!(parse(&args(&["resume", flag, "--model", "sonnet", "Alice"])).is_err());
    }
    assert!(parse(&args(&["resume", "--json"])).is_err());
    assert!(parse(&args(&["resume", "Alice", "command"])).is_err());
}

/// Moving names the identity explicitly and keeps the existing resume launch
/// options; inspection and forgetting never acquire stop authority.
#[test]
fn resume_move_requires_an_explicit_identity_and_conflicts_with_inspection() {
    for words in [
        vec!["resume", "worker", "--move"],
        vec!["resume", "--move", "worker"],
        vec!["resume", "--move", "--retry", "--no-channel", "worker"],
        vec!["resume", "--move", "--model", "model-a", "worker"],
    ] {
        match parse(&args(&words)).unwrap().invocation {
            Invocation::Resume {
                name, move_running, ..
            } => {
                assert_eq!(name.as_deref(), Some("worker"));
                assert!(move_running);
            }
            other => panic!("move must stay on the resume owner: {other:?}"),
        }
    }
    for words in [
        vec!["resume", "--move"],
        vec!["resume", "--move", "--show", "worker"],
        vec!["resume", "--move", "--forget", "worker"],
        vec!["resume", "--move", "--forget-launch", "worker"],
        vec!["run", "--move", "--resume", "worker"],
    ] {
        assert_eq!(
            parse(&args(&words)).unwrap_err().code,
            "USAGE_ERROR",
            "{words:?}"
        );
    }
}

#[test]
fn resume_driver_implies_the_stop_move_path_and_rejects_inspection() {
    for movement in [false, true] {
        let mut input = vec![
            "resume", "worker", "--driver", "claude", "--model", "sonnet",
        ];
        if movement {
            input.push("--move");
        }
        match parsed(&input).invocation {
            Invocation::Resume {
                driver,
                move_running,
                model,
                ..
            } => {
                assert_eq!(driver.as_deref(), Some("claude"));
                assert!(move_running);
                assert_eq!(model.as_deref(), Some("sonnet"));
            }
            other => panic!("unexpected driver switch: {other:?}"),
        }
    }
    for flag in ["--show", "--forget", "--forget-launch"] {
        assert!(parse(&args(&["resume", "worker", "--driver", "codex", flag])).is_err());
    }
    assert!(parse(&args(&["resume", "--driver", "claude"])).is_err());
}
