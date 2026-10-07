//! The bus over real Unix sockets: direction and correlation are checked before a frame
//! is queued or written, the reader never waits on a consumer, every fault is final and
//! sticky, and ending the bus joins its thread and releases the socket.
use super::super::tests::{
    GEN, admission, admit, admit_read, answer, big_read_result, done, generation, origin_state_n,
    quick, read_request, request, transfer,
};
use super::super::{Caps, Reason, Role};
use super::*;
use crate::{Expect, Offer, accept, initiate};
use std::{
    io::{Read, Write},
    sync::Weak,
};

const OTHER_GEN: &str = "7f3c1a52-9d4e-4b86-8a21-5c0e9b7d3f15";

/// Two buses joined by a real handshake: (Remote, extension).
fn buses() -> (Bus, Bus) {
    let (a, b) = UnixStream::pair().unwrap();
    let setup = Instant::now() + Duration::from_secs(5);
    let extension = thread::spawn(move || {
        let expect = Expect {
            host: "127.0.0.1:4100".into(),
            mount: "colab".into(),
        };
        accept(b, &expect, &quick(), setup)
    });
    let offer = Offer {
        host: "127.0.0.1:4100".into(),
        mount: "colab".into(),
        generation: generation(GEN),
    };
    let remote = initiate(a, &offer, setup).unwrap();
    let extension = extension.join().unwrap().unwrap();
    (
        Bus::start(remote, quick(), Caps::contract(), None).unwrap(),
        Bus::start(extension, quick(), Caps::contract(), None).unwrap(),
    )
}
/// One bus over a socket pair, the raw other end, and a way to make more.
fn bus_over(stream: UnixStream, role: Role, budget: Option<Budget>) -> Bus {
    let (reader, writer) = super::super::halves(stream, Vec::new()).unwrap();
    let link = Link {
        generation: generation(GEN),
        role,
        reader,
        writer,
    };
    Bus::start(link, quick(), Caps::contract(), budget).unwrap()
}
fn bus_and_peer(role: Role) -> (Bus, UnixStream) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    (bus_over(ours, role, None), theirs)
}
fn soon(millis: u64) -> Option<Instant> {
    Some(Instant::now() + Duration::from_millis(millis))
}
fn next(bus: &Bus) -> Result<Frame, Fault> {
    bus.recv(soon(5000))
}
fn write(peer: &mut UnixStream, frame: &Frame) {
    peer.write_all(&crate::encode(frame).unwrap()).unwrap();
}

#[test]
fn a_conversation_runs_end_to_end_in_order() {
    let (remote, extension) = buses();
    let t = transfer(1);
    extension.send(&request(1, t)).unwrap();
    assert_eq!(next(&remote).unwrap(), request(1, t));
    remote.send(&admit(1, 1, t)).unwrap();
    assert_eq!(next(&extension).unwrap(), admit(1, 1, t));
    extension.send(&answer(1, 1)).unwrap();
    assert_eq!(next(&remote).unwrap(), answer(1, 1));
    remote.send(&done(1, t)).unwrap();
    assert_eq!(next(&extension).unwrap(), done(1, t));
    assert_eq!(
        (remote.role(), extension.role()),
        (Role::Remote, Role::Extension)
    );
    assert_eq!(remote.generation(), generation(GEN));
    assert_eq!((remote.fault(), extension.fault()), (None, None));
}

#[test]
fn a_frame_this_end_may_not_send_is_returned_and_nothing_is_written() {
    let (remote, extension) = buses();
    let t = transfer(1);
    let refused = |reason| Err(Fault::Correlation(reason));
    assert_eq!(remote.send(&request(1, t)), refused(Reason::Direction));
    assert_eq!(extension.send(&done(1, t)), refused(Reason::Direction));
    extension.send(&request(2, t)).unwrap();
    assert_eq!(extension.send(&request(2, t)), refused(Reason::Order));
    assert_eq!(extension.send(&request(1, t)), refused(Reason::Order));
    // Another generation never leaves this end either.
    assert_eq!(
        extension.send(&admission(OTHER_GEN, 1)),
        Err(Fault::Generation)
    );
    // The channel is still good, and the peer saw only the one request.
    assert_eq!(next(&remote).unwrap(), request(2, t));
    assert_eq!(remote.recv(soon(100)), Err(Fault::Timeout(Stage::Idle)));
    assert_eq!((remote.fault(), extension.fault()), (None, None));
    remote.send(&done(2, t)).unwrap();
    assert_eq!(next(&extension).unwrap(), done(2, t));
}

