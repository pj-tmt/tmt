//! Generic wire primitives for the extension object channel: canonical identifiers,
//! fixed-width binary encodings, the protocol bounds, bounded strict JSON admission
//! and the typed request, result, admission callback and origin lifecycle frames.
//! Handshake and channel I/O are not part of this crate yet. It names no
//! Remote or Colab type and grants no authority: an identifier or digest proves
//! nothing about who may use it.
mod codec;
mod error;
mod frame;
mod ids;
pub mod limits;

pub use codec::decode_length;
pub use error::ErrorClass;
pub use frame::{
    Admission, Admit, AdmitInput, BeginInput, Call, Checkpoint, Config, ConfigInput, Context,
    Decision, Disclosure, ErrorCode, Frame, Limit, Limits, Method, Operation, Origin, OriginPhase,
    OriginState, Outcome, PartAdmit, PartInput, Projection, ReadInput, Request, ResultFrame,
    Retained, State, StatusInput, Success, TransferAdmit, TransferBounds, TransferInput, decode,
    encode,
};
pub use ids::{Bytes32, Chunk, Counter, Policy, Sha256Hex, Uuid4};
