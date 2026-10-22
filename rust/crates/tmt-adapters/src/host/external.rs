//! Terminal hosts that TMT doesn't build in, reached through consented
//! out-of-process drivers (#570, `contracts/driver-protocol-v1.md`). The
//! registry holds what the user approved; a `DriverProcess` runs one of them
//! under the protocol's bounds and checks every answer.

mod caller;
mod discovery;
use crate::driver_protocol::registry;
mod session;

pub use crate::driver_protocol::process::{CallError, DriverProcess};
pub(crate) use caller::Callers;
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
    register(&approved());
}

/// [`register_approved`] for the drivers a command already read, so its
/// pane-ID syntax is exactly what it may run.
pub fn register(records: &[registry::DriverRecord]) {
    let grammars = records
        .iter()
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
/// The drivers a command that must not change approvals may run, read once
/// ([`registry::admitted_read_only`]). Without configuration paths no host
/// is known.
pub fn admitted() -> registry::Admitted {
    admitted_at(ConfigPaths::discover().as_ref())
}

/// [`admitted`] for the configuration paths a command already resolved.
pub fn admitted_at(paths: Result<&ConfigPaths, &crate::config::ConfigError>) -> registry::Admitted {
    count_approval_read();
    match paths {
        Ok(paths) => {
            registry::admitted_read_only(&paths.global_dir, std::env::current_exe().ok().as_deref())
        }
        Err(error) => registry::Admitted {
            unreadable: Some(error.to_string()),
            ..registry::Admitted::default()
        },
    }
}

/// The drivers the user approved, best effort: a registry that can't be
/// read approves nothing. A first-party driver is the one the running
/// release ships (`registry::current_first_party`), or none.
pub fn approved() -> Vec<registry::DriverRecord> {
    count_approval_read();
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

#[cfg(any(test, feature = "test-isolation"))]
thread_local! {
    static APPROVAL_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How often this thread read the approvals ([`approved`], [`admitted_at`]),
/// so a test can prove that a step reads none again.
#[cfg(any(test, feature = "test-isolation"))]
pub fn approval_reads() -> usize {
    APPROVAL_READS.with(std::cell::Cell::get)
}

fn count_approval_read() {
    #[cfg(any(test, feature = "test-isolation"))]
    APPROVAL_READS.with(|reads| reads.set(reads.get() + 1));
}

/// Set on every driver call; a `tmt` that sees it runs no command.
pub use tmt_driver_protocol::CALL_ENV;

#[cfg(test)]
mod tests;
