//! Terminal hosts that TMT doesn't build in, reached through consented
//! out-of-process drivers (#570, `contracts/driver-protocol-v1.md`). The
//! registry holds what the user approved; a `DriverProcess` runs one of them
//! under the protocol's bounds and checks every answer.

mod caller;
mod process;
pub mod registry;
mod session;

pub use caller::ExternalCaller;
pub use process::{CallError, DriverProcess};
pub use session::{Drivers, ExternalDriver, Session};

use crate::config::ConfigPaths;
use tmt_driver_protocol::Grammar;

/// Registers the approved drivers' syntax with core, once per process and
/// before any name is validated, so their pane IDs and targets are known.
/// Best effort: a registry that can't be read registers nothing and fails no
/// command, and with no registry the only cost is one read attempt. Calling
/// it twice is a programming error.
pub fn register_approved() {
    let grammars = approved()
        .into_iter()
        .filter_map(|record| {
            Grammar::from_capabilities(&record.capabilities)
                .ok()
                .filter(|grammar| grammar.name() == record.name)
                .map(|grammar| grammar.host().clone())
        })
        .collect();
    let registered = tmt_core::host::register_external_hosts(grammars);
    debug_assert!(registered.is_ok(), "external hosts are registered once");
}
/// The drivers the user approved, best effort: a registry that can't be
/// read approves nothing.
pub fn approved() -> Vec<registry::DriverRecord> {
    ConfigPaths::discover()
        .ok()
        .and_then(|paths| registry::read(&paths.global_dir).ok())
        .unwrap_or_default()
}

/// Set on every driver call; a `tmt` that sees it runs no command.
pub use tmt_driver_protocol::CALL_ENV;

#[cfg(test)]
mod tests;
