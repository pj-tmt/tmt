//! Replacement delegates board discovery and migration to the verified Ops release.

use super::{Human, failure};
use crate::output::Failure;
use serde_json::Value;
use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    native_install::ManagedInstallation,
    process::{CommandFailure, CommandRequest, CommandRunner, UnixCommandRunner},
};

/// No installation lock crosses this child. A deferred switch keeps the former
/// executable available for the same operation to verify on a later invocation.
pub(super) fn run(
    installation: &ManagedInstallation,
    document: &mut Value,
    human: &mut Human,
) -> Result<bool, Failure> {
    let core =
        std::env::current_exe().map_err(|error| failure("EXTENSION_INSTALL_FAILED", error))?;
    let args: Vec<OsString> = vec![
        format!("TMT_EXECUTABLE={}", core.display()).into(),
        installation.active_executable.as_os_str().into(),
        "migration".into(),
        "switch".into(),
        "--yes".into(),
        "--prefix".into(),
        installation.prefix().as_os_str().into(),
        "--json".into(),
    ];
    let output = match UnixCommandRunner.execute(CommandRequest {
        program: Path::new("/usr/bin/env").as_os_str(),
        args: &args,
        input: b"",
        deadline: Instant::now() + Duration::from_secs(90),
        max_output_bytes: 1024 * 1024,
    }) {
        Ok(output) => output,
        Err(mut error)
            if !error.cleanup_failed()
                && matches!(
                    error.kind,
                    CommandFailure::Exit {
                        code: Some(1),
                        signal: None
                    }
                )
                && error.output.is_some() =>
        {
            error.output.take().expect("checked output")
        }
        Err(error) => {
            return Err(Failure::new(
                "EXTENSION_INSTALL_FAILED",
                format!(
                    "Ops is installed, but board switching did not finish: {error}. The former installation was retained."
                ),
                1,
            ));
        }
    };
    let result: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        Failure::new(
            "EXTENSION_INSTALL_FAILED",
            "Ops returned no board-switch report; the former installation was retained.",
            1,
        )
        .caused_by(error)
    })?;
    let complete = result["complete"].as_bool().ok_or_else(|| {
        Failure::new(
            "EXTENSION_INSTALL_FAILED",
            "Ops returned an invalid board-switch report; the former installation was retained.",
            1,
        )
    })?;
    let switched = result["switched"]
        .as_u64()
        .filter(|count| *count <= 32)
        .ok_or_else(|| {
            Failure::new(
                "EXTENSION_INSTALL_FAILED",
                "Ops returned an invalid board count; the former installation was retained.",
                1,
            )
        })?;
    document["boardSwitch"] = result.clone();
    if complete {
        human.push(format!("switched {switched} boards to Ops"));
    } else {
        let command = result["command"].as_str().ok_or_else(|| {
            Failure::new(
                "EXTENSION_INSTALL_FAILED",
                "Ops returned no recovery command; the former installation was retained.",
                1,
            )
        })?;
        human.push(command.into());
    }
    Ok(complete)
}
