//! Digest member settings through the public Core boundary.

mod core;
mod grammar;
mod settings;
mod tick;

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
        Ok(matches) => {
            if matches.subcommand_name() == Some("tick") {
                return match tick::execute() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => report_error(error),
                };
            }
            let Some(member) = matches.get_one::<String>("member") else {
                return print_help(&grammar::command().render_help());
            };
            let value = matches.get_one::<String>("value").expect("paired operands");
            match save(member, value) {
                Ok(message) => {
                    let mut output = tmt_cli_style::stream::stdout(false);
                    let terminal = output.terminal();
                    match tmt_cli_style::message::success(&mut output, terminal, &message)
                        .and_then(|()| output.flush())
                    {
                        Ok(()) => ExitCode::SUCCESS,
                        Err(_) => ExitCode::FAILURE,
                    }
                }
                Err(error) => report_error(error),
            }
        }
    }
}

fn report_error(error: core::Error) -> ExitCode {
    let mut output = tmt_cli_style::stream::stderr();
    let terminal = output.terminal();
    let _ = tmt_cli_style::message::error(
        &mut output,
        terminal,
        &format!("{} ({})", error.message, error.code),
        None,
    );
    ExitCode::FAILURE
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

fn save(member: &str, value: &str) -> Result<String, core::Error> {
    let setting = settings::MemberSetting::parse(value)?;
    let core = core::Core::discover()?;
    let (id, name) = core.member(member)?;
    let setter = core.setter()?;
    let path = core.settings_path()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| core::Error::new("DIGEST_CLOCK_INVALID", error.to_string()))?
        .as_millis();
    let now = i64::try_from(now)
        .map_err(|_| core::Error::new("DIGEST_CLOCK_INVALID", "Write timestamp overflow"))?;
    let (effective, flush_count) = settings::set(&path, &id, &setting, setter.as_deref(), now)?;
    let value = match &effective {
        settings::Mode::Interval { text, .. } => format!("every {text}"),
        _ => effective.text().to_owned(),
    };
    let mut message = if setting == settings::MemberSetting::Default {
        format!("{name} now uses the default digest: {value}")
    } else {
        format!("Saved {name}'s digest: {value} (member setting)")
    };
    if matches!(effective, settings::Mode::Interval { .. }) {
        message.push_str(&format!(
            ". Sends early once {flush_count} messages are held"
        ));
    }
    Ok(message)
}
