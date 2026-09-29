//! Workspace-local configuration initialization.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::io::{self, Write};
use tmt_adapters::config::{ConfigFiles, ConfigPaths};

fn run() -> Result<std::path::PathBuf, Failure> {
    let paths = ConfigPaths::discover()?;
    let local_config = paths.local_config.clone();
    ConfigFiles { paths }.initialize_local()?;
    Ok(local_config)
}

pub fn execute(mode: OutputMode) -> io::Result<u8> {
    let created = match run() {
        Ok(path) => path,
        Err(error) => return error.publish(mode),
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let terminal = output.terminal();
    if mode.json {
        writeln!(output, "{}", json!({"created": created}))?;
    } else {
        tmt_cli_style::message::success(
            &mut output,
            terminal,
            &format!("Created {}", created.display()),
        )?;
    }
    Ok(0)
}
