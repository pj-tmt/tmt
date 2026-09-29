//! The caller's own Herdr pane. The environment names a pane and a socket;
//! the pane counts only when its shell is an ancestor of the caller, the same
//! proof tmux caller resolution requires.

use super::{Herdr, HerdrError};
use crate::{
    host::CallerEnvironment,
    process::{CommandRunner, ancestry},
};
use std::time::Instant;
use tmt_core::host::HostKind;

/// The caller's pane: its terminal ID and shell, and where the shell sits in
/// the caller's ancestry (0 is the caller itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallerPane {
    pub terminal_id: String,
    pub shell_pid: u64,
    pub depth: usize,
}

impl<R: CommandRunner> Herdr<R> {
    /// `None` when the environment names no Herdr pane or the evidence does
    /// not verify; only a failed cleanup is an error.
    pub(crate) fn caller(
        &self,
        environment: &CallerEnvironment,
        deadline: Instant,
    ) -> Result<Option<CallerPane>, HerdrError> {
        match self.verify_caller(environment, deadline) {
            Ok(pane) => Ok(pane),
            Err(error) if error.cleanup_failed() => Err(error),
            Err(_) => Ok(None),
        }
    }

    fn verify_caller(
        &self,
        environment: &CallerEnvironment,
        deadline: Instant,
    ) -> Result<Option<CallerPane>, HerdrError> {
        let (Some(pane), Some(socket)) = (
            environment
                .herdr_pane
                .as_deref()
                .and_then(|value| value.to_str()),
            environment
                .herdr_socket
                .as_deref()
                .and_then(|value| value.to_str()),
        ) else {
            return Ok(None);
        };
        if !HostKind::Herdr.is_target(pane) || !socket.starts_with('/') {
            return Ok(None);
        }
        let socket = self.server_socket(Some(socket), deadline)?;
        let terminal_id =
            self.call(Some(&socket), &["pane", "get", pane], deadline)?["pane"]["terminal_id"]
                .as_str()
                .filter(|id| HostKind::Herdr.is_pane_id(id))
                .ok_or_else(|| HerdrError::evidence("Herdr caller pane evidence is incomplete"))?
                .to_owned();
        let Some(shell_pid) = self.call(
            Some(&socket),
            &["pane", "process-info", "--pane", pane],
            deadline,
        )?["process_info"]["shell_pid"]
            .as_u64()
        else {
            return Ok(None);
        };
        let chain = match ancestry::chain(self.runner(), environment.process_id, deadline) {
            Ok(chain) => chain,
            Err(ancestry::AncestryError::Command(cause)) => {
                return Err(HerdrError::command(cause));
            }
            Err(ancestry::AncestryError::Unavailable) => return Ok(None),
        };
        Ok(chain
            .iter()
            .position(|pid| *pid == shell_pid)
            .map(|depth| CallerPane {
                terminal_id,
                shell_pid,
                depth,
            }))
    }
}
