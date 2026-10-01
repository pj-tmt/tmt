//! Pane and server evidence from `pane list`, `pane process-info` and `ps`.

use super::{Herdr, HerdrError, marker};
use crate::process::{
    CommandRunner,
    ps::query_ps,
    runtime::{ProcessObservation, observe_runtime_process},
};
use serde_json::{Map, Value};
use std::{collections::HashMap, ffi::OsString, time::Instant};
use tmt_core::{
    endpoint::{PaneObservation, ProcessIncarnation, ServerEvidence, valid_process_id},
    host::HostKind,
};

/// Bound work before any per-pane call; an oversized scope is not evidence.
const MAX_SCOPE_PANES: usize = 256;

/// One pane from `pane list`.
#[derive(Debug, Clone)]
pub(super) struct ListedPane {
    pub pane_id: String,
    pub terminal_id: String,
    pub cwd: Option<String>,
    pub tokens: Option<Map<String, Value>>,
}

/// A server's socket and its process, which together name one incarnation.
/// The process is core's own observation, never Herdr's report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Incarnation {
    pub socket: String,
    pub process: ProcessIncarnation,
}

impl Incarnation {
    pub fn matches(&self, server: &ServerEvidence) -> bool {
        server.host == HostKind::Herdr
            && server.socket_path == self.socket
            && server.server_pid == self.process.pid()
            && server.server_start_time == self.process.start_identity()
    }
}

pub(super) struct Observed {
    pub incarnation: Incarnation,
    pub panes: Vec<PaneObservation>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

pub(super) fn listed(result: &Value) -> Result<Vec<ListedPane>, HerdrError> {
    let rows = result["panes"]
        .as_array()
        .ok_or_else(|| HerdrError::evidence("Herdr pane list is malformed"))?;
    let mut panes: Vec<ListedPane> = Vec::with_capacity(rows.len());
    for row in rows {
        let pane_id = text(row, "pane_id").filter(|id| HostKind::Herdr.is_target(id));
        let terminal_id = text(row, "terminal_id").filter(|id| HostKind::Herdr.is_pane_id(id));
        let (Some(pane_id), Some(terminal_id)) = (pane_id, terminal_id) else {
            return Err(HerdrError::evidence("Herdr pane list has an invalid pane"));
        };
        if panes
            .iter()
            .any(|pane| pane.pane_id == pane_id || pane.terminal_id == terminal_id)
        {
            return Err(HerdrError::evidence("Herdr pane list repeats a pane"));
        }
        panes.push(ListedPane {
            pane_id,
            terminal_id,
            cwd: text(row, "foreground_cwd").or_else(|| text(row, "cwd")),
            tokens: row["tokens"].as_object().cloned(),
        });
    }
    Ok(panes)
}

/// One pane's shell and the first foreground process, or `None` when the
/// pane closed after the list was taken.
struct Process {
    shell_pid: u64,
    command: String,
}

impl<R: CommandRunner> Herdr<R> {
    pub(super) fn list(
        &self,
        socket: &str,
        deadline: Instant,
    ) -> Result<Vec<ListedPane>, HerdrError> {
        listed(&self.call(Some(socket), &["pane", "list"], deadline)?)
    }

    fn process(
        &self,
        socket: &str,
        pane_id: &str,
        deadline: Instant,
    ) -> Result<Option<Process>, HerdrError> {
        let result = match self.call(
            Some(socket),
            &["pane", "process-info", "--pane", pane_id],
            deadline,
        ) {
            Ok(result) => result,
            Err(error) if error.code() == Some("pane_not_found") => return Ok(None),
            Err(error) => return Err(error),
        };
        let info = &result["process_info"];
        let shell_pid = info["shell_pid"]
            .as_u64()
            .filter(|pid| valid_process_id(*pid))
            .ok_or_else(|| HerdrError::evidence("Herdr pane process evidence is incomplete"))?;
        let command = info["foreground_processes"]
            .as_array()
            .and_then(|processes| processes.first())
            .and_then(|process| text(process, "name"))
            .unwrap_or_default();
        Ok(Some(Process { shell_pid, command }))
    }

