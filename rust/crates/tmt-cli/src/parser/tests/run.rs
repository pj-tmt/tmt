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
