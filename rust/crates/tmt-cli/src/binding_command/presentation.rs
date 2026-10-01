//! Public projections deliberately omit binding markers and process evidence.
//! Human output follows docs/cli-style.md: `ls` is agent-first, with a state
//! mark, the name, one `driver:identifier` address and the working folder.

use super::{ListedRow, Report};
use crate::output::identity_document;
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::Path,
};
use tmt_adapters::host::PaneRefresh;
use tmt_cli_style::{
    Terminal, Token,
    list::{self, Section},
    mark::Mark,
    message,
    table::{Cell, Column, Table},
    value,
};
use tmt_core::{
    binding::{IdentityPresence, Presence, session::RememberedSession, session::RuntimeState},
    endpoint::PaneObservation,
    host::HostKind,
    identity::{Identity, Lifetime},
};

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
            ..
        } => {
            let mut value = identity.as_ref().map_or_else(
                || json!({"bound": false, "pane": pane}),
                |identity| bound_document(identity, pane),
            );
            value["interfaceKind"] = "container".into();
            value["sessionState"] = runtime.as_str().into();
            value
        }
        Report::Unbound { pane, result, .. } => json!({"unbound": true, "id": result.identity.id,
            "name": result.identity.name, "pane": pane, "lifetime": result.identity.lifetime.as_str(), "retired": result.retired}),
        Report::Removed(entry) => {
            json!({"removed": true, "identity": identity_document(&entry.identity)})
        }
        Report::Renamed { result, pane } => json!({
            "renamed": result.changed(),
            "previousName": result.previous.name,
            "identity": identity_document(&result.identity),
            "pane": pane.as_ref().map(|(id, refresh)| json!({
                "id": id, "updated": *refresh == PaneRefresh::Updated,
            })),
        }),
        Report::Listed { rows, .. } => json!({"identities": rows.iter().map(|row| {
            let mut value = presence_document(&row.presence);
            value["session"] = json!({"activity":row.activity});
            if let Some(resume) = &row.resume {
                value["resume"] = resume.clone();
            }
            with_address(value, address(&row.presence, row.remembered.as_ref()))
        }).collect::<Vec<_>>()}),
        Report::Named {
            target,
            row,
            remembered,
        } => with_address(
            json!({"target": target, "identity": identity_document(&row.identity),
            "presence": row.presence.as_str(), "pane": row.pane.as_ref().map(pane_document)}),
            address(row, remembered.as_ref()),
        ),
        Report::Pane {
            target,
            pane,
            identity,
            ..
        } => json!({"target": target,
            "identity": identity.as_ref().map(identity_document), "pane": pane_document(pane)}),
    }
}

/// Additive `address`/`driver` keys, full values; existing keys keep their order.
fn with_address(mut value: Value, address: Option<Address>) -> Value {
    value["address"] = address.as_ref().map(Address::full).into();
    value["driver"] = address.map(|address| address.driver).into();
    value
}

#[cfg(test)]
mod caller_tests;
#[cfg(test)]
mod list_tests;
#[cfg(test)]
mod rename_tests;

/// Foreground commands that mean a bound pane has no agent in it. A pane
/// running anything else is treated as an agent, even one started without
/// `tmt run`, whose runtime state TMT cannot see.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "fish", "dash", "ksh", "tcsh", "nu"];

fn is_shell(command: &str) -> bool {
    // A login shell reports itself with a leading dash (`-zsh`).
    SHELLS.contains(&command.strip_prefix('-').unwrap_or(command))
}

/// What a row's leading mark says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// An agent is running in the bound pane.
    Running,
    /// The bound pane has only a shell in it.
    Shell,
    /// No live pane.
    Offline,
}

impl State {
    fn mark(self) -> Mark {
        match self {
            Self::Running => Mark::Running,
            Self::Shell => Mark::Idle,
            Self::Offline => Mark::Offline,
        }
    }
}

fn running(row: &IdentityPresence) -> bool {
    row.binding
        .as_ref()
        .is_some_and(|binding| binding.session.state == RuntimeState::Running)
}

fn state(row: &IdentityPresence) -> State {
    match (&row.presence, &row.pane) {
        (Presence::Active, Some(pane)) if running(row) || !is_shell(&pane.command) => {
            State::Running
        }
        (Presence::Active, Some(_)) => State::Shell,
        _ => State::Offline,
    }
}

/// Where an agent lives: its driver's session when TMT knows it, otherwise
/// the tmux pane. Human output shortens the identifier; JSON keeps it whole.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Address {
    driver: String,
    identifier: String,
}

impl Address {
    fn full(&self) -> String {
        format!("{}:{}", self.driver, self.identifier)
    }

    fn short(&self) -> String {
        value::address(&self.driver, &self.identifier)
    }
}

