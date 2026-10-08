//! Settlement of extension-owned skills after verified activation.

use super::{Human, ask_declining, failure};
use crate::{consent, invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{
    env,
    path::{Path, PathBuf},
};
use tmt_adapters::{
    config::ConfigPaths,
    extension_hooks,
    native_install::{self, Product},
    skill_installation::{self, OwnedReport, OwnedSkill, ProviderEnvironment},
};
use tmt_cli_style::value;

pub(super) fn global_dir() -> Result<PathBuf, Failure> {
    ConfigPaths::discover()
        .map(|paths| paths.global_dir)
        .map_err(Failure::from)
}

fn skill_names(skills: &[OwnedSkill]) -> Vec<String> {
    skills.iter().map(|skill| skill.name.clone()).collect()
}

pub(super) fn published_document(report: &OwnedReport) -> Value {
    report
        .published
        .iter()
        .map(|item| {
            json!({"name": item.name, "agent": item.agent.map(|agent| agent.name()),
                "target": item.target, "changed": item.changed, "backup": item.backup})
        })
        .collect()
}

pub(super) fn published_lines<'a>(
    report: &'a OwnedReport,
    home: Option<&'a Path>,
) -> impl Iterator<Item = String> + 'a {
    report.published.iter().map(move |item| {
        let mut line = format!(
            "{} agent skill {} at {}",
            if item.changed {
                "Published"
            } else {
                "Kept current"
            },
            item.name,
            value::home_path(&item.target, home)
        );
        if let Some(backup) = &item.backup {
            line.push_str(&format!(
                " (previous folder backed up to {})",
                value::home_path(backup, home)
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
pub(super) fn settle_skills(
    product: Product,
    executable: &Path,
    previous: &[String],
    requested: bool,
    offer: Option<OutputMode>,
    document: &mut Value,
    human: &mut Human,
) -> Result<(), Failure> {
    let name = product.as_str();
    if let Some(former) = product.former()
        && executable
            .file_name()
            .is_some_and(|name| name == former.executable)
    {
        // A pinned upgrade is a no-op. It cannot migrate skills or retire the
        // old installation before an Ops release has actually been activated.
        document["formerly"] = json!(former.name);
        human.push(format!(
            "Former {} installation retained; replace with: tmt extension upgrade {name} --unpin",
            former.name
        ));
        return Ok(());
    }
    let skills = native_install::release_skills(product, executable)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    let current = skill_names(&skills);
    let paths = ConfigPaths::discover()?;
    let global = &paths.global_dir;
    skill_installation::migrate_former_owned(global, product, &skills)
        .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
    let owned = skill_installation::owned_by(None, global, name)
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
    let mut expand_targets = requested;
    if !requested && !held && dropped.is_empty() && !current.is_empty() {
        let accepted = match offer {
            Some(mode) if consent::interactive(mode, &tmt_cli_style::stream::stdout(mode.json)) => {
                let env = ProviderEnvironment::capture()
                    .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
                let roots = skill_installation::owned_roots(&env, global)
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
            expand_targets = true;
            selected = native_install::release_skills(product, executable)
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
        }
    }
    let mut published = Value::Array(Vec::new());
    if !selected.is_empty() {
        let report = if product.former().is_some() && !expand_targets {
            skill_installation::refresh_owned(global, name, &selected)
                .map_err(|error| failure("EXTENSION_SKILLS_FAILED", std::io::Error::other(error)))?
        } else {
            publish(product, &selected)?
        };
        let home = env::home_dir();
        published_lines(&report, home.as_deref()).for_each(|line| human.push_success(line));
        published = published_document(&report);
    }
    let mut removed = Vec::new();
    if !dropped.is_empty() {
        let report = skill_installation::remove_owned(None, global, name, Some(&dropped))
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
    let installation = native_install::inspect_product(product, executable)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    settle_replacement(product, &installation, &paths, document, human)
}

/// Installation receipts and hook consent have separate owners. Verify both
/// activations before withdrawing consent, and retain the former installation
/// if that withdrawal fails so normal settlement can retry.
fn settle_replacement(
    product: Product,
    installation: &native_install::ManagedInstallation,
    paths: &ConfigPaths,
    document: &mut Value,
    human: &mut Human,
) -> Result<(), Failure> {
    let Some(former) = product.former() else {
        return Ok(());
    };
    let prefix = installation.prefix();
    if native_install::inspect_former_product(product, prefix)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?
        .is_none()
    {
        return Ok(());
    }
    let disabled = extension_hooks::disable(paths, former.name).map_err(|error| {
        Failure::new(
            error.code(),
            format!(
                "{} is installed, but former {} hook consent cleanup failed at {}: {error}. The former installation was retained; fix the settings and retry installation or an unpinned upgrade.",
                product.as_str(), former.name,
                extension_hooks::consent_path(&paths.global_dir).display()
            ),
            1,
        )
        .caused_by(error)
    })?;
    let hint = format!(
        "Removed former {} hook consent; Ops hooks require separate consent. Enable with: tmt extension hooks enable {}",
        former.name,
        product.as_str()
    );
    let replacement = native_install::finish_product_replacement(prefix, product)
        .map_err(|error| {
            Failure::new(
                "EXTENSION_INSTALL_FAILED",
                format!(
                    "{} is installed, but former {} replacement cleanup failed: {error}.{} Inspect with: tmt extension ls",
                    product.as_str(), former.name,
                    if disabled { format!(" {hint}.") } else { String::new() }
                ),
                1,
            ).caused_by(error)
        })?;
    if disabled {
        document["hooks"] = json!({
            "disabled": [former.name],
            "enableCommand": format!("tmt extension hooks enable {}", product.as_str()),
        });
        human.push(hint);
    }
    if !replacement.removed.is_empty() || !replacement.kept.is_empty() {
        document["replaced"] = json!(former.name);
        document["removed"] = json!(replacement.removed);
        document["kept"] = json!(replacement.kept);
        for path in &replacement.removed {
            human.push(format!("Removed former product {}", path.display()));
        }
    }
    Ok(())
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Removed former {} hook consent; Ops hooks require separate consent. Enable with: tmt extension hooks enable {}",
        &[""],
        &[
            (
                "tmt extension hooks enable {}",
                "tmt extension hooks enable ops",
            ),
            ("{}", "squad"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt extension hooks enable {}",
        &[""],
        &[("{}", "ops")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{} is installed, but former {} replacement cleanup failed: {error}.{} Inspect with: tmt extension ls",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{} agent skill{} available ({}); publish with: tmt extension install {name} --skills",
        &[""],
        &[
            ("{name}", "ops"),
            ("{}", "ops"),
            ("{SUGGESTED_EXTENSION}", "ops"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Former {} installation retained; replace with: tmt extension upgrade {name} --unpin",
        &[""],
        &[("{name}", "ops"), ("{}", "squad")],
    ),
];