/// Each row: our role, frames written first that must be delivered, the frame that
/// breaks the channel, and the fault it ends with.
#[test]
fn a_received_frame_that_breaks_direction_or_correlation_ends_the_channel_for_good() {
    let t = transfer(1);
    #[rustfmt::skip]
    let rows: Vec<(&str, Role, Vec<Frame>, Frame, Fault)> = vec![
        ("a result nobody asked for", Role::Extension, vec![], done(1, t), Fault::Correlation(Reason::Unknown)),
        ("a request sent to the extension", Role::Extension, vec![], request(1, t), Fault::Correlation(Reason::Direction)),
        ("a result sent to Remote", Role::Remote, vec![], done(1, t), Fault::Correlation(Reason::Direction)),
        ("a repeated request id", Role::Remote, vec![request(1, t)], request(1, t), Fault::Correlation(Reason::Order)),
        ("a lower request id", Role::Remote, vec![request(4, t)], request(3, t), Fault::Correlation(Reason::Order)),
        ("an answer to no callback", Role::Remote, vec![request(1, t)], answer(1, 1), Fault::Correlation(Reason::Unknown)),
        ("a frame of another generation", Role::Remote, vec![], admission(OTHER_GEN, 1), Fault::Generation),
    ];
    for (name, role, before, bad, fault) in rows {
        let (bus, mut peer) = bus_and_peer(role);
        for frame in before.iter().chain([&bad]) {
            write(&mut peer, frame);
        }
        for frame in &before {
            assert_eq!(&next(&bus).unwrap(), frame, "{name}");
        }
        assert_eq!(next(&bus), Err(fault), "{name}");
        // Final and sticky: later calls repeat the fault, and the socket is closed.
        assert_eq!(bus.fault(), Some(fault), "{name}");
        assert_eq!(bus.send(&request(9, t)), Err(fault), "{name}");
        assert_eq!(next(&bus), Err(fault), "{name}");
        peer.read_to_end(&mut Vec::new()).unwrap();
    }
}

#[test]
fn a_late_reply_never_satisfies_a_successor() {
    let (bus, mut peer) = bus_and_peer(Role::Extension);
    let (t1, t2) = (transfer(1), transfer(2));
    bus.send(&request(1, t1)).unwrap();
    write(&mut peer, &done(1, t1));
    assert_eq!(next(&bus).unwrap(), done(1, t1));
    // The first request is over; its result arriving again, while a successor is
    // outstanding, is not an answer to the successor.
    bus.send(&request(2, t2)).unwrap();
    write(&mut peer, &done(1, t1));
    assert_eq!(next(&bus), Err(Fault::Correlation(Reason::Unknown)));
    assert_eq!(
        bus.send(&request(3, t1)),
        Err(Fault::Correlation(Reason::Unknown))
    );
}

#[test]
fn bad_bytes_end_the_channel_with_the_exact_fault() {
    let good = crate::encode(&request(1, transfer(1))).unwrap();
    let rows: Vec<(&str, Vec<u8>, Fault)> = vec![
        ("end between frames", vec![], Fault::Closed),
        (
            "end inside a frame",
            good[..good.len() - 1].to_vec(),
            Fault::Truncated,
        ),
        (
            "one over the bound",
            vec![0, 1, 0, 1],
            Fault::Frame(crate::ErrorClass::Length),
        ),
        (
            "not JSON",
            [vec![0, 0, 0, 6], b"{nope}".to_vec()].concat(),
            Fault::Frame(crate::ErrorClass::Syntax),
        ),
    ];
    for (name, bytes, fault) in rows {
        let (bus, mut peer) = bus_and_peer(Role::Remote);
        peer.write_all(&bytes).unwrap();
        drop(peer);
        assert_eq!(next(&bus), Err(fault), "{name}");
        assert_eq!(bus.send(&done(1, transfer(1))), Err(fault), "{name}");
    }
}