fn address(row: &IdentityPresence, remembered: Option<&RememberedSession>) -> Option<Address> {
    let session = |session: &RememberedSession| Address {
        driver: session.harness.as_str().into(),
        identifier: session.provider_session.as_str().into(),
    };
    // An active row's pane is on its binding's host.
    let live = row
        .pane
        .as_ref()
        .zip(row.binding.as_ref())
        .filter(|_| row.presence == Presence::Active);
    // A running agent's own session, only when the binding observed that
    // session: Codex may start before it discloses its thread.
    let observed = row
        .binding
        .as_ref()
        .and_then(|binding| binding.session.key.as_ref())
        .and_then(|key| key.provider_session.as_ref());
    match (live, remembered) {
        (Some(_), Some(remembered))
            if running(row) && observed == Some(&remembered.provider_session) =>
        {
            Some(session(remembered))
        }
        (Some((pane, binding)), _) => Some(Address {
            driver: binding.server.host.as_str().into(),
            identifier: binding
                .server
                .host
                .pane_address(&pane.id, pane.target.as_deref())
                .into(),
        }),
        (None, Some(remembered)) => Some(session(remembered)),
        (None, None) => None,
    }
}

/// The trailing action, only where one is possible.
fn action(row: &IdentityPresence, remembered: Option<&RememberedSession>) -> Option<String> {
    let state = state(row);
    match remembered {
        Some(session) if session.stale_at_ms.is_some() => Some("stale".into()),
        Some(_) if state != State::Running => Some(format!(
            "{} tmt resume {}",
            Mark::Resumable.symbol(),
            row.identity.name
        )),
        _ => (state == State::Shell).then(|| "shell".into()),
    }
}

fn folder(pane: Option<&PaneObservation>, home: Option<&Path>) -> String {
    pane.and_then(|pane| pane.cwd.as_deref())
        .map_or_else(String::new, |cwd| value::home_path(Path::new(cwd), home))
}

const COLUMNS: [Column; 4] = [Column::Fixed, Column::Name, Column::Fixed, Column::Detail];

fn row_cells(row: &ListedRow, home: Option<&Path>) -> [Cell; 4] {
    let presence = &row.presence;
    let mark = state(presence).mark();
    let address = address(presence, row.remembered.as_ref());
    [
        Cell::styled(mark.symbol(), mark.token()),
        presence.identity.name.as_str().into(),
        address.map_or_else(
            || Cell::styled("-", Token::Dim),
            |address| Cell::styled(address.short(), crate::driver_style::token(&address.driver)),
        ),
        folder(presence.pane.as_ref(), home).into(),
    ]
}

fn write_listing(
    output: &mut impl Write,
    terminal: Terminal,
    rows: &[ListedRow],
    all: bool,
    home: Option<&Path>,
) -> io::Result<()> {
    if rows.is_empty() {
        writeln!(output, "No identities found.")?;
        return message::hint(output, terminal, "tmt name <name>");
    }
    let mut built = Vec::new();
    for (title, lifetime) in [
        ("saved", Lifetime::Saved),
        ("temporary", Lifetime::Temporary),
    ] {
        let mut members: Vec<&ListedRow> = rows
            .iter()
            .filter(|row| row.presence.identity.lifetime == lifetime)
            .collect();
        if members.is_empty() {
            continue;
        }
        members.sort_by(|a, b| {
            a.presence
                .identity
                .canonical_name
                .cmp(&b.presence.identity.canonical_name)
        });
        let mut table = Table::new(&COLUMNS);
        let mut folded = Vec::new();
        for row in &members {
            match action(&row.presence, row.remembered.as_ref()) {
                Some(action) => {
                    table.row_with_action(row_cells(row, home), &action);
                }
                // Offline with nothing to do: one line for all of them.
                None if !all && state(&row.presence) == State::Offline => {
                    folded.push(row.presence.identity.name.as_str());
                }
                None => {
                    table.row(row_cells(row, home));
                }
            }
        }
        let note = (!folded.is_empty()).then(|| format!("offline: {}", folded.join(" · ")));
        built.push((title, members.len(), table, note));
    }
    let sections: Vec<Section<'_>> = built
        .iter()
        .map(|(title, count, table, note)| Section {
            title,
            count: Some(*count),
            rows: table.clone(),
            note: note.as_deref(),
            hint: None,
        })
        .collect();
    list::write(output, terminal, &sections)
}

/// Full values for one identity or pane: the tmux location and the whole
/// session identifier live here, not in the list.
fn identity_fields(
    row: &IdentityPresence,
    remembered: Option<&RememberedSession>,
) -> Vec<(&'static str, String)> {
    let dash = || "-".to_owned();
    let pane = row.pane.as_ref();
    let mut fields = vec![
        ("lifetime", row.identity.lifetime.as_str().to_owned()),
        (
            "state",
            format!("{} {}", state(row).mark().symbol(), row.presence.as_str()),
        ),
        (
            "address",
            address(row, remembered).map_or_else(dash, |address| address.full()),
        ),
        ("pane", pane.map_or_else(dash, |pane| pane.id.clone())),
        (
            "target",
            pane.and_then(|pane| pane.target.clone())
                .unwrap_or_else(dash),
        ),
        (
            "cwd",
            pane.and_then(|pane| pane.cwd.clone()).unwrap_or_else(dash),
        ),
        (
            "command",
            pane.map_or_else(dash, |pane| pane.command.clone()),
        ),
    ];
    if let Some(session) = remembered {
        let mut resume = format!(
            "{}:{} ({})",
            session.harness.as_str(),
            session.provider_session.as_str(),
            session.mode.as_str()
        );
        if session.stale_at_ms.is_some() {
            resume.push_str(", stale");
        }
        fields.push(("resume", resume));
    }
    fields
}

