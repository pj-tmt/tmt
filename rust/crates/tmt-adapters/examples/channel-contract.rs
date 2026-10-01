//! Opt-in provider contract for the Claude channel launch (#329). Runs only
//! `--version` and `--help`, never a conversation, MCP server or installation.
//! It fails when the version leaves the accepted range or `--help` stops
//! documenting `--mcp-config`. Claude 2.1.285 does not list the preview
//! `--dangerously-load-development-channels` flag in `--help`, so that flag is
//! covered only by the version range and the recorded live evidence.

use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    drivers::claude::channel::{
        BuildStatus, MCP_CONFIG_FLAG, MINIMUM_VERSION, TESTED_VERSIONS, build_status,
    },
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

fn check() -> Result<BuildStatus, String> {
    let mut arguments = std::env::args_os().skip(1);
    let (Some(executable), None) = (arguments.next(), arguments.next()) else {
        return Err("Usage: channel-contract /absolute/claude".into());
    };
    let executable = Path::new(&executable);
    if !executable.is_absolute() {
        return Err("Usage: channel-contract /absolute/claude".into());
    }
    let version = output(executable, &["--version".into()])?;
    let Some(status) = build_status(version.trim()) else {
        return Err(format!(
            "claude channel version drift: accepted range is {MINIMUM_VERSION} or newer in its major line, received {:?}.",
            version.trim()
        ));
    };
    let help = output(executable, &["--help".into()])?;
    if !help.contains(MCP_CONFIG_FLAG) {
        return Err(format!(
            "claude --help no longer documents {MCP_CONFIG_FLAG}"
        ));
    }
    Ok(status)
}

fn main() {
    match check() {
        Ok(BuildStatus::Tested) => println!("claude channel launch contract matches"),
        Ok(BuildStatus::Untested) => println!(
            "claude channel launch contract matches an untested build (tested: {}); only --version and --help were checked",
            TESTED_VERSIONS.join(", ")
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
