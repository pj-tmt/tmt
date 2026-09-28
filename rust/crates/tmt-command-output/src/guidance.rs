//! Shared skill-install report presentation for core and Office commands.

use serde_json::{Value, json};
use std::io::{self, Write};
use tmt_adapters::skill_installation::{InstallReport, InstalledSkill};

pub(crate) fn document(item: &InstalledSkill) -> Value {
    let mut value = json!({"skill": item.name, "target": item.target, "changed": item.changed});
    if let Some(agent) = item.agent {
        value["agent"] = agent.as_str().into();
    }
    if let Some(backup) = &item.backup {
        value["backup"] = json!(backup);
    }
    if !item.legacy_backups.is_empty() {
        value["legacyBackups"] = json!(item.legacy_backups);
    }
    value
}

pub fn report_document(report: &InstallReport) -> Value {
    let mut value = json!({"installed": report.installed.iter().map(document).collect::<Vec<_>>()});
    if !report.warnings.is_empty() {
        value["warnings"] = json!(report.warnings);
    }
    value
}

pub fn write_report_human(report: &InstallReport, output: &mut impl Write) -> io::Result<()> {
    for item in &report.installed {
        writeln!(
            output,
            "{} {} skill '{}' at {}",
            if item.changed { "Installed" } else { "Current" },
            item.agent.map_or("shared", |agent| agent.as_str()),
            item.name,
            item.target.display()
        )?;
        for backup in item.backup.iter().chain(&item.legacy_backups) {
            writeln!(output, "Recoverable backup: {}", backup.display())?;
        }
    }
    for warning in &report.warnings {
        writeln!(output, "Warning: {warning}")?;
    }
    Ok(())
}
