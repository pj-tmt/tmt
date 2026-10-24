use std::ffi::OsString;

use crate::{grammar, invocation::Invocation, parser::parse};

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn core_help_flag_uses_shared_factory_wording() {
    let factory = tmt_cli_style::command(&tmt_cli_style::CommandSpec {
        name: "example",
        summary: "An example command",
        examples: &[tmt_cli_style::Example {
            command: "example --help",
            note: "Print help",
        }],
        outputs: tmt_cli_style::OutputModes::Human,
        details: "",
    });
    let expected = factory
        .get_arguments()
        .find(|arg| arg.get_id() == "help")
        .unwrap()
        .get_help()
        .unwrap()
        .to_string();
    for path in [
        vec![],
        vec!["run".into()],
        vec!["identity".into(), "meta".into(), "ls".into()],
    ] {
        let mut command = grammar::help_command(&path).unwrap();
        let actual = command
            .get_arguments()
            .find(|arg| arg.get_id() == "help")
            .unwrap()
            .get_help()
            .unwrap()
            .to_string();
        assert_eq!(actual, expected, "{path:?}");
        assert!(command.render_long_help().to_string().contains(&expected));
    }
}

#[test]
fn every_public_command_has_scoped_help_without_required_operands() {
    fn visit(command: &clap::Command, path: Vec<String>) {
        for flag in ["-h", "--help"] {
            let mut argv: Vec<OsString> = path.iter().map(OsString::from).collect();
            argv.push(flag.into());
            let parsed = parse(&argv).unwrap_or_else(|error| panic!("{argv:?}: {error:?}"));
            assert_eq!(parsed.invocation, Invocation::Help(path.clone()));
        }
        let mut argv = vec![OsString::from("help")];
        argv.extend(path.iter().map(OsString::from));
        assert_eq!(
            parse(&argv).unwrap().invocation,
            Invocation::Help(path.clone())
        );
        let rendered = grammar::help_command(&path)
            .unwrap()
            .render_long_help()
            .to_string();
        assert!(rendered.contains("Usage: tmt"), "{path:?}: {rendered}");
        assert!(rendered.contains("--help"), "{path:?}: {rendered}");
        for child in command
            .get_subcommands()
            .filter(|child| !child.is_hide_set())
        {
            let mut child_path = path.clone();
            child_path.push(child.get_name().to_owned());
            visit(child, child_path);
        }
    }
    visit(&grammar::grammar(), Vec::new());
}

#[test]
fn help_uses_public_ownership_and_resolves_aliases() {
    let mut command =
        grammar::help_command(&["identity".into(), "meta".into(), "list".into()]).unwrap();
    let text = command.render_long_help().to_string();
    assert!(text.contains("tmt identity meta ls"));
    assert!(text.contains("--identity"));
    assert!(!text.contains("--detach"));
    assert!(!text.contains("--wait"));
    assert_eq!(
        parse(&arguments(&["ls", "-h"])).unwrap().invocation,
        Invocation::Help(vec!["ls".into()])
    );
    assert!(grammar::help_command(&["__native-install".into()]).is_err());
    assert!(parse(&arguments(&["help", "identity", "not-a-command"])).is_err());
    assert!(crate::parser::parse_core(&arguments(&["not-a-command", "--help"])).is_err());
    assert_eq!(
        parse(&arguments(&["not-a-command", "--help"]))
            .unwrap()
            .invocation,
        Invocation::Extension {
            name: "not-a-command".into(),
            args: vec!["--help".into()],
            help: false,
            prefix: vec![]
        }
    );
}

#[test]
fn unknown_help_topics_point_to_the_nearest_public_parent() {
    for (path, parent) in [
        (vec!["nosuch"], "tmt"),
        (vec!["identity", "not-a-command"], "tmt identity"),
        (
            vec!["identity", "meta", "list", "nosuch"],
            "tmt identity meta ls",
        ),
        (vec!["__native-install"], "tmt"),
    ] {
        let topic = path.last().unwrap();
        let help = format!("tmt help{}", parent.strip_prefix("tmt").unwrap());
        let expected =
            format!("Unknown command '{topic}' after {parent}.\nRun {help} to see its commands.");
        let path: Vec<String> = path.into_iter().map(str::to_owned).collect();
        assert_eq!(grammar::help_command(&path).unwrap_err(), expected);
        for json in [false, true] {
            let mut argv = vec![OsString::from("help")];
            argv.extend(path.iter().map(OsString::from));
            if json {
                argv.push("--json".into());
            }
            let error = crate::parser::parse_core(&argv).unwrap_err();
            assert_eq!(error.code, "USAGE_ERROR");
            assert_eq!(error.message, expected);
            assert_eq!(error.mode.json, json);
        }
    }
}

#[test]
fn root_help_advertises_version_without_changing_other_projections() {
    let text = grammar::help_command(&[])
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(text.contains("-V, --version"), "{text}");
    for path in [vec!["run".into()], vec!["talk".into()]] {
        let text = grammar::help_command(&path)
            .unwrap()
            .render_long_help()
            .to_string();
        assert!(!text.contains("--version"), "{path:?}: {text}");
    }
    let public = grammar::public_grammar(&grammar::grammar(), true);
    assert!(!public.get_arguments().any(|arg| arg.get_id() == "version"));
    for flag in ["-V", "--version"] {
        assert_eq!(
            parse(&arguments(&[flag])).unwrap().invocation,
            Invocation::Version
        );
        let error = parse(&arguments(&["--json", flag])).unwrap_err();
        assert_eq!(error.code, "JSON_UNSUPPORTED");
        assert!(error.mode.json);
    }
}

