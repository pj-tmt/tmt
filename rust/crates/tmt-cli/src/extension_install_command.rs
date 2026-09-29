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
    config::ConfigPaths,
    interrupt::Interrupt,
    native_install::{self, InstallRequest, Product, UpgradeRequest},
    skill_installation::{self, OwnedReport, OwnedSkill, ProviderEnvironment},
};
use tmt_core::native_install::{Channel, PinAction};

const CONSENT: &str = "EXTENSION_CONSENT_REQUIRED";

/// Consent for one change to the user's installation.
fn ask(yes: bool, mode: OutputMode, action: &str) -> Result<bool, Failure> {
    ask_declining(yes, mode, action, "No changes made.")
}

fn ask_declining(
    yes: bool,
    mode: OutputMode,
    action: &str,
    declined: &str,
) -> Result<bool, Failure> {
    consent::ask(
        &mut tmt_cli_style::stream::stdout(mode.json),
        yes,
        mode,
        consent::Consent {
            code: CONSENT,
            refusal: &format!("{action} requires explicit --yes; no changes were made."),
            question: action,
            declined,
        },
        |error| {
            Failure::new("EXTENSION_IO_ERROR", "Could not ask for consent.", 1).caused_by(error)
        },
    )
}

pub fn execute(request: ExtensionInstallRequest, mode: OutputMode) -> io::Result<u8> {
    match run(request, mode) {
        Ok(Some((document, human))) => {
            let mut stdout = tmt_cli_style::stream::stdout(mode.json);
            let terminal = stdout.terminal();
            if mode.json {
                writeln!(stdout, "{document}")?;
            } else {
                human.write(&mut stdout, terminal)?;
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

/// Human output: a first line that is a success when `done`, then any
/// further lines as written.
struct Human {
    done: bool,
    text: String,
}

impl Human {
    fn done(text: String) -> Self {
        Self { done: true, text }
    }

    fn plain(text: String) -> Self {
        Self { done: false, text }
    }

    fn push(&mut self, line: String) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(&line);
    }

    fn write(&self, output: &mut impl Write, terminal: tmt_cli_style::Terminal) -> io::Result<()> {
        let mut lines = self.text.lines();
        if self.done {
            let first = lines.next().unwrap_or_default();
            tmt_cli_style::message::success(output, terminal, first)?;
        }
        for line in lines {
            writeln!(output, "{line}")?;
        }
        Ok(())
    }
}

type Outcome = Option<(Value, Human)>;

fn run(request: ExtensionInstallRequest, mode: OutputMode) -> Result<Outcome, Failure> {
    match request {
        ExtensionInstallRequest::Install {
            name,
            prefix: selected,
            channel,
            archive,
            manifest,
            yes,
            skills,
        } => {
            let product = extension(&name)?;
            let prefix = prefix(selected.as_deref())?;
            if !ask(yes, mode, &format!("Install the verified {name} extension"))? {
                return Ok(None);
            }
            let (mut document, human, executable, previous) = install(
                product,
                &prefix,
                channel,
                archive.as_deref(),
                manifest.as_deref(),
            )?;
            let mut human = human;
            settle_skills(
                product,
                &executable,
                &previous,
                skills,
                Some(mode),
                &mut document,
                &mut human,
            )?;
            Ok(Some((document, human)))
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
            // The skills the replaced release carried: only those may be pruned.
            let previous = skill_names(
                &native_install::release_skills(product, &executable)
                    .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?,
            );
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
                Human::plain(format!(
                    "{name} {version} is pinned; nothing changed. Clear the pin with: tmt extension upgrade {name} --unpin"
                ))
            } else if report.installation.changed {
                Human::done(format!("Updated {name} to {version}."))
            } else {
                Human::plain(format!("{name} {version} is current."))
            };
            let mut document = json!({"extension": name, "installed": true,
                "changed": report.installation.changed, "version": version,
                "skippedPinned": report.skipped_pinned,
                "executable": report.installation.executable});
            let mut human = human;
            if report.installation.changed {
                settle_skills(
                    product,
                    &executable,
                    &previous,
                    false,
                    None,
                    &mut document,
                    &mut human,
                )?;
            }
            Ok(Some((document, human)))
        }
        ExtensionInstallRequest::Uninstall {
            name,
            prefix: selected,
            yes,
        } => {
            let product = extension(&name)?;
            let prefix = prefix(selected.as_deref())?;
            let global = global_dir()?;
            // Every skill the extension's owner holds goes with its commands:
            // a skill that points at a removed command is broken guidance.
            let owned = skill_installation::owned_by(&global, product.as_str())
                .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
            let mut question =
                format!("Remove the {name} extension's commands (releases and data are kept)");
            if !owned.is_empty() {
                question.push_str(&format!(
                    " and its agent skills {} from {}",
                    owned.keys().cloned().collect::<Vec<_>>().join(", "),
                    owned
                        .values()
                        .flatten()
                        .map(|target| target.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !ask(yes, mode, &question)? {
                return Ok(None);
            }
            let changed = native_install::uninstall_extension(&prefix, product)
                .map_err(|error| failure("EXTENSION_UNINSTALL_FAILED", error))?;
            let skills = skill_installation::remove_owned(&global, product.as_str(), None)
                .map_err(|failure| {
                    Failure::new(
                        "EXTENSION_SKILLS_FAILED",
                        format!(
                            "Removed the {name} extension's commands, but not all of its agent skills: {failure}"
                        ),
                        1,
                    )
                })?;
            let kept = kept(product, &prefix);
            let mut lines = std::iter::once(if changed {
                format!("Removed the {name} extension's commands.")
            } else {
                format!("{name} was not installed; its commands were already gone.")
            })
            .chain(
                skills
                    .removed
                    .iter()
                    .map(|target| format!("Removed agent skill {}", target.display())),
            )
            .chain(skills.kept.iter().map(|target| {
                format!(
                    "Left {} alone: it no longer points at the {name} skill",
                    target.display()
                )
            }))
            .collect::<Vec<_>>();
            lines.push("Kept:".into());
            lines.extend(kept.iter().map(|(_, how)| format!("  - {how}")));
            let text = lines.join("\n");
            let human = if changed {
                Human::done(text)
            } else {
                Human::plain(text)
            };
            Ok(Some((
                json!({"extension": name, "installed": false, "changed": changed,
                    "skillsRemoved": skills.removed, "skillsKept": skills.kept,
                    "kept": kept.iter().map(|(what, _)| *what).collect::<Vec<_>>()}),
                human,
            )))
        }
        ExtensionInstallRequest::List {
            prefix: selected,
            check,
        } => listing(&prefix(selected.as_deref())?, check)
            .map(|(document, text)| Some((document, Human::plain(text)))),
    }
}

/// The installed extension's report and its active command link.
fn install(
    product: Product,
    prefix: &Path,
    channel: Option<Channel>,
    archive: Option<&str>,
    manifest: Option<&str>,
) -> Result<(Value, Human, PathBuf, Vec<String>), Failure> {
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
    // The skills the replaced release carried: only those may be pruned.
    let previous = match &current {
        Some(_) => skill_names(
            &native_install::release_skills(product, &executable)
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?,
        ),
        None => Vec::new(),
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
        Human::done(format!(
            "Installed {name} {} at {}.",
            report.version,
            report.executable.display()
        ))
    } else {
        Human::plain(format!("{name} {} is already installed.", report.version))
    };
    Ok((
        json!({"extension": name, "installed": true, "changed": report.changed,
            "version": report.version, "executable": report.executable}),
        human,
        report.executable,
        previous,
    ))
}

fn global_dir() -> Result<PathBuf, Failure> {
    ConfigPaths::discover()
        .map(|paths| paths.global_dir)
        .map_err(Failure::from)
}

fn skill_names(skills: &[OwnedSkill]) -> Vec<String> {
    skills.iter().map(|skill| skill.name.clone()).collect()
}

fn published_document(report: &OwnedReport) -> Value {
    report
        .published
        .iter()
        .map(|item| {
            json!({"name": item.name, "agent": item.agent.map(|agent| agent.name()),
                "target": item.target, "changed": item.changed, "backup": item.backup})
        })
        .collect()
}

fn published_lines(report: &OwnedReport) -> impl Iterator<Item = String> + '_ {
    report.published.iter().map(|item| {
        let mut line = format!(
            "{} agent skill {} at {}",
            if item.changed {
                "Published"
            } else {
                "Kept current"
            },
            item.name,
            item.target.display()
        );
        if let Some(backup) = &item.backup {
            line.push_str(&format!(
                " (previous folder backed up to {})",
                backup.display()
            ));
        }
        line
    })
}

fn publish(product: Product, skills: &[OwnedSkill]) -> Result<OwnedReport, Failure> {
    let env = ProviderEnvironment::capture()
        .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
    let global = global_dir()?;
    let name = product.as_str();
    skill_installation::install_owned(&env, &global, name, skills, false).map_err(|failure| {
        Failure::new(
            "EXTENSION_SKILLS_FAILED",
            format!("{name} is installed, but its agent skills were not published: {failure}"),
            1,
        )
    })
}

/// After activation, the release's agent skills, under one owner named after
/// the extension:
/// - `--skills` publishes every skill in the release tree.
/// - Skills the owner already holds from the tree are refreshed, and those the
///   new release dropped are removed by name. Other skills the owner holds,
///   such as playbooks, are never touched.
/// - Otherwise a terminal install asks once; any other run names the skills
///   and how to publish them.
///
/// Declining, or a refused publication, leaves the activated extension installed.
fn settle_skills(
    product: Product,
    executable: &Path,
    previous: &[String],
    requested: bool,
    offer: Option<OutputMode>,
    document: &mut Value,
    human: &mut Human,
) -> Result<(), Failure> {
    let name = product.as_str();
    let skills = native_install::release_skills(product, executable)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    let current = skill_names(&skills);
    let global = global_dir()?;
    let owned = skill_installation::owned_by(&global, name)
        .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
    let dropped: Vec<String> = previous
        .iter()
        .filter(|skill| owned.contains_key(*skill) && !current.contains(skill))
        .cloned()
        .collect();
    let held = current.iter().any(|skill| owned.contains_key(skill));
    let mut selected: Vec<OwnedSkill> = if requested {
        skills
    } else {
        skills
            .into_iter()
            .filter(|skill| owned.contains_key(&skill.name))
            .collect()
    };
    if !requested && !held && dropped.is_empty() && !current.is_empty() {
        let accepted = match offer {
            Some(mode) if consent::interactive(mode, &tmt_cli_style::stream::stdout(mode.json)) => {
                let env = ProviderEnvironment::capture()
                    .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
                let roots = skill_installation::owned_roots(&env, &global)
                    .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
                // The install result first, then the question about skills.
                let mut stdout = tmt_cli_style::stream::stdout(mode.json);
                let terminal = stdout.terminal();
                std::mem::replace(human, Human::plain(String::new()))
                    .write(&mut stdout, terminal)
                    .map_err(|error| {
                        Failure::new("EXTENSION_IO_ERROR", "Could not write output.", 1)
                            .caused_by(error)
                    })?;
                drop(stdout);
                ask_declining(
                    false,
                    mode,
                    &format!(
                        "Publish its agent skills {} into {}",
                        current.join(", "),
                        roots
                            .iter()
                            .map(|root| root.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    "Skipped the agent skills; the extension is installed.",
                )?
            }
            _ => false,
        };
        if accepted {
            selected = native_install::release_skills(product, executable)
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
        }
    }
    let mut published = Value::Array(Vec::new());
    if !selected.is_empty() {
        let report = publish(product, &selected)?;
        published_lines(&report).for_each(|line| human.push(line));
        published = published_document(&report);
    }
    let mut removed = Vec::new();
    if !dropped.is_empty() {
        let report = skill_installation::remove_owned(&global, name, Some(&dropped))
            .map_err(|failure| {
                Failure::new(
                    "EXTENSION_SKILLS_FAILED",
                    format!(
                        "{name} is installed, but its retired agent skills were not removed: {failure}"
                    ),
                    1,
                )
            })?;
        for target in &report.removed {
            human.push(format!("Removed retired agent skill {}", target.display()));
        }
        removed = report.removed;
    }
    let unpublished: Vec<&String> = current
        .iter()
        .filter(|skill| !selected.iter().any(|chosen| &chosen.name == *skill))
        .collect();
    if !unpublished.is_empty() {
        human.push(format!(
            "{} agent skill{} available ({}); publish with: tmt extension install {name} --skills",
            unpublished.len(),
            if unpublished.len() == 1 { "" } else { "s" },
            unpublished
                .iter()
                .map(|skill| skill.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !current.is_empty() || !removed.is_empty() {
        document["skills"] =
            json!({"available": current, "published": published, "removed": removed});
    }
    Ok(())
}

/// What uninstall keeps, with how to remove each explicitly. This command
/// never deletes data; the extension's own agent skills are not data.
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
            ["releases", "hookConsent"],
            "the extension's agent skills are removed, not kept"
        );
        assert!(squad[0].1.contains("/p/lib/tmt-squad/releases"));
        assert!(squad[1].1.contains("tmt extension hooks disable squad"));
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
