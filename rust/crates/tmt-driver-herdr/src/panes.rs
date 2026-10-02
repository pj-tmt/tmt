//! Pane and server evidence from `pane list`, `pane get`,
//! `pane process-info` and `ps`.

use crate::{
    herdr::{Herdr, HerdrError},
    run::{Output, RunError, Runner},
};
use serde_json::{Map, Value};
use std::time::Instant;
use tmt_driver_protocol::Grammar;

/// One pane as Herdr lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    /// Herdr's public `wN:pM` name, which its pane commands take.
    pub target: String,
    /// The stable terminal ID, the protocol's pane ID.
    pub id: String,
    pub cwd: Option<String>,
    pub tokens: Option<Map<String, Value>>,
}

/// A pane's shell and its first foreground process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    pub shell_pid: u64,
    pub command: String,
}

/// The largest pid a JSON number carries exactly.
const MAX_PID: u64 = (1 << 53) - 1;
/// Bounded text the protocol accepts as a command or path.
const MAX_TEXT: usize = 256;
const MAX_PATH: usize = 4096;

fn text(value: &Value, key: &str, max: usize) -> Option<String> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control))
        .map(str::to_owned)
}

pub fn valid_pid(pid: u64) -> bool {
    (1..=MAX_PID).contains(&pid)
}

fn listed(grammar: &Grammar, row: &Value) -> Result<Listed, HerdrError> {
    let target = text(row, "pane_id", MAX_TEXT).filter(|target| grammar.is_target(target));
    let id = text(row, "terminal_id", MAX_TEXT).filter(|id| grammar.is_pane_id(id));
    let (Some(target), Some(id)) = (target, id) else {
        return Err(HerdrError::Malformed("Herdr reported an invalid pane"));
    };
    Ok(Listed {
        target,
        id,
        cwd: text(row, "foreground_cwd", MAX_PATH)
            .or_else(|| text(row, "cwd", MAX_PATH))
            .filter(|cwd| cwd.starts_with('/')),
        tokens: row["tokens"].as_object().cloned(),
    })
}

impl<R: Runner> Herdr<R> {
    pub fn list(
        &self,
        grammar: &Grammar,
        socket: &str,
        deadline: Instant,
    ) -> Result<Vec<Listed>, HerdrError> {
        let result = self.call(socket, &["pane", "list"], deadline)?;
        let rows = result["panes"]
            .as_array()
            .ok_or(HerdrError::Malformed("Herdr pane list is malformed"))?;
        let mut panes: Vec<Listed> = Vec::with_capacity(rows.len());
        for row in rows {
            let pane = listed(grammar, row)?;
            if panes
                .iter()
                .any(|seen| seen.target == pane.target || seen.id == pane.id)
            {
                return Err(HerdrError::Malformed("Herdr pane list repeats a pane"));
            }
            panes.push(pane);
        }
        Ok(panes)
    }

    /// The pane a target names, or `None` when Herdr has no such pane.
    pub fn get(
        &self,
        grammar: &Grammar,
        socket: &str,
        target: &str,
        deadline: Instant,
    ) -> Result<Option<Listed>, HerdrError> {
        match self.call(socket, &["pane", "get", target], deadline) {
            Ok(result) => listed(grammar, &result["pane"]).map(Some),
            Err(error) if error.code() == Some("pane_not_found") => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The pane's shell and foreground command, or `None` when the pane
    /// closed after it was listed.
    pub fn process(
        &self,
        socket: &str,
        target: &str,
        deadline: Instant,
    ) -> Result<Option<Process>, HerdrError> {
        let result = match self.call(
            socket,
            &["pane", "process-info", "--pane", target],
            deadline,
        ) {
            Ok(result) => result,
            Err(error) if error.code() == Some("pane_not_found") => return Ok(None),
            Err(error) => return Err(error),
        };
        let info = &result["process_info"];
        let shell_pid = info["shell_pid"]
            .as_u64()
            .filter(|pid| valid_pid(*pid))
            .ok_or(HerdrError::Malformed(
                "Herdr pane process evidence is incomplete",
            ))?;
        let command = info["foreground_processes"]
            .as_array()
            .and_then(|processes| processes.first())
            .and_then(|process| text(process, "name", MAX_TEXT))
            .unwrap_or_default();
        Ok(Some(Process { shell_pid, command }))
    }

    /// The server process: Herdr does not report its pid, but every pane's
    /// shell is its child. `None` when the shell has already exited.
    pub fn server_process(
        &self,
        shell_pid: u64,
        deadline: Instant,
    ) -> Result<Option<(u64, String)>, HerdrError> {
        let Some(parent) = self
            .ps("ppid=", shell_pid, deadline)?
            .and_then(|ppid| ppid.parse::<u64>().ok())
            .filter(|pid| valid_pid(*pid) && *pid > 1)
        else {
            return Ok(None);
        };
        Ok(self
            .ps("lstart=", parent, deadline)?
            .map(|started| (parent, started)))
    }

    /// One `ps` column of one process; `None` when no such process runs.
    fn ps(&self, column: &str, pid: u64, deadline: Instant) -> Result<Option<String>, HerdrError> {
        let args = [
            "-o".to_owned(),
            column.to_owned(),
            "-p".to_owned(),
            pid.to_string(),
        ];
        let Output { code, stdout, .. } = self
            .runner()
            .run("ps", &[("LC_ALL", "C")], &args, deadline)
            .map_err(HerdrError::Process)?;
        let value = String::from_utf8_lossy(&stdout)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        match code {
            Some(0) if !value.is_empty() && value.len() <= MAX_TEXT => Ok(Some(value)),
            // `ps -p` exits 1 with no row when the process is gone.
            Some(1) if value.is_empty() => Ok(None),
            Some(127) => Err(HerdrError::Process(RunError::NotStarted)),
            _ => Err(HerdrError::Malformed(
                "ps returned malformed process evidence",
            )),
        }
    }
}
