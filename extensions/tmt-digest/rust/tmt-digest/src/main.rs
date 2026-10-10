//! Help-only entry for the unpublished Digest extension.

mod grammar;

use clap::error::ErrorKind;
use std::{ffi::OsString, io::Write, process::ExitCode};

fn main() -> ExitCode {
    let mut words: Vec<OsString> = std::env::args_os().skip(1).collect();
    if words.is_empty() || words == [OsString::from("help")] {
        words = vec!["--help".into()];
    }
    let result = grammar::command()
        .try_get_matches_from(std::iter::once(OsString::from("tmt-digest")).chain(words));
    match result {
        Err(error) if error.kind() == ErrorKind::DisplayHelp => print_help(&error.render()),
        Err(error) => {
            let code = if error.use_stderr() { 2 } else { 0 };
            match error.print() {
                Ok(()) => ExitCode::from(code),
                Err(_) => ExitCode::FAILURE,
            }
        }
        Ok(_) => print_help(&grammar::command().render_help()),
    }
}

fn print_help(help: &clap::builder::StyledStr) -> ExitCode {
    let mut output = tmt_cli_style::stream::stdout(false);
    let text = tmt_cli_style::rendered_help(help, output.terminal());
    match output
        .write_all(text.as_bytes())
        .and_then(|()| output.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
