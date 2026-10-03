//! Terminal hosts that TMT doesn't build in, reached through consented
//! out-of-process drivers (#570, `contracts/driver-protocol-v1.md`). The
//! registry holds what the user approved; a `DriverProcess` runs one of them
//! under the protocol's bounds and checks every answer.

mod caller;
use crate::driver_protocol::registry;
mod session;

pub use crate::driver_protocol::process::{CallError, DriverProcess};
pub use caller::ExternalCaller;
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
            Grammar::from_capabilities(record.capabilities.host()?)
                .ok()
                .filter(|grammar| grammar.name() == record.name)
                .map(|grammar| grammar.host().clone())
        })
        .collect();
    let registered = tmt_core::host::register_external_hosts(grammars);
    debug_assert!(registered.is_ok(), "external hosts are registered once");
}
/// The drivers the user approved, best effort: a registry that can't be
/// read approves nothing. A first-party driver is the one the running
/// release ships (`registry::current_first_party`), or none.
pub fn approved() -> Vec<registry::DriverRecord> {
    let Ok(paths) = ConfigPaths::discover() else {
        return Vec::new();
    };
    let records = registry::read(&paths.global_dir).unwrap_or_default();
    let tmt = std::env::current_exe().ok();
    records
        .into_iter()
        .filter_map(|record| match record.source {
            registry::DriverSource::Path => record.capabilities.host().is_some().then_some(record),
            registry::DriverSource::FirstParty if record.capabilities.host().is_some() => {
                registry::current_first_party(
                    &paths.global_dir,
                    &record,
                    tmt.as_deref()?,
                    &crate::process::UnixCommandRunner,
                )
            }
            _ => None,
        })
        .collect()
}

/// Set on every driver call; a `tmt` that sees it runs no command.
pub use tmt_driver_protocol::CALL_ENV;

#[cfg(test)]
mod tests;
