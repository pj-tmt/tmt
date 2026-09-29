//! The local service's only route to core: `tmt api` and JSON commands of the
//! invoking `tmt` (production), or the same operations run in-process (tests).
//! The service never opens the core database in production.

use serde_json::Value;
use tmt_adapters::config::ConfigPaths;
use tmt_office_storage::core_client::{CoreClient, Originator};

/// A core failure: core's own error code, or `CORE_UNAVAILABLE` when it could
/// not be reached or answered without one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CoreFault {
    pub code: String,
}

impl CoreFault {
    pub(super) const UNAVAILABLE: &'static str = "CORE_UNAVAILABLE";

    pub(super) fn unavailable() -> Self {
        Self {
            code: Self::UNAVAILABLE.into(),
        }
    }
}

pub(super) trait LocalCore {
    /// One `tmt api` operation; writes name an originator.
    fn api(
        &self,
        operation: &str,
        input: Value,
        originator: Option<Originator<'_>>,
    ) -> Result<Value, CoreFault>;
    /// One JSON command of `tmt`, for example `["list"]`.
    fn command(&self, args: &[&str]) -> Result<Value, CoreFault>;
}

struct ProcessCore(CoreClient);

fn fault(error: tmt_office_storage::core_client::CoreCallError) -> CoreFault {
    error
        .code
        .map_or_else(CoreFault::unavailable, |code| CoreFault { code })
}

impl LocalCore for ProcessCore {
    fn api(
        &self,
        operation: &str,
        input: Value,
        originator: Option<Originator<'_>>,
    ) -> Result<Value, CoreFault> {
        self.0.api(operation, input, originator).map_err(fault)
    }

    fn command(&self, args: &[&str]) -> Result<Value, CoreFault> {
        self.0.command(args).map_err(fault)
    }
}

/// Production core access through the invoking `tmt`.
#[cfg(not(test))]
pub(super) fn open(_paths: &ConfigPaths) -> Result<Box<dyn LocalCore>, CoreFault> {
    CoreClient::discover()
        .map(|client| Box::new(ProcessCore(client)) as Box<dyn LocalCore>)
        .map_err(fault)
}

/// Unit tests run the same operations in-process against a disposable root.
#[cfg(test)]
pub(super) fn open(paths: &ConfigPaths) -> Result<Box<dyn LocalCore>, CoreFault> {
    Ok(Box::new(in_process::InProcessCore {
        paths: paths.clone(),
    }))
}

#[cfg(test)]
mod in_process;
#[cfg(test)]
mod tests;
