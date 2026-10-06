//! Versioned CLI installer delegation. The verified candidate owns publication.

use super::{
    ActivatedInstallation, InstallReport, ManagedInstallation, artifact, invalid,
    receipt::{GitHubProvenance, Provenance},
    release::DownloadedRelease,
};
use crate::{
    bounded_file,
    process::{CommandError, CommandFailure, CommandRequest, CommandRunner},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    time::{Duration, Instant},
};
use tmt_core::native_install::{Channel, PinAction};
use uuid::Uuid;

pub const VERSION: u32 = 1;
pub const LIMIT: usize = 16 * 1024;
pub const PR_VERSION: u32 = 2;
pub const PR_LIMIT: usize = 64 * 1024;
pub const UNSUPPORTED: &str = "This release needs a newer installer: rerun install.sh with: curl -fsSL https://github.com/pj-tmt/tmt/releases/latest/download/install.sh | sh";

#[derive(Debug)]
pub(super) struct Unsupported(Option<CommandError>);
impl std::fmt::Display for Unsupported {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(UNSUPPORTED)
    }
}
impl std::error::Error for Unsupported {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.as_ref().map(|error| error as _)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: u32,
    archive: PathBuf,
    manifest: PathBuf,
    prefix: PathBuf,
    target: String,
    version: String,
    archive_sha256: String,
    manifest_sha256: String,
    channel: String,
    pin: String,
    expected_current: String,
    release_id: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    protocol: u32,
    installation: Option<InstallReport>,
    error: Option<String>,
}

/// Protocol 1 remains frozen. Protocol 2 wraps the same local byte transfer
/// with acquisition provenance and explicit channel intent, without teaching
/// an older executable to interpret unknown evidence.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrRequest {
    protocol: u32,
    transfer: Request,
    provenance: Provenance,
    explicit_channel: bool,
}

pub fn probe() -> serde_json::Value {
    serde_json::json!({"protocol": VERSION})
}
pub fn probe_version(version: u32) -> serde_json::Value {
    serde_json::json!({"protocol": version})
}
pub fn input_limit(version: u32) -> usize {
    if version == PR_VERSION {
        PR_LIMIT
    } else {
        LIMIT
    }
}

/// Only the candidate calls this. Revalidate local bytes and the complete
/// inventory before entering the existing publication/receipt owner.
pub fn install(
    input: &[u8],
    checkpoint: impl FnMut() -> io::Result<()>,
) -> (serde_json::Value, bool) {
    install_version(VERSION, input, checkpoint)
}

pub fn install_version(
    version: u32,
    input: &[u8],
    checkpoint: impl FnMut() -> io::Result<()>,
) -> (serde_json::Value, bool) {
    let result = (|| {
        if input.len() > input_limit(version) {
            return Err(invalid("Installer handoff exceeds its bound."));
        }
        let (request, provenance, explicit_channel) = match version {
            VERSION => {
                let request: Request = super::pr_json::parse(input, LIMIT)?;
                let provenance = GitHubProvenance {
                    release_id: request.release_id,
                    manifest_sha256: request.manifest_sha256.clone(),
                }
                .into();
                (request, provenance, true)
            }
            PR_VERSION => {
                let request: PrRequest = super::pr_json::parse(input, PR_LIMIT)?;
                if request.protocol != PR_VERSION {
                    return Err(invalid("Unsupported PR installer handoff."));
                }
                (
                    request.transfer,
                    request.provenance,
                    request.explicit_channel,
                )
            }
            _ => return Err(invalid("Unsupported native installer handoff.")),
        };
        if request.protocol != VERSION
            || (version == VERSION && request.release_id == 0)
            || provenance.manifest_sha256() != request.manifest_sha256
            || provenance.release().is_some_and(|release| {
                release.release_id == 0 || release.release_id != request.release_id
            })
            || (matches!(provenance, Provenance::Pr(_)) && request.release_id != 0)
        {
            return Err(invalid("Unsupported native installer handoff."));
        }
        if !request.archive.is_absolute()
            || !request.manifest.is_absolute()
            || !request.prefix.is_absolute()
        {
            return Err(invalid("Installer handoff paths must be absolute."));
        }
        let target =
            tmt_core::native_install::native_target(std::env::consts::OS, std::env::consts::ARCH)
                .ok_or_else(|| invalid("Unsupported native installer target."))?;
        if request.target != target {
            return Err(invalid("Installer handoff target mismatch."));
        }
        let channel = Channel::parse(&request.channel)
            .ok_or_else(|| invalid("Invalid installer channel."))?;
        let pin = match request.pin.as_str() {
            "preserve" => PinAction::Preserve,
            "pin" => PinAction::PinCandidate,
            "clear" => PinAction::Clear,
            _ => return Err(invalid("Invalid installer pin action.")),
        };
        let manifest = bounded_file::read_no_follow(&request.manifest, artifact::MANIFEST_LIMIT)
            .map_err(io::Error::other)?;
        if artifact::digest(&manifest) != request.manifest_sha256 {
            return Err(invalid("Installer handoff manifest checksum mismatch."));
        }
        let archive = bounded_file::read_no_follow(&request.archive, artifact::COMPRESSED_LIMIT)
            .map_err(io::Error::other)?;
        let archive_name = request
            .archive
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("Invalid handoff archive filename."))?;
        let artifact = artifact::acquire_bytes(
            super::Product::Cli,
            &manifest,
            archive_name,
            &archive,
            target,
        )?;
        if artifact.version.to_string() != request.version
            || artifact.sha256 != request.archive_sha256
        {
            return Err(invalid("Installer handoff release mismatch."));
        }
        if version == PR_VERSION {
            let schema = super::manifest_application_schema(super::Product::Cli, &manifest)?;
            if serde_json::to_value(&schema).map_err(io::Error::other)?
                != super::compiled_application_schema(&schema.source_sha)?
            {
                return Err(invalid(
                    "Candidate manifest schema does not match this compiled binary and source closure.",
                ));
            }
            if let Provenance::Pr(proof) = &provenance {
                proof.verify_manifest(&manifest)?;
            } else {
                // A return to a published channel cannot downgrade local data.
                let local = super::local_application_schema(super::Product::Cli)?;
                super::pr_receipt::admit_databases(
                    super::Product::Cli,
                    &schema,
                    &schema,
                    &local,
                    false,
                )?;
            }
        }
        super::activate(
            super::ActivationRequest {
                product: super::Product::Cli,
                prefix: &request.prefix,
                channel,
                pin,
                expected: Some(
                    Uuid::parse_str(&request.expected_current).map_err(io::Error::other)?,
                ),
                provenance: Some(provenance),
                verifier: None,
                explicit_channel,
                schema: if version == PR_VERSION {
                    Some(super::manifest_application_schema(
                        super::Product::Cli,
                        &manifest,
                    )?)
                } else {
                    None
                },
            },
            &artifact,
            checkpoint,
        )
    })();
    let (installation, error) = match result {
        Ok(report) => (Some(report), None),
        Err(error) => {
            let activated = error
                .get_ref()
                .and_then(|cause| cause.downcast_ref::<ActivatedInstallation>())
                .map(|cause| cause.report.clone());
            (activated, Some(error.to_string()))
        }
    };
    let failed = error.is_some();
    (
        serde_json::to_value(Response {
            protocol: version,
            installation,
            error,
        })
        .expect("installer report"),
        failed,
    )
}

