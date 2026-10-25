//! The terminal stream contract (`contracts/terminal-stream-v1.md`) driven by a view that uses
//! only the frame definitions and a Unix socket: no `Connection`, `Supervisor` or other adapter
//! code. It is what the Ops Terminal tab does, minus the screen it paints.
use super::*;
use std::{io::Read, io::Write, os::unix::net::UnixStream};
use tmt_adapters::pty::protocol::{
    ClientMessage, Decoder, Eviction, Exit, Frame, ServerMessage, Size, VERSION,
};

/// One connection to a member's stream.
struct View {
    stream: UnixStream,
    decoder: Decoder,
}

impl View {
    fn open(socket: &std::path::Path) -> Self {
        let stream = UnixStream::connect(socket).expect("connect to the member's stream");
        stream.set_read_timeout(Some(WAIT)).unwrap();
        let mut view = Self {
            stream,
            decoder: Decoder::new(),
        };
        view.send(&ClientMessage::Hello { version: VERSION });
        assert!(matches!(
            view.next(),
            ServerMessage::Welcome { version: 1, .. }
        ));
        view
    }

    fn send(&mut self, message: &ClientMessage) {
        self.stream.write_all(&message.encode()).unwrap();
    }

    fn next(&mut self) -> ServerMessage {
        let mut buffer = [0u8; 8192];
        loop {
            if let Some(message) = self.decoder.frame::<ServerMessage>().unwrap() {
                return message;
            }
            let count = self
                .stream
                .read(&mut buffer)
                .expect("the stream ended early");
            assert!(count > 0, "the stream closed");
            self.decoder.push(&buffer[..count]).unwrap();
        }
    }

    /// Output until it contains `needle`; any other frame that is not output is returned as a
    /// failure so a test cannot pass over a frame it did not expect.
    fn output_until(&mut self, needle: &str) -> String {
        let mut text = String::new();
        while !text.contains(needle) {
            match self.next() {
                ServerMessage::Output(bytes) => text.push_str(&String::from_utf8_lossy(&bytes)),
                ServerMessage::Resized(_) => {}
                other => panic!("waiting for {needle:?}: {other:?} after {text:?}"),
            }
        }
        text
    }

    /// The frames of one snapshot: the bytes and the size it was taken at.
    fn snapshot(&mut self) -> (String, Size) {
        self.send(&ClientMessage::Snapshot);
        let mut data = Vec::new();
        let mut started = false;
        loop {
            match self.next() {
                ServerMessage::SnapshotStart => (data, started) = (Vec::new(), true),
                ServerMessage::SnapshotData(bytes) => {
                    assert!(started && bytes.len() <= 16 * 1024);
                    data.extend(bytes);
                }
                ServerMessage::SnapshotEnd(size) => {
                    assert!(started);
                    return (String::from_utf8_lossy(&data).into_owned(), size);
                }
                // Output and size changes queued before the request come ahead of it.
                ServerMessage::Output(_) | ServerMessage::Resized(_) if !started => {}
                other => panic!("inside a snapshot: {other:?}"),
            }
        }
    }
}

#[test]
fn a_view_that_needs_nothing_from_core_reads_types_resizes_and_follows_the_stream() {
    let mut world = World::new();
    // The program reports its terminal size with `stty`, which the world's PATH does not carry.
    let stty = ["/bin/stty", "/usr/bin/stty"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
        .expect("stty");
    std::os::unix::fs::symlink(stty, world.root.join("bin/stty")).unwrap();
    let script = r#"printf '\033[?2004h\033[32mready\033[0m\r\n'; \
while IFS= read -r line; do \
  case "$line" in quit) exit 0 ;; size) stty size ;; *) printf 'got:%s\r\n' "$line" ;; esac; \
done"#;
    let started = world.run(&[
        "run",
        "--no-channel",
        "--save",
        "streamer",
        "sh",
        "-c",
        script,
    ]);
    assert!(started.status.success(), "{started:?}");
    let listed = world.json(&["ls", "--json"]);
    let row = listed["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "streamer")
        .expect("the member is listed")
        .clone();
    let identity = row["id"].as_str().unwrap().to_owned();

    // Core publishes where the stream is; the view never derives the path.
    let found = api(&world, &identity);
    assert_eq!(found["host"], "pty", "{found}");
    assert_eq!(found["identityId"], identity.as_str());
    assert_eq!(found["pane"], row["pane"], "{found}");
    let socket = std::path::PathBuf::from(found["socket"].as_str().unwrap());
    assert!(socket.is_absolute() && socket.exists(), "{found}");

    let mut view = View::open(&socket);
    // The tab sizes the terminal before it types.
    view.send(&ClientMessage::Resize(Size::new(100, 30).unwrap()));
    loop {
        if view.next() == ServerMessage::Resized(Size::new(100, 30).unwrap()) {
            break;
        }
    }

    // A view that joins after the program drew gets the screen, its colors and input modes.
    let (screen, size) = view.snapshot();
    assert_eq!((size.cols, size.rows), (100, 30));
    assert!(screen.contains("ready"), "{screen:?}");
    assert!(
        screen.contains("\x1b[?2004h"),
        "bracketed paste is in the snapshot: {screen:?}"
    );
    assert!(
        screen.contains("\x1b[32m") || screen.contains(";32m"),
        "{screen:?}"
    );

    // Typing reaches the program and its answer comes back as live output.
    view.send(&ClientMessage::Input(b"hello\r".to_vec()));
    view.output_until("got:hello");

    // A resize reaches the program: it reports the size it now has.
    view.send(&ClientMessage::Resize(Size::new(50, 20).unwrap()));
    view.send(&ClientMessage::Input(b"size\r".to_vec()));
    view.output_until("20 50");

    // Another view takes sole use; the first is told, and keeps no access.
    let mut other = View::open(&socket);
    other.send(&ClientMessage::TakeSole);
    loop {
        match view.next() {
            ServerMessage::Evicted(Eviction::TakenOver) => break,
            ServerMessage::Output(_) | ServerMessage::Resized(_) => {}
            unexpected => panic!("expected an eviction: {unexpected:?}"),
        }
    }

    // The program ends and the stream says how.
    other.send(&ClientMessage::Input(b"quit\r".to_vec()));
    loop {
        match other.next() {
            ServerMessage::Exited(exit) => {
                assert_eq!(exit, Exit::Code(0));
                break;
            }
            ServerMessage::Output(_) | ServerMessage::Resized(_) => {}
            unexpected => panic!("expected the exit: {unexpected:?}"),
        }
    }
    // Once the member has ended there is nothing to connect to, and Core says so.
    let deadline = Instant::now() + WAIT;
    loop {
        let after = api(&world, &identity);
        if after["error"]["code"] == "TERMINAL_STREAM_UNAVAILABLE" {
            break;
        }
        assert!(Instant::now() < deadline, "still published: {after}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(world.clean());
}

/// `terminal.stream` through the public API, as an extension would call it.
fn api(world: &World, identity: &str) -> serde_json::Value {
    let mut child = world
        .command(&["api"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let request = serde_json::json!({
        "version": 1,
        "operation": "terminal.stream",
        "input": {"identityId": identity},
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"))
}
