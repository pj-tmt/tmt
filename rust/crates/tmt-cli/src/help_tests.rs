use std::ffi::OsString;

use crate::{grammar, invocation::Invocation, parser::parse};

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
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
        grammar::help_command(&["office".into(), "board".into(), "list".into()]).unwrap();
    let text = command.render_long_help().to_string();
    assert!(text.contains("tmt office board list"));
    assert!(text.contains("--general"));
    assert!(!text.contains("--detach"));
    assert!(!text.contains("--wait"));
    assert_eq!(
        parse(&arguments(&["ls", "-h"])).unwrap().invocation,
        Invocation::Help(vec!["list".into()])
    );
    assert!(grammar::help_command(&["__native-install".into()]).is_err());
    assert!(parse(&arguments(&["help", "office", "not-a-command"])).is_err());
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
fn notes_help_identifies_the_shared_file_and_explicit_creation_boundary() {
    let text = grammar::help_command(&["notes".into(), "path".into()])
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(text.contains("Saved identities only"));
    assert!(text.contains("Edit the returned Markdown file directly"));
    assert!(text.contains("Office reads never create missing notes"));
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
        vec!["office", "--help", "--json"],
        vec!["help", "office", "--json"],
    ] {
        let error = parse(&arguments(&argv)).unwrap_err();
        assert_eq!(error.code, "JSON_UNSUPPORTED");
        assert!(error.mode.json);
    }
}

#[test]
fn bare_office_groups_report_usage_instead_of_panicking() {
    for group in ["block", "profile", "prop", "avatar", "board", "extension"] {
        let error = parse(&arguments(&["office", group])).unwrap_err();
        assert_eq!(error.code, "USAGE_ERROR");
        assert!(error.message.contains("subcommand"), "{error:?}");
    }
}

#[test]
fn help_does_not_accept_unrelated_or_unknown_options() {
    for argv in [
        vec!["--json", "--help", "--timeout", "1s"],
        vec!["office", "--help", "--detach"],
        vec!["talk", "--help", "--unknown"],
        vec!["talk", "--help", "--wait"],
        vec!["--help", "--json=false"],
    ] {
        assert_eq!(parse(&arguments(&argv)).unwrap_err().code, "USAGE_ERROR");
    }
}

#[test]
fn explicit_value_options_must_not_be_left_empty_before_help() {
    for mut argv in [
        vec!["talk", "--identity", "--help"],
        vec!["office", "start", "--port", "--help"],
        vec!["office", "board", "list", "--repo", "--help"],
        vec!["office", "--prefix", "--help"],
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
    let mut board = grammar::help_command(&["office".into(), "board".into()]).unwrap();
    assert!(board.render_long_help().to_string().contains("<COMMAND>"));
    let mut post =
        grammar::help_command(&["office".into(), "board".into(), "post".into()]).unwrap();
    for id in ["board-category", "board-body"] {
        assert!(
            post.get_groups()
                .any(|group| group.get_id() == id && group.is_required_set())
        );
    }
    let text = post.render_long_help().to_string();
    assert!(
        text.contains("<--repo <repo>|--general|--room <room>>"),
        "{text}"
    );
    assert!(text.contains("<--body <body>|--file <file>>"), "{text}");
}
