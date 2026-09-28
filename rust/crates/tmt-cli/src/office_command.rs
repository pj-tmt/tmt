//! Optional Office composition; no application database, implicit service or PATH dispatch.

use crate::{
    invocation::{OfficeOperation, OutputMode},
    output::Failure,
};
use serde_json::json;
use std::{
    fs,
    io::{self, BufRead, IsTerminal, Read, Write},
    path::{Path, PathBuf},
};
use tmt_adapters::{
    config::ConfigPaths,
    native_install::{self, InstallRequest, Product, UpgradeRequest},
    office_companion::probe_office_companion,
    office_service::{self, ServiceError},
    skill_installation::{self, ProviderEnvironment},
};
use tmt_core::native_install::{Channel, PinAction};

const INSTALL_HINT: &str = "Install Office with: tmt office install --yes";

fn consent(yes: bool, mode: OutputMode, action: &str) -> Result<bool, Failure> {
    if yes {
        return Ok(true);
    }
    if mode.json || !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(Failure::new(
            "OFFICE_CONSENT_REQUIRED",
            format!("{action} requires explicit --yes; no changes were made."),
            1,
        ));
    }
    let mut stderr = io::stderr().lock();
    write!(stderr, "{action}? [y/N] ")
        .and_then(|()| stderr.flush())
        .map_err(|e| failure("OFFICE_IO_ERROR", e))?;
    let mut answer = Vec::new();
    io::stdin()
        .lock()
        .take(256)
        .read_until(b'\n', &mut answer)
        .map_err(|e| failure("OFFICE_IO_ERROR", e))?;
    Ok(matches!(
        std::str::from_utf8(&answer)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "y" | "yes"
    ))
}

fn failure(code: &'static str, error: impl std::error::Error + 'static) -> Failure {
    Failure::new(code, error.to_string(), 1).caused_by(error)
}

fn report(value: serde_json::Value, human: &str, mode: OutputMode) -> io::Result<u8> {
    writeln!(
        io::stdout().lock(),
        "{}",
        if mode.json {
            value.to_string()
        } else {
            human.into()
        }
    )?;
    Ok(0)
}

fn service_failure(error: ServiceError) -> Failure {
    let code = match error {
        ServiceError::PortUnavailable => "OFFICE_PORT_UNAVAILABLE",
        ServiceError::Conflict => "OFFICE_SERVICE_CONFLICT",
        ServiceError::RestartRequired => "OFFICE_RESTART_REQUIRED",
        ServiceError::Uncertain => "OFFICE_SERVICE_UNCERTAIN",
        ServiceError::Unavailable(_) => "OFFICE_SERVICE_UNAVAILABLE",
    };
    Failure::new(code, error.to_string(), 1).caused_by(error)
}

fn installed(executable: &Path) -> Result<bool, Failure> {
    match fs::symlink_metadata(executable) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // A missing command link is recoverable partial removal, not proof
            // that the owned activation disappeared.
            let prefix = executable
                .parent()
                .and_then(Path::parent)
                .expect("Office command prefix");
            match fs::symlink_metadata(prefix.join(Product::Office.namespace()).join("current")) {
                Ok(_) => Err(Failure::new(
                    "OFFICE_INSTALLATION_INVALID",
                    "Office has an activation but no command link. Finish removal with office uninstall --yes, then reinstall.",
                    1,
                )),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(failure("OFFICE_INSTALLATION_INVALID", e)),
            }
        }
        Err(e) => Err(failure("OFFICE_INSTALLATION_INVALID", e)),
    }
}

