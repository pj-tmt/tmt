//! Public projections deliberately omit binding markers and process evidence.

use super::Report;
use crate::output::{identity_document, table};
use serde_json::{Value, json};
use std::io::{self, Write};
use tmt_core::{binding::IdentityPresence, endpoint::PaneObservation, identity::Identity};

fn pane_document(pane: &PaneObservation) -> Value {
    let mut value = json!({"id": pane.id, "command": pane.command});
    if let Some(target) = &pane.target {
        value["target"] = target.clone().into();
    }
    if let Some(cwd) = &pane.cwd {
        value["cwd"] = cwd.clone().into();
    }
    value
}

fn bound_document(identity: &Identity, pane: &str) -> Value {
    json!({"bound": true, "id": identity.id, "name": identity.name,
        "pane": pane, "lifetime": identity.lifetime.as_str()})
}

fn presence_document(row: &IdentityPresence) -> Value {
    let mut value = identity_document(&row.identity);
    value["presence"] = row.presence.as_str().into();
    value["pane"] = row.pane.as_ref().map(|pane| pane.id.clone()).into();
    value["command"] = row
        .pane
        .as_ref()
        .map_or("", |pane| pane.command.as_str())
        .into();
    if let Some(pane) = &row.pane {
        let details = pane_document(pane);
        for key in ["target", "cwd"] {
            if let Some(detail) = details.get(key) {
                value[key] = detail.clone();
            }
        }
    }
    value
}

pub(super) fn document(report: &Report) -> Value {
    match report {
        Report::Bound(result) => bound_document(
            &result.presence.identity,
            &result.presence.pane.as_ref().expect("verified binding").id,
        ),
        Report::Caller {
            pane,
            identity,
            runtime,
        } => {
            let mut value = identity.as_ref().map_or_else(
                || json!({"bound": false, "pane": pane}),
                |identity| bound_document(identity, pane),
            );
            value["interfaceKind"] = "container".into();
            value["sessionState"] = runtime.as_str().into();
            value
        }
        Report::Unbound { pane, result } => json!({"unbound": true, "id": result.identity.id,
            "name": result.identity.name, "pane": pane, "lifetime": result.identity.lifetime.as_str(), "retired": result.retired}),
        Report::Removed(entry) => {
            json!({"removed": true, "identity": identity_document(&entry.identity)})
        }
        Report::Listed(rows) => {
            json!({"identities": rows.iter().map(presence_document).collect::<Vec<_>>()})
        }
        Report::Named { target, row } => {
            json!({"target": target, "identity": identity_document(&row.identity),
            "presence": row.presence.as_str(), "pane": row.pane.as_ref().map(pane_document)})
        }
        Report::Pane {
            target,
            pane,
            identity,
        } => json!({"target": target,
            "identity": identity.as_ref().map(identity_document), "pane": pane_document(pane)}),
    }
}

const HEADERS: [&str; 7] = [
    "NAME", "LIFETIME", "STATUS", "PANE", "TARGET", "CWD", "COMMAND",
];

#[cfg(test)]
mod caller_tests;

fn identity_row(identity: &Identity, status: &str, pane: Option<&PaneObservation>) -> [String; 7] {
    [
        identity.name.as_str(),
        identity.lifetime.as_str(),
        status,
        pane.map_or("-", |pane| pane.id.as_str()),
        pane.and_then(|pane| pane.target.as_deref()).unwrap_or("-"),
        pane.and_then(|pane| pane.cwd.as_deref()).unwrap_or("-"),
        pane.map_or("", |pane| pane.command.as_str()),
    ]
    .map(str::to_owned)
}

pub(super) fn text(output: &mut impl Write, report: &Report) -> io::Result<()> {
    match report {
        Report::Bound(result) => writeln!(
            output,
            "Bound {} identity '{}' on pane {}.",
            result.presence.identity.lifetime.as_str(),
            result.presence.identity.name,
            result.presence.pane.as_ref().expect("verified binding").id
        ),
        Report::Caller {
            pane,
            identity: Some(identity),
            ..
        } => writeln!(
            output,
            "Bound {} identity '{}' on pane {pane}.",
            identity.lifetime.as_str(),
            identity.name
        ),
        Report::Caller {
            pane,
            identity: None,
            ..
        } => writeln!(output, "Pane {pane} is unbound."),
        Report::Unbound { pane, result } => writeln!(
            output,
            "Unbound '{}' from pane {pane}; identity {}.",
            result.identity.name,
            if result.retired {
                "retired"
            } else {
                "saved offline"
            }
        ),
        Report::Removed(entry) => writeln!(
            output,
            "Removed identity '{}'. Exchanges are retained.",
            entry.identity.name
        ),
        Report::Listed(rows) if rows.is_empty() => writeln!(output, "No identities found."),
        Report::Listed(rows) => table::write(
            output,
            HEADERS,
            rows.iter()
                .map(|row| identity_row(&row.identity, row.presence.as_str(), row.pane.as_ref())),
        ),
        Report::Named { row, .. } => table::write(
            output,
            HEADERS,
            [identity_row(
                &row.identity,
                row.presence.as_str(),
                row.pane.as_ref(),
            )],
        ),
        Report::Pane { pane, identity, .. } => {
            writeln!(
                output,
                "Pane: {}\nCWD:  {}\nCMD:  {}",
                pane.id,
                pane.cwd.as_deref().unwrap_or("-"),
                pane.command
            )?;
            if let Some(identity) = identity {
                table::write(
                    output,
                    HEADERS,
                    [identity_row(identity, "active", Some(pane))],
                )
            } else {
                writeln!(output, "Pane has no active global identity.")
            }
        }
    }
}
