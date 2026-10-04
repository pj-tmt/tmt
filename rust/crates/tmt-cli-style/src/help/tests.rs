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
            theme: None,
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

fn wrapping_command() -> Command {
    let mut definition = spec(
        &[Example {
            command: "tmt talk worker 'Keep this runnable command on one line even when its quoted message makes it longer than eighty display cells'",
            note: "This example note also stays on one line even when its explanation is longer than eighty display cells",
        }],
        OutputModes::Human,
    );
    definition.details = "Only the recorded user or squad lead may change scheduled jobs; announcements are best effort and no catch-up or run results are stored.";
    command(&definition).arg(Arg::new("actor").long("actor").help(
        "Explicit actor; otherwise use the verified caller, then the recorded user, while preserving the scheduled job's owner and permission checks",
    ))
}

#[test]
fn terminal_help_wraps_prose_at_80_cells_without_splitting_words() {
    let command = wrapping_command();
    let original = help_text(&command, Terminal::PLAIN);
    for color in [false, true] {
        let terminal = Terminal {
            color,
            width: Some(80),
            theme: None,
        };
        let text = help_text(&command, terminal);
        assert_eq!(
            text.split_whitespace().collect::<Vec<_>>(),
            rendered_help(
                &command.clone().render_help(),
                Terminal {
                    width: None,
                    ..terminal
                }
            )
            .split_whitespace()
            .collect::<Vec<_>>(),
            "wrapping preserves the ANSI bytes attached to every word"
        );
        let text = anstream::adapter::strip_str(&text).to_string();
        assert_eq!(
            text,
            help_text(
                &command,
                Terminal {
                    color: false,
                    ..terminal
                }
            )
        );
        assert_ne!(text, original);
        assert_eq!(
            text.split_whitespace().collect::<Vec<_>>(),
            original.split_whitespace().collect::<Vec<_>>(),
            "every original word survives intact"
        );
        let prose = text.split("Examples:").next().unwrap();
        assert!(prose.lines().all(|line| line.width() <= 80));
        let details = prose.split("Details:\n").nth(1).unwrap();
        assert!(details.lines().filter(|line| !line.is_empty()).count() > 1);
        assert!(
            details
                .lines()
                .filter(|line| !line.is_empty())
                .all(|line| line.starts_with("  ") && !line.starts_with("   "))
        );
        assert_eq!(examples(&text), examples(&original));
        let error = command
            .clone()
            .try_get_matches_from(["ls", "--help"])
            .unwrap_err();
        assert_eq!(
            rendered_help(&error.render(), terminal),
            help_text(&command, terminal)
        );
    }
}

#[test]
fn cjk_help_paragraphs_use_display_cells_and_preserve_indentation() {
    // CJK fixture words intentionally exercise double-width display cells.
    let paragraph = "  日本語 表示幅 保存済み 利用者 実行予定 確認事項 日本語 表示幅 保存済み 利用者 実行予定 確認事項 日本語 表示幅 保存済み 利用者 実行予定 確認事項";
    let rendered = clap::builder::StyledStr::from(format!("Details:\n{paragraph}\n"));
    let text = rendered_help(
        &rendered,
        Terminal {
            width: Some(80),
            ..Terminal::PLAIN
        },
    );
    let lines: Vec<_> = text.lines().skip(1).collect();
    assert!(
        lines.len() > 1,
        "byte or scalar width must not stand in for cells"
    );
    assert_eq!(lines[0].width(), 79);
    assert!(
        lines
            .iter()
            .all(|line| line.width() <= 80 && line.starts_with("  "))
    );
    assert_eq!(
        lines.join(" ").split_whitespace().collect::<Vec<_>>(),
        paragraph.split_whitespace().collect::<Vec<_>>()
    );
}

#[test]
fn usage_commands_and_examples_never_wrap() {
    let long = "tmt talk worker 'A runnable command with a quoted message that exceeds the terminal width must stay on its original line'";
    let input = format!(
        "Usage: {long}\n\nDetails:\n  {long}\n\nExamples:\n  # A very long example explanation that must stay on its original line just like the command below it\n  {long}\n"
    );
    let rendered = clap::builder::StyledStr::from(input.clone());
    assert_eq!(
        rendered_help(
            &rendered,
            Terminal {
                width: Some(80),
                ..Terminal::PLAIN
            }
        ),
        input
    );
}

#[test]
fn pipes_json_unknown_and_small_widths_keep_existing_help_bytes() {
    let command = wrapping_command();
    let rendered = command.clone().render_help();
    assert_eq!(
        help_text(&command, Terminal::stdout(true)),
        rendered.to_string()
    );
    for color in [false, true] {
        for width in [None, Some(0), Some(39)] {
            let terminal = Terminal {
                color,
                width,
                theme: None,
            };
            let expected = if color {
                rendered.ansi().to_string()
            } else {
                rendered.to_string()
            };
            assert_eq!(help_text(&command, terminal), expected);
            assert_eq!(rendered_help(&rendered, terminal), expected);
        }
    }
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