fn install(
    prefix: &Path,
    archive: Option<&str>,
    manifest: Option<&str>,
    channel: Channel,
) -> Result<native_install::InstallReport, Failure> {
    let interrupt = tmt_adapters::interrupt::Interrupt::install()
        .map_err(|e| failure("OFFICE_INSTALL_FAILED", e))?;
    let checkpoint = || {
        if interrupt.is_interrupted() {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Office installation interrupted. Inspect office status before retrying.",
            ))
        } else {
            Ok(())
        }
    };
    let target =
        tmt_core::native_install::native_target(std::env::consts::OS, std::env::consts::ARCH)
            .ok_or_else(|| {
                Failure::new(
                    "OFFICE_UNSUPPORTED",
                    "No Office artifact supports this platform.",
                    1,
                )
            })?;
    let result = match (archive, manifest) {
        (Some(archive), Some(manifest)) => native_install::install_product(
            Product::Office,
            InstallRequest {
                archive: Path::new(archive),
                manifest: Path::new(manifest),
                prefix,
                target,
                channel,
                pin: PinAction::Preserve,
            },
            checkpoint,
        ),
        (None, None) => {
            let executable = prefix.join("bin/tmt-office");
            if installed(&executable)? {
                native_install::upgrade_product(
                    Product::Office,
                    UpgradeRequest {
                        executable: &executable,
                        channel: Some(channel),
                        exact: None,
                        unpin: false,
                    },
                    checkpoint,
                )
                .map(|report| report.installation)
                .map_err(|error| io::Error::new(error.kind(), error))
            } else {
                native_install::install_release(
                    Product::Office,
                    prefix,
                    target,
                    channel,
                    checkpoint,
                )
            }
        }
        _ => {
            return Err(Failure::new(
                "USAGE_ERROR",
                "Archive and manifest must be supplied together.",
                1,
            ));
        }
    };
    result.map_err(|e| {
        Failure::new(
            if archive.is_none() && e.kind() == io::ErrorKind::NotFound {
                "OFFICE_RELEASE_UNAVAILABLE"
            } else {
                "OFFICE_INSTALL_FAILED"
            },
            format!("{e} Inspect office status before retrying."),
            if e.kind() == io::ErrorKind::Interrupted {
                130
            } else {
                1
            },
        )
        .caused_by(e)
    })
}

fn install_guidance(
    force: bool,
    version: &impl std::fmt::Display,
) -> Result<skill_installation::InstallReport, GuidanceFailure> {
    let environment = ProviderEnvironment::capture().map_err(|error| {
        GuidanceFailure {
            report: skill_installation::InstallReport::default(),
            pending_backup: None,
            error: Box::new(
                Failure::new(
                    "OFFICE_SKILLS_FAILED",
                    format!(
                        "Office {version} is active, but optional skill installation could not inspect provider paths: {error}"
                    ),
                    1,
                )
                .caused_by(error),
            ),
        }
    })?;
    let paths = ConfigPaths::discover().map_err(|error| {
        GuidanceFailure {
            report: skill_installation::InstallReport::default(),
            pending_backup: None,
            error: Box::new(
                Failure::new(
                    "OFFICE_SKILLS_FAILED",
                    format!(
                        "Office {version} is active, but optional skill installation could not resolve managed paths: {error}"
                    ),
                    1,
                )
                .caused_by(error),
            ),
        }
    })?;
    match skill_installation::install_office(&environment, &paths.global_dir, force) {
        Ok(report) => Ok(report),
        Err(mut error) => {
            let detail = error.to_string();
            let report = std::mem::take(&mut error.report);
            let pending_backup = error.pending_backup.take();
            Err(GuidanceFailure {
                report,
                pending_backup,
                error: Box::new(
                    Failure::new(
                        "OFFICE_SKILLS_FAILED",
                        format!(
                            "Office {version} is active, but optional skill installation failed; user-owned content was preserved. {detail}"
                        ),
                        1,
                    )
                    .caused_by(error),
                ),
            })
        }
    }
}

struct GuidanceFailure {
    report: skill_installation::InstallReport,
    pending_backup: Option<PathBuf>,
    error: Box<Failure>,
}

fn guidance_document(
    report: &skill_installation::InstallReport,
    pending_backup: Option<&Path>,
) -> serde_json::Value {
    let mut value = crate::install_command::report_document(report);
    if let Some(backup) = pending_backup {
        value["pendingBackup"] = json!(backup);
    }
    value
}

fn write_guidance_human(
    report: &skill_installation::InstallReport,
    pending_backup: Option<&Path>,
    output: &mut impl Write,
) -> io::Result<()> {
    crate::install_command::write_report_human(report, output)?;
    if let Some(backup) = pending_backup {
        writeln!(
            output,
            "Failed target's recoverable backup: {}",
            backup.display()
        )?;
    }
    Ok(())
}

