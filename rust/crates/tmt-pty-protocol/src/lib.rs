//! The stream between a built-in-host supervisor and the terminals attached to
//! its member's pseudo-terminal. Every frame is `kind: u8`, `length: u32` (big
//! endian), then `length` payload bytes; a payload is at most [`MAX_PAYLOAD`].
//! Decoding is strict: an unknown kind, a short or long payload and an
//! out-of-range value are errors, and a connection that sends one is closed.
//! The crate has no dependencies and no effects.

use std::fmt;

/// The only version this crate speaks; `Hello` and `Welcome` carry it.
pub const VERSION: u16 = 1;
/// The largest payload of any frame.
pub const MAX_PAYLOAD: usize = 64 * 1024;
/// The largest chunk of keystrokes one `Input` frame carries.
pub const MAX_INPUT: usize = 16 * 1024;
/// The largest chunk of program output one `Output` frame carries.
pub const MAX_OUTPUT: usize = 16 * 1024;
/// The largest window, in columns and in rows.
pub const MAX_DIMENSION: u16 = 10_000;
const HEADER: usize = 5;

/// A pseudo-terminal window size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Size {
    /// A size with both dimensions in `1..=`[`MAX_DIMENSION`].
    pub fn new(cols: u16, rows: u16) -> Result<Self, ProtocolError> {
        if (1..=MAX_DIMENSION).contains(&cols) && (1..=MAX_DIMENSION).contains(&rows) {
            Ok(Self { cols, rows })
        } else {
            Err(ProtocolError::Malformed(
                "a window dimension is out of range",
            ))
        }
    }
}

/// How the member's program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

/// Why a client was disconnected by the supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eviction {
    /// Another client asked for sole use.
    TakenOver,
    /// It did not read output fast enough; the program is never made to wait.
    TooSlow,
}

/// Why a connection was refused before it was attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The `Hello` named a version the supervisor does not speak.
    Version,
    /// The first frame was not a `Hello`.
    Handshake,
    /// The peer is not the user who owns the supervisor.
    Owner,
    /// As many clients are attached as the supervisor allows.
    Full,
}

/// A frame from a client to the supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    /// The first frame: the protocol version the client speaks.
    Hello { version: u16 },
    /// Keystrokes or pasted bytes for the program, at most [`MAX_INPUT`].
    Input(Vec<u8>),
    /// The client's window size. The supervisor resizes the program's terminal
    /// to the client that last sent this or typed; a client that never sends it,
    /// such as a delivery, never affects the size.
    Resize(Size),
    /// Disconnect every other client.
    TakeSole,
    /// Leave; the program keeps running.
    Detach,
}

/// A frame from the supervisor to a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMessage {
    /// The reply to an accepted `Hello`: the version and the window size now in force.
    Welcome { version: u16, size: Size },
    /// Raw program output, at most [`MAX_OUTPUT`], passed through untouched.
    Output(Vec<u8>),
    /// The window size in force changed.
    Resized(Size),
    /// The program ended; the supervisor closes after this.
    Exited(Exit),
    /// The client is disconnected.
    Evicted(Eviction),
    /// The connection is refused.
    Refused(Refusal),
}

/// Why bytes are not a valid frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    /// A frame longer than [`MAX_PAYLOAD`], or more buffered than one frame can be.
    Oversized,
    UnknownKind(u8),
    Malformed(&'static str),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oversized => f.write_str("a frame is larger than the protocol allows"),
            Self::UnknownKind(kind) => write!(f, "unknown frame kind {kind:#04x}"),
            Self::Malformed(reason) => write!(f, "malformed frame: {reason}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// A frame type with a kind byte and a payload.
pub trait Frame: Sized {
    fn kind(&self) -> u8;
    fn write_payload(&self, out: &mut Vec<u8>);
    fn read(kind: u8, payload: &[u8]) -> Result<Self, ProtocolError>;

    /// The whole frame: header and payload.
    fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.kind(), 0, 0, 0, 0];
        self.write_payload(&mut out);
        let length = u32::try_from(out.len() - HEADER).expect("a frame fits u32");
        debug_assert!(
            length as usize <= MAX_PAYLOAD,
            "an oversized frame was built"
        );
        out[1..HEADER].copy_from_slice(&length.to_be_bytes());
        out
    }
}

