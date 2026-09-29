//! New-executable composition for the native updater's managed-skill phase.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    skill_installation::{self, RefreshReport},
};

fn document(report: &RefreshReport) -> Value {
    json!({
        "refreshed": report.refreshed.iter().map(|item| json!({"target": item.target, "changed": item.changed})).collect::<Vec<_>>(),
        "skipped": report.skipped,
        "conflicts": report.conflicts,
    })
}

/// The updater consumes this same owner's bounded JSON contract, not arbitrary
/// child stdout. Error details retain the shared CLI error-envelope shape.
pub(crate) fn parse_document(bytes: &[u8]) -> Option<Value> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    if object.len() != 3 + usize::from(object.contains_key("error")) {
        return None;
    }
    if !value["refreshed"].as_array()?.iter().all(|item| {
        item.as_object().is_some_and(|item| item.len() == 2)
            && item["target"].is_string()
            && item["changed"].is_boolean()
    }) {
        return None;
    }
    for key in ["skipped", "conflicts"] {
        if !value[key].as_array()?.iter().all(Value::is_string) {
            return None;
        }
    }
    if !value["conflicts"].as_array()?.is_empty() && !object.contains_key("error") {
        return None;
    }
    if let Some(error) = object.get("error")
        && (!error["code"].is_string() || !error["message"].is_string())
    {
        return None;
    }
    Some(value)
}

pub fn execute(mode: OutputMode) -> io::Result<u8> {
    let (report, failure) = match ConfigPaths::discover() {
        Err(error) => (RefreshReport::default(), Some(Failure::from(error))),
        Ok(paths) => match skill_installation::refresh(&paths.global_dir) {
            Ok(report) => (report, None),
            Err(mut error) => {
                let report = std::mem::take(&mut error.report);
                let failure =
                    Failure::new("SKILL_REFRESH_FAILED", error.to_string(), 1).caused_by(error);
                (report, Some(failure))
            }
        },
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let terminal = output.terminal();
    if mode.json {
        let mut value = document(&report);
        if let Some(failure) = &failure {
            value["error"] = failure.document()["error"].clone();
        }
        writeln!(output, "{value}")?;
    } else {
        for item in &report.refreshed {
            writeln!(
                output,
                "{} skill at {}",
                if item.changed { "Refreshed" } else { "Current" },
                item.target.display()
            )?;
        }
        for target in &report.skipped {
            writeln!(output, "Skipped missing skill at {}", target.display())?;
        }
        for target in &report.conflicts {
            writeln!(
                output,
                "Preserved conflicting skill at {}",
                target.display()
            )?;
        }
        if !report.refreshed.is_empty() {
            tmt_cli_style::message::hint(
                &mut output,
                terminal,
                "reload or restart your agent to use the current skill; existing conversations can read tmt learn --skill",
            )?;
        }
        if let Some(failure) = &failure {
            failure.publish(mode)?;
        }
    }
    Ok(if failure.is_some() { 1 } else { 0 })
}