fn report_guidance(
    mut value: serde_json::Value,
    skills: &skill_installation::InstallReport,
    human: &str,
    mode: OutputMode,
) -> Result<u8, Failure> {
    value["skills"] = guidance_document(skills, None);
    let mut output = io::stdout().lock();
    if mode.json {
        writeln!(output, "{value}").map_err(|io_error| failure("OFFICE_IO_ERROR", io_error))?;
    } else {
        writeln!(output, "{human}").map_err(|io_error| failure("OFFICE_IO_ERROR", io_error))?;
        write_guidance_human(skills, None, &mut output)
            .map_err(|io_error| failure("OFFICE_IO_ERROR", io_error))?;
    }
    Ok(0)
}

fn report_partial(
    mut value: serde_json::Value,
    guidance: GuidanceFailure,
    human: &str,
    mode: OutputMode,
) -> Result<u8, Failure> {
    value["skills"] = guidance_document(&guidance.report, guidance.pending_backup.as_deref());
    value["error"] = guidance.error.document()["error"].clone();
    if mode.json {
        writeln!(io::stdout().lock(), "{value}")
            .map_err(|io_error| failure("OFFICE_IO_ERROR", io_error))?;
    } else {
        let mut output = io::stdout().lock();
        writeln!(output, "{human}").map_err(|io_error| failure("OFFICE_IO_ERROR", io_error))?;
        write_guidance_human(
            &guidance.report,
            guidance.pending_backup.as_deref(),
            &mut output,
        )
        .map_err(|io_error| failure("OFFICE_IO_ERROR", io_error))?;
        drop(output);
        guidance
            .error
            .publish(mode)
            .map_err(|io_error| failure("OFFICE_IO_ERROR", io_error))?;
    }
    Ok(guidance.error.status)
}

pub fn execute(
    prefix: Option<String>,
    operation: OfficeOperation,
    mode: OutputMode,
) -> io::Result<u8> {
    match run(prefix, operation, mode) {
        Ok(code) => Ok(code),
        Err(error) => error.publish(mode),
    }
}

