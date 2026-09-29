use super::*;
use crate::{CommandSpec, Example, OutputModes, command};
use clap::{Arg, error::ErrorKind};

fn spec(name: &'static str, examples: &'static [Example]) -> CommandSpec {
    CommandSpec {
        name,
        summary: "Do one thing",
        examples,
        outputs: OutputModes::HumanAndJson,
    }
}

const ROOT: &[Example] = &[Example {
    command: "demo ls",
    note: "List agents",
}];
const LS: &[Example] = &[Example {
    command: "demo ls --json",
    note: "List agents as JSON",
}];
const TALK: &[Example] = &[
    Example {
        command: "demo talk worker 'Run the tests'",
        note: "Send a message",
    },
    Example {
        command: "demo talk worker hello --json",
        note: "Send and report JSON",
    },
];

fn cli(talk: &'static [Example]) -> Command {
    command(&spec("demo", ROOT))
        .subcommand_required(true)
        .subcommand(command(&spec("ls", LS)))
        .subcommand(
            command(&spec("talk", talk))
                .arg(Arg::new("target").required(true))
                .arg(Arg::new("message").required(true)),
        )
}

/// What a CLI built on `command()` shows: clap's own `-h`/`--help`, and a
/// `help <command>` route that renders the same text.
fn check(root: &Command, help_route: bool) -> Vec<Violation> {
    let help = |arguments: &[String]| -> Result<String, String> {
        let arguments = match arguments.split_first() {
            Some((first, path)) if first == "help" => {
                if !help_route {
                    return Err("no help command".into());
                }
                path.iter().cloned().chain(["-h".to_owned()]).collect()
            }
            _ => arguments.to_vec(),
        };
        let argv = std::iter::once("demo".to_owned()).chain(arguments);
        match root.clone().try_get_matches_from(argv) {
            Err(error) if error.kind() == ErrorKind::DisplayHelp => Ok(error.to_string()),
            other => Err(format!("not help: {:?}", other.map(|_| ()))),
        }
    };
    let parse = |arguments: &[String]| -> Result<(), String> {
        let argv = std::iter::once("demo".to_owned()).chain(arguments.iter().cloned());
        root.clone()
            .try_get_matches_from(argv)
            .map(drop)
            .map_err(|error| error.kind().to_string())
    };
    walk(
        root,
        &Probe {
            program: &["demo"],
            help: &help,
            parse: &parse,
        },
    )
}

fn rules(violations: &[Violation]) -> Vec<(String, Rule)> {
    violations
        .iter()
        .map(|violation| (violation.command(&["demo"]), violation.rule))
        .collect()
}

#[test]
fn a_cli_built_from_specs_passes_every_rule() {
    let root = cli(TALK);
    assert_eq!(check(&root, true), []);
    assert_eq!(
        commands(&root, &["demo"]),
        BTreeSet::from(["demo".into(), "demo ls".into(), "demo talk".into()])
    );
}

#[test]
fn examples_must_parse_and_invoke_their_own_command() {
    const BROKEN: &[Example] = &[
        Example {
            command: "demo talk worker --wait hello",
            note: "A flag the grammar does not have",
        },
        Example {
            command: "demo talk worker",
            note: "A missing operand",
        },
        Example {
            command: "demo ls",
            note: "Another command",
        },
    ];
    let violations = check(&cli(BROKEN), true);
    assert_eq!(
        rules(&violations),
        [
            ("demo talk".into(), Rule::ExampleParse),
            ("demo talk".into(), Rule::ExampleParse),
            ("demo talk".into(), Rule::ExampleTarget),
        ]
    );
    assert_eq!(
        failing(&violations, &["demo"]),
        BTreeSet::from(["demo talk".into()])
    );
}

#[test]
fn a_command_outside_the_contract_fails_summary_template_and_examples() {
    let root = cli(TALK).subcommand(
        Command::new("raw").arg(Arg::new("value")).arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(clap::ArgAction::Help),
        ),
    );
    assert_eq!(
        rules(&check(&root, true)),
        [
            ("demo raw".into(), Rule::Summary),
            ("demo raw".into(), Rule::Template),
            ("demo raw".into(), Rule::Examples),
        ]
    );
}

#[test]
fn every_help_form_must_print_the_same_text() {
    let violations = check(&cli(TALK), false);
    assert_eq!(
        failing(&violations, &["demo"]),
        BTreeSet::from(["demo".into(), "demo ls".into(), "demo talk".into()])
    );
    assert!(
        violations
            .iter()
            .all(|violation| violation.rule == Rule::HelpForms)
    );
}

#[test]
fn the_template_requires_its_order_with_examples_last() {
    assert!(
        template("Do it\n\nUsage: demo\n\nOptions:\n  -h\n\nExamples:\n  # x\n  demo\n").is_ok()
    );
    assert!(template("Preamble\nDo it\n\nUsage: demo\n\nExamples:\n").is_err());
    assert!(template("Do it\n\nUsage: demo\n\nExamples:\n  # x\n  demo\n\nOptions:\n").is_err());
    assert!(template("Do it\n\nUsage: demo\n\nCommands:\n\nOptions:\n\nExamples:\n").is_ok());
    assert!(template("Do it\n\nUsage: demo\n\nOptions:\n\nCommands:\n\nExamples:\n").is_err());
    assert!(template("Do it\n\nUsage: demo\n\nOptions:\n").is_err());
}

#[test]
fn the_allowlist_must_equal_what_still_fails() {
    const BROKEN: &[Example] = &[Example {
        command: "demo talk worker",
        note: "A missing operand",
    }];
    let violations = check(&cli(BROKEN), true);
    let exact: &[(&str, &[Rule])] = &[("demo talk", &[Rule::ExampleParse])];
    assert_eq!(
        allowlist_report(&violations, &["demo"], exact),
        [] as [String; 0]
    );
    let stale: &[(&str, &[Rule])] = &[
        ("demo talk", &[Rule::ExampleParse]),
        ("demo ls", &[Rule::Examples]),
    ];
    assert_eq!(
        allowlist_report(&violations, &["demo"], stale),
        [r#"remove "demo ls": it now follows the style"#]
    );
    assert_eq!(
        allowlist_report(&violations, &["demo"], &[]),
        [r#"list ("demo talk", &[ExampleParse])"#]
    );
    let wrong_rule: &[(&str, &[Rule])] = &[("demo talk", &[Rule::Examples])];
    assert_eq!(
        allowlist_report(&violations, &["demo"], wrong_rule).len(),
        1
    );
}
