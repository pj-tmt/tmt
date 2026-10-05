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
/// Largest request body any handler may admit: the base64 envelope bound for
/// the largest accepted core input.
pub const BODY_BYTES: usize = 4 * CORE_INPUT_BYTES.div_ceil(3) + METADATA_BYTES;
/// Request body bytes all workers may hold at once before authentication;
/// fits one maximum body while bounding total door memory.
pub const IN_FLIGHT_BODY_BYTES: usize = 32 * 1024 * 1024;
/// Bounded discard after a reply so a late request tail cannot reset it.
pub const DRAIN_BYTES: usize = 64 * 1024;
pub const ACQUISITION: Duration = Duration::from_secs(5);
pub const RESPONSE: Duration = Duration::from_secs(1);
/// Total time for a mounted extension to accept a request and reply.
pub const MOUNT_RESPONSE: Duration = Duration::from_secs(15);
/// No-progress bound for pending bytes inside an upgraded tunnel.
pub const SPLICE_WRITE: Duration = Duration::from_secs(5);

/// Deadline for one fixed public core subprocess, followed by the runner's bounded cleanup.
pub const CORE_CALL: Duration = Duration::from_secs(15);
/// A dispatch fence can perform receipt lookup and creation, each with a one-second
/// cleanup budget. Authority writes wait beyond both calls with eight seconds' margin.
pub const AUTHORITY_WAIT: Duration = Duration::from_secs(40);

/// Bounded graceful-stop confirmation after the control acknowledgment.
pub const STOP_WAIT: Duration = Duration::from_secs(40);

/// Minimum interval between background session authority and cleanup scans.
pub const SESSION_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(1);

/// Sessions that have never acquired a mounted transport expire without activity.
pub const SESSION_UNATTACHED_IDLE: Duration = Duration::from_secs(60);
/// Keep signed end reasons briefly after cleanup; later admission is generic.
pub const SESSION_END_NOTICE: Duration = Duration::from_secs(60);

/// Mounted sessions expire after twelve hours without activity.
pub const SESSION_IDLE: Duration = Duration::from_secs(12 * 60 * 60);
