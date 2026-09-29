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
