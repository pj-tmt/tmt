//! `tmt extension install|upgrade|uninstall|list`: consented installation of
//! the official extensions over the native installer. The product table is
//! fixed; nothing here discovers an extension from archive data or PATH.

use crate::{
    consent,
    invocation::{ExtensionInstallRequest, OutputMode},
    output::Failure,
};
use serde_json::{Value, json};
use std::{
    env, fs, io,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tmt_adapters::{
    interrupt::Interrupt,
    native_install::{self, InstallRequest, Product, UpgradeRequest},
};
use tmt_core::native_install::{Channel, PinAction};

const CONSENT: &str = "EXTENSION_CONSENT_REQUIRED";

/// Consent for one change to the user's installation.
fn ask(yes: bool, mode: OutputMode, action: &str) -> Result<bool, Failure> {
    consent::ask(
        &mut io::stdout().lock(),
        yes,
        mode,
        consent::Consent {
            code: CONSENT,
            refusal: &format!("{action} requires explicit --yes; no changes were made."),
            question: action,
        },
        |error| {
            Failure::new("EXTENSION_IO_ERROR", "Could not ask for consent.", 1).caused_by(error)
        },
    )
}

pub fn execute(request: ExtensionInstallRequest, mode: OutputMode) -> io::Result<u8> {
    match run(request, mode) {
        Ok(Some((document, human))) => {
            let mut stdout = io::stdout().lock();
            if mode.json {
                writeln!(stdout, "{document}")?;
            } else {
                writeln!(stdout, "{human}")?;
            }
            Ok(0)
        }
        Ok(None) => Ok(0),
        Err(error) => error.publish(mode),
    }
}

/// The official extensions, by the name users type.
fn extension(name: &str) -> Result<Product, Failure> {
    Product::ALL
        .into_iter()
        .filter(|product| *product != Product::Cli)
        .find(|product| product.as_str() == name)
        .ok_or_else(|| {
            Failure::new(
                "EXTENSION_UNKNOWN",
                format!(
                    "Unknown extension '{name}'. Official extensions: {}.",
                    names()
                ),
                1,
            )
        })
}

fn names() -> String {
    Product::ALL
        .into_iter()
        .filter(|product| *product != Product::Cli)
        .map(Product::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn prefix(prefix: Option<&str>) -> Result<PathBuf, Failure> {
    match prefix {
        Some(prefix) => Ok(PathBuf::from(prefix)),
        None => native_install::default_install_prefix().map_err(|error| {
            Failure::new("EXTENSION_INSTALL_FAILED", error.to_string(), 1).caused_by(error)
        }),
    }
}

fn failure(code: &'static str, error: io::Error) -> Failure {
    Failure::new(
        code,
        format!("{error} Inspect with: tmt extension list"),
        if error.kind() == io::ErrorKind::Interrupted {
            130
        } else {
            1
        },
    )
    .caused_by(error)
}

/// A missing command link beside a retained activation is recoverable
/// partial removal, not proof that the extension is gone.
fn installed(product: Product, prefix: &Path) -> Result<bool, Failure> {
    let executable = prefix.join("bin").join(product.executable());
    match fs::symlink_metadata(&executable) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match fs::symlink_metadata(prefix.join(product.namespace()).join("current")) {
                Ok(_) => Err(Failure::new(
                    "EXTENSION_INSTALLATION_INVALID",
                    format!(
                        "{} has an activation but no command link. Finish removal with: tmt extension uninstall {} --yes",
                        product.as_str(),
                        product.as_str()
                    ),
                    1,
                )),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(failure("EXTENSION_INSTALLATION_INVALID", error)),
            }
        }
        Err(error) => Err(failure("EXTENSION_INSTALLATION_INVALID", error)),
    }
}

fn interruptible(message: &'static str) -> Result<impl FnMut() -> io::Result<()>, Failure> {
    let interrupt = Interrupt::install().map_err(|error| {
        Failure::new("EXTENSION_INSTALL_FAILED", error.to_string(), 1).caused_by(error)
    })?;
    Ok(move || {
        if interrupt.is_interrupted() {
            Err(io::Error::new(io::ErrorKind::Interrupted, message))
        } else {
            Ok(())
        }
    })
}

type Outcome = Option<(Value, String)>;

fn run(request: ExtensionInstallRequest, mode: OutputMode) -> Result<Outcome, Failure> {
    match request {
        ExtensionInstallRequest::Install {
            name,
            prefix: selected,
            channel,
            archive,
            manifest,
            yes,
        } => {
            let product = extension(&name)?;
            let prefix = prefix(selected.as_deref())?;
            if !ask(yes, mode, &format!("Install the verified {name} extension"))? {
                return Ok(None);
            }
            install(
                product,
                &prefix,
                channel,
                archive.as_deref(),
                manifest.as_deref(),
            )
        }
        ExtensionInstallRequest::Upgrade {
            name,
            prefix: selected,
            channel,
            to,
            unpin,
            yes,
        } => {
            let product = extension(&name)?;
            let prefix = prefix(selected.as_deref())?;
            if !installed(product, &prefix)? {
                return Err(Failure::new(
                    "EXTENSION_NOT_INSTALLED",
                    format!(
                        "{name} is not installed. Install it with: tmt extension install {name}"
                    ),
                    1,
                ));
            }
            if !ask(yes, mode, &format!("Update the {name} extension"))? {
                return Ok(None);
            }
            let executable = prefix.join("bin").join(product.executable());
            let report = native_install::upgrade_product(
                product,
                UpgradeRequest {
                    executable: &executable,
                    channel,
                    exact: to.as_deref(),
                    unpin,
                },
                crate::office_facade::release_verifier(product),
                interruptible("Extension update interrupted before activation.")?,
            )
            .map_err(|error| {
                failure(
                    "EXTENSION_UPGRADE_FAILED",
                    io::Error::new(error.kind(), error),
                )
            })?;
            let version = report.installation.version.clone();
            let human = if report.skipped_pinned {
                format!(
                    "{name} {version} is pinned; nothing changed. Clear the pin with: tmt extension upgrade {name} --unpin"
                )
            } else if report.installation.changed {
                format!("Updated {name} to {version}.")
            } else {
                format!("{name} {version} is current.")
            };
            Ok(Some((
                json!({"extension": name, "installed": true, "changed": report.installation.changed,
                    "version": version, "skippedPinned": report.skipped_pinned,
                    "executable": report.installation.executable}),
                human,
            )))
        }
        ExtensionInstallRequest::Uninstall {
            name,
            prefix: selected,
            yes,
        } => {
            let product = extension(&name)?;
            let prefix = prefix(selected.as_deref())?;
            if !ask(
                yes,
                mode,
                &format!("Remove the {name} extension's commands (releases and data are kept)"),
            )? {
                return Ok(None);
            }
            let changed = native_install::uninstall_extension(&prefix, product)
                .map_err(|error| failure("EXTENSION_UNINSTALL_FAILED", error))?;
            let kept = kept(product, &prefix);
            let human = std::iter::once(if changed {
                format!("Removed the {name} extension's commands. Kept:")
            } else {
                format!("{name} was not installed; nothing changed. Kept, if present:")
            })
            .chain(kept.iter().map(|(_, how)| format!("  - {how}")))
            .collect::<Vec<_>>()
            .join("\n");
            Ok(Some((
                json!({"extension": name, "installed": false, "changed": changed,
                    "kept": kept.iter().map(|(what, _)| *what).collect::<Vec<_>>()}),
                human,
            )))
        }
        ExtensionInstallRequest::List {
            prefix: selected,
            check,
        } => listing(&prefix(selected.as_deref())?, check).map(Some),
    }
}

fn install(
    product: Product,
    prefix: &Path,
    channel: Option<Channel>,
    archive: Option<&str>,
    manifest: Option<&str>,
) -> Result<Outcome, Failure> {
    let name = product.as_str();
    let target = tmt_core::native_install::native_target(env::consts::OS, env::consts::ARCH)
        .ok_or_else(|| {
            Failure::new(
                "EXTENSION_UNSUPPORTED",
                format!("No {name} artifact supports this platform."),
                1,
            )
        })?;
    let executable = prefix.join("bin").join(product.executable());
    let current = if installed(product, prefix)? {
        Some(
            native_install::inspect_product(product, &executable)
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?,
        )
    } else {
        None
    };
    let channel = channel
        .or_else(|| current.as_ref().map(|current| current.state.channel))
        .unwrap_or(Channel::Alpha);
    let checkpoint = interruptible("Extension installation interrupted before activation.")?;
    let result = match (archive, manifest) {
        (Some(archive), Some(manifest)) => native_install::install_product(
            product,
            InstallRequest {
                archive: Path::new(archive),
                manifest: Path::new(manifest),
                prefix,
                target,
                channel,
                pin: PinAction::Preserve,
            },
            crate::office_facade::release_verifier(product),
            checkpoint,
        ),
        (None, None) if current.is_some() => native_install::upgrade_product(
            product,
            UpgradeRequest {
                executable: &executable,
                channel: Some(channel),
                exact: None,
                unpin: false,
            },
            crate::office_facade::release_verifier(product),
            checkpoint,
        )
        .map(|report| report.installation)
        .map_err(|error| io::Error::new(error.kind(), error)),
        (None, None) => native_install::install_release(
            product,
            prefix,
            target,
            channel,
            crate::office_facade::release_verifier(product),
            checkpoint,
        ),
        _ => {
            return Err(Failure::new(
                "USAGE_ERROR",
                "Archive and manifest must be supplied together.",
                1,
            ));
        }
    };
    let report = result.map_err(|error| {
        failure(
            if archive.is_none() && error.kind() == io::ErrorKind::NotFound {
                "EXTENSION_RELEASE_UNAVAILABLE"
            } else {
                "EXTENSION_INSTALL_FAILED"
            },
            error,
        )
    })?;
    let human = if report.changed {
        format!(
            "Installed {name} {} at {}.",
            report.version,
            report.executable.display()
        )
    } else {
        format!("{name} {} is already installed.", report.version)
    };
    Ok(Some((
        json!({"extension": name, "installed": true, "changed": report.changed,
            "version": report.version, "executable": report.executable}),
        human,
    )))
}

/// What uninstall keeps, with how to remove each explicitly. This command
/// never deletes data.
fn kept(product: Product, prefix: &Path) -> Vec<(&'static str, String)> {
    let name = product.as_str();
    let mut kept = vec![
        (
            "releases",
            format!(
                "releases in {}; delete that folder to reclaim space",
                prefix.join(product.namespace()).join("releases").display()
            ),
        ),
        (
            "skills",
            format!(
                "agent skills it installed, in your agents' skill folders (for example ~/.claude/skills/tmt-{name}); delete those folders to remove them"
            ),
        ),
        (
            "hookConsent",
            format!("lifecycle hook consent; withdraw it with: tmt extension hooks disable {name}"),
        ),
    ];
    if product == Product::Office {
        kept.push((
            "officeData",
            "Office data (office.db) and its backups; see their paths with: tmt office storage status, and delete them there only if you no longer need them".to_owned(),
        ));
    }
    kept
}

fn listing(prefix: &Path, check: bool) -> Result<(Value, String), Failure> {
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    for product in Product::ALL
        .into_iter()
        .filter(|product| *product != Product::Cli)
    {
        let name = product.as_str();
        let executable = prefix.join("bin").join(product.executable());
        let state = match installed(product, prefix) {
            Ok(true) => native_install::inspect_product(product, &executable)
                .map(|installation| Some(installation.state))
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?,
            Ok(false) => None,
            Err(error) => return Err(error),
        };
        let shadowed = shadowing(product, prefix);
        // Only an explicit --check touches the network; unreachable is unknown.
        let update = if check {
            let channel = state.as_ref().map_or(Channel::Alpha, |state| state.channel);
            Some(
                match native_install::latest_release_version(product, channel) {
                    Ok(latest) => match &state {
                        Some(state) if latest > state.version => "available",
                        Some(_) => "current",
                        None => "available",
                    },
                    Err(_) => "unknown",
                },
            )
        } else {
            None
        };
        let mut row = json!({
            "name": name,
            "installed": state.is_some(),
            "version": state.as_ref().map(|state| state.version.to_string()),
            "channel": state.as_ref().map(|state| state.channel.as_str()),
            "pinned": state.as_ref().and_then(|state| state.pinned_version.as_ref()).map(ToString::to_string),
            "commands": product.links(),
            "shadowedBy": shadowed,
        });
        if let Some(update) = update {
            row["update"] = update.into();
        }
        let mut line = match &state {
            Some(state) => format!(
                "{name}  {}{}",
                state.version,
                state
                    .pinned_version
                    .as_ref()
                    .map(|pinned| format!(" (pinned {pinned})"))
                    .unwrap_or_default()
            ),
            None => format!("{name}  not installed"),
        };
        if let Some(update) = update {
            line.push_str(&format!("  update: {update}"));
        }
        for path in &shadowed {
            line.push_str(&format!("\n  shadowed on PATH by {path}"));
        }
        lines.push(line);
        rows.push(row);
    }
    Ok((json!({"extensions": rows}), lines.join("\n")))
}

/// Every other `tmt-<name>` command on PATH that does not resolve to this
/// installation's file. Paths are canonicalized; nothing is executed.
fn shadowing(product: Product, prefix: &Path) -> Vec<String> {
    let search = env::var_os("PATH").unwrap_or_default();
    shadowing_in(product, prefix, &search)
}

fn shadowing_in(product: Product, prefix: &Path, search: &std::ffi::OsStr) -> Vec<String> {
    let mut found = Vec::new();
    for link in product.links() {
        let ours = fs::canonicalize(prefix.join("bin").join(link)).ok();
        for directory in env::split_paths(search) {
            let candidate = directory.join(link);
            let Ok(metadata) = fs::metadata(&candidate) else {
                continue;
            };
            if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
            let Ok(resolved) = fs::canonicalize(&candidate) else {
                continue;
            };
            if Some(&resolved) != ours.as_ref() {
                let shown = candidate.display().to_string();
                if !found.contains(&shown) {
                    found.push(shown);
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn only_official_extensions_are_named_and_the_cli_is_not_one() {
        assert_eq!(extension("squad").unwrap(), Product::Squad);
        assert_eq!(extension("office").unwrap(), Product::Office);
        for name in ["cli", "tmt", "sq", "unknown"] {
            let error = extension(name).unwrap_err();
            assert_eq!(error.code, "EXTENSION_UNKNOWN");
            assert!(
                error
                    .message
                    .ends_with("Official extensions: office, squad.")
            );
        }
    }

    #[test]
    fn shadowing_reports_other_commands_by_canonical_path_without_running_them() {
        let root = std::env::temp_dir().join(format!("tmt-shadow-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let prefix = root.join("prefix");
        let release = prefix.join("lib/tmt-squad/current");
        fs::create_dir_all(&release).unwrap();
        fs::create_dir_all(prefix.join("bin")).unwrap();
        let binary = release.join("tmt-squad");
        // A marker the test can check: executing it would create this file.
        let marker = root.join("executed");
        fs::write(&binary, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        for link in Product::Squad.links() {
            symlink(&binary, prefix.join("bin").join(link)).unwrap();
        }
        // The same file reached through another directory is not shadowing.
        let alias = root.join("alias");
        fs::create_dir_all(&alias).unwrap();
        symlink(prefix.join("bin/tmt-squad"), alias.join("tmt-squad")).unwrap();
        let other = root.join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("tmt-sq"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(other.join("tmt-sq"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(other.join("tmt-squad"), "not executable").unwrap();
        let search = env::join_paths([other.clone(), alias, prefix.join("bin")]).unwrap();
        assert_eq!(
            shadowing_in(Product::Squad, &prefix, &search),
            [other.join("tmt-sq").display().to_string()]
        );
        assert!(!marker.exists(), "nothing was executed");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn uninstall_names_what_stays_and_how_to_remove_it() {
        let prefix = Path::new("/p");
        let squad = kept(Product::Squad, prefix);
        assert_eq!(
            squad.iter().map(|(what, _)| *what).collect::<Vec<_>>(),
            ["releases", "skills", "hookConsent"]
        );
        assert!(squad[0].1.contains("/p/lib/tmt-squad/releases"));
        assert!(squad[2].1.contains("tmt extension hooks disable squad"));
        let office = kept(Product::Office, prefix);
        assert_eq!(office.last().unwrap().0, "officeData");
        assert!(
            office
                .last()
                .unwrap()
                .1
                .contains("tmt office storage status")
        );
    }
}
