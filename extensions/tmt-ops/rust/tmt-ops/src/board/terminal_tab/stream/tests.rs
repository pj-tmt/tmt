//! The stream thread against a fake supervisor that speaks `tmt_pty_protocol` on a Unix socket in a
//! private temporary directory. No tmux, no host server and no Core: the only peer is the test.
use super::*;
use std::{
    io::{Read, Write},
    os::unix::net::UnixListener,
    path::PathBuf,
    sync::atomic::AtomicU32,
    sync::mpsc,
};
use tmt_pty_protocol::{ClientMessage, ServerMessage};

const WAIT: Duration = Duration::from_secs(5);

static NEXT: AtomicU32 = AtomicU32::new(0);

/// Short waits, so a retry or a stop takes milliseconds.
fn quick() -> Timing {
    Timing {
        poll: Duration::from_millis(5),
        publish: Duration::from_millis(5),
        handshake: Duration::from_millis(400),
        backoff: Duration::from_millis(10),
        backoff_cap: Duration::from_millis(40),
        attempts: 3,
    }
}

fn size(cols: u16, rows: u16) -> Size {
    Size::new(cols, rows).unwrap()
}

/// A listening socket in a directory of its own.
struct Fake {
    listener: UnixListener,
    address: Address,
    dir: PathBuf,
}

impl Fake {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tmt-term-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("s");
        let listener = UnixListener::bind(&socket).unwrap();
        Self {
            listener,
            address: Address {
                socket,
                pane: "p1".into(),
            },
            dir,
        }
    }

    fn accept(&self) -> Peer {
        let (socket, _) = self.listener.accept().unwrap();
        socket.set_read_timeout(Some(WAIT)).unwrap();
        Peer {
            socket,
            decoder: Decoder::new(),
        }
    }

    /// The viewer under test, with its events on a channel.
    fn viewer(&self, timing: Timing) -> (Stream, mpsc::Receiver<StreamEvent>) {
        let (events, received) = mpsc::channel();
        let stream = Stream::open_with(self.address.clone(), timing, move |event| {
            let _ = events.send(event);
        });
        (stream, received)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Peer {
    socket: UnixStream,
    decoder: Decoder,
}

impl Peer {
    fn receive(&mut self) -> Option<ClientMessage> {
        let mut buffer = [0; 4096];
        loop {
            if let Some(message) = self.decoder.frame::<ClientMessage>().unwrap() {
                return Some(message);
            }
            match self.socket.read(&mut buffer) {
                Ok(0) | Err(_) => return None,
                Ok(read) => self.decoder.push(&buffer[..read]).unwrap(),
            }
        }
    }

    fn expect(&mut self) -> ClientMessage {
        self.receive().expect("a frame from the viewer")
    }

    fn send(&mut self, message: ServerMessage) {
        self.socket.write_all(&message.encode()).unwrap();
    }

    /// Welcome, the viewer's `Snapshot`, and a snapshot of `bytes` at `size`.
    fn attach(&mut self, size: Size, bytes: &[u8]) {
        assert_eq!(self.expect(), ClientMessage::Hello { version: VERSION });
        self.send(ServerMessage::Welcome {
            version: VERSION,
            size,
        });
        assert_eq!(self.expect(), ClientMessage::Snapshot);
        self.send(ServerMessage::SnapshotStart);
        for chunk in bytes.chunks(7) {
            self.send(ServerMessage::SnapshotData(chunk.to_vec()));
        }
        self.send(ServerMessage::SnapshotEnd(size));
    }
}

fn next(events: &mpsc::Receiver<StreamEvent>) -> StreamEvent {
    events.recv_timeout(WAIT).expect("an event")
}

/// The next screen, skipping the connection notices.
fn screen(events: &mpsc::Receiver<StreamEvent>) -> Arc<Screenful> {
    loop {
        if let StreamEvent::Screen(frame) = next(events) {
            return frame;
        }
    }
}

/// Screens until one satisfies `done`.
fn screen_where(
    events: &mpsc::Receiver<StreamEvent>,
    done: impl Fn(&Screenful) -> bool,
) -> Arc<Screenful> {
    loop {
        let frame = screen(events);
        if done(&frame) {
            return frame;
        }
    }
}

#[test]
fn the_snapshot_is_the_first_screen_and_live_output_lands_on_top_of_it() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let mut peer = fake.accept();
    assert_eq!(next(&events), StreamEvent::Connecting { attempt: 1 });
    peer.attach(size(20, 4), b"\x1b[1mhello\x1b[0m\r\nsecond");
    let first = screen(&events);
    assert_eq!(first.row_text(0), "hello");
    assert_eq!(first.row_text(1), "second");
    assert!(first.cell(0, 0).unwrap().attr.bold);
    peer.send(ServerMessage::Output(b"\x1b[3;1Hthird".to_vec()));
    let live = screen_where(&events, |frame| frame.row_text(2) == "third");
    assert_eq!(
        live.row_text(0),
        "hello",
        "the snapshot is still under the output"
    );
}

