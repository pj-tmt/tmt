//! `tmt colab settings [open on|off | page-expiry DAYS|off|default]`: show the settings, or change
//! one.
use crate::cli_management::fail;
use std::{io::Write, path::Path};
use tmt_colab::{
    Result,
    page_expiry::{self, SpaceExpiry},
    settings,
};

fn invalid(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    fail("COLAB_INPUT_INVALID", message)
}
pub fn run(root: &Path, args: &clap::ArgMatches, json_output: bool) -> Result<()> {
    let value = args.get_one::<String>("value");
    let loaded = match args.get_one::<String>("key").map(String::as_str) {
        None => settings::read_or_default(root),
        Some("open") => match value {
            Some(value) if ["on", "off"].contains(&value.as_str()) => {
                settings::set_open(root, value == "on")?
            }
            _ => return Err(invalid("open takes on or off.")),
        },
        Some("page-expiry") => match value {
            None => settings::read_or_default(root),
            Some(value) => settings::set_page_expiry(
                root,
                match value.as_str() {
                    "off" => SpaceExpiry::Off,
                    "default" => SpaceExpiry::Default,
                    days => SpaceExpiry::Days(
                        days.parse::<u64>()
                            .ok()
                            .filter(|n| {
                                (1..=page_expiry::MAX_DAYS).contains(n)
                                    && days.bytes().all(|b| b.is_ascii_digit())
                            })
                            .ok_or_else(|| {
                                invalid(&format!(
                                    "page-expiry takes a whole number of days from 1 to {}, off, or default.",
                                    page_expiry::MAX_DAYS
                                ))
                            })?,
                    ),
                },
            )?,
        },
        Some(_) => return Err(invalid("Settings are open and page-expiry.")),
    };
    let mut out = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(out, "{}", loaded.json())?;
    } else {
        let terminal = out.terminal();
        let shown = format!(
            "{} ({})",
            if loaded.open() { "on" } else { "off" },
            loaded.source()
        );
        let expiry_text = match loaded.page_expiry() {
            SpaceExpiry::Default => format!("{} days (default)", page_expiry::DEFAULT_DAYS),
            SpaceExpiry::Days(days) => format!("{days} days"),
            SpaceExpiry::Off => "off".to_owned(),
        };
        tmt_cli_style::detail::write(
            &mut out,
            terminal,
            "COLAB SETTINGS",
            &[("open", shown), ("page-expiry", expiry_text)],
        )?;
    }
    if loaded.malformed && !json_output {
        let mut err = tmt_cli_style::stream::stderr();
        let terminal = err.terminal();
        tmt_cli_style::message::warning(
            &mut err,
            terminal,
            settings::UNREADABLE,
            Some("tmt colab settings open on"),
        )?;
    }
    Ok(())
}
