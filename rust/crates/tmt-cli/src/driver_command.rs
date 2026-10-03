//! `tmt driver`: approve, list and remove consented drivers (#570).
//! Every check and the registry belong to
//! `tmt_adapters::driver_protocol::registry`; this is its front end. A driver
//! is approved only after the user sees what it declares and consents.

use crate::consent;
use crate::invocation::{DriverRequest, OutputMode};
use crate::output::Failure;
use serde_json::json;
use std::io::{self, Write};
use std::path::Path;
use tmt_adapters::{
    config::ConfigPaths,
    driver_protocol::{
        Declaration,
        registry::{self, DriverRecord, RegistryError},
    },
    process::UnixCommandRunner,
};
use tmt_cli_style::{
    detail, list,
    mark::Mark,
    palette::Terminal,
    table::{Cell, Column, Table},
    value,
};

const CONSENT_REQUIRED: &str = "DRIVER_CONSENT_REQUIRED";
const NOT_FOUND: &str = "DRIVER_NOT_FOUND";

fn failure(error: RegistryError) -> Failure {
    Failure::new(error.code(), error.to_string(), 1)
}

fn record_json(record: &DriverRecord) -> serde_json::Value {
    let mut value = json!({
        "name": record.name,
        "version": record.capabilities.version(),
        "path": record.path,
        "sha256": record.digest,
        "protocol": record.protocol,
        "ops": record.capabilities.ops(),
        "callerEnv": record.capabilities.host().map(|value| &value.caller_env),
        "approvedAtMs": record.approved_at_ms,
    });
    if let Declaration::Runtime(capabilities) = &record.capabilities {
        value.as_object_mut().unwrap().remove("callerEnv");
        value["kind"] = json!("runtime");
        value["executables"] = json!(capabilities.executables);
        value["claims"] = json!(capabilities.claims);
        value["env"] = json!(capabilities.env);
        value["sessionEnv"] = json!(capabilities.session_env);
        value["hooks"] = json!(capabilities.hooks);
        value["locations"] = json!(record.locations);
    }
    value
}

