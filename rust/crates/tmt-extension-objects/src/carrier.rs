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
mod handshake;

pub use handshake::{Expect, Offer, accept, initiate};

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

/// An opened channel: the stream after a successful upgrade, bound to its generation.
#[derive(Debug)]
pub struct Link {
    generation: Uuid4,
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
    /// The next frame. The wait for its first byte is unbounded unless `idle` says
    /// otherwise; from that byte on, `budgets.frame` is absolute.
    pub fn read_frame(&mut self, idle: Idle<'_>, budgets: &Budgets) -> Result<Frame, Fault> {
        let body = self.reader.frame(idle, budgets.frame)?;
        let frame = crate::decode(&body).map_err(Fault::Frame)?;
        if generation_of(&frame) != self.generation {
            return Err(Fault::Generation);
        }
        Ok(frame)
    }
    /// One frame within `budgets.write`.
    pub fn write_frame(&mut self, frame: &Frame, budgets: &Budgets) -> Result<(), Fault> {
        if generation_of(frame) != self.generation {
            return Err(Fault::Generation);
        }
        let bytes = crate::encode(frame).map_err(Fault::Frame)?;
        self.writer.send(&bytes, budgets.write)
    }
}

#[cfg(test)]
mod tests;
