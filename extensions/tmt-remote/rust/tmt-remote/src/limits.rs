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

/// Default per-device session cap; an explicit setting may change it or disable it.
pub const DEFAULT_SESSIONS_PER_DEVICE: usize = 8;

/// Minimum interval between background session authority and cleanup scans.
pub const SESSION_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(1);

/// Sessions without a live mounted transport expire after this reattach grace.
pub const SESSION_UNATTACHED_IDLE: Duration = Duration::from_secs(60);
/// Keep signed end reasons briefly after cleanup; later admission is generic.
pub const SESSION_END_NOTICE: Duration = Duration::from_secs(60);

/// Mounted sessions expire after twelve hours without activity.
pub const SESSION_IDLE: Duration = Duration::from_secs(12 * 60 * 60);

/// Grants materialized per browser management page (plus one lookahead row).
pub const MANAGEMENT_PAGE: usize = 50;

/// Decoded payload bound for fixed Remote management requests.
pub const MANAGEMENT_INPUT_BYTES: usize = 1024;

// Object backend bounds. These are the storage-v1 proposal's technical starting
// bounds, not deployed settings; an object adapter enforces them through `Quotas`.
/// Canonical raw part size; every part but the last is exactly this long.
pub const OBJECT_CHUNK_BYTES: u32 = 32 * 1024;
/// Largest serialized opaque payload one object may hold.
pub const OBJECT_PAYLOAD_BYTES: u64 = 12 * 1024 * 1024;
/// Largest service-generated immutable policy binding.
pub const OBJECT_BINDING_BYTES: usize = 2048;
/// Incomplete staging closes this long after its first durable adoption.
pub const OBJECT_STAGING: Duration = Duration::from_secs(24 * 60 * 60);
pub const OBJECT_NAMESPACE_BYTES: u64 = 64 * 1024 * 1024;
pub const OBJECT_EXTENSION_BYTES: u64 = 512 * 1024 * 1024;
pub const OBJECT_INSTALLATION_BYTES: u64 = 1024 * 1024 * 1024;
pub const OBJECT_NAMESPACE_ENTRIES: u32 = 1024;
pub const OBJECT_EXTENSION_ENTRIES: u32 = 8192;
pub const OBJECT_INSTALLATION_ENTRIES: u32 = 32768;
pub const OBJECT_ACTIVE_INTENTS: u32 = 32;
pub const OBJECT_RETAINED_EXTENSION: u32 = 16_384;
pub const OBJECT_RETAINED_INSTALLATION: u32 = 65_536;
/// Payload bytes are charged rounded up to this physical block size.
pub const OBJECT_BLOCK_BYTES: u64 = 4096;
/// Charge per retained intent row, including its index entries and page slack.
/// A maximal row occupies one 4 KiB ledger page plus index share (see the
/// charge-bound test); the remainder is margin.
pub const OBJECT_RECORD_BYTES: u64 = 8192;
/// Charge per namespace: its directory block, directory entry and fence row. The
/// footprint test measured a namespace holding one tiny object at more than the
/// record and payload charges alone, so the directory block is charged here.
pub const OBJECT_FENCE_BYTES: u64 = 4096 + 512;
/// Charge once per extension that has any namespace: its directory, `objects`,
/// `staging` and `blobs` blocks plus one block of directory growth. The two-body
/// footprint test showed these fixed blocks were uncovered when payloads dominate.
pub const OBJECT_TREE_BASE_BYTES: u64 = 5 * 4096;
/// Installation-wide charge for ledger bootstrap pages and the rollback journal's peak.
pub const OBJECT_LEDGER_BASE_BYTES: u64 = 256 * 1024;

/// Absolute private background startup admission deadline, before cleanup.
pub const SERVE_STARTUP: Duration = Duration::from_secs(35);
/// Maximum complete private handoff frame and fixed local diagnostic record.
pub const SERVE_RECORD_BYTES: usize = 4096;
