//! Generic wire primitives for the extension object channel: canonical identifiers,
//! fixed-width binary encodings, the protocol bounds and bounded strict JSON
//! admission. Typed frames, handshake and channel I/O are not part of this crate yet. It names no
//! Remote or Colab type and grants no authority: an identifier or digest proves
//! nothing about who may use it.
mod codec;
mod error;
mod ids;
pub mod limits;

pub use codec::decode_length;
pub use error::ErrorClass;
pub use ids::{Bytes32, Counter, Sha256Hex, Uuid4};
