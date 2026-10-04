//! Where the clock's holder pane lives, resolved on the refresh worker.

use crate::effects::pane_place;
use std::{collections::HashMap, path::PathBuf};

/// One lookup per pane id: the holder rarely changes, and a failed lookup is
/// remembered too, so a refresh never repeats a call that cannot work.
pub struct Places {
    program: PathBuf,
    socket: Option<String>,
    known: HashMap<String, Option<String>>,
}

impl Places {
    /// `socket` is the invoker's tmux server; outside tmux nothing is resolved.
    pub fn new(socket: Option<String>) -> Self {
        Self::with_program("tmux".into(), socket)
    }

    pub fn with_program(program: PathBuf, socket: Option<String>) -> Self {
        Self {
            program,
            socket,
            known: HashMap::new(),
        }
    }

    /// `session:window` of the pane, or `None` to fall back to the pane id.
    pub fn resolve(&mut self, pane: &str) -> Option<String> {
        let socket = self.socket.as_deref()?;
        self.known
            .entry(pane.to_owned())
            .or_insert_with(|| pane_place(&self.program, socket, pane))
            .clone()
    }
}

#[cfg(test)]
mod tests;
