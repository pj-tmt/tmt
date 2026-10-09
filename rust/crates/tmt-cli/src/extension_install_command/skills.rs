//! Settlement of extension-owned skills after verified activation.

use super::{Human, failure};
use crate::output::Failure;
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

fn publish(
    product: Product,
    executable: &Path,
    document: &mut Value,
    human: &mut Human,
) -> Result<OwnedReport, Failure> {
    let env = ProviderEnvironment::capture()
        .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
    let global = global_dir()?;
    let name = product.as_str();
    skill_installation::install_release_skills(&env, &global, product, executable).map_err(
        |failure| {
            document["skills"]["published"] = published_document(&failure.report);
            let home = env::home_dir();
            published_lines(&failure.report, home.as_deref())
                .for_each(|line| human.push_success(line));
            Failure::new(
                "EXTENSION_SKILLS_FAILED",
                format!("{name} is installed, but its agent skills were not published: {failure}"),
                1,
            )
        },
    )
}

/// Publish the activated release's skills without a second consent step.
/// Skill failures are warnings; binary activation and replacement keep their
/// own outcomes, and partial publication remains visible in the skill report.
pub(super) fn settle_skills(
    product: Product,
    executable: &Path,
    previous: &[String],
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
    if let Err(error) = settle_publication(product, executable, previous, document, human) {
        document["skills"]["status"] = json!("warning");
        document["skills"]["warning"] = error.document()["error"].clone();
        human.push(format!("Warning: {}", error.message));
    }
    let installation = native_install::inspect_product(product, executable)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    settle_replacement(product, &installation, document, human)
}

fn settle_publication(
    product: Product,
    executable: &Path,
    previous: &[String],
    document: &mut Value,
    human: &mut Human,
) -> Result<(), Failure> {
    let name = product.as_str();
    document["skills"] =
        json!({"status": "current", "available": [], "published": [], "removed": []});
    let skills = native_install::release_skills(product, executable)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    let current = skill_names(&skills);
    document["skills"]["available"] = json!(current);
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
    if !skills.is_empty() {
        let report = publish(product, executable, document, human)?;
        let home = env::home_dir();
        published_lines(&report, home.as_deref()).for_each(|line| human.push_success(line));
        document["skills"]["published"] = published_document(&report);
    }
    if !dropped.is_empty() {
        let removal = skill_installation::remove_owned(None, global, name, Some(&dropped));
        let report = match &removal {
            Ok(report) => report,
            Err(error) => &error.report,
        };
        for target in &report.removed {
            human.push(format!("Removed retired agent skill {}", target.display()));
        }
        document["skills"]["removed"] = json!(report.removed);
        removal
            .map_err(|error| failure("EXTENSION_SKILLS_FAILED", std::io::Error::other(error)))?;
    }
    Ok(())
}

/// Installation receipts and hook consent have separate owners. Verify both
/// activations before withdrawing consent, and retain the former installation
/// if that withdrawal fails so normal settlement can retry.
fn settle_replacement(
    product: Product,
    installation: &native_install::ManagedInstallation,
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
    let paths = ConfigPaths::discover()?;
    let disabled = extension_hooks::disable(&paths, former.name).map_err(|error| {
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
    if disabled {
        document["hooks"] = json!({
            "disabled": [former.name],
            "enableCommand": format!("tmt extension hooks enable {}", product.as_str()),
        });
        human.push(hint.clone());
    }
    if !super::board_switch::run(installation, document, human)? {
        return Ok(());
    }
    native_install::inspect_product(product, &installation.active_executable)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
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
        "Former {} installation retained; replace with: tmt extension upgrade {name} --unpin",
        &[""],
        &[("{name}", "ops"), ("{}", "squad")],
    ),
];
