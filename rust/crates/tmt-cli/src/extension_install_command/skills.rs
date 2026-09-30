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
        let home = env::home_dir();
        published_lines(&report, home.as_deref()).for_each(|line| human.push_success(line));
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
