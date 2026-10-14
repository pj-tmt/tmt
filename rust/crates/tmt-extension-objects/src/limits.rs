//! Protocol bounds shared by every consumer of the leaf.

/// Bytes of the big-endian length prefix that precedes every frame body.
pub const PREFIX_BYTES: usize = 4;
/// Smallest JSON body: an empty object.
pub const MIN_FRAME_BYTES: usize = 2;
/// Largest JSON body, excluding its length prefix.
pub const FRAME_BYTES: usize = 65_536;
/// Deepest object/array nesting an admitted body may have. The typed frames need
/// at most five levels; the bound keeps validation cost independent of the sender.
pub const JSON_DEPTH: usize = 8;
/// Largest value of a JSON numeric field: integers beyond it are not exact in
/// every JSON consumer. Counters are decimal strings and are not bound by it.
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
/// Largest decoded part or read chunk. Every part but the last is exactly the
/// backend's canonical size, which may be smaller; this is the transport ceiling.
pub const CHUNK_BYTES: usize = 32_768;
/// Largest decoded policy input. The service turns it into the backend's immutable
/// policy binding, which has the same bound.
pub const POLICY_BYTES: usize = 2_048;
/// Largest serialized opaque payload one object may hold.
pub const PAYLOAD_BYTES: u64 = 12 * 1024 * 1024;
/// Longest backend identifier.
pub const BACKEND_ID_BYTES: usize = 32;

/// v3 sync JSON body, excluding its four-byte prefix.
pub const V3_FRAME_BYTES: usize = 397_312;
/// Canonical fields map of one sync document.
pub const SYNC_DOCUMENT_BYTES: usize = 393_216;
/// Encoded non-document metadata in one sync frame.
pub const SYNC_METADATA_BYTES: usize = 4_096;
/// Documents in one query response, also constrained by the frame cap.
pub const SYNC_PAGE_DOCUMENTS: usize = 256;
/// Stateless, printable discovery run continuation.
pub const SYNC_CONTINUATION_BYTES: usize = 1_024;

/// Page interests, independent of writers or query positions.
pub const SYNC_INTERESTS: usize = 3;
pub const INSTALLATION_SYNC_INTERESTS: usize = 12;

/// Generation-scoped bound contexts; repeated bind of a space reuses its context.
pub const SYNC_BINDINGS: usize = 8;
