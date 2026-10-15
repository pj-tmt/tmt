//! Herdr's agent names, from `agent list` and `agent rename`. An agent name
//! belongs to the agent Herdr recognizes in a pane and is its address
//! (`herdr agent prompt <name>`). It is not the pane label (`pane rename`),
//! which this driver never reads or writes.

use crate::{
    herdr::{Herdr, HerdrError},
    run::Runner,
};
use serde_json::Value;
use std::time::Instant;

/// The most characters Herdr allows in an agent name.
const NAME_MAX: usize = 32;

/// Herdr's agent name rule: a lowercase letter, then lowercase letters,
/// digits, `-` or `_`, 32 characters at most. Checked here too because the
/// name is a command-line argument, and `--clear` in its place would take
/// the name away.
pub fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && name.len() <= NAME_MAX
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

/// The agent name of the terminal for display, where a wrong guess must not
/// happen: `Some(None)` for the one listed agent without a name, and `None`
/// (unknown) for no row, more than one row or a name that cannot be read.
pub fn name_of(rows: &[Value], terminal: &str) -> Option<Option<String>> {
    agent_of(rows, terminal).ok()?
}

/// The agent in the terminal as evidence for a change: `None` when Herdr
/// lists no agent there, `Some(None)` for an agent without a name. A list
/// that does not say exactly what the terminal hosts is refused.
pub fn agent_of(rows: &[Value], terminal: &str) -> Result<Option<Option<String>>, HerdrError> {
    let mut found = None;
    for row in rows {
        let id = row["terminal_id"].as_str().ok_or(HerdrError::Malformed(
            "Herdr listed an agent without a terminal",
        ))?;
        if id != terminal {
            continue;
        }
        if found.is_some() {
            return Err(HerdrError::Malformed(
                "Herdr listed two agents in one terminal",
            ));
        }
        found = Some(match &row["name"] {
            Value::Null => None,
            Value::String(name) if valid_name(name) => Some(name.clone()),
            _ => {
                return Err(HerdrError::Malformed(
                    "Herdr listed an agent name that cannot be read",
                ));
            }
        });
    }
    Ok(found)
}

impl<R: Runner> Herdr<R> {
    /// Every agent Herdr recognizes now, as it lists them.
    pub fn agent_rows(&self, socket: &str, deadline: Instant) -> Result<Vec<Value>, HerdrError> {
        let mut result = self.call(socket, &["agent", "list"], deadline)?;
        match result["agents"].take() {
            Value::Array(rows) => Ok(rows),
            _ => Err(HerdrError::Malformed("Herdr agent list is malformed")),
        }
    }
}
