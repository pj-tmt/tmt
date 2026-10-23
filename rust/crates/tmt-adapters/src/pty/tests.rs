//! The supervisor against real pseudo-terminals, in this process: the programs
//! are model-free `sh` one-liners and every client is a real socket connection.

use super::{Connection, Launch, Supervisor, lock_path, socket_path, supervisor::MAX_CLIENTS};
use crate::{private_file::ensure_private_directory, test_support::TestDirectory};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use std::{
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tmt_pty_protocol::{
    ClientMessage, Decoder, Eviction, Exit, Frame, Refusal, ServerMessage, Size, VERSION,
};

const WAIT: Duration = Duration::from_secs(20);

fn size(cols: u16, rows: u16) -> Size {
    Size::new(cols, rows).unwrap()
}

/// A supervisor running on a thread of the test, with its own owner-only directory.
struct Member {
    _root: TestDirectory,
    directory: PathBuf,
    id: String,
    pid: u32,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl Member {
    fn start(program: &[&str]) -> Self {
        let root = TestDirectory::new();
        let directory = root.path.join("pty");
        let supervisor = Supervisor::start(launch(&directory, "pty_test", program))
            .expect("start the supervisor");
        let pid = supervisor.program_pid();
        Self {
            _root: root,
            directory,
            id: "pty_test".to_owned(),
            pid,
            thread: Some(std::thread::spawn(move || supervisor.run())),
        }
    }

    fn connect(&self) -> Connection {
        Connection::connect(&self.directory, &self.id).expect("attach to the supervisor")
    }

    /// Waits for the supervisor to end and returns how.
    fn finish(mut self) -> io::Result<()> {
        let thread = self.thread.take().unwrap();
        let deadline = Instant::now() + WAIT;
        while !thread.is_finished() {
            assert!(Instant::now() < deadline, "the supervisor did not end");
            std::thread::sleep(Duration::from_millis(10));
        }
        thread.join().expect("the supervisor thread panicked")
    }
}

impl Drop for Member {
    fn drop(&mut self) {
        // A test that failed leaves the program running; a supervisor that
        // ended has already reaped it, so only a live one is signalled.
        if let Some(thread) = self.thread.take() {
            if !thread.is_finished() {
                let _ = kill(Pid::from_raw(self.pid as i32), Signal::SIGKILL);
            }
            let _ = thread.join();
        }
    }
}

fn launch(directory: &Path, id: &str, program: &[&str]) -> Launch {
    Launch {
        directory: directory.to_path_buf(),
        id: id.to_owned(),
        size: size(80, 24),
        program: program.iter().map(OsString::from).collect(),
        environment: Vec::new(),
    }
}

/// What a client has received so far: the tail of its output as text, and every
/// other frame.
#[derive(Default)]
struct Transcript {
    text: String,
    frames: Vec<ServerMessage>,
}

/// Receives until `done` holds; a closed connection or the timeout fails the test.
fn collect(connection: &mut Connection, done: impl Fn(&Transcript) -> bool) -> Transcript {
    let deadline = Instant::now() + WAIT;
    let mut seen = Transcript::default();
    while !done(&seen) {
        let left = deadline.saturating_duration_since(Instant::now());
        match connection.receive(left) {
            Ok(Some(ServerMessage::Output(bytes))) => {
                seen.text.push_str(&String::from_utf8_lossy(&bytes));
                // Only the tail can complete a needle; megabytes of output stay cheap.
                if seen.text.len() > 16 * 1024 {
                    let cut = seen
                        .text
                        .char_indices()
                        .rev()
                        .nth(4096)
                        .map_or(0, |(i, _)| i);
                    seen.text.drain(..cut);
                }
            }
            Ok(Some(other)) => seen.frames.push(other),
            Ok(None) => panic!("timed out; saw {:?} and {:?}", seen.text, seen.frames),
            Err(error) => panic!("{error}; saw {:?} and {:?}", seen.text, seen.frames),
        }
    }
    seen
}

fn says(needle: &'static str) -> impl Fn(&Transcript) -> bool {
    move |seen| seen.text.contains(needle)
}

fn exited(code: Exit) -> impl Fn(&Transcript) -> bool {
    move |seen| seen.frames.contains(&ServerMessage::Exited(code))
}

/// A raw socket for what `Connection` refuses to send.
struct Raw {
    stream: UnixStream,
    decoder: Decoder,
}

impl Raw {
    fn connect(member: &Member) -> Self {
        let stream = UnixStream::connect(socket_path(&member.directory, &member.id)).unwrap();
        stream.set_read_timeout(Some(WAIT)).unwrap();
        Self {
            stream,
            decoder: Decoder::new(),
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        match (&self.stream).write_all(bytes) {
            // A supervisor that refuses answers and closes without reading.
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {}
            sent => sent.unwrap(),
        }
    }

    /// The next frame, or `None` once the supervisor closed the connection.
    fn next(&mut self) -> Option<ServerMessage> {
        let mut buffer = [0u8; 4096];
        loop {
            if let Some(message) = self.decoder.frame::<ServerMessage>().unwrap() {
                return Some(message);
            }
            match (&self.stream).read(&mut buffer) {
                Ok(0) => return None,
                Ok(count) => self.decoder.push(&buffer[..count]).unwrap(),
                Err(error) => panic!("reading from the supervisor: {error}"),
            }
        }
    }
}

#[test]
fn every_attached_client_sees_the_output_and_the_exit() {
    let member = Member::start(&["sh", "-c", "read l; echo got:$l"]);
    let (mut first, mut second) = (member.connect(), member.connect());
    assert_eq!(first.size(), size(80, 24));
    first.input(b"hello\r").unwrap();
    for client in [&mut first, &mut second] {
        collect(client, says("got:hello"));
    }
    for client in [&mut first, &mut second] {
        collect(client, exited(Exit::Code(0)));
    }
    let directory = member.directory.clone();
    member.finish().unwrap();
    assert!(!socket_path(&directory, "pty_test").exists());
    assert!(!lock_path(&directory, "pty_test").exists());
}

#[test]
fn the_exit_status_and_the_signal_are_reported() {
    let member = Member::start(&["sh", "-c", "read l; exit 7"]);
    let mut client = member.connect();
    client.input(b"\r").unwrap();
    collect(&mut client, exited(Exit::Code(7)));
    member.finish().unwrap();

    let member = Member::start(&["sh", "-c", "read l; kill -9 $$"]);
    let mut client = member.connect();
    client.input(b"\r").unwrap();
    collect(&mut client, exited(Exit::Signal(9)));
    member.finish().unwrap();
}

#[test]
fn a_program_runs_to_the_end_with_nobody_attached() {
    // More output than any terminal or socket buffer holds: it finishes only if
    // the supervisor keeps reading while no client is there.
    let member = Member::start(&["sh", "-c", "yes | head -c 3000000"]);
    member.finish().unwrap();
}

#[test]
fn a_client_that_detaches_leaves_the_program_running() {
    let member = Member::start(&["sh", "-c", "read l; echo got:$l"]);
    let mut leaving = member.connect();
    leaving.send(&ClientMessage::Detach).unwrap();
    drop(leaving);
    let mut staying = member.connect();
    staying.input(b"later\r").unwrap();
    collect(&mut staying, says("got:later"));
    drop(member.finish());
}

#[test]
fn the_window_follows_the_client_that_resizes_or_types() {
    let member = Member::start(&["sh", "-c", "while read l; do stty size; done"]);
    let mut first = member.connect();
    first.send(&ClientMessage::Resize(size(100, 30))).unwrap();
    collect(&mut first, |seen| {
        seen.frames.contains(&ServerMessage::Resized(size(100, 30)))
    });
    first.input(b"go\r").unwrap();
    collect(&mut first, says("30 100"));

    // A new client is told the size in force and can take the window.
    let mut second = member.connect();
    assert_eq!(second.size(), size(100, 30));
    second.send(&ClientMessage::Resize(size(50, 20))).unwrap();
    collect(&mut first, |seen| {
        seen.frames.contains(&ServerMessage::Resized(size(50, 20)))
    });
    second.input(b"go\r").unwrap();
    collect(&mut second, says("20 50"));

    // Typing in the first window brings the terminal back to it.
    first.input(b"go\r").unwrap();
    collect(&mut first, says("30 100"));

    // A client that never sent a size, a delivery, does not move the window.
    let mut delivery = member.connect();
    delivery.input(b"go\r").unwrap();
    collect(&mut delivery, says("30 100"));
}

#[test]
fn sole_use_disconnects_the_other_clients() {
    let member = Member::start(&["sh", "-c", "read l; echo got:$l"]);
    let mut other = member.connect();
    let mut taker = member.connect();
    taker.send(&ClientMessage::TakeSole).unwrap();
    collect(&mut other, |seen| {
        seen.frames
            .contains(&ServerMessage::Evicted(Eviction::TakenOver))
    });
    assert!(
        other.receive(WAIT).is_err(),
        "the evicted connection is closed"
    );
    taker.input(b"alone\r").unwrap();
    collect(&mut taker, says("got:alone"));
}

#[test]
fn a_stalled_client_is_evicted_and_the_program_goes_on_for_the_others() {
    let member = Member::start(&[
        "sh",
        "-c",
        "yes | head -c 4000000; echo END; read l; echo done",
    ]);
    let mut stalled = member.connect();
    let mut reading = member.connect();
    // `stalled` reads nothing: output pauses until it is evicted, then resumes.
    collect(&mut reading, says("END"));
    // It was told if its connection lasted; either way its stream ends.
    let mut told = None;
    loop {
        match stalled.receive(WAIT) {
            Ok(Some(ServerMessage::Evicted(reason))) => told = Some(reason),
            Ok(Some(ServerMessage::Exited(_))) => panic!("the program has not ended"),
            Ok(Some(_)) => {}
            Ok(None) => panic!("the stalled client was never disconnected"),
            Err(_) => break,
        }
    }
    assert!(told.is_none_or(|reason| reason == Eviction::TooSlow));
    reading.input(b"\r").unwrap();
    collect(&mut reading, exited(Exit::Code(0)));
    member.finish().unwrap();
}

#[test]
fn a_connection_that_breaks_the_protocol_is_refused_or_closed() {
    let member = Member::start(&["sh", "-c", "read l; echo got:$l"]);

    // Input before the handshake.
    let mut raw = Raw::connect(&member);
    raw.send(&ClientMessage::Input(b"x".to_vec()).encode());
    assert_eq!(raw.next(), Some(ServerMessage::Refused(Refusal::Handshake)));
    assert_eq!(raw.next(), None);

    // A version this supervisor does not speak.
    let mut raw = Raw::connect(&member);
    raw.send(
        &ClientMessage::Hello {
            version: VERSION + 1,
        }
        .encode(),
    );
    assert_eq!(raw.next(), Some(ServerMessage::Refused(Refusal::Version)));
    assert_eq!(raw.next(), None);

    // A frame that is not valid, and a second hello, close the connection.
    let mut raw = Raw::connect(&member);
    raw.send(&ClientMessage::Hello { version: VERSION }.encode());
    assert!(matches!(raw.next(), Some(ServerMessage::Welcome { .. })));
    raw.send(&[0x7f, 0, 0, 0, 0]);
    assert_eq!(raw.next(), None);
    let mut raw = Raw::connect(&member);
    raw.send(&ClientMessage::Hello { version: VERSION }.encode());
    assert!(matches!(raw.next(), Some(ServerMessage::Welcome { .. })));
    raw.send(&ClientMessage::Hello { version: VERSION }.encode());
    assert_eq!(raw.next(), None);

    // None of that reached the program.
    let mut client = member.connect();
    client.input(b"fine\r").unwrap();
    collect(&mut client, |seen| seen.text.contains("got:fine"));
}

#[test]
fn clients_beyond_the_limit_are_refused() {
    let member = Member::start(&["sh", "-c", "read l; echo got:$l"]);
    let attached: Vec<Connection> = (0..MAX_CLIENTS).map(|_| member.connect()).collect();
    let mut raw = Raw::connect(&member);
    raw.send(&ClientMessage::Hello { version: VERSION }.encode());
    assert_eq!(raw.next(), Some(ServerMessage::Refused(Refusal::Full)));
    assert_eq!(attached.len(), MAX_CLIENTS);
}

#[test]
fn a_refusal_is_reported_even_when_the_supervisor_closed_before_the_hello() {
    let (client, mut supervisor) = UnixStream::pair().unwrap();
    client.set_write_timeout(Some(WAIT)).unwrap();
    supervisor
        .write_all(&ServerMessage::Refused(Refusal::Full).encode())
        .unwrap();
    drop(supervisor);
    let error = Connection::handshake(client).err().expect("refused");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    assert!(error.to_string().contains("Full"), "{error}");

    // Without a refusal to read, the failed Hello is what is reported.
    let (client, supervisor) = UnixStream::pair().unwrap();
    drop(supervisor);
    let error = Connection::handshake(client).err().expect("closed");
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe, "{error}");
}

#[test]
fn a_client_that_never_says_hello_is_dropped() {
    let member = Member::start(&["sh", "-c", "read l; echo got:$l"]);
    let mut silent = Raw::connect(&member);
    assert_eq!(
        silent.next(),
        Some(ServerMessage::Refused(Refusal::Handshake))
    );
    assert_eq!(silent.next(), None);
}

#[test]
fn a_second_supervisor_for_the_member_is_refused_and_a_dead_ones_socket_is_replaced() {
    let member = Member::start(&["sh", "-c", "read l; echo got:$l"]);
    let error = Supervisor::start(launch(&member.directory, &member.id, &["sh", "-c", "true"]))
        .err()
        .expect("a second supervisor must not start");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    // The first is untouched.
    let mut client = member.connect();
    client.input(b"still\r").unwrap();
    collect(&mut client, says("got:still"));
    member.finish().unwrap();

    // A killed supervisor leaves its socket behind; its lock died with it.
    let root = TestDirectory::new();
    let directory = root.path.join("pty");
    ensure_private_directory(&directory).unwrap();
    drop(UnixListener::bind(socket_path(&directory, "pty_old")).unwrap());
    assert!(socket_path(&directory, "pty_old").exists());
    let supervisor =
        Supervisor::start(launch(&directory, "pty_old", &["sh", "-c", "exit 0"])).unwrap();
    supervisor.run().unwrap();
    assert!(!socket_path(&directory, "pty_old").exists());
}

#[test]
fn a_start_that_cannot_proceed_leaves_nothing_behind() {
    let root = TestDirectory::new();

    // A directory other users can enter.
    let open = root.path.join("open");
    fs::create_dir(&open).unwrap();
    fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
    let error = Supervisor::start(launch(&open, "pty_a", &["sh"]))
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read_dir(&open).unwrap().count(), 0);

    // A program that does not exist.
    let private = root.path.join("private");
    let error = Supervisor::start(launch(&private, "pty_b", &["/nonexistent/program"]))
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(fs::read_dir(&private).unwrap().count(), 0);

    // Nothing to run, a bad id and a path too long for a socket.
    for (id, program, directory) in [
        ("pty_c", Vec::new(), private.clone()),
        ("../escape", vec!["sh"], private.clone()),
        ("", vec!["sh"], private.clone()),
        ("pty_d", vec!["sh"], private.join("x".repeat(100))),
    ] {
        let error = Supervisor::start(launch(&directory, id, &program))
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{id:?}");
    }
    assert_eq!(fs::read_dir(&private).unwrap().count(), 0);
}

#[test]
fn host_controls_pin_the_pane_marker_and_keep_only_the_visible_screen() {
    use tmt_pty_protocol::{Control, ControlReply};
    let member = Member::start(&[
        "sh",
        "-c",
        r"printf '\033[2J\033[Hscreen-ready'; read line; printf '\033[2J\033[Hfresh-screen'; read line",
    ]);
    let mut client = member.connect();
    let deadline = Instant::now() + WAIT;
    loop {
        if let ControlReply::Screen(screen) = client.control(Control::Capture, WAIT).unwrap()
            && screen.contains("screen-ready")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "program never painted its screen"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        client
            .control(
                Control::Publish {
                    pane_pid: member.pid + 1,
                    key: "first".into(),
                    marker: "bad".into()
                },
                WAIT
            )
            .unwrap(),
        ControlReply::Applied(false)
    );
    assert_eq!(
        client
            .control(
                Control::Publish {
                    pane_pid: member.pid,
                    key: "first".into(),
                    marker: "correct".into()
                },
                WAIT
            )
            .unwrap(),
        ControlReply::Applied(true)
    );
    assert_eq!(
        client
            .control(
                Control::Clear {
                    key: "other".into()
                },
                WAIT
            )
            .unwrap(),
        ControlReply::Applied(false)
    );
    let ControlReply::State {
        pane_pid,
        marker,
        key_idle_ms,
        ..
    } = client.control(Control::Inspect, WAIT).unwrap()
    else {
        panic!("missing state")
    };
    assert_eq!(pane_pid, member.pid);
    assert_eq!(marker.as_deref(), Some("correct"));
    assert_eq!(key_idle_ms, None);
    client.control(Control::Enter, WAIT).unwrap();
    loop {
        if let ControlReply::Screen(screen) = client.control(Control::Capture, WAIT).unwrap()
            && screen.contains("fresh-screen")
        {
            assert!(!screen.contains("screen-ready"));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(client.size(), size(80, 24));
    client
        .control(
            Control::Clear {
                key: "first".into(),
            },
            WAIT,
        )
        .unwrap();
    client.input(b"\r").unwrap();
    drop(client);
    member.finish().unwrap();
}

#[test]
fn delivery_wraps_paste_only_in_the_program_mode_and_never_counts_as_typing() {
    use tmt_pty_protocol::{Control, ControlReply};
    for (mode, expected) in [
        ("", "61 62"),
        (r"\033[?2004h", "1b 5b 32 30 30 7e 61 62 1b 5b 32 30 31 7e"),
    ] {
        let script = format!(
            "stty -echo; printf '{mode}ready'; IFS= read -r line; printf '%s' \"$line\" | od -An -tx1; IFS= read -r line"
        );
        let member = Member::start(&["sh", "-c", &script]);
        let mut client = member.connect();
        let deadline = Instant::now() + WAIT;
        loop {
            if let ControlReply::Screen(text) = client.control(Control::Capture, WAIT).unwrap()
                && text.contains("ready")
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(
            client
                .control(Control::Paste(b"ab".to_vec()), WAIT)
                .unwrap(),
            ControlReply::Applied(true)
        );
        client.control(Control::Enter, WAIT).unwrap();
        loop {
            let ControlReply::Screen(text) = client.control(Control::Capture, WAIT).unwrap() else {
                panic!("capture refused")
            };
            if text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains(expected)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "expected {expected}; screen={text:?}"
            );
            std::thread::yield_now();
        }
        assert!(matches!(
            client.control(Control::Inspect, WAIT).unwrap(),
            ControlReply::State {
                key_idle_ms: None,
                ..
            }
        ));
        client.input(b"key").unwrap();
        assert!(matches!(
            client.control(Control::Inspect, WAIT).unwrap(),
            ControlReply::State {
                key_idle_ms: Some(_),
                ..
            }
        ));
        client.input(b"\r").unwrap();
        drop(client);
        member.finish().unwrap();
    }
}

#[test]
fn launch_preparation_waits_for_a_listener_and_readiness_carries_the_committed_marker() {
    use tmt_pty_protocol::{Control, ControlReply};
    let member = Member::start(&["sh", "-c", "read line"]);
    let mut wrapper = member.connect();
    wrapper
        .send(&ClientMessage::Control(Control::WaitLauncher))
        .unwrap();
    assert_eq!(wrapper.receive(Duration::from_millis(10)).unwrap(), None);
    let mut launcher = member.connect();
    launcher
        .send(&ClientMessage::Control(Control::AwaitLaunch))
        .unwrap();
    assert_eq!(
        wrapper.receive(WAIT).unwrap(),
        Some(ServerMessage::Control(ControlReply::Applied(true)))
    );
    assert_eq!(
        wrapper
            .control(
                Control::LaunchReady {
                    pane_pid: member.pid + 1
                },
                WAIT
            )
            .unwrap(),
        ControlReply::Applied(false)
    );
    assert_eq!(launcher.receive(Duration::from_millis(10)).unwrap(), None);
    wrapper
        .control(
            Control::Publish {
                pane_pid: member.pid,
                key: "binding".into(),
                marker: "committed-marker".into(),
            },
            WAIT,
        )
        .unwrap();
    assert_eq!(
        wrapper
            .control(
                Control::LaunchReady {
                    pane_pid: member.pid
                },
                WAIT
            )
            .unwrap(),
        ControlReply::Applied(true)
    );
    assert_eq!(
        launcher.receive(WAIT).unwrap(),
        Some(ServerMessage::Control(ControlReply::Launched(
            "committed-marker".into()
        )))
    );
    wrapper.input(b"\r").unwrap();
    drop(wrapper);
    drop(launcher);
    member.finish().unwrap();
}
