//! L2 product bounds, distinct from the disposable #830 fixture budgets.
use std::time::Duration;

/// Immutable app inventory, below remote's per-reply 16 MiB cap.
pub use crate::app_inventory::{APP_BYTES, APP_FILES};

pub const SOCKETS: usize = 16;
/// Live colab-sync-v1 tunnels, matching the remote door's colab mount cap.
pub const TUNNELS: usize = 16;
/// A tunnel that receives no inbound bytes for this long closes, until
/// colab-sync-v1 heartbeats exist (colab sends nothing on it yet).
pub const TUNNEL_IDLE: Duration = Duration::from_secs(120);
pub const HEADER_BYTES: usize = 8 * 1024;
pub const HEADER_FIELDS: usize = 32;
pub const HTTP_BODY_BYTES: usize = 64 * 1024;
/// Raw update-v1 content bytes, before envelope/base64 expansion.
pub use crate::decoder::UPDATE_BYTES as CONTENT_UPDATE_BYTES;
/// All-or-none owner wrap publication bound for a single transaction.
pub const OWNER_WRAPS: usize = 512;
/// Local page catalog bound, including retained tombstones.
pub const PAGES: usize = 1000;
/// A prepared local update has two base64 layers plus a certified chain.
pub const LOCAL_PAGE_BODY_BYTES: usize = 512 * 1024;
/// Worst-case JSON escaping of bounded source/title plus the payload's base64 layer.
pub const LOCAL_CREATE_PAYLOAD_BYTES: usize =
    6 * (crate::decoder::BASELINE_BYTES + crate::decoder::BASELINE_TITLE_BYTES) + 1024;
pub const LOCAL_MANAGEMENT_BODY_BYTES: usize = LOCAL_CREATE_PAYLOAD_BYTES.div_ceil(3) * 4 + 2048;
/// One route-owned body rule for acquisition and local callers.
pub fn http_body_bytes(path: &str) -> usize {
    if path == crate::page::ipc::PATH {
        LOCAL_PAGE_BODY_BYTES
    } else if path == crate::management::LOCAL_PATH {
        LOCAL_MANAGEMENT_BODY_BYTES
    } else {
        HTTP_BODY_BYTES
    }
}
pub const WS_FRAME_BYTES: usize = 64 * 1024;
pub const SEND_QUEUE_FRAMES: usize = 8;
pub const OBJECT_BYTES: usize = 16 * 1024 * 1024 + 2 * 1024;
pub const PAGE_BYTES: usize = 64 * 1024 * 1024;
pub const PAGE_RECEIPTS: usize = 100_000;
pub const ACQUISITION: Duration = Duration::from_secs(2);
pub const RESPONSE: Duration = Duration::from_secs(1);

/// Per-page sync namespace inventory / cursor budget. Store writes are unaffected.
pub const SYNC_NAMESPACES: usize = 256;
/// Raw bytes per chunk; base64 and control fields fit in a 64 KiB frame.
pub const CHUNK_BYTES: usize = 32 * 1024;
pub const CHUNK_COUNT: usize = OBJECT_BYTES.div_ceil(CHUNK_BYTES);
/// Serialized update envelope reserve including base64 expansion and JSON syntax.
pub const UPDATE_BYTES: usize = (CONTENT_UPDATE_BYTES + 2048) * 4 / 3 + 2048;
/// Exact signed statement JSON cap, including base64 expansion and framing.
pub const STATEMENT_BYTES: usize = (tmt_colab_model::payload::MAX_BYTES + 1024) * 4 / 3 + 2048;
/// Bootstrap descriptor is metadata, not the baseline object itself.
pub const SYNC_CONTEXT_BYTES: usize = 8 * 1024;

/// Inert discussion text and quote-selector context bounds.
pub const PUBLISHER_AGENT_BYTES: usize = 128;
pub const COMMENT_BODY_BYTES: usize = 16 * 1024;
pub const COMMENT_CONTEXT_BYTES: usize = 128;
pub const COMMENT_CONTEXT_POINTS: usize = 32;
