//! The built-in host's member supervisor as a process (#4605): `tmt __pty-serve`
//! with model-free `sh` programs. Process-level lifecycle only; the protocol
//! behavior is covered next to the supervisor.
#![cfg(any(target_os = "macos", target_os = "linux"))]
// This fixture uses only the shared private command environment.
#[allow(dead_code)]
mod support;

use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Stdio},
    time::{Duration, Instant},
};
use tmt_adapters::pty::{
    Connection, lock_path,
    protocol::{Exit, ServerMessage},
    socket_path,
};

const WAIT: Duration = Duration::from_secs(20);

/// A disposable root; the member directory is created by the supervisor.
struct Fixture {
    root: PathBuf,
    directory: PathBuf,
    supervisors: Vec<Child>,
}

/// A started supervisor, after its readiness line.
struct Started {
    child: usize,
    program: i32,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("tmt-pty-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let directory = root.join("pty");
        Self {
            root,
            directory,
            supervisors: Vec::new(),
        }
    }

    fn serve(&self, id: &str, cols: &str, rows: &str, program: &[&str]) -> Child {
        let mut arguments = vec![
            "__pty-serve",
            self.directory.to_str().unwrap(),
            id,
            cols,
            rows,
        ];
        arguments.extend(program);
        support::command(&self.root, &arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the supervisor")
    }

    /// Starts a supervisor and waits for its readiness line, then for the end of
    /// its output: the launcher must be able to read its pipes to the end.
    fn start(&mut self, id: &str, program: &[&str]) -> Started {
        let mut child = self.serve(id, "80", "24", program);
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout
            .read_line(&mut line)
            .expect("read the readiness line");
        let program = line
            .strip_prefix("ready ")
            .unwrap_or_else(|| panic!("no readiness line: {line:?}"))
            .trim()
            .parse()
            .unwrap();
        line.clear();
        assert_eq!(
            stdout.read_line(&mut line).unwrap(),
            0,
            "the supervisor kept its output open"
        );
        self.supervisors.push(child);
        Started {
            child: self.supervisors.len() - 1,
            program,
        }
    }

    fn connect(&self, id: &str) -> Connection {
        Connection::connect(&self.directory, id).expect("attach to the supervisor")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for child in &mut self.supervisors {
            // The program is hung up with its terminal when the supervisor goes.
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn alive(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None) != Err(Errno::ESRCH)
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn output_until(connection: &mut Connection, needle: &str) -> String {
    let deadline = Instant::now() + WAIT;
    let mut seen = String::new();
    while !seen.contains(needle) {
        let left = deadline.saturating_duration_since(Instant::now());
        match connection.receive(left).unwrap() {
            Some(ServerMessage::Output(bytes)) => seen.push_str(&String::from_utf8_lossy(&bytes)),
            Some(_) => {}
            None => panic!("timed out waiting for {needle:?}; saw {seen:?}"),
        }
    }
    seen
}

#[test]
fn the_program_has_a_terminal_of_its_own_and_the_supervisor_ends_with_it() {
    let mut fixture = Fixture::new("terminal");
    // Standard streams are the terminal, and /dev/tty opens only with a
    // controlling terminal, which a fresh session's leader acquires.
    let started = fixture.start(
        "pty_a",
        &[
            "sh",
            "-c",
            "read ready; [ -t 0 ] && [ -t 1 ] && [ -t 2 ] && exec 3</dev/tty && echo terminal-ok; read l; exit 5",
        ],
    );
    let mut client = fixture.connect("pty_a");
    // Start the terminal probe only after this output subscriber is connected.
    client.input(b"ready\r").unwrap();
    output_until(&mut client, "terminal-ok");
    assert!(socket_path(&fixture.directory, "pty_a").exists());
    assert!(lock_path(&fixture.directory, "pty_a").exists());
    client.input(b"\r").unwrap();
    loop {
        match client.receive(WAIT).unwrap() {
            Some(ServerMessage::Exited(exit)) => {
                assert_eq!(exit, Exit::Code(5));
                break;
            }
            Some(_) => {}
            None => panic!("no exit"),
        }
    }
    let status = fixture.supervisors[started.child].wait().unwrap();
    assert!(status.success(), "the supervisor ends cleanly: {status:?}");
    assert!(!socket_path(&fixture.directory, "pty_a").exists());
    assert!(!lock_path(&fixture.directory, "pty_a").exists());
    wait_until("the program to be gone", || !alive(started.program));
}

#[test]
fn a_killed_supervisor_hangs_up_its_program_and_its_member_can_start_again() {
    let mut fixture = Fixture::new("killed");
    let (armed, marker) = (fixture.root.join("armed"), fixture.root.join("hung-up"));
    let script = format!(
        "trap 'echo hup > {}; exit 0' HUP; : > {}; while true; do sleep 1; done",
        marker.display(),
        armed.display()
    );
    let started = fixture.start("pty_b", &["sh", "-c", &script]);
    // Hung up before it set its handler, it would die without a trace.
    wait_until("the program to arm its handler", || armed.exists());
    assert!(alive(started.program));
    let mut supervisor = fixture.supervisors.remove(started.child);
    // No handler runs: the operating system closes the terminal.
    supervisor.kill().unwrap();
    supervisor.wait().unwrap();
    wait_until("the program to be hung up", || marker.exists());
    wait_until("the program to be gone", || !alive(started.program));

    // The dead supervisor left its socket and lock, and neither blocks the id.
    assert!(socket_path(&fixture.directory, "pty_b").exists());
    fixture.start("pty_b", &["sh", "-c", "read l; echo back"]);
    let mut client = fixture.connect("pty_b");
    client.input(b"\r").unwrap();
    output_until(&mut client, "back");
}

#[test]
fn a_second_supervisor_for_a_running_member_fails_and_leaves_the_first_alone() {
    let mut fixture = Fixture::new("twice");
    fixture.start("pty_c", &["sh", "-c", "read l; echo still-here"]);
    let second = fixture
        .serve("pty_c", "80", "24", &["sh", "-c", "true"])
        .wait_with_output()
        .unwrap();
    assert_eq!(second.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("already serves"), "{stderr}");
    assert!(second.stdout.is_empty(), "no readiness line on failure");
    let mut client = fixture.connect("pty_c");
    client.input(b"\r").unwrap();
    output_until(&mut client, "still-here");
}

#[test]
fn a_start_that_cannot_proceed_fails_without_a_readiness_line_or_leftovers() {
    let fixture = Fixture::new("refused");
    let code = |output: std::process::Output| {
        assert!(output.stdout.is_empty(), "no readiness line");
        output.status.code()
    };

    // A terminal size outside the protocol's range.
    for cols in ["0", "1001"] {
        let output = fixture
            .serve("pty_d", cols, "24", &["sh"])
            .wait_with_output()
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stderr).contains("1 to 1000"));
        assert_eq!(code(output), Some(2));
    }

    // A program that does not exist.
    let output = fixture
        .serve("pty_e", "80", "24", &["/nonexistent/program"])
        .wait_with_output()
        .unwrap();
    assert_eq!(code(output), Some(1));
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 0);

    // A directory other users can enter.
    fs::set_permissions(&fixture.directory, fs::Permissions::from_mode(0o755)).unwrap();
    let output = fixture
        .serve("pty_f", "80", "24", &["sh", "-c", "true"])
        .wait_with_output()
        .unwrap();
    assert_eq!(code(output), Some(1));
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 0);
}
