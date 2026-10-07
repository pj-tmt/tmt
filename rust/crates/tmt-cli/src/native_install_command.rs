//! Internal offline installer composition; no config discovery or skill mutation.

use crate::{invocation::OutputMode, output::Failure};
use std::{
    io::{self, Write},
    path::Path,
};
use tmt_adapters::native_install;
use tmt_core::native_install::{Channel, PinAction};

pub fn schema(source_sha: &str, mode: OutputMode) -> io::Result<u8> {
    match native_install::compiled_application_schema(source_sha) {
        Ok(record) => {
            let mut stdout = tmt_cli_style::stream::stdout(true);
            writeln!(stdout, "{record}")?;
            Ok(0)
        }
        Err(error) => Failure::new("NATIVE_SCHEMA_EXPORT_FAILED", error.to_string(), 1)
            .caused_by(error)
            .publish(mode),
    }
}

pub fn execute(
    product: tmt_core::native_install::Product,
    archive: &str,
    manifest: &str,
    prefix: &str,
    channel: Channel,
    pin: PinAction,
    mode: OutputMode,
) -> io::Result<u8> {
    let target =
        match tmt_core::native_install::native_target(std::env::consts::OS, std::env::consts::ARCH)
        {
            Some(target) => target,
            None => {
                return Failure::new(
                    "NATIVE_INSTALL_UNSUPPORTED",
                    "No native artifact is supported for this platform.",
                    1,
                )
                .publish(mode);
            }
        };
    let interrupt = match tmt_adapters::interrupt::Interrupt::install() {
        Ok(interrupt) => interrupt,
        Err(error) => {
            return Failure::new("NATIVE_INSTALL_FAILED", error.to_string(), 1)
                .caused_by(error)
                .publish(mode);
        }
    };
    let request = native_install::InstallRequest {
        archive: Path::new(archive),
        manifest: Path::new(manifest),
        prefix: Path::new(prefix),
        target,
        channel,
        pin,
    };
    let verifier = crate::office_facade::release_verifier(product);
    let report = match native_install::install_product(product, request, verifier, || {
        if interrupt.is_interrupted() {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Native installation interrupted before activation.",
            ))
        } else {
            Ok(())
        }
    }) {
        Ok(report) => report,
        Err(error) => {
            let exit = if error.kind() == io::ErrorKind::Interrupted {
                130
            } else {
                1
            };
            return Failure::new("NATIVE_INSTALL_FAILED", error.to_string(), exit)
                .caused_by(error)
                .publish(mode);
        }
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    let terminal = stdout.terminal();
    if mode.json {
        writeln!(
            stdout,
            "{}",
            serde_json::json!({"executable": report.executable, "version": report.version, "changed": report.changed})
        )?;
    } else {
        let what = format!(
            "{} {} at {}",
            product.executable(),
            report.version,
            Path::new(prefix)
                .join("bin")
                .join(product.executable())
                .display()
        );
        if report.changed {
            tmt_cli_style::message::success(&mut stdout, terminal, &format!("Installed {what}"))?;
        } else {
            writeln!(stdout, "Current {what}")?;
        }
    }
    Ok(0)
}

/// The version is admitted by the typed grammar before reading input or writing.
pub fn handoff(version: u32, probe: bool, mode: OutputMode) -> io::Result<u8> {
    if !mode.json {
        return Failure::new(
            "NATIVE_INSTALL_FAILED",
            "Installer handoff requires --json.",
            1,
        )
        .publish(mode);
    }
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if probe {
        writeln!(
            stdout,
            "{}",
            native_install::handoff::probe_version(version)
        )?;
        return Ok(0);
    }
    let input = match tmt_adapters::response_input::read_stdin_bounded(
        std::time::Duration::from_secs(5),
        native_install::handoff::input_limit(version),
    ) {
        Ok(input) => input,
        Err(error) => {
            return Failure::new("NATIVE_INSTALL_FAILED", error.to_string(), 1).publish(mode);
        }
    };
    let interrupt = tmt_adapters::interrupt::Interrupt::install()?;
    let (report, failed) =
        native_install::handoff::install_version(version, input.as_bytes(), || {
            if interrupt.is_interrupted() {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Native installation interrupted before activation.",
                ))
            } else {
                Ok(())
            }
        });
    writeln!(stdout, "{report}")?;
    Ok(u8::from(failed))
}
