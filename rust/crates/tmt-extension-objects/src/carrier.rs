//! The Unix carrier of the object channel: the route-owned upgrade that opens it and
//! the bounded, interruptible frame I/O over the resulting stream. It is the only
//! part of this crate that touches the operating system; the protocol modules do
//! not, and the crate builds without it on other platforms.
//!
//! The carrier names no Remote or Colab type and grants no authority. A handshake
//! proves only that both ends agreed on one generation of one mount.
use crate::{ErrorClass, Frame, Uuid4};
use std::{
    io,
    os::unix::net::UnixStream,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

mod bounded;
mod bus;
mod handshake;
mod ledger;

pub use bus::{Bus, StampedObjectFrame};
pub use handshake::{Expect, Offer, accept, accept_head, initiate};
pub use ledger::{Budget, Caps, Reason};

/// The reserved route that opens the channel.
pub(crate) const ROUTE: &str = "/.tmt/remote/object-channel-v1";
/// The upgrade token, repeated in the reply.
pub(crate) const PROTOCOL: &str = "tmt-object-channel-v1";

/// Every time bound of the carrier. [`Budgets::contract`] is the protocol's; tests and
/// callers with a tighter outer deadline pass shorter ones. A bound is absolute from
/// its start and never renewed by partial progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets {
    /// The acceptor reads the whole request head within this.
    pub head: Duration,
    /// The acceptor writes its reply within this.
    pub reply: Duration,
    /// A frame is complete this long after its first prefix byte.
    pub frame: Duration,
    /// One frame is written within this.
    pub write: Duration,
}
impl Budgets {
    pub const fn contract() -> Self {
        Self {
            head: Duration::from_secs(2),
            reply: Duration::from_secs(1),
            frame: Duration::from_secs(2),
            write: Duration::from_secs(1),
        }
    }
}

/// Which end of the channel this is. Remote opens it and sends results, callbacks and
/// origin state; the extension answers the upgrade and sends requests and admissions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Remote,
    Extension,
}

/// Where a bound ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Reading the request or reply head.
    Head,
    /// Writing the request or reply head.
    Reply,
    /// Waiting for the first byte of a frame, when the caller bounded the wait.
    Idle,
    /// Reading the rest of a frame after its first prefix byte.
    Frame,
    /// Writing a frame.
    Write,
}

/// Why a handshake head was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not one complete head of at most 8 KiB and 32 fields.
    Head,
    Method,
    /// Not exactly the reserved route (a query is a different path).
    Path,
    Version,
    /// Not `101 Switching Protocols`.
    Status,
    UnknownHeader,
    DuplicateHeader,
    MissingHeader(&'static str),
    /// A header value outside its exact grammar or not the one expected.
    BadValue(&'static str),
    /// Bytes followed the head before the upgrade was complete.
    Pipelined,
    /// The reply names a different generation than the request.
    Generation,
}

/// Why a carrier operation ended. Each is final for the link it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The stream ended between frames, or the caller closed or interrupted the wait.
    Closed,
    /// The stream ended inside a head or a frame.
    Truncated,
    Timeout(Stage),
    Io(io::ErrorKind),
    Handshake(Refusal),
    /// The bytes are not a valid frame.
    Frame(ErrorClass),
    /// A frame names a generation other than the link's.
    Generation,
    /// A frame breaks direction or correlation.
    Correlation(Reason),
}

/// What a caller does while waiting for the next frame to begin.
#[derive(Clone, Copy, Debug)]
pub struct Idle<'a> {
    /// Set by another thread to end the wait.
    pub stop: &'a AtomicBool,
    /// The wait ends at this instant; `None` waits until a byte, EOF or `stop`.
    pub until: Option<Instant>,
}

/// The generation every kind of frame carries.
fn generation_of(frame: &Frame) -> Uuid4 {
    match frame {
        Frame::Request(value) => value.generation,
        Frame::Result(value) => value.generation,
        Frame::Admit(value) => value.generation,
        Frame::Admission(value) => value.generation,
        Frame::OriginState(value) => value.generation,
    }
}

/// An opened channel: the stream after a successful upgrade, bound to its generation
/// and to this end's role. [`Bus::start`] takes it over.
#[derive(Debug)]
pub struct Link {
    generation: Uuid4,
    role: Role,
    reader: bounded::Reader,
    writer: bounded::Writer,
}
/// Both halves of one stream, the reader holding any bytes that followed the head.
fn halves(
    stream: UnixStream,
    first_bytes: Vec<u8>,
) -> Result<(bounded::Reader, bounded::Writer), Fault> {
    let io = |error: io::Error| Fault::Io(error.kind());
    let writer = stream.try_clone().map_err(io)?;
    Ok((
        bounded::Reader::new(stream, first_bytes).map_err(io)?,
        bounded::Writer::new(writer).map_err(io)?,
    ))
}

impl Link {
    pub fn generation(&self) -> Uuid4 {
        self.generation
    }
    pub fn role(&self) -> Role {
        self.role
    }
}

/// The next frame and the size of its body. The wait for its first byte is unbounded
/// unless `idle` says otherwise; from that byte on, `budgets.frame` is absolute.
fn read_checked(
    reader: &mut bounded::Reader,
    generation: Uuid4,
    idle: Idle<'_>,
    budgets: &Budgets,
) -> Result<(StampedObjectFrame, usize), Fault> {
    let (body, first_prefix) = reader.frame(idle, budgets.frame)?;
    let frame = crate::decode(&body).map_err(Fault::Frame)?;
    if generation_of(&frame) != generation {
        return Err(Fault::Generation);
    }
    Ok((
        StampedObjectFrame {
            frame,
            first_prefix,
        },
        body.len(),
    ))
}
/// The bytes to write for `frame`, once it is known to belong to this generation and to
/// be a valid frame. Nothing is sent or recorded yet.
fn prepared(generation: Uuid4, frame: &Frame) -> Result<Vec<u8>, Fault> {
    if generation_of(frame) != generation {
        return Err(Fault::Generation);
    }
    crate::encode(frame).map_err(Fault::Frame)
}

#[cfg(test)]
mod tests;
