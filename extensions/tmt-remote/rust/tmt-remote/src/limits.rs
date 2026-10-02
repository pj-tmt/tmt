//! Door bounds from the remote-client-v1 `loopback-http` binding.
use std::time::Duration;

/// Concurrent unauthenticated edge connections.
pub const SOCKETS: usize = 32;
pub const HEADER_BYTES: usize = 8 * 1024;
pub const HEADER_FIELDS: usize = 32;
/// Unauthenticated `/r/` attempts admitted per minute.
pub const ATTEMPTS_PER_MINUTE: usize = 20;
pub const PAIR_BODY_BYTES: usize = 16 * 1024;
/// Envelope metadata budget added to the core input bound.
pub const METADATA_BYTES: usize = 8 * 1024;
/// Largest core input bound the door accepts from capabilities.
pub const CORE_INPUT_BYTES: usize = 16 * 1024 * 1024;
/// Bounded discard after a reply so a late request tail cannot reset it.
pub const DRAIN_BYTES: usize = 64 * 1024;
pub const ACQUISITION: Duration = Duration::from_secs(5);
pub const RESPONSE: Duration = Duration::from_secs(1);
