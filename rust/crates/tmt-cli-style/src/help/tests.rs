use super::*;

const EXAMPLES: &[Example] = &[Example {
    command: "tmt ls",
    note: "List every agent",
}];

fn spec(examples: &'static [Example], outputs: OutputModes) -> CommandSpec {
    CommandSpec {
        name: "ls",
        summary: "List agents",
        examples,
        outputs,
        details: "",
    }
}

#[test]
fn short_long_and_rendered_help_are_the_same_text() {
    let command = command(&spec(EXAMPLES, OutputModes::HumanAndJson));
    let expected = help_text(&command, Terminal::PLAIN);
    for flag in ["-h", "--help"] {
        let error = command
            .clone()
            .try_get_matches_from(["ls", flag])
            .unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
        assert_eq!(error.to_string(), expected, "{flag}");
    }
    assert!(expected.contains("--json"));
    assert!(expected.ends_with("Examples:\n  # List every agent\n  tmt ls\n"));
}

#[test]
fn json_is_offered_only_with_human_output() {
    for outputs in [OutputModes::Human, OutputModes::Json] {
        let command = command(&spec(EXAMPLES, outputs));
        assert!(command.get_arguments().all(|arg| arg.get_id() != "json"));
    }
}

#[test]
#[should_panic(expected = "ls needs one to three examples")]
fn a_command_without_examples_is_refused() {
    command(&spec(&[], OutputModes::Human));
}

#[test]
fn examples_split_into_the_argv_a_shell_passes() {
    let argv = |command: &'static str| Example { command, note: "" }.argv();
    assert_eq!(
        argv(r#"tmt talk worker "fix the \"build\"" --wait"#).unwrap(),
        ["tmt", "talk", "worker", "fix the \"build\"", "--wait"]
    );
    assert_eq!(
        argv("tmt  x  'a b' c\\ d ''").unwrap(),
        ["tmt", "x", "a b", "c d", ""]
    );
    assert!(argv("tmt 'open").is_err());
    assert!(argv("tmt \"open").is_err());
    assert!(argv("tmt x\\").is_err());
}

#[test]
fn examples_read_back_exactly_what_the_help_shows() {
    const THREE: &[Example] = &[
        Example {
            command: "tmt talk worker \"Run the tests\"",
            note: "Send a message to one agent",
        },
        Example {
            command: "tmt talk worker --wait \"Is it green?\"",
            note: "Wait for the reply",
        },
        Example {
            command: "tmt talk worker --detach 'Deploy later'",
            note: "Queue without waiting",
        },
    ];
    let sections = [HelpSection {
        title: "Extensions".into(),
        entries: vec![("sq".into(), "Squad".into())],
    }];
    for terminal in [
        Terminal::PLAIN,
        Terminal {
            color: true,
            width: None,
        },
    ] {
        let command = command_with_sections(&spec(THREE, OutputModes::Human), &sections);
        let text = anstream::adapter::strip_str(&help_text(&command, terminal)).to_string();
        let shown = examples(&text).unwrap();
        let expected: Vec<_> = THREE
            .iter()
            .map(|example| ShownExample {
                note: example.note.into(),
                command: example.command.into(),
            })
            .collect();
        assert_eq!(shown, expected);
        for (shown, example) in shown.iter().zip(THREE) {
            assert_eq!(shown.argv(), example.argv());
        }
    }
}

#[test]
fn a_malformed_or_missing_examples_section_is_reported() {
    assert!(examples("Usage: tmt ls\n").is_err());
    assert!(examples("Examples:\n  tmt ls\n").is_err());
    assert!(examples("Examples:\n  # List agents\n").is_err());
    assert!(examples("Examples:\n  # List agents\n  # again\n").is_err());
    assert_eq!(examples("Examples:\n").unwrap(), []);
}

#[test]
fn details_render_just_before_examples_and_only_when_set() {
    let mut with = spec(EXAMPLES, OutputModes::Human);
    with.details = "Saved identities only.\nEdit the file directly.";
    let text = help_text(&command(&with), Terminal::PLAIN);
    assert!(text.ends_with(
        "Details:\n  Saved identities only.\n  Edit the file directly.\n\nExamples:\n  # List every agent\n  tmt ls\n"
    ));
    let text = help_text(
        &command(&spec(EXAMPLES, OutputModes::Human)),
        Terminal::PLAIN,
    );
    assert!(!text.contains("Details:"));
}

fn nested() -> clap::Command {
    let leaf = |name: &'static str| {
        command(&CommandSpec {
            name,
            summary: "A leaf",
            examples: &[Example {
                command: "tmt hotkeys install",
                note: "Install",
            }],
            outputs: OutputModes::Human,
            details: "",
        })
    };
    command(&spec(EXAMPLES, OutputModes::Human))
        .name("tmt")
        .bin_name("tmt")
        .subcommand(leaf("hotkeys").subcommand(leaf("install").visible_alias("add")))
        .subcommand(leaf("__complete").hide(true))
}

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

fn routed(line: &str) -> String {
    match route(&nested(), &words(line)) {
        Route::Help(command) => help_text(&command, Terminal::PLAIN),
        other => panic!("{line:?} did not resolve: {other:?}"),
    }
}

fn dash_h(line: &str) -> String {
    nested()
        .try_get_matches_from(std::iter::once("tmt".to_owned()).chain(words(line)))
        .unwrap_err()
        .to_string()
}

#[test]
fn help_resolves_to_the_text_dash_h_prints() {
    assert_eq!(routed("help"), dash_h("-h"));
    assert_eq!(routed("help hotkeys"), dash_h("hotkeys -h"));
    assert_eq!(
        routed("help hotkeys install"),
        dash_h("hotkeys install --help")
    );
    assert!(routed("help hotkeys install").contains("Usage: tmt hotkeys install"));
}

#[test]
fn help_follows_aliases_and_reports_the_first_unknown_word() {
    assert_eq!(routed("help hotkeys add"), routed("help hotkeys install"));
    for (line, word) in [
        ("help nope", "nope"),
        ("help hotkeys nope", "nope"),
        ("help hotkeys install extra", "extra"),
        ("help hotkeys -h", "-h"),
        ("help --json", "--json"),
        ("help __complete", "__complete"),
    ] {
        match route(&nested(), &words(line)) {
            Route::Unknown(found) => assert_eq!(found, word, "{line}"),
            other => panic!("{line:?}: {other:?}"),
        }
    }
}

#[test]
fn only_help_first_is_a_help_request() {
    for line in [
        "",
        "hotkeys -h",
        "hotkeys help",
        "nope help hotkeys",
        "--help",
    ] {
        assert!(
            matches!(route(&nested(), &words(line)), Route::Other),
            "{line:?}"
        );
    }
}