struct Staging(PathBuf);
impl Staging {
    fn create() -> io::Result<Self> {
        let root = std::env::temp_dir().join(format!("tmt-upgrade-{}", Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        Ok(Self(root.canonicalize()?))
    }
    fn write(&self, name: &str, bytes: &[u8], mode: u32) -> io::Result<PathBuf> {
        let path = self.0.join(name);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(path)
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn upgrade(
    current: &ManagedInstallation,
    downloaded: &DownloadedRelease,
    channel: Channel,
    pin: PinAction,
    mut checkpoint: impl FnMut() -> io::Result<()>,
    runner: &impl CommandRunner,
) -> io::Result<InstallReport> {
    let version = if matches!(downloaded.provenance, Provenance::Pr(_))
        || matches!(current.state.channel, Channel::Pr(_))
    {
        PR_VERSION
    } else {
        VERSION
    };
    let limit = input_limit(version);
    let artifact = artifact::acquire_candidate(
        &downloaded.manifest,
        &downloaded.archive_name,
        &downloaded.archive,
        &current.target,
    )?;
    if artifact.version != downloaded.version
        || artifact::digest(&downloaded.manifest) != downloaded.provenance.manifest_sha256()
    {
        return Err(invalid(
            "Candidate does not match the verified release metadata.",
        ));
    }
    let staging = Staging::create()?;
    let result = (|| {
        // The transport decoder validated every entry before creating this private tree.
        let extracted = staging.0.join("candidate");
        fs::DirBuilder::new().mode(0o700).create(&extracted)?;
        for (name, bytes) in &artifact.files {
            let relative = format!("candidate/{name}");
            let parent = staging
                .0
                .join(&relative)
                .parent()
                .expect("candidate parent")
                .to_path_buf();
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
            let mode = if artifact.executable_files.contains(name) {
                0o700
            } else {
                0o600
            };
            staging.write(&relative, bytes, mode)?;
        }
        let candidate = extracted.join("tmt");
        let archive = staging.write(&downloaded.archive_name, &downloaded.archive, 0o600)?;
        let manifest = staging.write("dist-manifest.json", &downloaded.manifest, 0o600)?;
        checkpoint()?;
        let probe = runner
            .execute(CommandRequest {
                program: candidate.as_os_str(),
                args: &[
                    "__native-install".into(),
                    "--handoff-version".into(),
                    version.to_string().into(),
                    "--probe".into(),
                    "--json".into(),
                ],
                input: &[],
                deadline: Instant::now() + Duration::from_secs(10),
                max_output_bytes: limit,
            })
            .map_err(|error| io::Error::other(Unsupported(Some(error))))?;
        if !probe.stderr.is_empty()
            || serde_json::from_slice::<serde_json::Value>(&probe.stdout).ok()
                != Some(probe_version(version))
        {
            return Err(io::Error::other(Unsupported(None)));
        }
        checkpoint()?;
        let transfer = Request {
            protocol: VERSION,
            archive,
            manifest,
            prefix: current.prefix.clone(),
            target: current.target.clone(),
            version: downloaded.version.to_string(),
            archive_sha256: artifact.sha256,
            manifest_sha256: downloaded.provenance.manifest_sha256().into(),
            channel: channel.as_str().into(),
            pin: match pin {
                PinAction::Preserve => "preserve",
                PinAction::PinCandidate => "pin",
                PinAction::Clear => "clear",
            }
            .into(),
            expected_current: current.id.to_string(),
            release_id: downloaded
                .provenance
                .release()
                .map_or(0, |proof| proof.release_id),
        };
        let input = if version == PR_VERSION {
            serde_json::to_vec(&PrRequest {
                protocol: version,
                transfer,
                provenance: downloaded.provenance.clone(),
                explicit_channel: downloaded.explicit_channel,
            })
        } else {
            serde_json::to_vec(&transfer)
        }
        .map_err(io::Error::other)?;
        if input.len() > limit {
            return Err(invalid("Installer handoff exceeds its bound."));
        }
        let result = runner.execute(CommandRequest {
            program: candidate.as_os_str(),
            args: &[
                "__native-install".into(),
                "--handoff-version".into(),
                version.to_string().into(),
                "--json".into(),
            ],
            input: &input,
            deadline: Instant::now() + Duration::from_secs(60),
            max_output_bytes: limit,
        });
        let (output, failed) = match result {
            Ok(output) => (output, false),
            Err(mut error) => {
                if !matches!(
                    error.kind,
                    CommandFailure::Exit {
                        code: Some(1),
                        signal: None
                    }
                ) || error.cleanup_failed()
                {
                    return Err(io::Error::other(format!(
                        "Candidate installer did not return a completed report; inspect the active installation before retrying: {error}"
                    )));
                }
                let output = error.output.take().ok_or_else(|| io::Error::other(error))?;
                (output, true)
            }
        };
        let response = parse_response(&output.stdout)?;
        if !output.stderr.is_empty()
            || response.protocol != version
            || failed != response.error.is_some()
        {
            return Err(invalid(
                "Invalid candidate installer report; inspect the active installation before retrying.",
            ));
        }
        if let Some(report) = &response.installation {
            validate_report(current, downloaded, report)?;
        }
        match (response.installation, response.error) {
            (Some(report), None) => Ok(report),
            (Some(report), Some(error)) => Err(io::Error::other(ActivatedInstallation {
                report,
                cause: io::Error::other(error),
            })),
            (None, Some(error)) => Err(io::Error::other(error)),
            _ => Err(invalid(
                "Candidate installer omitted its installation report.",
            )),
        }
    })();
    if let Err(cleanup) = fs::remove_dir_all(&staging.0) {
        return Err(match result {
            Ok(report) => io::Error::other(ActivatedInstallation {
                report,
                cause: io::Error::other(format!(
                    "Native installation completed, but owned staging cleanup failed: {cleanup}"
                )),
            }),
            Err(error) => {
                let message = format!("{error}; owned staging cleanup failed: {cleanup}");
                if let Some(active) = error
                    .get_ref()
                    .and_then(|cause| cause.downcast_ref::<ActivatedInstallation>())
                {
                    io::Error::new(
                        error.kind(),
                        ActivatedInstallation {
                            report: active.report.clone(),
                            cause: io::Error::other(message),
                        },
                    )
                } else {
                    io::Error::new(error.kind(), message)
                }
            }
        });
    }
    result
}

fn parse_response(bytes: &[u8]) -> io::Result<Response> {
    let invalid_report = || {
        invalid(
            "Invalid candidate installer report; inspect the active installation before retrying.",
        )
    };
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid_report())?;
    if value.as_object().is_none_or(|fields| fields.len() != 3) {
        return Err(invalid_report());
    }
    serde_json::from_value(value).map_err(|_| invalid_report())
}

fn validate_report(
    current: &ManagedInstallation,
    downloaded: &DownloadedRelease,
    report: &InstallReport,
) -> io::Result<()> {
    let releases = current
        .prefix
        .join(super::Product::Cli.namespace())
        .join("releases");
    let parent = report
        .active_executable
        .parent()
        .ok_or_else(|| invalid("Invalid candidate installation path."))?;
    if report.version != downloaded.version.to_string()
        || report.executable != current.executable
        || report.active_executable.file_name() != Some(std::ffi::OsStr::new("tmt"))
        || parent.parent() != Some(releases.as_path())
        || parent
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .is_none()
    {
        return Err(invalid(
            "Candidate installation report does not match the selected release.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