/// What the user approves: the executable, its digest, and what TMT will
/// run it for, as one detail view.
fn write_details(
    output: &mut impl Write,
    terminal: Terminal,
    record: &DriverRecord,
) -> io::Result<()> {
    if let Declaration::Runtime(capabilities) = &record.capabilities {
        let locations = record.locations.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Runtime locations are not resolved; approve the driver again.",
            )
        })?;
        return detail::write(
            output,
            terminal,
            &format!("Runtime driver {}", record.name),
            &[
                ("version", capabilities.version.clone()),
                ("protocol", record.protocol.to_string()),
                ("executable", record.path.display().to_string()),
                ("sha256", record.digest.clone()),
                ("operations", capabilities.ops.join(", ")),
                ("executables", capabilities.executables.join(", ")),
                ("claims", capabilities.claims.to_string()),
                (
                    "argv rule",
                    "Direct launch; argv[0] must be a declared executable, never a shell.".into(),
                ),
                ("env", capabilities.env.join(", ")),
                (
                    "sessionEnv",
                    capabilities
                        .session_env
                        .clone()
                        .unwrap_or_else(|| "none".into()),
                ),
                (
                    "hooks",
                    capabilities
                        .hooks
                        .as_ref()
                        .map(|hooks| {
                            hooks
                                .events
                                .iter()
                                .map(|event| event.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_else(|| "none".into()),
                ),
                ("configDirs", locations.config_dirs.join(", ")),
                ("skills", locations.skills.clone()),
                (
                    "hookSettings",
                    locations
                        .hook_settings
                        .clone()
                        .unwrap_or_else(|| "none".into()),
                ),
                (
                    "transcriptRoot",
                    locations
                        .transcript_root
                        .clone()
                        .unwrap_or_else(|| "none".into()),
                ),
            ],
        );
    }
    let capabilities = record.capabilities.host().expect("host declaration");
    let reads = if capabilities.caller_env.is_empty() {
        "nothing".to_owned()
    } else {
        capabilities.caller_env.join(", ")
    };
    detail::write(
        output,
        terminal,
        &format!("Host driver {}", record.name),
        &[
            ("version", capabilities.version.clone()),
            ("protocol", record.protocol.to_string()),
            ("executable", record.path.display().to_string()),
            ("sha256", record.digest.clone()),
            ("operations", capabilities.ops.join(", ")),
            ("reads", reads),
        ],
    )?;
    writeln!(
        output,
        "TMT will run this program for these operations whenever it works with {} panes.",
        record.name
    )
}

/// One approved driver, its state, and why when it isn't `ok`.
type Listed = (DriverRecord, registry::ApprovalState, Option<String>);

/// What a run prints for a human.
enum Human {
    Nothing,
    Success(String),
    Listing(Vec<Listed>),
}

/// The JSON document and the human output.
type Answer = (serde_json::Value, Human);

/// The running `tmt`, whose release ships the first-party drivers.
fn running_tmt() -> Result<std::path::PathBuf, Failure> {
    std::env::current_exe().map_err(|error| {
        Failure::new("DRIVER_IO_ERROR", "Could not locate tmt.", 1).caused_by(error)
    })
}

fn install(
    paths: &ConfigPaths,
    path: &str,
    yes: bool,
    mode: OutputMode,
) -> Result<Answer, Failure> {
    // Every refusal comes before the question. A bare name (no `/`) names
    // a first-party driver shipped with this tmt; anything else is a path.
    let record = if !path.contains('/') && registry::is_first_party(path) {
        registry::inspect_first_party(&paths.global_dir, path, &running_tmt()?, &UnixCommandRunner)
    } else {
        registry::inspect(&paths.global_dir, Path::new(path), &UnixCommandRunner)
    }
    .map_err(failure)?;
    let name = &record.name;
    let kind = record.capabilities.kind();
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if !mode.json {
        let terminal = output.terminal();
        write_details(&mut output, terminal, &record).map_err(|error| {
            Failure::new("DRIVER_IO_ERROR", "Could not show the driver.", 1).caused_by(error)
        })?;
    }
    let consented = consent::ask(
        &mut output,
        yes,
        mode,
        consent::Consent {
            code: CONSENT_REQUIRED,
            refusal: &format!(
                "Approving {kind} driver {name} requires explicit --yes; nothing changed."
            ),
            question: &format!("Approve {kind} driver {name}"),
            declined: &format!(
                "{} driver {name} was not approved; nothing changed.",
                if kind == "host" { "Host" } else { "Runtime" }
            ),
        },
        |error| Failure::new("DRIVER_IO_ERROR", "Could not ask for consent.", 1).caused_by(error),
    )?;
    if !consented {
        return Ok((json!({"approved": null}), Human::Nothing));
    }
    let record = registry::commit(&paths.global_dir, record).map_err(failure)?;
    let text = format!(
        "Approved {kind} driver {name}. Remove it with: tmt driver rm {name}",
        kind = record.capabilities.kind(),
        name = record.name
    );
    Ok((
        json!({"approved": record_json(&record)}),
        Human::Success(text),
    ))
}

fn list_approved(paths: &ConfigPaths) -> Result<Answer, Failure> {
    let records = registry::read(&paths.global_dir).map_err(failure)?;
    let tmt = running_tmt()?;
    let rows: Vec<Listed> = records
        .into_iter()
        .map(|record| {
            let (state, reason) =
                registry::state(&paths.global_dir, &record, &tmt, &UnixCommandRunner);
            (record, state, reason)
        })
        .collect();
    let drivers: Vec<serde_json::Value> = rows
        .iter()
        .map(|(record, state, reason)| {
            let mut value = record_json(record);
            value["state"] = json!(state.as_str());
            value["reason"] = json!(reason);
            value
        })
        .collect();
    Ok((json!({"drivers": drivers}), Human::Listing(rows)))
}

fn remove(paths: &ConfigPaths, name: &str) -> Result<Answer, Failure> {
    // Withdrawing trust is always safe, so nothing is asked.
    let kind = registry::read(&paths.global_dir)
        .map_err(failure)?
        .into_iter()
        .find(|record| record.name == name)
        .map(|record| record.capabilities.kind().to_owned());
    if !registry::remove(&paths.global_dir, name).map_err(failure)? {
        return Err(Failure::new(
            NOT_FOUND,
            format!("No host driver named {name} is approved."),
            1,
        ));
    }
    let text = if kind.as_deref() == Some("runtime") {
        format!("Removed runtime driver {name}.")
    } else {
        format!(
            "Removed host driver {name}. Its bindings stay stored and read as unavailable until it is approved again."
        )
    };
    Ok((json!({"removed": name}), Human::Success(text)))
}

/// A driver's leading mark: `●` runs as approved, `✗` changed since its
/// approval so it won't run, `○` gone.
fn mark(state: registry::ApprovalState) -> Mark {
    match state {
        registry::ApprovalState::Ok => Mark::Running,
        registry::ApprovalState::Changed => Mark::Failed,
        registry::ApprovalState::Missing => Mark::Offline,
    }
}

fn write_listing(output: &mut impl Write, terminal: Terminal, rows: &[Listed]) -> io::Result<()> {
    if rows.is_empty() {
        return writeln!(output, "No host drivers are approved.");
    }
    let home = std::env::home_dir();
    // Why each driver that can't run as approved can't, in one dimmed line.
    let reasons = rows
        .iter()
        .filter_map(|(record, _, reason)| {
            reason
                .as_ref()
                .map(|reason| format!("{}: {reason}", record.name))
        })
        .collect::<Vec<_>>();
    let reasons = (!reasons.is_empty()).then(|| reasons.join(" · "));
    let mut table = Table::new(&[
        Column::Fixed,
        Column::Name,
        Column::Fixed,
        Column::Fixed,
        Column::Detail,
    ]);
    for (record, state, _) in rows {
        let mark = mark(*state);
        let cells: [Cell; 5] = [
            Cell::styled(mark.symbol(), mark.token()),
            record.name.as_str().into(),
            record.capabilities.version().into(),
            state.as_str().into(),
            value::home_path(&record.path, home.as_deref()).into(),
        ];
        if *state == registry::ApprovalState::Ok {
            table.row(cells);
        } else {
            // Approving it again is the one thing to do.
            let again = match record.source {
                registry::DriverSource::FirstParty => record.name.clone(),
                registry::DriverSource::Path => record.path.display().to_string(),
            };
            table.row_with_action(cells, &format!("tmt driver install {again}"));
        }
    }
    list::write(
        output,
        terminal,
        &[list::Section {
            title: if rows
                .iter()
                .any(|(record, _, _)| record.capabilities.kind() == "runtime")
            {
                "drivers"
            } else {
                "host drivers"
            },
            count: Some(rows.len()),
            rows: table,
            note: reasons.as_deref(),
            hint: None,
        }],
    )
}

pub fn execute(request: DriverRequest, mode: OutputMode) -> io::Result<u8> {
    let result = (|| -> Result<Answer, Failure> {
        let paths = ConfigPaths::discover()?;
        match request {
            DriverRequest::Install { path, yes } => install(&paths, &path, yes, mode),
            DriverRequest::List => list_approved(&paths),
            DriverRequest::Remove(name) => remove(&paths, &name),
        }
    })();
    match result {
        Ok((value, human)) => {
            let mut output = tmt_cli_style::stream::stdout(mode.json);
            let terminal = output.terminal();
            if mode.json {
                writeln!(output, "{value}")?;
            } else {
                match human {
                    Human::Nothing => {}
                    Human::Success(text) => {
                        tmt_cli_style::message::success(&mut output, terminal, &text)?
                    }
                    Human::Listing(rows) => write_listing(&mut output, terminal, &rows)?,
                }
            }
            Ok(0)
        }
        Err(error) => error.publish(mode),
    }
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Approved {kind} driver {name}. Remove it with: tmt driver rm {name}",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core("tmt driver install {again}", &[""], &[]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_without_locations_refuses_disclosure_without_panicking() {
        let record: DriverRecord = serde_json::from_value(json!({
            "name": "agent", "path": "/tmp/driver", "digest": "test", "protocol": 1,
            "fingerprint": {"device": 0, "inode": 0, "size": 0, "modifiedNs": 0,
                "changedNs": 0, "uid": 0, "mode": 0}, "approvedAtMs": 0,
            "capabilities": {"kind": "runtime", "name": "agent", "version": "test",
                "protocols": [1], "ops": ["locations"], "executables": ["agent"]}
        }))
        .unwrap();
        let mut output = Vec::new();
        let error = write_details(&mut output, Terminal::PLAIN, &record).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("approve the driver again"));
        assert!(output.is_empty());
    }
}
