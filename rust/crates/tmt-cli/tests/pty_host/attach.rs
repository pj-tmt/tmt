//! Public attach inside an owned outer PTY; the shared World retains every
//! supervisor, shell and attacher for native-incarnation-checked cleanup.
use super::*;
use std::io::{BufRead, BufReader};
use tmt_adapters::pty::protocol::{ClientMessage, ServerMessage, Size};

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ready() {
        assert!(Instant::now() < deadline, "waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn output_until(connection: &mut Connection, needle: &[u8]) -> Vec<u8> {
    let deadline = Instant::now() + WAIT;
    let mut output = Vec::new();
    while !output.windows(needle.len()).any(|part| part == needle) {
        match connection
            .receive(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            Some(ServerMessage::Output(bytes)) => {
                output.extend(bytes);
                if output
                    .windows(b"OUTER-DONE".len())
                    .any(|part| part == b"OUTER-DONE")
                    && !output.windows(needle.len()).any(|part| part == needle)
                {
                    panic!(
                        "outer command completed before {needle:?}: {}",
                        String::from_utf8_lossy(&output)
                    );
                }
            }
            Some(ServerMessage::Exited(exit)) => panic!("outer exited {exit:?}: {output:?}"),
            Some(_) => {}
            None => panic!("no {needle:?}; saw {}", String::from_utf8_lossy(&output)),
        }
    }
    output
}

impl World {
    fn own_pid(&mut self, pid: u64) -> ProcessIncarnation {
        let process = match observe_runtime_process(&UnixCommandRunner, pid, Instant::now() + WAIT)
            .unwrap()
        {
            ProcessObservation::Live(process) => process,
            other => panic!("owned process is not live: {other:?}"),
        };
        if !self.owned.contains(&process) {
            self.owned.push(process.clone());
        }
        process
    }

    fn member(&mut self) {
        let stty = stty();
        let script = format!(
            "printf '%s' \"$$\" > {}; {stty} -echo -isig; while IFS= read -r line; do case \"$line\" in size) printf 'SIZE:'; {stty} size;; exit) exit 7;; *) printf '\\033[31mRAW:%s\\033[0m\\n' \"$line\";; esac; done",
            quote(self.root.join("alpha.pid").to_str().unwrap()),
        );
        let result = self.run(&[
            "run",
            "--no-channel",
            "--save",
            "alpha",
            "/bin/sh",
            "-c",
            &script,
        ]);
        assert!(result.status.success(), "{result:?}");
        wait_for("member pid", || self.root.join("alpha.pid").exists());
        self.remember_programs();
    }

    fn outer(&mut self, label: &str, sole: bool) -> Connection {
        self.outer_with(label, if sole { "attach -d" } else { "attach" }, 80)
    }

    fn outer_with(&mut self, label: &str, command: &str, cols: u16) -> Connection {
        let directory = self.root.join("outer");
        let before = self.root.join(format!("{label}.before"));
        let after = self.root.join(format!("{label}.after"));
        let status = self.root.join(format!("{label}.status"));
        let pid = self.root.join(format!("{label}.attach-pid"));
        let script = format!(
            "{stty} cols {cols}; {stty} -g > {before}; read ready; exec 3<&0; {cli} {command} -- alpha <&3 3<&- & attached=$!; printf '%s' \"$attached\" > {pid}; wait \"$attached\"; result=$?; {stty} -g > {after}; printf '%s' \"$result\" > {status}; printf 'OUTER-DONE\\n'; read end",
            stty = stty(),
            cli = quote(env!("CARGO_BIN_EXE_tmt")),
            command = command,
            cols = cols,
            before = quote(before.to_str().unwrap()),
            after = quote(after.to_str().unwrap()),
            status = quote(status.to_str().unwrap()),
            pid = quote(pid.to_str().unwrap()),
        );
        let index = self.start(&[
            "__pty-serve",
            directory.to_str().unwrap(),
            label,
            "80",
            "24",
            "/bin/sh",
            "-c",
            &script,
        ]);
        let child = self.commands[index].as_mut().unwrap();
        let supervisor = child.id();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let program: u64 = line
            .strip_prefix("ready ")
            .expect("outer readiness")
            .trim()
            .parse()
            .unwrap();
        line.clear();
        assert_eq!(stdout.read_line(&mut line).unwrap(), 0);
        self.own_pid(u64::from(supervisor));
        self.own_pid(program);
        let mut client = Connection::connect(&directory, label).unwrap();
        client.input(b"ready\r").unwrap();
        let entry = output_until(&mut client, b"\x1b[?1049h");
        fs::write(self.root.join(format!("{label}.entry")), entry).unwrap();
        wait_for("attacher pid", || pid.exists());
        self.own_pid(fs::read_to_string(pid).unwrap().parse().unwrap());
        client
    }

    fn restored(&self, label: &str, expected: i32) {
        wait_for("terminal restoration and exit status", || {
            self.root.join(format!("{label}.status")).exists()
        });
        assert_eq!(
            terminal_modes(self.root.join(format!("{label}.before"))),
            terminal_modes(self.root.join(format!("{label}.after"))),
            "exact original stty modes restored"
        );
        assert_eq!(
            fs::read_to_string(self.root.join(format!("{label}.status"))).unwrap(),
            expected.to_string()
        );
    }
}

fn terminal_modes(path: PathBuf) -> String {
    let modes = fs::read_to_string(path).unwrap();
    // Darwin sets PENDIN when canonical input is restored. It is pending-input
    // kernel state, not a configured mode (sys/termios.h); compare every other
    // flag and control character exactly, without changing the tested terminal.
    #[cfg(target_os = "macos")]
    {
        modes
            .trim()
            .split(':')
            .map(|part| {
                if let Some(value) = part.strip_prefix("lflag=") {
                    format!(
                        "lflag={:x}",
                        u64::from_str_radix(value, 16).unwrap() & !0x2000_0000
                    )
                } else {
                    part.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(":")
    }
    #[cfg(not(target_os = "macos"))]
    {
        modes
    }
}

fn stty() -> &'static str {
    ["/bin/stty", "/usr/bin/stty"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
        .unwrap()
}

#[test]
fn attach_forwards_live_bytes_detaches_and_restores_before_reattaching() {
    let mut world = World::new();
    world.member();
    let mut first = world.outer("pty_first", false);
    first.input(b"causal-one\r").unwrap();
    let bytes = output_until(&mut first, b"RAW:causal-one\x1b[0m\r\n");
    assert!(
        bytes
            .windows(b"\x1b[31mRAW:causal-one\x1b[0m\r\n".len())
            .any(|part| part == b"\x1b[31mRAW:causal-one\x1b[0m\r\n"),
        "program ANSI output is unchanged"
    );
    first.input(b"control-\x03-byte\r").unwrap();
    output_until(&mut first, b"RAW:control-\x03-byte");
    first.input(b"\x1cdiscarded\r").unwrap();
    let ending = output_until(&mut first, b"OUTER-DONE");
    assert!(
        ending
            .windows(b"\x1b[?1049l".len())
            .any(|part| part == b"\x1b[?1049l")
    );
    world.restored("pty_first", 0);
    let mut again = world.outer("pty_again", false);
    again.input(b"causal-two\r").unwrap();
    output_until(&mut again, b"RAW:causal-two");
    again.input(b"exit\r").unwrap();
    let ended = output_until(&mut again, b"OUTER-DONE");
    assert!(String::from_utf8_lossy(&ended).contains("tmt resume -- alpha"));
    world.restored("pty_again", 7);
    assert!(world.clean(), "all owned processes gone");
}

#[test]
fn sole_attachment_restores_the_evicted_viewer_without_stopping_the_member() {
    let mut world = World::new();
    world.member();
    let mut first = world.outer("pty_first", false);
    let mut second = world.outer("pty_second", true);
    let ending = output_until(&mut first, b"OUTER-DONE");
    assert!(String::from_utf8_lossy(&ending).contains("sole use"));
    world.restored("pty_first", 0);
    second.input(b"still-live\r").unwrap();
    output_until(&mut second, b"RAW:still-live");
    second.input(b"\x1c").unwrap();
    output_until(&mut second, b"OUTER-DONE");
    world.restored("pty_second", 0);
    assert!(world.clean());
}

#[test]
fn catchable_signal_restores_the_terminal_without_signalling_the_member() {
    for signal in [Signal::SIGTERM, Signal::SIGHUP, Signal::SIGINT] {
        let mut world = World::new();
        world.member();
        let mut outer = world.outer("pty_signal", false);
        let pid: u64 = fs::read_to_string(world.root.join("pty_signal.attach-pid"))
            .unwrap()
            .parse()
            .unwrap();
        let expected = world.own_pid(pid);
        assert!(
            matches!(observe_runtime_process(&UnixCommandRunner, pid, Instant::now()+WAIT).unwrap(), ProcessObservation::Live(ref now) if now == &expected)
        );
        kill(Pid::from_raw(pid as i32), signal).unwrap();
        output_until(&mut outer, b"OUTER-DONE");
        world.restored("pty_signal", 128 + signal as i32);
        let mut again = world.outer("pty_again", false);
        again.input(b"after-signal\r").unwrap();
        output_until(&mut again, b"RAW:after-signal");
        again.input(b"\x1c").unwrap();
        output_until(&mut again, b"OUTER-DONE");
        world.restored("pty_again", 0);
        assert!(world.clean());
    }
}

#[test]
fn attachment_refuses_pipes_and_json_without_touching_a_member() {
    let mut world = World::new();
    world.member();
    for args in [&["attach", "alpha"][..], &["attach", "alpha", "--json"][..]] {
        let output = world.run(args);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        if args.contains(&"--json") {
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            let expected = if args[0] == "focus" {
                "HOST_UNSUPPORTED"
            } else {
                "JSON_UNSUPPORTED"
            };
            assert_eq!(value["error"]["code"], expected);
        } else {
            assert!(error.contains("terminal"), "{output:?}");
        }
    }
    let mut viewer = world.outer("pty_after_refusal", false);
    viewer.input(b"not-touched\r").unwrap();
    output_until(&mut viewer, b"RAW:not-touched");
    viewer.input(b"\x1c").unwrap();
    output_until(&mut viewer, b"OUTER-DONE");
    world.restored("pty_after_refusal", 0);
    assert!(world.clean());
}

#[test]
fn local_resize_is_clamped_and_peer_resize_never_resizes_the_outer_terminal() {
    let mut world = World::new();
    world.member();
    let mut outer = world.outer_with("pty_size", "attach", 1001);
    let entry = fs::read_to_string(world.root.join("pty_size.entry")).unwrap();
    assert!(entry.contains("1000 columns"), "{entry}");
    outer.input(b"size\r").unwrap();
    output_until(&mut outer, b"SIZE:24 1000");
    let rows = world.json(&["ls", "--json"]);
    let member = rows["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "alpha")
        .unwrap();
    let pane = member["pane"].as_str().unwrap();
    let directory = support::state_dir(&world.root).join("pty");
    let mut peer = Connection::connect(&directory, pane).unwrap();
    peer.send(&ClientMessage::Resize(Size::new(73, 19).unwrap()))
        .unwrap();
    let deadline = Instant::now() + WAIT;
    while peer.size() != Size::new(73, 19).unwrap() {
        assert!(
            peer.receive(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
                .is_some()
        );
    }
    drop(peer);
    // Input returns size ownership to this attacher; it must still know its
    // physical 1001-column terminal and clamp it, never adopt the peer's size.
    outer.input(b"size\r").unwrap();
    let output = output_until(&mut outer, b"SIZE:24 1000");
    assert!(!output.windows(4).any(|part| part == b"\x1b[8;"));
    outer
        .send(&ClientMessage::Resize(Size::new(92, 31).unwrap()))
        .unwrap();
    let deadline = Instant::now() + WAIT;
    while outer.size() != Size::new(92, 31).unwrap() {
        assert!(
            outer
                .receive(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
                .is_some()
        );
    }
    // Resize wakes the real attacher through SIGWINCH; observe its committed
    // server size before asking the program, rather than sleeping.
    wait_for("attacher resize reaches member", || {
        Connection::connect(&directory, pane)
            .map(|client| client.size() == Size::new(92, 31).unwrap())
            .unwrap_or(false)
    });
    outer.input(b"size\r").unwrap();
    output_until(&mut outer, b"SIZE:31 92");
    outer.input(b"\x1c").unwrap();
    output_until(&mut outer, b"OUTER-DONE");
    world.restored("pty_size", 0);
    assert!(world.clean());
}

#[test]
fn focus_uses_attach_and_supervisor_loss_restores_without_claiming_program_exit() {
    let mut world = World::new();
    world.member();
    let mut outer = world.outer_with("pty_focus", "focus", 80);
    outer.input(b"focused-input\r").unwrap();
    output_until(&mut outer, b"RAW:focused-input");
    let rows = world.json(&["ls", "--json"]);
    let member = rows["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "alpha")
        .unwrap();
    let mut client = Connection::connect(
        &support::state_dir(&world.root).join("pty"),
        member["pane"].as_str().unwrap(),
    )
    .unwrap();
    let ControlReply::State { supervisor_pid, .. } =
        client.control(Control::Inspect, WAIT).unwrap()
    else {
        panic!("supervisor evidence")
    };
    drop(client);
    let expected = world.own_pid(u64::from(supervisor_pid));
    assert!(
        matches!(observe_runtime_process(&UnixCommandRunner, expected.pid(), Instant::now()+WAIT).unwrap(), ProcessObservation::Live(ref now) if now == &expected)
    );
    kill(Pid::from_raw(supervisor_pid as i32), Signal::SIGKILL).unwrap();
    let output = output_until(&mut outer, b"OUTER-DONE");
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("does not prove the agent ended"), "{text}");
    assert!(!text.contains("resume with:"));
    world.restored("pty_focus", 1);
    assert!(world.clean());
}