#[test]
fn notes_help_identifies_the_shared_file_and_explicit_creation_boundary() {
    let text = grammar::help_command(&["notes".into(), "path".into()])
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(text.contains("Saved identities only"));
    assert!(text.contains("Edit the returned Markdown file directly"));
    assert!(text.contains("Reads never create missing notes"));
    assert!(text.contains("--identity"));
}

#[test]
fn marked_help_comes_from_the_public_grammar() {
    let mut command = grammar::help_command(&["marked".into()]).unwrap();
    let text = command.render_long_help().to_string();
    assert!(text.contains("tmt marked"));
    assert!(text.contains("<name>"));
    assert!(text.contains("--save"));
    assert!(text.contains("explicitly marked in tmux"));
}

#[test]
fn help_respects_data_boundaries_and_machine_mode() {
    for argv in [
        vec!["talk", "peer", "--", "--help"],
        vec!["talk", "peer", "hello", "--identity=--help"],
        vec!["name", "--", "--help"],
    ] {
        assert!(!matches!(
            parse(&arguments(&argv)).unwrap().invocation,
            Invocation::Help(_)
        ));
    }
    for argv in [
        vec!["identity", "--help", "--json"],
        vec!["help", "identity", "--json"],
    ] {
        let error = parse(&arguments(&argv)).unwrap_err();
        assert_eq!(error.code, "JSON_UNSUPPORTED");
        assert!(error.mode.json);
    }
}

#[test]
fn help_with_json_rejects_machine_output_before_required_operands() {
    for command in ["run", "resume", "talk"] {
        let explicit = parse(&arguments(&["help", command, "--json"])).unwrap_err();
        assert_eq!(explicit.code, "JSON_UNSUPPORTED");
        for flag in ["-h", "--help"] {
            assert_eq!(
                parse(&arguments(&[command, flag])).unwrap().invocation,
                Invocation::Help(vec![command.into()])
            );
            for argv in [
                vec![command, flag, "--json"],
                vec![command, "--json", flag],
                vec!["--json", command, flag],
            ] {
                let error = parse(&arguments(&argv)).unwrap_err();
                assert_eq!(error.code, explicit.code, "{argv:?}: {error:?}");
                assert_eq!(error.message, explicit.message, "{argv:?}");
                assert!(error.mode.json, "{argv:?}");
            }
        }
    }
}

#[test]
fn bare_identity_groups_report_usage_instead_of_panicking() {
    for group in ["meta", "status"] {
        let error = parse(&arguments(&["identity", group])).unwrap_err();
        assert_eq!(error.code, "USAGE_ERROR");
        assert!(error.message.contains("subcommand"), "{error:?}");
    }
}

#[test]
fn help_does_not_accept_unrelated_or_unknown_options() {
    for argv in [
        vec!["--json", "--help", "--timeout", "1s"],
        vec!["identity", "--help", "--detach"],
        vec!["talk", "--help", "--unknown"],
        vec!["talk", "--help", "--wait"],
        vec!["run", "--help", "--json", "--unknown"],
        vec!["run", "--help", "--json=false"],
        vec!["--help", "--json=false"],
    ] {
        assert_eq!(parse(&arguments(&argv)).unwrap_err().code, "USAGE_ERROR");
    }
}

#[test]
fn explicit_value_options_must_not_be_left_empty_before_help() {
    for mut argv in [
        vec!["talk", "--identity", "--help"],
        vec!["reply", "--file", "--help"],
        vec!["install", "--dir", "--help"],
        vec!["extension", "install", "--prefix", "--help"],
    ] {
        assert_eq!(parse(&arguments(&argv)).unwrap_err().code, "USAGE_ERROR");
        argv.push("--json");
        let error = parse(&arguments(&argv)).unwrap_err();
        assert_eq!(error.code, "USAGE_ERROR");
        assert!(error.mode.json);
    }
}

#[test]
fn public_help_preserves_required_subcommands_and_argument_groups() {
    let mut identity = grammar::help_command(&["identity".into()]).unwrap();
    assert!(
        identity
            .render_long_help()
            .to_string()
            .contains("<COMMAND>")
    );
    let mut inspect = grammar::help_command(&["channel".into(), "inspect".into()]).unwrap();
    assert!(
        inspect
            .get_groups()
            .any(|group| group.get_id() == "enrollment" && group.is_required_set())
    );
    let text = inspect.render_long_help().to_string();
    assert!(text.contains("<target|--binding <binding>>"), "{text}");
}

#[test]
fn talk_help_distinguishes_recipient_pull_from_live_notification() {
    let text = grammar::help_command(&["talk".into()])
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(text.contains("An identity with no binding receives through its inbox"));
    assert!(text.contains(
        "talk waits for its reply (180 seconds unless configured; --timeout overrides it)"
    ));
    assert!(text.contains("recipient must pull with tmt inbox or tmt x listen"));
    assert!(text.contains("Use --detach to return at once"));
    assert!(text.contains("For a bound recipient, talk attempts live notification"));
    assert!(!text.contains("--inbox"), "{text}");
    assert!(text.contains("--passthrough"), "{text}");
    assert!(text.contains("not a request, returns at once"), "{text}");
}

#[test]
fn focus_help_describes_directional_swap_and_preserved_focus() {
    let text = grammar::help_command(&["focus".into()])
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(text.contains("--into"));
    for direction in ["left", "right", "up", "down"] {
        assert!(text.contains(direction));
    }
    assert!(text.contains("keep focus"));
    assert!(text.contains("tmt focus worker --into left"));
    assert!(text.contains("built-in member attaches to live output"));
    assert!(text.contains("screen may stay blank until the program draws"));
}
