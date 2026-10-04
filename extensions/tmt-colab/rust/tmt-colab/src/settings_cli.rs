//! `tmt colab settings [open on|off]`: show the settings, or change one.
use std::{io::Write, path::Path};
use tmt_colab::{Result, settings};

pub fn run(root: &Path, args: &clap::ArgMatches, json_output: bool) -> Result<()> {
    let loaded = match args.get_one::<String>("value") {
        Some(value) => settings::set_open(root, value == "on")?,
        None => settings::read_or_default(root),
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
        tmt_cli_style::detail::write(&mut out, terminal, "COLAB SETTINGS", &[("open", shown)])?;
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
