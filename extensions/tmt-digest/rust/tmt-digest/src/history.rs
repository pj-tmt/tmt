//! Digest-owned evidence: a tick collects; status reads; neither grants delivery.

mod collect;
mod model;
mod store;

pub use collect::collect;
pub use model::Window;
pub use store::Store;

pub const MAX: u64 = 9_007_199_254_740_991;
pub const METHOD: &str = "context-input-avoided-v1";
const DAY: u64 = 86_400_000;
const RETENTION: u64 = 30 * DAY;

fn invalid(message: impl Into<String>) -> crate::core::Error {
    crate::core::Error::new("DIGEST_HISTORY_INVALID", message)
}
