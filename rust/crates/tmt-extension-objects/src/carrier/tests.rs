//! Frame I/O over real Unix sockets. Every refusal is paired with an accepted twin it
//! differs from by one token, and every bound is shown on the clock with short budgets
//! rather than by sleeping for a fixed time.
use super::*;
use crate::{
    Admission, Chunk, Counter, Decision, Method, Outcome, ResultFrame, Success,
    limits::PREFIX_BYTES,
};
use std::{
    io::{Read, Write},
    sync::atomic::Ordering,
    thread,
};

pub(super) const GEN: &str = "7f3c1a52-9d4e-4b86-8a21-5c0e9b7d3f14";
const OTHER_GEN: &str = "7f3c1a52-9d4e-4b86-8a21-5c0e9b7d3f15";
pub(super) static NEVER: AtomicBool = AtomicBool::new(false);

pub(super) fn generation(text: &str) -> Uuid4 {
    Uuid4::parse(text).unwrap()
}
/// Short bounds so the clock cases finish quickly; the shapes are the contract's.
pub(super) fn quick() -> Budgets {
    Budgets {
        head: Duration::from_millis(150),
        reply: Duration::from_millis(150),
        frame: Duration::from_millis(200),
        write: Duration::from_millis(200),
    }
}
pub(super) fn offer() -> Offer {
    Offer {
        host: "127.0.0.1:4100".into(),
        mount: "colab".into(),
        generation: generation(GEN),
    }
}
pub(super) fn expect() -> Expect {
    Expect {
        host: "127.0.0.1:4100".into(),
        mount: "colab".into(),
    }
}
pub(super) fn setup() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
pub(super) fn idle() -> Idle<'static> {
    Idle {
        stop: &NEVER,
        until: Some(Instant::now() + Duration::from_secs(5)),
    }
}
pub(super) fn admission(generation_text: &str, callback: u64) -> Frame {
    Frame::Admission(Admission {
        generation: generation(generation_text),
        callback_id: Counter::new(callback).unwrap(),
        request_id: Counter::new(1).unwrap(),
        decision: Decision::Allow,
    })
}
fn big_result() -> Frame {
    Frame::Result(ResultFrame {
        generation: generation(GEN),
        request_id: Counter::new(1).unwrap(),
        method: Method::Read,
        transfer_id: None,
        outcome: Outcome::Success(Success::Read {
            offset: 0,
            total_bytes: 32_768,
            bytes: Chunk::new(vec![0xa5; 32_768]).unwrap(),
        }),
    })
}

impl Link {
    pub(super) fn read_frame(&mut self, idle: Idle<'_>, budgets: &Budgets) -> Result<Frame, Fault> {
        read_checked(&mut self.reader, self.generation, idle, budgets).map(|(frame, _)| frame)
    }
    pub(super) fn write_frame(&mut self, frame: &Frame, budgets: &Budgets) -> Result<(), Fault> {
        let bytes = prepared(self.generation, frame)?;
        self.writer.send(&bytes, budgets.write)
    }
}

/// A link over one end of a socket pair and the raw other end.
fn raw_link() -> (Link, UnixStream) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    let (reader, writer) = halves(ours, Vec::new()).unwrap();
    let link = Link {
        generation: generation(GEN),
        role: Role::Extension,
        reader,
        writer,
    };
    (link, theirs)
}
/// Two links joined by a real handshake over a socket pair.
pub(super) fn linked() -> (Link, Link) {
    let (a, b) = UnixStream::pair().unwrap();
    let accepted = thread::spawn(move || accept(b, &expect(), &quick(), setup()));
    let initiated = initiate(a, &offer(), setup()).unwrap();
    (initiated, accepted.join().unwrap().unwrap())
}

