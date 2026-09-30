//! Terminal hosts that TMT doesn't build in, reached through consented
//! out-of-process drivers (#570, `contracts/driver-protocol-v1.md`). The
//! registry holds what the user approved; a `DriverProcess` runs one of them
//! under the protocol's bounds and checks every answer.

mod process;
pub mod registry;

pub use process::{CallError, DriverProcess};
/// Set on every driver call; a `tmt` that sees it runs no command.
pub use tmt_driver_protocol::CALL_ENV;

#[cfg(test)]
mod tests;