#[test]
fn output_that_comes_before_the_snapshot_is_superseded_by_it() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let mut peer = fake.accept();
    assert_eq!(peer.expect(), ClientMessage::Hello { version: VERSION });
    peer.send(ServerMessage::Welcome {
        version: VERSION,
        size: size(10, 2),
    });
    assert_eq!(peer.expect(), ClientMessage::Snapshot);
    peer.send(ServerMessage::Output(b"stale".to_vec()));
    peer.send(ServerMessage::SnapshotStart);
    peer.send(ServerMessage::SnapshotData(b"fresh".to_vec()));
    peer.send(ServerMessage::SnapshotEnd(size(10, 2)));
    let frame = screen(&events);
    assert_eq!(frame.row_text(0), "fresh");
}

#[test]
fn a_second_snapshot_start_discards_one_that_was_not_finished() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let mut peer = fake.accept();
    assert_eq!(peer.expect(), ClientMessage::Hello { version: VERSION });
    peer.send(ServerMessage::Welcome {
        version: VERSION,
        size: size(10, 2),
    });
    assert_eq!(peer.expect(), ClientMessage::Snapshot);
    peer.send(ServerMessage::SnapshotStart);
    peer.send(ServerMessage::SnapshotData(b"half".to_vec()));
    peer.send(ServerMessage::SnapshotStart);
    peer.send(ServerMessage::SnapshotData(b"whole".to_vec()));
    peer.send(ServerMessage::SnapshotEnd(size(10, 2)));
    assert_eq!(screen(&events).row_text(0), "whole");
}

#[test]
fn the_size_in_force_is_the_one_the_snapshot_and_resized_report() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let mut peer = fake.accept();
    peer.attach(size(12, 3), b"abc");
    let first = screen(&events);
    assert_eq!((first.cols, first.rows), (12, 3));
    peer.send(ServerMessage::Resized(size(30, 5)));
    let resized = screen_where(&events, |frame| frame.cols == 30);
    assert_eq!((resized.cols, resized.rows), (30, 5));
    assert_eq!(resized.row_text(0), "abc");
}

#[test]
fn a_viewer_that_only_watches_never_sends_resize_or_take_sole() {
    let fake = Fake::new();
    let (stream, events) = fake.viewer(quick());
    let mut peer = fake.accept();
    peer.attach(size(10, 2), b"x");
    screen(&events);
    peer.send(ServerMessage::Resized(size(40, 10)));
    screen_where(&events, |frame| frame.cols == 40);
    drop(stream);
    // `attach` saw Hello and Snapshot; after that the viewer sent nothing but its Detach.
    let mut sent = Vec::new();
    while let Some(message) = peer.receive() {
        sent.push(message);
    }
    assert_eq!(sent, vec![ClientMessage::Detach]);
}

#[test]
fn input_and_resize_reach_the_program_in_order_and_input_is_cut_to_frames() {
    let fake = Fake::new();
    let (stream, events) = fake.viewer(quick());
    let mut peer = fake.accept();
    peer.attach(size(10, 2), b"x");
    screen(&events);
    stream.resize(size(33, 7));
    stream.input(b"ls\r");
    let big = vec![b'a'; MAX_INPUT + 10];
    stream.input(&big);
    assert_eq!(peer.expect(), ClientMessage::Resize(size(33, 7)));
    assert_eq!(peer.expect(), ClientMessage::Input(b"ls\r".to_vec()));
    assert_eq!(peer.expect(), ClientMessage::Input(vec![b'a'; MAX_INPUT]));
    assert_eq!(peer.expect(), ClientMessage::Input(vec![b'a'; 10]));
}