#[test]
fn frames_cross_both_ways_exactly_and_in_order() {
    let (mut left, mut right) = linked();
    assert_eq!(left.generation(), generation(GEN));
    for callback in 1..=5 {
        let sent = admission(GEN, callback);
        left.write_frame(&sent, &quick()).unwrap();
        assert_eq!(right.read_frame(idle(), &quick()).unwrap(), sent);
        let back = admission(GEN, callback + 100);
        right.write_frame(&back, &quick()).unwrap();
        assert_eq!(left.read_frame(idle(), &quick()).unwrap(), back);
    }
    // Two frames in one burst are read as two.
    let (first, second) = (admission(GEN, 7), admission(GEN, 8));
    left.write_frame(&first, &quick()).unwrap();
    left.write_frame(&second, &quick()).unwrap();
    assert_eq!(right.read_frame(idle(), &quick()).unwrap(), first);
    assert_eq!(right.read_frame(idle(), &quick()).unwrap(), second);
}

#[test]
fn a_frame_sent_with_the_reply_is_not_lost() {
    let (a, b) = UnixStream::pair().unwrap();
    let accepted = thread::spawn(move || {
        let mut link = accept(b, &expect(), &quick(), setup()).unwrap();
        link.write_frame(&admission(GEN, 1), &quick()).unwrap();
        link
    });
    let mut initiated = initiate(a, &offer(), setup()).unwrap();
    assert_eq!(
        initiated.read_frame(idle(), &quick()).unwrap(),
        admission(GEN, 1)
    );
    drop(accepted.join().unwrap());
}

/// Each row: what the peer sends before closing, and the fault the read ends with.
#[test]
fn malformed_streams_end_the_read_with_the_exact_fault() {
    let good = crate::encode(&admission(GEN, 1)).unwrap();
    let body = &good[PREFIX_BYTES..];
    let framed = |bytes: &[u8]| {
        let mut out = (bytes.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(bytes);
        out
    };
    let moved_generation = String::from_utf8(body.to_vec())
        .unwrap()
        .replace(GEN, OTHER_GEN);
    #[rustfmt::skip]
    let rows: Vec<(&str, Vec<u8>, Fault)> = vec![
        ("closed between frames", vec![], Fault::Closed),
        ("two prefix bytes then EOF", good[..2].to_vec(), Fault::Truncated),
        ("prefix then EOF", good[..4].to_vec(), Fault::Truncated),
        ("last body byte missing", good[..good.len() - 1].to_vec(), Fault::Truncated),
        ("zero length", vec![0, 0, 0, 0], Fault::Frame(ErrorClass::Length)),
        ("one byte", vec![0, 0, 0, 1, b'{'], Fault::Frame(ErrorClass::Length)),
        ("one over the bound", vec![0, 1, 0, 1], Fault::Frame(ErrorClass::Length)),
        ("32-bit maximum", vec![0xff; 4], Fault::Frame(ErrorClass::Length)),
        ("not UTF-8", framed(&[0xff, 0xfe]), Fault::Frame(ErrorClass::Utf8)),
        ("not JSON", framed(b"{nope}"), Fault::Frame(ErrorClass::Syntax)),
        ("an empty object", framed(b"{}"), Fault::Frame(ErrorClass::Shape)),
        ("another generation", framed(moved_generation.as_bytes()), Fault::Generation),
    ];
    // The accepted twin of every row.
    let (mut link, mut peer) = raw_link();
    peer.write_all(&good).unwrap();
    assert!(link.read_frame(idle(), &quick()).is_ok());
    for (name, bytes, fault) in rows {
        let (mut link, mut peer) = raw_link();
        peer.write_all(&bytes).unwrap();
        drop(peer);
        assert_eq!(link.read_frame(idle(), &quick()), Err(fault), "{name}");
    }
}

#[test]
fn the_frame_bound_runs_from_the_first_byte_and_is_never_renewed() {
    let good = crate::encode(&admission(GEN, 1)).unwrap();
    let budget = quick().frame;
    // Partial prefix, partial body, and silence after each.
    for sent in [1, 3, 4, 20] {
        let (mut link, mut peer) = raw_link();
        peer.write_all(&good[..sent]).unwrap();
        let started = Instant::now();
        assert_eq!(
            link.read_frame(idle(), &quick()),
            Err(Fault::Timeout(Stage::Frame)),
            "{sent} bytes"
        );
        let took = started.elapsed();
        assert!(
            took >= budget - Duration::from_millis(5) && took < budget * 4,
            "{took:?}"
        );
        drop(peer);
    }
    // A byte every so often would finish in seconds; the bound does not move.
    let (mut link, mut peer) = raw_link();
    let trickle = thread::spawn(move || {
        for byte in good {
            if peer.write_all(&[byte]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
    });
    let started = Instant::now();
    assert_eq!(
        link.read_frame(idle(), &quick()),
        Err(Fault::Timeout(Stage::Frame))
    );
    assert!(started.elapsed() < budget * 4);
    drop(link);
    trickle.join().unwrap();
}

#[test]
fn waiting_for_a_frame_ends_on_stop_or_the_callers_own_deadline() {
    let (mut link, peer) = raw_link();
    let limit = Duration::from_millis(120);
    let started = Instant::now();
    let bounded = Idle {
        stop: &NEVER,
        until: Some(started + limit),
    };
    assert_eq!(
        link.read_frame(bounded, &quick()),
        Err(Fault::Timeout(Stage::Idle))
    );
    assert!(started.elapsed() >= limit - Duration::from_millis(5));

    let stop = AtomicBool::new(false);
    let started = Instant::now();
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(Duration::from_millis(80));
            stop.store(true, Ordering::Release);
        });
        link.read_frame(
            Idle {
                stop: &stop,
                until: None,
            },
            &quick(),
        )
    });
    assert_eq!(result, Err(Fault::Closed));
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(peer);
}

