//! `tmt driver`: approve, list and remove consented host drivers (#570).
//! Every check and the registry belong to
//! `tmt_adapters::host::external::registry`; this is its front end. A driver
//! is approved only after the user sees what it declares and consents.

use crate::consent;
use crate::invocation::{DriverRequest, OutputMode};
use crate::output::Failure;
use serde_json::json;
use std::io::{self, Write};
use std::path::Path;
use tmt_adapters::{
    config::ConfigPaths,
    host::external::registry::{self, DriverRecord, RegistryError},
    process::UnixCommandRunner,
};

const CONSENT_REQUIRED: &str = "DRIVER_CONSENT_REQUIRED";
const NOT_FOUND: &str = "DRIVER_NOT_FOUND";

fn failure(error: RegistryError) -> Failure {
    Failure::new(error.code(), error.to_string(), 1)
}

fn record_json(record: &DriverRecord) -> serde_json::Value {
    json!({
        "name": record.name,
        "version": record.capabilities.version,
        "path": record.path,
        "sha256": record.digest,
        "protocol": record.protocol,
        "ops": record.capabilities.ops,
        "callerEnv": record.capabilities.caller_env,
        "approvedAtMs": record.approved_at_ms,
    })
}

/// What the user approves: the executable, its digest, and what TMT will
/// run it for.
fn details(record: &DriverRecord) -> String {
    let capabilities = &record.capabilities;
    let reads = if capabilities.caller_env.is_empty() {
        "nothing".to_owned()
    } else {
        capabilities.caller_env.join(", ")
    };
    format!(
        "Host driver {name} {version} (protocol {protocol})\n  \
         Executable: {path}\n  \
         SHA-256:    {digest}\n  \
         Operations: {ops}\n  \
         Reads:      {reads}\n\
         TMT will run this program for these operations whenever it works with {name} panes.",
        name = record.name,
        version = capabilities.version,
        protocol = record.protocol,
        path = record.path.display(),
        digest = record.digest,
        ops = capabilities.ops.join(", "),
    )
}

/// The document, whether the human text reports a change, and the text.
type Answer = (serde_json::Value, bool, String);

fn install(
    paths: &ConfigPaths,
    path: &str,
    yes: bool,
    mode: OutputMode,
) -> Result<Answer, Failure> {
    // Every refusal comes before the question.
    let record = registry::inspect(&paths.global_dir, Path::new(path), &UnixCommandRunner)
        .map_err(failure)?;
    let name = &record.name;
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if !mode.json {
        writeln!(output, "{}", details(&record)).map_err(|error| {
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
                "Approving host driver {name} requires explicit --yes; nothing changed."
            ),
            question: &format!("Approve host driver {name}"),
            declined: &format!("Host driver {name} was not approved; nothing changed."),
        },
        |error| Failure::new("DRIVER_IO_ERROR", "Could not ask for consent.", 1).caused_by(error),
    )?;
    if !consented {
        return Ok((json!({"approved": null}), false, String::new()));
    }
    let record = registry::commit(&paths.global_dir, record).map_err(failure)?;
    let text = format!(
        "Approved host driver {name}. Remove it with: tmt driver rm {name}",
        name = record.name
    );
    Ok((json!({"approved": record_json(&record)}), true, text))
}

fn list_approved(paths: &ConfigPaths) -> Result<Answer, Failure> {
    let records = registry::read(&paths.global_dir).map_err(failure)?;
    let rows: Vec<(DriverRecord, registry::ApprovalState)> = records
        .into_iter()
        .map(|record| {
            let state = registry::state(&record);
            (record, state)
        })
        .collect();
    let text = if rows.is_empty() {
        "No host drivers are approved.".to_owned()
    } else {
        rows.iter()
            .map(|(record, state)| {
                let mut line = format!(
                    "{}  {}  {}  {}",
                    record.name,
                    record.capabilities.version,
                    state.as_str(),
                    record.path.display()
                );
                if *state != registry::ApprovalState::Ok {
                    line.push_str(&format!(
                        "\n  Approve it again with: tmt driver install {}",
                        record.path.display()
                    ));
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let drivers: Vec<serde_json::Value> = rows
        .iter()
        .map(|(record, state)| {
            let mut value = record_json(record);
            value["state"] = json!(state.as_str());
            value
        })
        .collect();
    Ok((json!({"drivers": drivers}), false, text))
}

fn remove(paths: &ConfigPaths, name: &str) -> Result<Answer, Failure> {
    // Withdrawing trust is always safe, so nothing is asked.
    if !registry::remove(&paths.global_dir, name).map_err(failure)? {
        return Err(Failure::new(
            NOT_FOUND,
            format!("No host driver named {name} is approved."),
            1,
        ));
    }
    let text = format!(
        "Removed host driver {name}. Its bindings stay stored and read as unavailable until it is approved again."
    );
    Ok((json!({"removed": name}), true, text))
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
        Ok((value, changed, text)) => {
            let mut output = tmt_cli_style::stream::stdout(mode.json);
            let terminal = output.terminal();
            if mode.json {
                writeln!(output, "{value}")?;
            } else if changed {
                tmt_cli_style::message::success(&mut output, terminal, &text)?;
            } else if !text.is_empty() {
                writeln!(output, "{text}")?;
            }
            Ok(0)
        }
        Err(error) => error.publish(mode),
    }
}
