//! Pure external-command recognition and grammar-owned precedence.

use crate::invocation::{Invocation, OutputMode, ParseError, Parsed};
use clap::Command;
use std::{collections::BTreeSet, ffi::OsString};
use tmt_core::extension_command::valid_extension_name;

pub fn reserved(definition: &Command) -> BTreeSet<String> {
    definition
        .get_subcommands()
        .flat_map(|command| std::iter::once(command.get_name()).chain(command.get_all_aliases()))
        .map(str::to_owned)
        .collect()
}

/// Called before help-intent scanning: all tokens after an external name are
/// opaque, including --help and --json. A core name always retains its parser.
pub fn candidate(definition: &Command, argv: &[OsString]) -> Option<Result<Parsed, ParseError>> {
    let names = reserved(definition);
    let first = argv.first()?.to_str()?;
    let selection = if first == "help" && argv.len() == 2 {
        let name = argv[1].to_str()?;
        if names.contains(name) || !valid_extension_name(name) {
            return None;
        }
        (name.to_owned(), vec!["--help".into()], true, vec![])
    } else if names.contains(first) {
        return None;
    } else if valid_extension_name(first) {
        (first.to_owned(), argv[1..].to_vec(), false, vec![])
    } else {
        // Let Clap locate the root command through option arities. Do not guess
        // that an option value or a malformed core operand is an extension.
        let matches = definition
            .clone()
            .allow_external_subcommands(true)
            .external_subcommand_value_parser(clap::value_parser!(OsString))
            .try_get_matches_from(
                std::iter::once(OsString::from("tmt")).chain(argv.iter().cloned()),
            )
            .ok()?;
        let (name, tail) = matches.subcommand()?;
        if names.contains(name) || !valid_extension_name(name) {
            return None;
        }
        let args = tail
            .get_many::<OsString>("")
            .map(|values| values.cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let offset = argv.len() - args.len() - 1;
        (name.to_owned(), args, false, argv[..offset].to_vec())
    };
    Some(Ok(Parsed {
        invocation: Invocation::Extension {
            name: selection.0,
            args: selection.1,
            help: selection.2,
            prefix: selection.3,
        },
        mode: OutputMode::default(),
    }))
}

pub fn suggestion(name: &str, available: impl IntoIterator<Item = String>) -> Option<String> {
    let names = reserved(&crate::grammar::grammar());
    let mut definition = Command::new("tmt").disable_help_subcommand(true);
    for external in available
        .into_iter()
        .filter(|name| valid_extension_name(name) && !names.contains(name))
    {
        definition = definition.subcommand(Command::new(external));
    }
    let error = definition.try_get_matches_from(["tmt", name]).err()?;
    error
        .get(clap::error::ContextKind::SuggestedSubcommand)
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    fn parse(words: &[&str]) -> Option<Result<Parsed, ParseError>> {
        candidate(
            &crate::grammar::grammar(),
            &words.iter().map(OsString::from).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn grammar_reserves_hidden_names_and_aliases() {
        let definition = crate::grammar::grammar();
        let names = reserved(&definition);
        for name in ["office", "run", "this", "ls", "__complete"] {
            assert!(names.contains(name), "{name}");
            assert!(parse(&[name, "--help"]).is_none());
        }
    }

    #[test]
    fn extra_hint_only_suggests_available_extensions_not_core_commands() {
        assert_eq!(suggestion("rnu", vec![]), None);
        assert_eq!(
            suggestion("vaultt", vec!["vault".into()]),
            Some("vault".into())
        );
        assert_eq!(suggestion("rnu", vec!["run".into()]), None);
    }

    #[test]
    fn external_tail_is_not_parsed_or_utf8_normalized() {
        let tail = vec![
            "--json".into(),
            "--help".into(),
            OsString::from_vec(vec![0xff]),
            "".into(),
        ];
        let words = std::iter::once(OsString::from("example"))
            .chain(tail.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            candidate(&crate::grammar::grammar(), &words)
                .unwrap()
                .unwrap()
                .invocation,
            Invocation::Extension {
                name: "example".into(),
                args: tail,
                help: false,
                prefix: vec![]
            }
        );
        assert_eq!(
            parse(&["help", "example"]).unwrap().unwrap().invocation,
            Invocation::Extension {
                name: "example".into(),
                args: vec!["--help".into()],
                help: true,
                prefix: vec![]
            }
        );
    }

    #[test]
    fn invalid_names_keep_core_errors_and_root_options_are_not_forwarded() {
        for name in ["../x", "Office", "a.b", "-example"] {
            assert!(parse(&[name]).is_none(), "{name}");
        }
        assert_eq!(
            parse(&["--json", "example", "x"])
                .unwrap()
                .unwrap()
                .invocation,
            Invocation::Extension {
                name: "example".into(),
                args: vec!["x".into()],
                help: false,
                prefix: vec!["--json".into()]
            }
        );
    }
}