#[test]
fn a_peer_that_stops_reading_ends_the_write_at_its_bound() {
    let (mut link, _peer) = raw_link();
    let frame = big_result();
    let budget = quick().write;
    let mut outcome = None;
    for _ in 0..2000 {
        let started = Instant::now();
        if let Err(fault) = link.write_frame(&frame, &quick()) {
            outcome = Some((fault, started.elapsed()));
            break;
        }
    }
    let (fault, took) = outcome.expect("the socket buffer never filled");
    assert_eq!(fault, Fault::Timeout(Stage::Write));
    assert!(
        took >= budget - Duration::from_millis(5) && took < budget * 4,
        "{took:?}"
    );
}

#[test]
fn writes_refuse_a_wrong_generation_and_a_closed_peer() {
    let (mut link, mut peer) = raw_link();
    assert_eq!(
        link.write_frame(&admission(OTHER_GEN, 1), &quick()),
        Err(Fault::Generation)
    );
    // Nothing was sent for the refused frame.
    peer.set_nonblocking(true).unwrap();
    assert_eq!(
        peer.read(&mut [0; 1]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(peer);
    assert_eq!(
        link.write_frame(&admission(GEN, 1), &quick()),
        Err(Fault::Io(std::io::ErrorKind::BrokenPipe))
    );
}

// Builders for the correlated frames the ledger and bus tests exchange. Every one is
// a `commit` conversation about one transfer unless a name says otherwise.
use crate::{
    Admit, AdmitInput, Bytes32, Call, Checkpoint, Context, Operation, Origin, OriginPhase,
    OriginState, Policy, ReadInput, Request, Retained, Sha256Hex, TransferAdmit, TransferInput,
};

pub(super) fn transfer(n: u8) -> Uuid4 {
    generation(&format!("0b5e6d1c-2a47-4f93-b8e0-61c4d7a92f{n:02x}"))
}
fn counter(n: u64) -> Counter {
    Counter::new(n).unwrap()
}
fn retained() -> Retained {
    Retained {
        namespace: Bytes32::from_bytes([1; 32]),
        opaque_key: Bytes32::from_bytes([2; 32]),
        policy: Policy::new(Vec::new()).unwrap(),
        payload_sha256: Sha256Hex::from_bytes([3; 32]),
        payload_bytes: 3,
    }
}
/// A `commit` request about `transfer`.
pub(super) fn request(id: u64, transfer: Uuid4) -> Frame {
    Frame::Request(Request {
        generation: generation(GEN),
        request_id: counter(id),
        origin: Origin::LocalExtension,
        call: Call::Commit(TransferInput {
            transfer_id: transfer,
        }),
    })
}
/// A `read` request, which names no transfer.
pub(super) fn read_request(id: u64) -> Frame {
    Frame::Request(Request {
        generation: generation(GEN),
        request_id: counter(id),
        origin: Origin::LocalExtension,
        call: Call::Read(ReadInput {
            namespace: Bytes32::from_bytes([1; 32]),
            opaque_key: Bytes32::from_bytes([2; 32]),
            policy: Policy::new(Vec::new()).unwrap(),
            payload_sha256: Sha256Hex::from_bytes([3; 32]),
            payload_bytes: 3,
            offset: 0,
            count: 3,
        }),
    })
}
fn admit_about(callback: u64, request: u64, input: AdmitInput) -> Frame {
    Frame::Admit(Admit {
        generation: generation(GEN),
        callback_id: counter(callback),
        request_id: counter(request),
        boundary: Checkpoint::Acquire,
        context: Context::LocalExtension,
        operation: Operation {
            input,
            disclosure: None,
        },
    })
}
/// An acquire callback for a `commit` of `transfer`.
pub(super) fn admit(callback: u64, request: u64, transfer: Uuid4) -> Frame {
    admit_about(
        callback,
        request,
        AdmitInput::Commit(TransferAdmit {
            transfer_id: transfer,
            retained: retained(),
        }),
    )
}
/// An acquire callback for a `read`.
pub(super) fn admit_read(callback: u64, request: u64) -> Frame {
    admit_about(
        callback,
        request,
        AdmitInput::Read(ReadInput {
            namespace: Bytes32::from_bytes([1; 32]),
            opaque_key: Bytes32::from_bytes([2; 32]),
            policy: Policy::new(Vec::new()).unwrap(),
            payload_sha256: Sha256Hex::from_bytes([3; 32]),
            payload_bytes: 3,
            offset: 0,
            count: 3,
        }),
    )
}
pub(super) fn answer(callback: u64, request: u64) -> Frame {
    Frame::Admission(Admission {
        generation: generation(GEN),
        callback_id: counter(callback),
        request_id: counter(request),
        decision: Decision::Allow,
    })
}
/// The committed result of the `commit` request `request` about `transfer`.
pub(super) fn done(request: u64, transfer: Uuid4) -> Frame {
    Frame::Result(ResultFrame {
        generation: generation(GEN),
        request_id: counter(request),
        method: Method::Commit,
        transfer_id: Some(transfer),
        outcome: Outcome::Success(Success::Committed {
            opaque_key: Bytes32::from_bytes([2; 32]),
            payload_sha256: Sha256Hex::from_bytes([3; 32]),
            payload_bytes: 3,
        }),
    })
}
/// A large result of the `read` request `request`: 32 KiB of bytes.
pub(super) fn big_read_result(request: u64) -> Frame {
    Frame::Result(ResultFrame {
        generation: generation(GEN),
        request_id: counter(request),
        method: Method::Read,
        transfer_id: None,
        outcome: Outcome::Success(Success::Read {
            offset: 0,
            total_bytes: 32_768,
            bytes: Chunk::new(vec![0xa5; 32_768]).unwrap(),
        }),
    })
}
pub(super) fn origin_state() -> Frame {
    origin_state_n(0)
}
/// A lifecycle frame whose origin differs by `n`, so a run of them stays ordered.
pub(super) fn origin_state_n(n: u8) -> Frame {
    Frame::OriginState(OriginState {
        generation: generation(GEN),
        origin_id: generation(&format!("c1d2e3f4-a5b6-4c7d-9e8f-0a1b2c3d4e{n:02x}")),
        phase: OriginPhase::Established,
    })
}