const HELLO: u8 = 0x01;
const INPUT: u8 = 0x02;
const RESIZE: u8 = 0x03;
const TAKE_SOLE: u8 = 0x04;
const DETACH: u8 = 0x05;
const WELCOME: u8 = 0x81;
const OUTPUT: u8 = 0x82;
const EXITED: u8 = 0x83;
const EVICTED: u8 = 0x84;
const REFUSED: u8 = 0x85;
const RESIZED: u8 = 0x86;

fn put_size(out: &mut Vec<u8>, size: Size) {
    out.extend_from_slice(&size.cols.to_be_bytes());
    out.extend_from_slice(&size.rows.to_be_bytes());
}

fn size_of(payload: &[u8]) -> Result<Size, ProtocolError> {
    let [c0, c1, r0, r1] = payload else {
        return Err(ProtocolError::Malformed("a size is four bytes"));
    };
    Size::new(
        u16::from_be_bytes([*c0, *c1]),
        u16::from_be_bytes([*r0, *r1]),
    )
}

fn bytes(payload: &[u8], limit: usize) -> Result<Vec<u8>, ProtocolError> {
    if payload.is_empty() || payload.len() > limit {
        Err(ProtocolError::Malformed(
            "a data frame carries 1 to its limit in bytes",
        ))
    } else {
        Ok(payload.to_vec())
    }
}

fn empty(payload: &[u8]) -> Result<(), ProtocolError> {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(ProtocolError::Malformed("this frame has no payload"))
    }
}

impl Frame for ClientMessage {
    fn kind(&self) -> u8 {
        match self {
            Self::Hello { .. } => HELLO,
            Self::Input(_) => INPUT,
            Self::Resize(_) => RESIZE,
            Self::TakeSole => TAKE_SOLE,
            Self::Detach => DETACH,
        }
    }

    fn write_payload(&self, out: &mut Vec<u8>) {
        match self {
            Self::Hello { version } => out.extend_from_slice(&version.to_be_bytes()),
            Self::Input(data) => out.extend_from_slice(data),
            Self::Resize(size) => put_size(out, *size),
            Self::TakeSole | Self::Detach => {}
        }
    }

    fn read(kind: u8, payload: &[u8]) -> Result<Self, ProtocolError> {
        match kind {
            HELLO => {
                let [v0, v1] = payload else {
                    return Err(ProtocolError::Malformed("a hello is two bytes"));
                };
                Ok(Self::Hello {
                    version: u16::from_be_bytes([*v0, *v1]),
                })
            }
            INPUT => bytes(payload, MAX_INPUT).map(Self::Input),
            RESIZE => size_of(payload).map(Self::Resize),
            TAKE_SOLE => empty(payload).map(|()| Self::TakeSole),
            DETACH => empty(payload).map(|()| Self::Detach),
            other => Err(ProtocolError::UnknownKind(other)),
        }
    }
}

impl Frame for ServerMessage {
    fn kind(&self) -> u8 {
        match self {
            Self::Welcome { .. } => WELCOME,
            Self::Output(_) => OUTPUT,
            Self::Resized(_) => RESIZED,
            Self::Exited(_) => EXITED,
            Self::Evicted(_) => EVICTED,
            Self::Refused(_) => REFUSED,
        }
    }

    fn write_payload(&self, out: &mut Vec<u8>) {
        match self {
            Self::Welcome { version, size } => {
                out.extend_from_slice(&version.to_be_bytes());
                put_size(out, *size);
            }
            Self::Output(data) => out.extend_from_slice(data),
            Self::Resized(size) => put_size(out, *size),
            Self::Exited(exit) => {
                let (tag, value) = match exit {
                    Exit::Code(code) => (0, *code),
                    Exit::Signal(signal) => (1, *signal),
                };
                out.push(tag);
                out.extend_from_slice(&value.to_be_bytes());
            }
            Self::Evicted(reason) => out.push(match reason {
                Eviction::TakenOver => 0,
                Eviction::TooSlow => 1,
            }),
            Self::Refused(reason) => out.push(match reason {
                Refusal::Version => 0,
                Refusal::Handshake => 1,
                Refusal::Owner => 2,
                Refusal::Full => 3,
            }),
        }
    }

