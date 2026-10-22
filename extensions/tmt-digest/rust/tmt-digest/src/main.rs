//! Digest member settings through the public Core boundary.

mod core;
mod decisions;
mod grammar;
mod history;
mod pricing;
mod private;
mod settings;
mod status;
mod tick;
mod usage;

use clap::error::ErrorKind;
use std::{ffi::OsString, io::Write, process::ExitCode};

const SKILL: &str = include_str!("../../../skills/tmt-digest/SKILL.md");

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
            if matches.subcommand_name() == Some("status") {
                let selected = matches
                    .subcommand_matches("status")
                    .expect("status matches");
                let window = history::Window::parse(
                    selected
                        .get_one::<String>("window")
                        .expect("default window"),
                )
                .expect("admitted window");
                let result = status::execute(window);
                let successful = result.is_ok();
                let document = match result {
                    Ok(document) => document,
                    Err(error) => {
                        serde_json::json!({"version":1,"error":{"code":error.code,"message":error.message}})
                    }
                };
                let mut output = tmt_cli_style::stream::stdout(true);
                return match serde_json::to_writer(&mut output, &document)
                    .map_err(std::io::Error::other)
                    .and_then(|()| output.write_all(b"\n"))
                    .and_then(|()| output.flush())
                {
                    Ok(()) if successful => ExitCode::SUCCESS,
                    _ => ExitCode::FAILURE,
                };
            }
            if matches.subcommand_name() == Some("decisions") {
                let selected = matches
                    .subcommand_matches("decisions")
                    .expect("decisions matches");
                let since = usage::Since::parse(
                    selected.get_one::<String>("since").expect("default since"),
                )
                .expect("admitted since");
                let mode = if selected.get_flag("report") {
                    decisions::Mode::Report(selected.get_one::<String>("recompute").map(|text| {
                        decisions::Alternative {
                            interval_ms: match settings::Mode::parse(text) {
                                Ok(settings::Mode::Interval { milliseconds, .. }) => milliseconds,
                                _ => unreachable!("admitted interval"),
                            },
                            flush_count: selected.get_one::<u64>("flush-count").copied(),
                        }
                    }))
                } else {
                    decisions::Mode::List
                };
                let result = decisions::execute(since, mode);
                let successful = result.is_ok();
                let document = match result {
                    Ok(document) => document,
                    Err(error) => {
                        serde_json::json!({"version":1,"error":{"code":error.code,"message":error.message}})
                    }
                };
                let mut output = tmt_cli_style::stream::stdout(true);
                return match serde_json::to_writer(&mut output, &document)
                    .map_err(std::io::Error::other)
                    .and_then(|()| output.write_all(b"\n"))
                    .and_then(|()| output.flush())
                {
                    Ok(()) if successful => ExitCode::SUCCESS,
                    _ => ExitCode::FAILURE,
                };
            }
            if matches.subcommand_name() == Some("usage") {
                let selected = matches
                    .subcommand_matches("usage")
                    .and_then(|usage| usage.subcommand_matches("report"))
                    .expect("report matches");
                let since = usage::Since::parse(
                    selected.get_one::<String>("since").expect("default since"),
                )
                .expect("admitted since");
                let format = selected
                    .get_one::<String>("csv")
                    .map_or(usage::Format::Json, |table| {
                        usage::Format::Csv(usage::Table::parse(table).expect("admitted table"))
                    });
                return match usage::render(since, format) {
                    Ok(text) => {
                        let mut output = tmt_cli_style::stream::stdout(true);
                        match output
                            .write_all(text.as_bytes())
                            .and_then(|()| output.flush())
                        {
                            Ok(()) => ExitCode::SUCCESS,
                            Err(_) => ExitCode::FAILURE,
                        }
                    }
                    Err(error) if format == usage::Format::Json => {
                        let document = serde_json::json!({"version":1,"error":{"code":error.code,"message":error.message}});
                        let mut output = tmt_cli_style::stream::stdout(true);
                        let _ = serde_json::to_writer(&mut output, &document)
                            .map_err(std::io::Error::other)
                            .and_then(|()| output.write_all(b"\n"))
                            .and_then(|()| output.flush());
                        ExitCode::FAILURE
                    }
                    Err(error) => report_error(error),
                };
            }
            if matches.subcommand_name() == Some("skill") {
                let mut output = tmt_cli_style::stream::stdout(false);
                return match output
                    .write_all(SKILL.as_bytes())
                    .and_then(|()| output.flush())
                {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(_) => ExitCode::FAILURE,
                };
            }
            if matches.subcommand_name() == Some("tick") {
                return match tick::execute() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => report_error(error),
                };
            }
            let outcome = if let Some(selected) = matches.subcommand_matches("global") {
                save_default(selected.get_one::<String>("value").expect("required value"))
            } else {
                let Some(member) = matches.get_one::<String>("member") else {
                    return print_help(&grammar::command().render_help());
                };
                let value = matches.get_one::<String>("value").expect("paired operands");
                save(member, value)
            };
            match outcome {
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
    let now = write_time()?;
    let (effective, flush_count) = settings::set(&path, &id, &setting, setter.as_deref(), now)?;
    let value = shown(&effective);
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
    if !settings::DigestConfig::load(&path)?.enabled() {
        message.push_str(
            ". Digest is off for everyone, so this applies when it is turned on (tmt digest global on)",
        );
    }
    Ok(message)
}

/// Turn Digest on or off for everyone; every stored setting stays.
fn switch(on: bool) -> Result<String, core::Error> {
    let core = core::Core::discover()?;
    let path = core.settings_path()?;
    settings::set_enabled(&path, on)?;
    let config = settings::DigestConfig::load(&path)?;
    let mut message = if on {
        format!(
            "Digest is on (default: {}). Member settings apply again",
            shown(&config.default)
        )
    } else {
        "Digest is off for everyone: messages are sent one by one. Member settings and the default are kept"
            .to_owned()
    };
    message.push_str(if tick::kick(&core) {
        "; the change is applied now."
    } else {
        "; it is applied by the next tick."
    });
    Ok(message)
}

fn save_default(value: &str) -> Result<String, core::Error> {
    match value {
        "on" => return switch(true),
        "off" => return switch(false),
        _ => {}
    }
    let mode = settings::Mode::parse(value)?;
    let core = core::Core::discover()?;
    let setter = core.setter()?;
    let path = core.settings_path()?;
    settings::set_default(&path, &mode, setter.as_deref(), write_time()?)?;
    let mut message = format!(
        "Saved the global digest default: {} (Digest is on)",
        shown(&mode)
    );
    if !tick::kick(&core) {
        message.push_str(". It is applied by the next tick");
    }
    if setter.is_none() {
        message.push_str(
            ". No caller identity was found, so no one is recorded as the setter; only --urgent and a person marked with tmt identity meta set user true pass the hold. Run it from the pane of the person who should reach members directly.",
        );
    }
    Ok(message)
}

fn write_time() -> Result<i64, core::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| core::Error::new("DIGEST_CLOCK_INVALID", error.to_string()))?
        .as_millis();
    i64::try_from(now)
        .map_err(|_| core::Error::new("DIGEST_CLOCK_INVALID", "Write timestamp overflow"))
}

fn shown(mode: &settings::Mode) -> String {
    match mode {
        settings::Mode::Interval { text, .. } => format!("every {text}"),
        _ => mode.text().to_owned(),
    }
}