pub(super) fn text(output: &mut impl Write, terminal: Terminal, report: &Report) -> io::Result<()> {
    match report {
        Report::Renamed { result, .. } if result.changed() => message::success(
            output,
            terminal,
            &format!(
                "Renamed {} to {}",
                result.previous.name, result.identity.name
            ),
        ),
        Report::Renamed { result, .. } => writeln!(
            output,
            "{} already has this name",
            tmt_cli_style::table::escape(&result.identity.name)
        ),
        Report::Bound(result) => message::success(
            output,
            terminal,
            &format!(
                "Bound {} identity '{}' on pane {}",
                result.presence.identity.lifetime.as_str(),
                result.presence.identity.name,
                result
                    .presence
                    .pane
                    .as_ref()
                    .map(|pane| HostKind::label(&pane.id, pane.target.as_deref()))
                    .expect("verified binding")
            ),
        ),
        Report::Caller {
            label: pane,
            identity: Some(identity),
            ..
        } => writeln!(
            output,
            "{} ({}) on pane {pane}",
            identity.name,
            identity.lifetime.as_str()
        ),
        Report::Caller {
            label: pane,
            identity: None,
            ..
        } => {
            writeln!(output, "Pane {pane} is unbound")?;
            message::hint(output, terminal, "tmt name <name>")
        }
        Report::Unbound {
            label: pane,
            result,
            ..
        } => message::success(
            output,
            terminal,
            &format!(
                "Unbound '{}' from pane {pane}; identity {}",
                result.identity.name,
                if result.retired {
                    "retired"
                } else {
                    "saved offline"
                }
            ),
        ),
        Report::Removed(entry) => message::success(
            output,
            terminal,
            &format!(
                "Removed identity '{}'; exchanges are retained",
                entry.identity.name
            ),
        ),
        Report::Listed { rows, scope } => {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
            write_listing(output, terminal, rows, scope.all, home.as_deref())?;
            // A name kept from before Herdr that now reads as a pane target.
            match rows
                .iter()
                .find(|row| tmt_core::names::is_pane_target(&row.presence.identity.name))
            {
                Some(row) => message::hint(
                    output,
                    terminal,
                    &format!(
                        "{} reads as a pane target; rename it: tmt rename {} <name>",
                        row.presence.identity.name,
                        crate::output::shell_word(&row.presence.identity.name)
                    ),
                ),
                None => Ok(()),
            }
        }
        Report::Named {
            row, remembered, ..
        } => tmt_cli_style::detail::write(
            output,
            terminal,
            &row.identity.name,
            &identity_fields(row, remembered.as_ref()),
        ),
        Report::Pane {
            pane,
            identity,
            remembered,
            ..
        } => match identity {
            Some(identity) => {
                let row = IdentityPresence {
                    identity: identity.clone(),
                    presence: Presence::Active,
                    pane: Some(pane.clone()),
                    binding: None,
                };
                tmt_cli_style::detail::write(
                    output,
                    terminal,
                    &identity.name,
                    &identity_fields(&row, remembered.as_ref()),
                )
            }
            None => {
                let label = HostKind::label(&pane.id, pane.target.as_deref());
                tmt_cli_style::detail::write(
                    output,
                    terminal,
                    label,
                    &[
                        ("target", pane.target.clone().unwrap_or_else(|| "-".into())),
                        ("cwd", pane.cwd.clone().unwrap_or_else(|| "-".into())),
                        ("command", pane.command.clone()),
                    ],
                )?;
                message::hint(output, terminal, &format!("tmt add {label} <name>"))
            }
        },
    }
}

/// Non-fatal problems after a committed change, for standard error.
pub(super) fn warnings(
    output: &mut tmt_cli_style::stream::Stream<impl Write>,
    report: &Report,
) -> io::Result<()> {
    let terminal = output.terminal();
    match report {
        Report::Renamed {
            result,
            pane: Some((pane, PaneRefresh::Failed)),
        } => message::warning(
            output,
            terminal,
            &format!(
                "Pane {pane} still shows the previous name {}.",
                result.previous.name
            ),
            Some(&format!(
                "In pane {pane}, run: tmt this {}",
                crate::output::shell_word(&result.identity.name)
            )),
        ),
        _ => Ok(()),
    }
}