fn run(
    prefix: Option<String>,
    operation: OfficeOperation,
    mode: OutputMode,
) -> Result<u8, Failure> {
    let core_access = crate::office_core_access::InProcessCoreAccess;
    let prefix = prefix
        .map(PathBuf::from)
        .map_or_else(native_install::default_install_prefix, Ok)
        .map_err(|e| failure("OFFICE_LOCATION_INVALID", e))?;
    if prefix.as_os_str().is_empty() {
        return Err(Failure::new(
            "OFFICE_LOCATION_INVALID",
            "Installation prefix must not be empty.",
            1,
        ));
    }
    let executable = prefix.join("bin/tmt-office");
    match operation {
        OfficeOperation::Start { port } => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            let version = probe_office_companion(&executable)
                .map_err(|error| failure("OFFICE_INCOMPATIBLE", error))?;
            let paths = ConfigPaths::discover()
                .map_err(|error| failure("OFFICE_LOCATION_INVALID", error))?;
            let started = office_service::start(&paths, &executable, &version.to_string(), port)
                .map_err(service_failure)?;
            let url = started.receipt.session_url();
            report(
                json!({"running":true,"url":url,"changed":started.changed,"reused":started.reused,"version":version}),
                &url,
                mode,
            )
            .map_err(|error| failure("OFFICE_IO_ERROR", error))
            .inspect(|_| {
                if started.changed {
                    crate::skill_reminder::present(
                        crate::skill_reminder::Outcome::OfficeStarted,
                        mode,
                        false,
                    );
                }
            })
        }
        OfficeOperation::Stop => {
            let paths = ConfigPaths::discover()
                .map_err(|error| failure("OFFICE_LOCATION_INVALID", error))?;
            let changed = office_service::stop(&paths).map_err(service_failure)?;
            report(
                json!({"running":false,"changed":changed}),
                if changed {
                    "Local Office stopped."
                } else {
                    "Local Office was not running."
                },
                mode,
            )
            .map_err(|error| failure("OFFICE_IO_ERROR", error))
        }
        OfficeOperation::Pair { .. }
        | OfficeOperation::PairStatus { .. }
        | OfficeOperation::Unpair { .. }
        | OfficeOperation::Inspect { .. }
        | OfficeOperation::Sync => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            crate::office_pairing_command::run(&executable, operation, mode, &core_access)
        }
        OfficeOperation::Layout(operation) => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            crate::office_layout_command::run(&executable, operation, mode)
        }
        OfficeOperation::Block { .. } => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            crate::office_block_command::run(&executable, operation, mode, &core_access)
        }
        OfficeOperation::Profile { .. } => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            crate::office_profile_command::run(&executable, operation, mode, &core_access)
        }
        OfficeOperation::Prop(_) => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            crate::office_prop_command::run(&executable, operation, mode)
        }
        OfficeOperation::Avatar(_) => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            crate::office_avatar_command::run(&executable, operation, mode)
        }
        OfficeOperation::ExtensionValidate { file, instance } => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            crate::office_extension_command::run(&executable, &file, &instance, mode)
        }
        OfficeOperation::Board(_) => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            crate::office_board_command::run(&executable, operation, mode, &core_access)
        }
        OfficeOperation::WhiteboardSnapshot { reference, output } => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            crate::office_whiteboard_command::run(&executable, &reference, output.as_deref(), mode)
        }
        OfficeOperation::Status | OfficeOperation::Open => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            let interrupt = tmt_adapters::interrupt::Interrupt::install()
                .map_err(|e| failure("OFFICE_IO_ERROR", e))?;
            let result = probe_office_companion(&executable);
            if interrupt.is_interrupted() {
                return Err(Failure::new(
                    "OFFICE_INTERRUPTED",
                    "Office inspection interrupted; no connection was started.",
                    130,
                ));
            }
            let version = result.map_err(|e| failure("OFFICE_INCOMPATIBLE", e))?;
            let paths = ConfigPaths::discover()
                .map_err(|error| failure("OFFICE_LOCATION_INVALID", error))?;
            let service =
                office_service::status(&paths, &version.to_string()).map_err(service_failure)?;
            let service_value = service
                .receipt
                .as_ref()
                .map(|receipt| {
                    json!({
                        "running":true,
                        "endpoint":receipt.endpoint(),
                        "port":receipt.port,
                        "restartNeeded":service.restart_needed,
                        "runningVersion":receipt.running_version,
                    })
                })
                .unwrap_or_else(|| json!({"running":false,"restartNeeded":false}));
            let human = if service.running {
                format!(
                    "Office {version} is installed; local service is running{}.",
                    if service.restart_needed {
                        " and needs restart"
                    } else {
                        ""
                    }
                )
            } else {
                format!("Office {version} is installed and compatible. Local service is stopped.")
            };
            let human = if matches!(operation, OfficeOperation::Open) {
                format!(
                    "{human} Use `tmt office start` to get the local browser URL, or `tmt learn --skill tmt-office` for Office guidance."
                )
            } else {
                human
            };
            report(json!({"installed": true, "version": version, "protocolVersion": tmt_office_model::office_protocol::OFFICE_PROTOCOL_VERSION, "executable": executable, "service":service_value}), &human, mode).map_err(|e| failure("OFFICE_IO_ERROR", e))
        }
        OfficeOperation::Install {
            yes,
            force,
            archive,
            manifest,
            channel,
        } => {
            if !consent(yes, mode, "Install the verified optional Office companion")? {
                return Ok(0);
            }
            let current = if installed(&executable)? {
                Some(
                    native_install::inspect_product(Product::Office, &executable)
                        .map_err(|e| failure("OFFICE_INSTALLATION_INVALID", e))?,
                )
            } else {
                None
            };
            let channel = channel
                .or_else(|| current.as_ref().map(|current| current.state.channel))
                .unwrap_or(Channel::Alpha);
            let result = install(&prefix, archive.as_deref(), manifest.as_deref(), channel)?;
            let skills = match install_guidance(force, &result.version) {
                Ok(skills) => skills,
                Err(guidance) => {
                    return report_partial(
                        json!({"installed":true,"changed":result.changed,"version":result.version,"executable":result.executable}),
                        guidance,
                        &format!(
                            "Office {} installed; optional agent guidance needs attention.",
                            result.version
                        ),
                        mode,
                    );
                }
            };
            report_guidance(
                json!({"installed":true,"changed":result.changed,"version":result.version,"executable":result.executable}),
                &skills,
                &format!(
                    "Office {} installed with optional agent guidance. Pairing is separate.",
                    result.version
                ),
                mode,
            )
        }
        OfficeOperation::Upgrade { force, channel } => {
            if !installed(&executable)? {
                return Err(Failure::new("OFFICE_NOT_INSTALLED", INSTALL_HINT, 1));
            }
            let interrupt = tmt_adapters::interrupt::Interrupt::install()
                .map_err(|e| failure("OFFICE_UPGRADE_FAILED", e))?;
            let result = native_install::upgrade_product(
                Product::Office,
                UpgradeRequest {
                    executable: &executable,
                    channel,
                    exact: None,
                    unpin: false,
                },
                || {
                    if interrupt.is_interrupted() {
                        Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Office update interrupted.",
                        ))
                    } else {
                        Ok(())
                    }
                },
            )
            .map_err(|e| {
                Failure::new(
                    "OFFICE_UPGRADE_FAILED",
                    format!("{e} Inspect office status before retrying."),
                    if e.kind() == io::ErrorKind::Interrupted {
                        130
                    } else {
                        1
                    },
                )
                .caused_by(e)
            })?;
            let skills = match install_guidance(force, &result.installation.version) {
                Ok(skills) => skills,
                Err(guidance) => {
                    return report_partial(
                        json!({"installed":true,"changed":result.installation.changed,"version":result.installation.version,"skippedPinned":result.skipped_pinned,"executable":result.installation.executable}),
                        guidance,
                        &format!(
                            "Office {} is current; optional agent guidance needs attention.",
                            result.installation.version
                        ),
                        mode,
                    );
                }
            };
            report_guidance(
                json!({"installed":true,"changed":result.installation.changed,"version":result.installation.version,"skippedPinned":result.skipped_pinned,"executable":result.installation.executable}),
                &skills,
                &format!(
                    "Office {} and optional agent guidance are current.",
                    result.installation.version
                ),
                mode,
            )
        }
        OfficeOperation::Uninstall { yes } => {
            if !consent(
                yes,
                mode,
                "Deactivate Office (retained releases and user data will be kept)",
            )? {
                return Ok(0);
            }
            let changed = native_install::uninstall_office(&prefix)
                .map_err(|e| failure("OFFICE_UNINSTALL_FAILED", e))?;
            report(
                json!({"installed":false,"changed":changed,"retainedReleases":true}),
                "Office is deactivated. Release bytes and user data were retained.",
                mode,
            )
            .map_err(|e| failure("OFFICE_IO_ERROR", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_adapters::skill_installation::{InstallReport, InstalledSkill};

    fn installed(target: &str, backup: Option<&str>) -> InstalledSkill {
        InstalledSkill {
            name: "tmt-office",
            agent: None,
            target: PathBuf::from(target),
            changed: true,
            backup: backup.map(PathBuf::from),
            legacy_backups: Vec::new(),
        }
    }

    #[test]
    fn guidance_renderers_keep_successful_and_partial_recovery_evidence() {
        let forced = InstallReport {
            installed: vec![installed("/skills/tmt-office", Some("/backups/forced"))],
            warnings: Vec::new(),
        };
        assert_eq!(
            guidance_document(&forced, None),
            json!({"installed":[{"skill":"tmt-office","target":"/skills/tmt-office","changed":true,"backup":"/backups/forced"}]})
        );
        let mut human = Vec::new();
        write_guidance_human(&forced, None, &mut human).unwrap();
        let human = String::from_utf8(human).unwrap();
        assert!(human.contains("Installed shared skill 'tmt-office' at /skills/tmt-office"));
        assert!(human.contains("Recoverable backup: /backups/forced"));

        let partial = InstallReport {
            installed: vec![
                installed("/skills/first/tmt-office", None),
                installed("/skills/second/tmt-office", Some("/backups/second")),
            ],
            warnings: Vec::new(),
        };
        assert_eq!(
            guidance_document(&partial, Some(Path::new("/backups/pending"))),
            json!({"installed":[
                {"skill":"tmt-office","target":"/skills/first/tmt-office","changed":true},
                {"skill":"tmt-office","target":"/skills/second/tmt-office","changed":true,"backup":"/backups/second"}
            ],"pendingBackup":"/backups/pending"})
        );
        let mut human = Vec::new();
        write_guidance_human(&partial, Some(Path::new("/backups/pending")), &mut human).unwrap();
        let human = String::from_utf8(human).unwrap();
        assert!(human.contains("/skills/first/tmt-office"));
        assert!(human.contains("Recoverable backup: /backups/second"));
        assert!(human.contains("Failed target's recoverable backup: /backups/pending"));
    }
}