#[test]
fn a_paste_is_never_cut_through_a_character() {
    let text = "é".repeat(MAX_INPUT);
    let chunks = input_chunks(text.as_bytes());
    assert!(chunks.iter().all(|chunk| chunk.len() <= MAX_INPUT));
    assert_eq!(chunks.concat(), text.as_bytes());
    for chunk in chunks {
        assert!(
            std::str::from_utf8(chunk).is_ok(),
            "a whole character at each end"
        );
    }
    assert!(input_chunks(b"").is_empty());
}

#[test]
fn input_before_the_connection_is_welcomed_is_dropped_not_queued() {
    let fake = Fake::new();
    let (stream, events) = fake.viewer(quick());
    stream.input(b"too early");
    let mut peer = fake.accept();
    assert_eq!(next(&events), StreamEvent::Connecting { attempt: 1 });
    peer.attach(size(10, 2), b"x");
    screen(&events);
    stream.input(b"on time");
    let mut got = Vec::new();
    loop {
        match peer.expect() {
            ClientMessage::Input(bytes) => {
                got.push(bytes);
                break;
            }
            ClientMessage::Hello { .. } | ClientMessage::Snapshot => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(
        got,
        vec![b"on time".to_vec()],
        "only what was typed while attached"
    );
}

#[test]
fn a_lost_connection_reconnects_with_a_new_hello_and_a_new_snapshot() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let mut first = fake.accept();
    first.attach(size(10, 2), b"before");
    assert_eq!(screen(&events).row_text(0), "before");
    drop(first);
    let mut second = fake.accept();
    second.attach(size(10, 2), b"after");
    let again = screen_where(&events, |frame| frame.row_text(0) == "after");
    assert_eq!(again.row_text(0), "after");
}

#[test]
fn a_reconnect_while_typing_asks_for_the_window_it_had_before_the_snapshot() {
    let fake = Fake::new();
    let (stream, events) = fake.viewer(quick());
    let mut first = fake.accept();
    first.attach(size(10, 2), b"x");
    screen(&events);
    stream.resize(size(50, 9));
    assert_eq!(first.expect(), ClientMessage::Resize(size(50, 9)));
    drop(first);
    let mut second = fake.accept();
    assert_eq!(second.expect(), ClientMessage::Hello { version: VERSION });
    second.send(ServerMessage::Welcome {
        version: VERSION,
        size: size(50, 9),
    });
    assert_eq!(second.expect(), ClientMessage::Resize(size(50, 9)));
    assert_eq!(second.expect(), ClientMessage::Snapshot);
}

#[test]
fn a_reconnect_after_typing_ended_sends_no_resize_and_takes_no_size_back() {
    let fake = Fake::new();
    let (stream, events) = fake.viewer(quick());
    let mut first = fake.accept();
    first.attach(size(10, 2), b"x");
    screen(&events);
    stream.resize(size(50, 9));
    assert_eq!(first.expect(), ClientMessage::Resize(size(50, 9)));
    stream.stop_typing();
    // The command is applied before the next read; give the thread a poll or two.
    thread::sleep(Duration::from_millis(50));
    drop(first);
    let mut second = fake.accept();
    assert_eq!(second.expect(), ClientMessage::Hello { version: VERSION });
    second.send(ServerMessage::Welcome {
        version: VERSION,
        size: size(10, 2),
    });
    assert_eq!(
        second.expect(),
        ClientMessage::Snapshot,
        "the watching viewer asks for the screen and nothing else"
    );
    second.send(ServerMessage::SnapshotStart);
    second.send(ServerMessage::SnapshotData(b"x".to_vec()));
    second.send(ServerMessage::SnapshotEnd(size(10, 2)));
    screen(&events);
    drop(stream);
    let mut rest = Vec::new();
    while let Some(message) = second.receive() {
        rest.push(message);
    }
    assert_eq!(rest, vec![ClientMessage::Detach], "no Resize at any point");
}

#[test]
fn a_viewer_evicted_for_being_too_slow_comes_back_and_one_taken_over_does_not() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let mut first = fake.accept();
    first.attach(size(10, 2), b"slow");
    screen(&events);
    first.send(ServerMessage::Evicted(Eviction::TooSlow));
    let mut second = fake.accept();
    second.attach(size(10, 2), b"back");
    screen_where(&events, |frame| frame.row_text(0) == "back");
    second.send(ServerMessage::Evicted(Eviction::TakenOver));
    loop {
        match next(&events) {
            StreamEvent::Evicted(Eviction::TakenOver) => break,
            StreamEvent::Screen(_) | StreamEvent::Connecting { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    fake.listener.set_nonblocking(true).unwrap();
    thread::sleep(Duration::from_millis(150));
    assert!(fake.listener.accept().is_err(), "no third connection");
}

#[test]
fn the_program_ending_is_reported_with_its_exit_and_ends_the_stream() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let mut peer = fake.accept();
    peer.attach(size(10, 2), b"bye");
    screen(&events);
    peer.send(ServerMessage::Output(b"!".to_vec()));
    peer.send(ServerMessage::Exited(Exit::Code(3)));
    let mut last_screen = None;
    loop {
        match next(&events) {
            StreamEvent::Screen(frame) => last_screen = Some(frame),
            StreamEvent::Exited(exit) => {
                assert_eq!(exit, Exit::Code(3));
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(
        last_screen.unwrap().row_text(0),
        "bye!",
        "output that arrived before the exit is shown"
    );
    assert!(
        events.recv_timeout(Duration::from_millis(100)).is_err(),
        "nothing follows"
    );
    fake.listener.set_nonblocking(true).unwrap();
    assert!(
        fake.listener.accept().is_err(),
        "the viewer does not reconnect"
    );
}

#[test]
fn a_refused_connection_says_why_and_does_not_retry() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let mut peer = fake.accept();
    assert_eq!(peer.expect(), ClientMessage::Hello { version: VERSION });
    peer.send(ServerMessage::Refused(Refusal::Owner));
    loop {
        match next(&events) {
            StreamEvent::Refused(Refusal::Owner) => break,
            StreamEvent::Connecting { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    fake.listener.set_nonblocking(true).unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(fake.listener.accept().is_err());
}

#[test]
fn a_stream_that_cannot_be_reached_is_lost_after_the_allowed_tries() {
    let fake = Fake::new();
    let address = fake.address.clone();
    drop(fake);
    let (events, received) = mpsc::channel();
    let _stream = Stream::open_with(address, quick(), move |event| {
        let _ = events.send(event);
    });
    let mut attempts = 0;
    loop {
        match next(&received) {
            StreamEvent::Connecting { attempt } => attempts = attempt,
            StreamEvent::Lost(reason) => {
                assert!(reason.contains("could not reach"), "{reason}");
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(attempts, 4, "the first try and three retries");
}

#[test]
fn a_connection_that_is_never_welcomed_is_dropped_and_retried() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let _silent = fake.accept();
    let mut second = fake.accept();
    second.attach(size(10, 2), b"ok");
    assert_eq!(screen(&events).row_text(0), "ok");
}

#[test]
fn a_frame_the_protocol_does_not_allow_ends_the_connection_and_it_comes_back() {
    let fake = Fake::new();
    let (_stream, events) = fake.viewer(quick());
    let mut first = fake.accept();
    assert_eq!(first.expect(), ClientMessage::Hello { version: VERSION });
    first.socket.write_all(&[0x7f, 0, 0, 0, 0]).unwrap();
    let mut second = fake.accept();
    second.attach(size(10, 2), b"again");
    assert_eq!(screen(&events).row_text(0), "again");
}

#[test]
fn dropping_the_stream_detaches_and_ends_the_thread() {
    let fake = Fake::new();
    let (stream, events) = fake.viewer(quick());
    let mut peer = fake.accept();
    peer.attach(size(10, 2), b"x");
    screen(&events);
    drop(stream);
    loop {
        match peer.receive() {
            Some(ClientMessage::Detach) => break,
            Some(_) => {}
            None => panic!("closed without a Detach"),
        }
    }
    // The sink is dropped with the thread, which closes the channel.
    loop {
        match events.recv_timeout(WAIT) {
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("the thread did not end"),
            Ok(_) => {}
        }
    }
}

#[test]
fn a_busy_program_is_published_at_the_publish_rate_not_per_frame() {
    let fake = Fake::new();
    let timing = Timing {
        publish: Duration::from_millis(80),
        ..quick()
    };
    let (_stream, events) = fake.viewer(timing);
    let mut peer = fake.accept();
    peer.attach(size(20, 2), b"x");
    screen(&events);
    for _ in 0..200 {
        peer.send(ServerMessage::Output(b"y".to_vec()));
    }
    thread::sleep(Duration::from_millis(200));
    let mut screens = 0;
    while let Ok(event) = events.try_recv() {
        screens += usize::from(matches!(event, StreamEvent::Screen(_)));
    }
    assert!(
        (1..=4).contains(&screens),
        "200 output frames came out as {screens} screens"
    );
}