    /// Each shell's parent, from one bounded `ps` call.
    fn parents(&self, shells: &[u64], deadline: Instant) -> Result<HashMap<u64, u64>, HerdrError> {
        let list = shells
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let args: Vec<OsString> = ["-o", "pid=,ppid=", "-p", &list]
            .into_iter()
            .map(Into::into)
            .collect();
        let output =
            query_ps(self.runner(), &args, deadline, 64 * 1024).map_err(HerdrError::command)?;
        let text = String::from_utf8_lossy(&output.stdout);
        let mut parents = HashMap::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let fields: Vec<u64> = line
                .split_whitespace()
                .map(|field| field.parse().ok().filter(|pid| valid_process_id(*pid)))
                .collect::<Option<_>>()
                .ok_or_else(|| {
                    HerdrError::evidence("Herdr server process evidence is malformed")
                })?;
            let [pid, parent] = fields[..] else {
                return Err(HerdrError::evidence(
                    "Herdr server process evidence is malformed",
                ));
            };
            parents.insert(pid, parent);
        }
        Ok(parents)
    }

    /// Observe the server on `socket` and the scoped panes (all panes when
    /// `scope` is `None`). `None` when the server has no pane to prove its
    /// process with; a scoped pane that is gone is simply absent.
    pub(super) fn observe(
        &self,
        socket: &str,
        scope: Option<&[String]>,
        deadline: Instant,
    ) -> Result<Option<Observed>, HerdrError> {
        if let Some(scope) = scope {
            if scope.len() > MAX_SCOPE_PANES {
                return Err(HerdrError::evidence(
                    "Herdr pane scope exceeds observation limits",
                ));
            }
            if !scope.iter().all(|id| HostKind::Herdr.is_pane_id(id)) {
                return Err(HerdrError::evidence(
                    "Herdr pane scope contains an invalid pane ID",
                ));
            }
        }
        let listed = self.list(socket, deadline)?;
        let selected: Vec<&ListedPane> = listed
            .iter()
            .filter(|pane| scope.is_none_or(|scope| scope.contains(&pane.terminal_id)))
            .collect();
        if selected.len() > MAX_SCOPE_PANES {
            return Err(HerdrError::evidence(
                "Herdr pane scope exceeds observation limits",
            ));
        }
        let mut processes = Vec::new();
        for pane in &selected {
            if let Some(process) = self.process(socket, &pane.pane_id, deadline)? {
                processes.push((*pane, process));
            }
        }
        // With no scoped pane left, any pane of the server proves its process.
        let witness = if processes.is_empty() {
            let mut witness = None;
            for pane in &listed {
                if let Some(process) = self.process(socket, &pane.pane_id, deadline)? {
                    witness = Some(process.shell_pid);
                    break;
                }
            }
            match witness {
                Some(shell) => vec![shell],
                None => return Ok(None),
            }
        } else {
            processes
                .iter()
                .map(|(_, process)| process.shell_pid)
                .collect()
        };
        let parents = self.parents(&witness, deadline)?;
        let servers: Vec<Option<u64>> = witness
            .iter()
            .map(|shell| parents.get(shell).copied())
            .collect();
        let Some(pid) = servers[0].filter(|pid| servers.iter().all(|parent| *parent == Some(*pid)))
        else {
            return Err(HerdrError::evidence(
                "Herdr panes disagree about their server process",
            ));
        };
        let process = match observe_runtime_process(self.runner(), pid, deadline)
            .map_err(HerdrError::command)?
        {
            ProcessObservation::Live(incarnation) => incarnation,
            _ => {
                return Err(HerdrError::evidence(
                    "Herdr server process evidence is unavailable",
                ));
            }
        };
        let panes = processes
            .into_iter()
            .map(|(pane, process)| PaneObservation {
                id: pane.terminal_id.clone(),
                target: Some(pane.pane_id.clone()),
                cwd: pane.cwd.clone(),
                suggested_name: crate::drivers::suggested_name(&process.command),
                command: process.command,
                pane_pid: process.shell_pid,
                // Read paths don't observe it: a binding records it when made.
                pane_incarnation: None,
                marker: marker::decode(pane.tokens.as_ref()),
            })
            .collect();
        Ok(Some(Observed {
            incarnation: Incarnation {
                socket: socket.into(),
                process,
            },
            panes,
        }))
    }
}
