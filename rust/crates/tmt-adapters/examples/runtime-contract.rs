//! Opt-in parser contract for the provider versions observed in issue #321.
//! Runs only --version and --help, never a conversation or provider installation.

use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
    runtime::{CLAUDE_MODE_DEFAULT, CODEX_MODE_EMBEDDED, CODEX_MODE_SHARED, RuntimeRegistry},
};
use tmt_core::{
    binding::session::{HarnessId, ProviderSessionId, RememberedSession, RuntimeMode},
    driver::ActionResult,
};

fn output(executable: &Path, args: &[OsString]) -> Result<String, String> {
    let result = UnixCommandRunner
        .execute(CommandRequest {
            program: executable.as_os_str(),
            args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(10),
            max_output_bytes: 128 * 1024,
        })
        .map_err(|error| format!("Provider parser check failed: {error}"))?;
    String::from_utf8(result.stdout).map_err(|_| "Provider output is not UTF-8".into())
}

fn check() -> Result<(), String> {
    let executables: Vec<_> = std::env::args_os().skip(1).collect();
    if executables.len() != 2
        || executables
            .iter()
            .any(|value| !Path::new(value).is_absolute())
    {
        return Err("Usage: runtime-contract /absolute/claude /absolute/codex".into());
    }
    let mut registry = RuntimeRegistry::first_party();
    for (executable, harness, version, modes, syntax) in [
        (
            &executables[0],
            "claude",
            "2.1.283 (Claude Code)",
            &[CLAUDE_MODE_DEFAULT][..],
            "--resume",
        ),
        (
            &executables[1],
            "codex",
            "codex-cli 0.157.1",
            &[CODEX_MODE_SHARED, CODEX_MODE_EMBEDDED][..],
            "[SESSION_ID]",
        ),
    ] {
        let executable = Path::new(executable);
        let actual = output(executable, &["--version".into()])?;
        if actual.trim() != version {
            return Err(format!(
                "{harness} version drift: expected {version:?}, received {:?}. Revalidate provider behavior before updating this contract.",
                actual.trim()
            ));
        }
        for mode in modes {
            let session = RememberedSession {
                harness: HarnessId::new(harness).map_err(|error| error.to_string())?,
                mode: RuntimeMode::new(mode).map_err(|error| error.to_string())?,
                provider_session: ProviderSessionId::new("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
                    .map_err(|error| error.to_string())?,
                state: None,
                stale_at_ms: None,
            };
            let ActionResult::Completed(mut command) = registry.resume(&session) else {
                return Err(format!("{harness}/{mode}: resume mapping unavailable"));
            };
            // Help is parsed by the actual provider, with the production driver's
            // exact resume arguments. It must not look up the synthetic session.
            command.args.push("--help".into());
            let help = output(executable, &command.args)?;
            if !help.contains(syntax) {
                return Err(format!(
                    "{harness}/{mode}: expected resume syntax missing from help"
                ));
            }
            println!(
                "{harness}/{mode}: {version}; generated resume arguments accepted by help parser"
            );
        }
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match check() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
