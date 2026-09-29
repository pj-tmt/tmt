//! Internal offline installer composition; no config discovery or skill mutation.

use crate::{invocation::OutputMode, output::Failure};
use std::{
    io::{self, Write},
    path::Path,
};
use tmt_adapters::native_install;
use tmt_core::native_install::{Channel, PinAction};

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
            report.executable.display()
        );
        if report.changed {
            tmt_cli_style::message::success(&mut stdout, terminal, &format!("Installed {what}"))?;
        } else {
            writeln!(stdout, "Current {what}")?;
        }
    }
    Ok(0)
}
