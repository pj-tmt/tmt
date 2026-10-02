//! L2 product bounds, distinct from the disposable #830 fixture budgets.
use std::time::Duration;

pub const SOCKETS: usize = 16;
/// Live colab-sync-v1 tunnels, matching the remote door's colab mount cap.
pub const TUNNELS: usize = 16;
/// A tunnel without bytes either way closes after this, until heartbeats exist.
pub const TUNNEL_IDLE: Duration = Duration::from_secs(120);
pub const HEADER_BYTES: usize = 8 * 1024;
pub const HEADER_FIELDS: usize = 32;
pub const HTTP_BODY_BYTES: usize = 64 * 1024;
pub const WS_FRAME_BYTES: usize = 64 * 1024;
pub const SEND_QUEUE_FRAMES: usize = 8;
pub const OBJECT_BYTES: usize = 16 * 1024 * 1024 + 2 * 1024;
pub const PAGE_BYTES: usize = 64 * 1024 * 1024;
pub const PAGE_RECEIPTS: usize = 100_000;
pub const ACQUISITION: Duration = Duration::from_secs(2);
pub const RESPONSE: Duration = Duration::from_secs(1);
