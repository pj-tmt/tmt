use super::*;

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
        }
    );
    assert_eq!(
        parsed(&["run", "--resume", "Alice"]).invocation,
        Invocation::Run {
            name: "Alice".into(),
            command: vec![],
            resume: true,
            save: false,
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
                save: true
            }
        );
        assert_eq!(
            parsed(&["run", option, "--resume", "Alice"]).invocation,
            Invocation::Run {
                name: "Alice".into(),
                command: vec![],
                resume: true,
                save: true
            }
        );
    }
}