    fn read(kind: u8, payload: &[u8]) -> Result<Self, ProtocolError> {
        match kind {
            WELCOME => {
                let [v0, v1, rest @ ..] = payload else {
                    return Err(ProtocolError::Malformed("a welcome is six bytes"));
                };
                Ok(Self::Welcome {
                    version: u16::from_be_bytes([*v0, *v1]),
                    size: size_of(rest)?,
                })
            }
            OUTPUT => bytes(payload, MAX_OUTPUT).map(Self::Output),
            RESIZED => size_of(payload).map(Self::Resized),
            EXITED => {
                let [tag, a, b, c, d] = payload else {
                    return Err(ProtocolError::Malformed("an exit is five bytes"));
                };
                let value = i32::from_be_bytes([*a, *b, *c, *d]);
                match tag {
                    0 => Ok(Self::Exited(Exit::Code(value))),
                    1 => Ok(Self::Exited(Exit::Signal(value))),
                    _ => Err(ProtocolError::Malformed("an exit is a code or a signal")),
                }
            }
            EVICTED => match payload {
                [0] => Ok(Self::Evicted(Eviction::TakenOver)),
                [1] => Ok(Self::Evicted(Eviction::TooSlow)),
                _ => Err(ProtocolError::Malformed("unknown eviction reason")),
            },
            REFUSED => match payload {
                [0] => Ok(Self::Refused(Refusal::Version)),
                [1] => Ok(Self::Refused(Refusal::Handshake)),
                [2] => Ok(Self::Refused(Refusal::Owner)),
                [3] => Ok(Self::Refused(Refusal::Full)),
                _ => Err(ProtocolError::Malformed("unknown refusal reason")),
            },
            other => Err(ProtocolError::UnknownKind(other)),
        }
    }
}

