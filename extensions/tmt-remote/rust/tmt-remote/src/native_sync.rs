//! Native ordinary-user REST transport over extension-declared sealed documents.
mod budget_state;
pub(crate) mod client;
mod discovery;
mod poll;
mod subscription;
pub use budget_state::{NativeBudgetError, NativeBudgetFile};
pub use client::{NativeBudgetPort, NativeClient, NativeRuntime};
pub use poll::PollCadence;
