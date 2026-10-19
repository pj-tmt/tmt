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
/// Skill failures are warnings; binary activation keeps its own outcome, and
/// partial publication remains visible in the skill report.
pub(super) fn settle_skills(
    product: Product,
    executable: &Path,
    previous: &[String],
    document: &mut Value,
    human: &mut Human,
) -> Result<(), Failure> {
    if let Err(error) = settle_publication(product, executable, previous, document, human) {
        document["skills"]["status"] = json!("warning");
        document["skills"]["warning"] = error.document()["error"].clone();
        human.push(format!("Warning: {}", error.message));
    }
    let installation = native_install::inspect_product(product, executable)
        .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
    if product == Product::Ops
        && let Some(note) = retired_squad_note(installation.prefix())
    {
        document["retiredSquadHint"] = json!(note);
        human.push(note);
    }
    Ok(())
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
    let owned = skill_installation::owned_by(global, name)
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
        let removal = skill_installation::remove_owned(global, name, Some(&dropped));
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

/// What a Squad installation that Ops replaced still has under `prefix`. TMT no
/// longer upgrades it, replaces it with Ops or switches its board, and removes
/// nothing of it, so say what is left and what to do.
pub(crate) fn retired_squad_note(prefix: &Path) -> Option<String> {
    let found = native_install::retired_squad(prefix)?;
    let mut note = format!(
        "Squad at {} is left as it is: TMT no longer upgrades it, replaces it with Ops or switches its board.",
        found.root.display()
    );
    if std::fs::symlink_metadata(prefix.join("bin").join(Product::Ops.executable())).is_err() {
        note.push_str(" Install Ops with: tmt extension install ops --yes");
        note.push('.');
    }
    note.push_str(
        " Ops moves your Squad config and data on first use; a running former board defers that until it exits.",
    );
    let leftovers = found
        .links
        .iter()
        .chain([&found.root])
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    note.push_str(&format!(
        " Remove {} by hand when you no longer need them.",
        leftovers.join(", ")
    ));
    Some(note)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        " Install Ops with: tmt extension install ops --yes",
        &[""],
        &[],
    )];

#[cfg(test)]
mod retired_squad_tests {
    use super::retired_squad_note;
    use std::fs;

    fn prefix(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("tmt-retired-squad-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("bin")).unwrap();
        path
    }

    #[test]
    fn nothing_is_said_without_a_squad_installation() {
        let prefix = prefix("none");
        fs::create_dir_all(prefix.join("lib/tmt-ops")).unwrap();
        assert_eq!(retired_squad_note(&prefix), None);
        fs::remove_dir_all(&prefix).unwrap();
    }

    #[test]
    fn a_leftover_squad_is_named_with_what_to_do_and_nothing_is_removed() {
        let prefix = prefix("left");
        fs::create_dir_all(prefix.join("lib/tmt-squad")).unwrap();
        std::os::unix::fs::symlink(
            "../lib/tmt-squad/current/tmt-squad",
            prefix.join("bin/tmt-squad"),
        )
        .unwrap();
        let note = retired_squad_note(&prefix).unwrap();
        assert!(note.contains(&prefix.join("lib/tmt-squad").display().to_string()));
        assert!(note.contains("no longer upgrades it"));
        assert!(note.contains("tmt extension install ops --yes"));
        assert!(
            note.contains(&format!(
                "Remove {}, {} by hand",
                prefix.join("bin/tmt-squad").display(),
                prefix.join("lib/tmt-squad").display()
            )),
            "only the links that exist are listed: {note}"
        );
        assert!(
            prefix.join("lib/tmt-squad").is_dir(),
            "the note changes nothing"
        );

        // With Ops installed there is nothing to install.
        std::os::unix::fs::symlink("../lib/tmt-ops/current/tmt-ops", prefix.join("bin/tmt-ops"))
            .unwrap();
        let note = retired_squad_note(&prefix).unwrap();
        assert!(!note.contains("tmt extension install ops"));
        fs::remove_dir_all(&prefix).unwrap();
    }
}