/// Incremental frame decoding over a byte stream. It holds at most one frame
/// of undecoded bytes beyond what the caller has pushed and not yet drained.
#[derive(Debug, Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends received bytes. Refused when the undecoded bytes could not be
    /// part of valid frames, so a peer cannot make the buffer grow without bound.
    pub fn push(&mut self, received: &[u8]) -> Result<(), ProtocolError> {
        if self.buffer.len() + received.len() > HEADER + MAX_PAYLOAD + MAX_PAYLOAD {
            return Err(ProtocolError::Oversized);
        }
        self.buffer.extend_from_slice(received);
        Ok(())
    }

    /// The next whole frame, or `None` until more bytes arrive.
    pub fn frame<F: Frame>(&mut self) -> Result<Option<F>, ProtocolError> {
        if self.buffer.len() < HEADER {
            return Ok(None);
        }
        let length = u32::from_be_bytes([
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
            self.buffer[4],
        ]) as usize;
        if length > MAX_PAYLOAD {
            return Err(ProtocolError::Oversized);
        }
        if self.buffer.len() < HEADER + length {
            return Ok(None);
        }
        let frame = F::read(self.buffer[0], &self.buffer[HEADER..HEADER + length])?;
        self.buffer.drain(..HEADER + length);
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(cols: u16, rows: u16) -> Size {
        Size::new(cols, rows).unwrap()
    }

    fn round_trip<F: Frame + PartialEq + fmt::Debug>(frame: F) {
        let mut decoder = Decoder::new();
        decoder.push(&frame.encode()).unwrap();
        assert_eq!(decoder.frame::<F>().unwrap(), Some(frame));
        assert_eq!(decoder.frame::<F>().unwrap(), None);
    }

    #[test]
    fn every_frame_round_trips() {
        round_trip(ClientMessage::Hello { version: VERSION });
        round_trip(ClientMessage::Input(b"ls\r".to_vec()));
        round_trip(ClientMessage::Resize(size(200, 50)));
        round_trip(ClientMessage::TakeSole);
        round_trip(ClientMessage::Detach);
        round_trip(ServerMessage::Welcome {
            version: VERSION,
            size: size(80, 24),
        });
        round_trip(ServerMessage::Output(vec![0, 27, b'[', 255]));
        round_trip(ServerMessage::Resized(size(1, 1)));
        round_trip(ServerMessage::Exited(Exit::Code(-3)));
        round_trip(ServerMessage::Exited(Exit::Signal(9)));
        round_trip(ServerMessage::Evicted(Eviction::TakenOver));
        round_trip(ServerMessage::Evicted(Eviction::TooSlow));
        for reason in [
            Refusal::Version,
            Refusal::Handshake,
            Refusal::Owner,
            Refusal::Full,
        ] {
            round_trip(ServerMessage::Refused(reason));
        }
    }

    #[test]
    fn frames_arrive_in_pieces_and_in_batches() {
        let first = ClientMessage::Input(b"abc".to_vec()).encode();
        let second = ClientMessage::Detach.encode();
        let mut decoder = Decoder::new();
        // One byte at a time: nothing until the whole first frame is there.
        for byte in &first[..first.len() - 1] {
            decoder.push(&[*byte]).unwrap();
            assert_eq!(decoder.frame::<ClientMessage>().unwrap(), None);
        }
        let mut rest = first[first.len() - 1..].to_vec();
        rest.extend_from_slice(&second);
        decoder.push(&rest).unwrap();
        assert_eq!(
            decoder.frame::<ClientMessage>().unwrap(),
            Some(ClientMessage::Input(b"abc".to_vec()))
        );
        assert_eq!(
            decoder.frame::<ClientMessage>().unwrap(),
            Some(ClientMessage::Detach)
        );
        assert_eq!(decoder.frame::<ClientMessage>().unwrap(), None);
    }

    #[test]
    fn an_invalid_frame_is_an_error_not_a_guess() {
        let decode = |bytes: &[u8]| {
            let mut decoder = Decoder::new();
            decoder.push(bytes).unwrap();
            decoder.frame::<ClientMessage>()
        };
        // Unknown kind; a server kind sent as a client kind.
        assert_eq!(
            decode(&[0x7f, 0, 0, 0, 0]),
            Err(ProtocolError::UnknownKind(0x7f))
        );
        assert_eq!(
            decode(&[WELCOME, 0, 0, 0, 0]),
            Err(ProtocolError::UnknownKind(WELCOME))
        );
        // A payload where none belongs, and an empty or oversized data frame.
        assert!(decode(&[DETACH, 0, 0, 0, 1, 0]).is_err());
        assert!(decode(&[INPUT, 0, 0, 0, 0]).is_err());
        let mut too_much = vec![INPUT];
        too_much.extend_from_slice(&((MAX_INPUT as u32) + 1).to_be_bytes());
        too_much.extend(vec![b'x'; MAX_INPUT + 1]);
        assert!(decode(&too_much).is_err());
        // A short size, a zero dimension and one past the limit.
        assert!(decode(&[RESIZE, 0, 0, 0, 3, 0, 80, 0]).is_err());
        assert!(decode(&[RESIZE, 0, 0, 0, 4, 0, 0, 0, 24]).is_err());
        assert!(
            decode(&[RESIZE, 0, 0, 0, 4, 0x27, 0x11, 0, 24]).is_err(),
            "10001 columns"
        );
    }

    #[test]
    fn the_length_is_bounded_before_anything_is_buffered_for_it() {
        let mut decoder = Decoder::new();
        let mut header = vec![OUTPUT];
        header.extend_from_slice(&((MAX_PAYLOAD as u32) + 1).to_be_bytes());
        decoder.push(&header).unwrap();
        assert_eq!(
            decoder.frame::<ServerMessage>(),
            Err(ProtocolError::Oversized)
        );
        // A peer that keeps sending cannot grow the buffer past two frames.
        let mut flood = Decoder::new();
        assert!(flood.push(&vec![0; HEADER + 2 * MAX_PAYLOAD]).is_ok());
        assert_eq!(flood.push(&[0]), Err(ProtocolError::Oversized));
    }

    #[test]
    fn exit_and_reason_bytes_are_closed_sets() {
        let decode = |bytes: &[u8]| {
            let mut decoder = Decoder::new();
            decoder.push(bytes).unwrap();
            decoder.frame::<ServerMessage>()
        };
        assert!(decode(&[EXITED, 0, 0, 0, 5, 2, 0, 0, 0, 0]).is_err());
        assert!(decode(&[EXITED, 0, 0, 0, 4, 0, 0, 0, 0]).is_err());
        assert!(decode(&[EVICTED, 0, 0, 0, 1, 2]).is_err());
        assert!(decode(&[REFUSED, 0, 0, 0, 1, 4]).is_err());
    }
}