#[test]
fn a_partial_frame_ends_at_the_bound_even_when_bytes_keep_arriving() {
    let good = crate::encode(&request(1, transfer(1))).unwrap();
    let (bus, mut peer) = bus_and_peer(Role::Remote);
    let trickle = thread::spawn(move || {
        for byte in good {
            if peer.write_all(&[byte]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
    });
    let started = Instant::now();
    assert_eq!(next(&bus), Err(Fault::Timeout(Stage::Frame)));
    assert!(started.elapsed() < quick().frame * 4);
    drop(bus);
    trickle.join().unwrap();
}

/// Remote answers reads with 32 KiB results while the extension never receives, so
/// its queue fills, its reader pauses and the sockets fill: Remote's write has to end.
fn stall(remote: &Bus, extension: &Bus) -> (Fault, Duration) {
    for id in 1..=2000 {
        extension.send(&read_request(id)).unwrap();
        assert_eq!(next(remote).unwrap(), read_request(id));
        let started = Instant::now();
        if let Err(fault) = remote.send(&big_read_result(id)) {
            return (fault, started.elapsed());
        }
    }
    panic!("the sockets never filled");
}

#[test]
fn a_stalled_write_ends_the_channel_at_the_write_bound() {
    let (remote, extension) = buses();
    let (fault, took) = stall(&remote, &extension);
    assert_eq!(fault, Fault::Timeout(Stage::Write));
    assert!(took >= quick().write - Duration::from_millis(5) && took < quick().write * 4);
    assert_eq!(remote.fault(), Some(fault));
    assert_eq!(remote.send(&done(1, transfer(1))), Err(fault));
    // The stalled end closed its socket, so the other end learns of it.
    assert!(extension.recv(soon(5000)).is_ok() || extension.fault().is_some());
}

#[test]
fn the_inbound_queue_holds_eight_frames_and_reading_pauses_behind_it() {
    let (bus, mut peer) = bus_and_peer(Role::Extension);
    let frames: Vec<Frame> = (1..=12).map(origin_state_n).collect();
    for frame in &frames {
        write(&mut peer, frame);
    }
    // The reader fills the queue to eight and then waits, without a fault.
    let started = Instant::now();
    while locked(&bus.shared.inbox).frames.len() < 8 && started.elapsed() < Duration::from_secs(5) {
        thread::yield_now();
    }
    assert_eq!(locked(&bus.shared.inbox).frames.len(), 8);
    assert_eq!(bus.fault(), None);
    for frame in &frames {
        assert_eq!(&next(&bus).unwrap(), frame);
    }
    assert_eq!(bus.recv(soon(100)), Err(Fault::Timeout(Stage::Idle)));
}

#[test]
fn both_ends_writing_large_frames_with_callbacks_outstanding_never_deadlock() {
    let (remote, extension) = buses();
    let count = 8u64;
    thread::scope(|scope| {
        // The extension asks eight reads, answers every callback and collects results.
        let asker = scope.spawn(|| {
            for id in 1..=count {
                extension.send(&read_request(id)).unwrap();
            }
            let mut results = 0;
            while results < count {
                match next(&extension).unwrap() {
                    Frame::Admit(admit) => extension
                        .send(&answer(admit.callback_id.get(), admit.request_id.get()))
                        .unwrap(),
                    Frame::Result(_) => results += 1,
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        // Remote raises a callback per request and answers each with 32 KiB once the
        // callback comes back, writing while the extension is still writing.
        let server = scope.spawn(|| {
            let mut finished = 0;
            while finished < count {
                match next(&remote).unwrap() {
                    Frame::Request(request) => {
                        let id = request.request_id.get();
                        remote.send(&admit_read(id, id)).unwrap();
                    }
                    Frame::Admission(admission) => {
                        remote
                            .send(&big_read_result(admission.request_id.get()))
                            .unwrap();
                        finished += 1;
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        asker.join().unwrap();
        server.join().unwrap();
    });
    assert_eq!((remote.fault(), extension.fault()), (None, None));
}

#[test]
fn closing_joins_the_reader_releases_the_socket_and_wakes_every_waiter() {
    for _ in 0..20 {
        let (bus, mut peer) = bus_and_peer(Role::Remote);
        let watch: Weak<Shared> = bus.watch();
        let started = Instant::now();
        thread::scope(|scope| {
            let waiter = scope.spawn(|| bus.recv(None));
            thread::sleep(Duration::from_millis(30));
            // An end from another thread wakes a recv that has no deadline.
            bus.shared.fail(Fault::Closed);
            assert_eq!(waiter.join().unwrap(), Err(Fault::Closed));
        });
        bus.close();
        assert!(started.elapsed() < Duration::from_secs(2));
        // No thread or handle still holds the shared state, and the peer sees the end.
        assert!(watch.upgrade().is_none());
        let mut seen = Vec::new();
        peer.read_to_end(&mut seen).unwrap();
        assert!(seen.is_empty());
    }
    // Drop does the same as close.
    let (bus, _peer) = bus_and_peer(Role::Extension);
    let watch = bus.watch();
    drop(bus);
    assert!(watch.upgrade().is_none());
}

#[test]
fn ending_a_bus_whose_write_is_stalled_does_not_hang() {
    let (remote, extension) = buses();
    thread::scope(|scope| {
        let sender = scope.spawn(|| stall(&remote, &extension));
        thread::sleep(Duration::from_millis(100));
        remote.shared.fail(Fault::Closed);
        let (fault, took) = sender.join().unwrap();
        assert!(matches!(
            fault,
            Fault::Closed | Fault::Timeout(Stage::Write) | Fault::Io(_)
        ));
        assert!(took < Duration::from_secs(2));
    });
    let watch = remote.watch();
    remote.close();
    extension.close();
    assert!(watch.upgrade().is_none());
}

#[test]
fn an_installation_budget_is_returned_when_a_bus_ends() {
    let budget = Budget::new(1, 1);
    let t = transfer(1);
    let (first, _a) = {
        let (ours, theirs) = UnixStream::pair().unwrap();
        (
            bus_over(ours, Role::Extension, Some(budget.clone())),
            theirs,
        )
    };
    let (second, _b) = {
        let (ours, theirs) = UnixStream::pair().unwrap();
        (bus_over(ours, Role::Extension, Some(budget)), theirs)
    };
    first.send(&request(1, t)).unwrap();
    assert_eq!(
        second.send(&request(1, t)),
        Err(Fault::Correlation(Reason::Capacity))
    );
    first.close();
    second.send(&request(1, t)).unwrap();
}
