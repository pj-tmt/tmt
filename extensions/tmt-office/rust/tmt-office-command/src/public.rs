//! Direct public Office entrypoint; protocol invocations remain in tmt-office.

use clap::{Command, error::ErrorKind};
use std::{
    ffi::OsString,
    io::{self, Write},
};
use tmt_command_output::{Failure, OutputMode};

fn with_help(command: Command) -> Command {
    command
        .disable_help_flag(false)
        .disable_help_subcommand(false)
        .mut_subcommands(with_help)
}

fn help(argv: &[OsString]) -> Option<String> {
    let arguments = std::iter::once(OsString::from("tmt-office")).chain(argv.iter().cloned());
    match with_help(crate::grammar::grammar())
        .name("tmt-office")
        .try_get_matches_from(arguments)
    {
        Err(error) if error.kind() == ErrorKind::DisplayHelp => Some(error.to_string()),
        _ => None,
    }
}

pub fn execute(argv: &[OsString]) -> io::Result<u8> {
    if let Some(help) = help(argv) {
        let mode = OutputMode {
            json: argv.iter().any(|word| word == "--json"),
        };
        if mode.json {
            return Failure::new(
                "JSON_UNSUPPORTED",
                "This command does not support --json.",
                1,
            )
            .publish(mode);
        }
        io::stdout().lock().write_all(help.as_bytes())?;
        return Ok(0);
    }
    let parsed = match crate::parser::parse_public(argv) {
        Ok(parsed) => parsed,
        Err(error) => return Failure::new(error.code, error.message, 1).publish(error.mode),
    };
    let core = match crate::process_core_access::ProcessCoreAccess::discover() {
        Ok(core) => core,
        Err(error) => return error.publish(parsed.mode),
    };
    crate::office_command::execute(parsed.prefix, parsed.operation, parsed.mode, &core)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }
    #[test]
    fn help_uses_the_command_grammar_and_never_interprets_option_values() {
        assert!(help(&args(&["--help"])).unwrap().contains("tmt-office"));
        assert!(
            help(&args(&["help", "profile", "show"]))
                .unwrap()
                .contains("--identity")
        );
        assert!(
            help(&args(&["profile", "show", "--help"]))
                .unwrap()
                .contains("--identity")
        );
        assert!(help(&args(&["help", "absent"])).is_none());
        assert!(help(&args(&["--absent", "--help"])).is_none());
        assert!(help(&args(&["profile", "show", "--identity=--help"])).is_none());
        assert!(help(&args(&["profile", "show", "--", "--help"])).is_none());
    }
}
