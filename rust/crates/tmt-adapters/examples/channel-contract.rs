//! Opt-in provider contract for the Claude channel launch (#329). Runs only
//! `--version` and `--help`, never a conversation, MCP server or installation.
//! It fails when the version leaves the recorded range or `--help` stops
//! documenting `--mcp-config`. Claude 2.1.285 does not list the preview
//! `--dangerously-load-development-channels` flag in `--help`, so that flag is
//! covered only by the exact-version pin and the spike's evidence.

use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    drivers::claude::channel::{MCP_CONFIG_FLAG, SUPPORTED_VERSIONS},
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
};

fn output(executable: &Path, args: &[OsString]) -> Result<String, String> {
    let result = UnixCommandRunner
        .execute(CommandRequest {
            program: executable.as_os_str(),
            args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(10),
            max_output_bytes: 256 * 1024,
        })
        .map_err(|error| format!("Provider channel check failed: {error}"))?;
    String::from_utf8(result.stdout).map_err(|_| "Provider output is not UTF-8".into())
}

fn check() -> Result<(), String> {
    let mut arguments = std::env::args_os().skip(1);
    let (Some(executable), None) = (arguments.next(), arguments.next()) else {
        return Err("Usage: channel-contract /absolute/claude".into());
    };
    let executable = Path::new(&executable);
    if !executable.is_absolute() {
        return Err("Usage: channel-contract /absolute/claude".into());
    }
    let version = output(executable, &["--version".into()])?;
    if !SUPPORTED_VERSIONS.contains(&version.trim()) {
        return Err(format!(
            "claude channel version drift: supported {SUPPORTED_VERSIONS:?}, received {:?}. Record channel evidence for the new version before widening the range.",
            version.trim()
        ));
    }
    let help = output(executable, &["--help".into()])?;
    if !help.contains(MCP_CONFIG_FLAG) {
        return Err(format!(
            "claude --help no longer documents {MCP_CONFIG_FLAG}"
        ));
    }
    Ok(())
}

fn main() {
    match check() {
        Ok(()) => println!("claude channel launch contract matches"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
