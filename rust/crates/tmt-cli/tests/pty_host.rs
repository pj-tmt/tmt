//! Model-free public CLI proof for the built-in host, with no tmux executable.
#![cfg(any(target_os = "macos", target_os = "linux"))]
#[allow(dead_code)]
mod support;

use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
use tmt_adapters::{
    process::{
        UnixCommandRunner,
        runtime::{ProcessObservation, observe_runtime_process, recorded_process_gone},
    },
    pty::{
        Connection,
        protocol::{Control, ControlReply},
    },
};
use tmt_core::endpoint::ProcessIncarnation;

const WAIT: Duration = Duration::from_secs(90);
static NEXT_WORLD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct World {
    root: PathBuf,
    owned: Vec<ProcessIncarnation>,
    known_sockets: std::collections::BTreeSet<PathBuf>,
    unconfirmed: bool,
    commands: Vec<Option<std::process::Child>>,
    interpreter: Option<PathBuf>,
}
impl World {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let serial = NEXT_WORLD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = PathBuf::from(format!(
            "/tmp/tp{:x}-{nonce:x}-{serial:x}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create a new owned short socket root");
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        for (name, candidates) in [
            ("ps", ["/bin/ps", "/usr/bin/ps"]),
            ("sh", ["/bin/sh", "/usr/bin/sh"]),
            // A basic driver's name on sh: see `agent`.
            ("opencode", ["/bin/sh", "/usr/bin/sh"]),
        ] {
            let source = candidates
                .iter()
                .find(|p| std::path::Path::new(p).is_file())
                .unwrap();
            std::os::unix::fs::symlink(source, bin.join(name)).unwrap();
        }
        Self {
            root,
            owned: Vec::new(),
            known_sockets: Default::default(),
            unconfirmed: false,
            commands: Vec::new(),
            interpreter: None,
        }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = support::command(&self.root, args);
        command
            .env("PATH", self.root.join("bin"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(interpreter) = &self.interpreter {
            command.env("TMT_TEST_PROVIDER_INTERPRETER", interpreter);
        }
        command
    }
    fn start(&mut self, args: &[&str]) -> usize {
        let child = self.command(args).spawn().unwrap();
        self.commands.push(Some(child));
        self.commands.len() - 1
    }
    fn finish(&mut self, index: usize) -> Output {
        let child = support::wait(&mut self.commands[index], WAIT);
        let output = child.wait_with_output().unwrap();
        self.remember();
        self.remember_programs();
        output
    }
    /// A scripted member that reads its messages. A plain `/bin/sh` is a plain program that
    /// receives nothing (`RECIPIENT_NOT_AGENT`), so a member that must be talked to runs sh
    /// under a known (basic) driver's executable name.
    fn agent(&self) -> String {
        self.root.join("bin/opencode").display().to_string()
    }

    fn run(&mut self, args: &[&str]) -> Output {
        let index = self.start(args);
        self.finish(index)
    }
    fn json(&mut self, args: &[&str]) -> Value {
        let result = self.run(args);
        assert!(result.status.success(), "{args:?}: {result:?}");
        serde_json::from_slice(&result.stdout).unwrap()
    }
    fn remember(&mut self) {
        let directory = support::state_dir(&self.root).join("pty");
        // A fast launcher may close its socket before a cleanup client connects.
        // Retain its already verified incarnation from this private world's DB;
        // cleanup still rechecks that exact PID/start before any signal.
        let database = support::state_dir(&self.root).join("tmt.db");
        if database.is_file() {
            let recorded = (|| -> rusqlite::Result<Vec<(String, i64, String)>> {
                let db = rusqlite::Connection::open_with_flags(
                    &database,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                let mut statement = db.prepare("SELECT socket_path,server_pid,server_start_time FROM host_servers WHERE host='pty'")?;
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect()
            })();
            match recorded {
                Ok(rows) => {
                    for (socket, pid, start) in rows {
                        let socket = PathBuf::from(socket);
                        if socket.parent() != Some(directory.as_path()) {
                            eprintln!("owned supervisor socket outside its directory: {socket:?}");
                            self.unconfirmed = true;
                            continue;
                        }
                        if let Ok(pid) = u64::try_from(pid)
                            && let Ok(process) = ProcessIncarnation::new(pid, &start)
                        {
                            self.known_sockets.insert(socket);
                            if !self.owned.contains(&process) {
                                self.owned.push(process);
                            }
                        } else {
                            eprintln!("invalid owned supervisor incarnation {pid}/{start:?}");
                            self.unconfirmed = true;
                        }
                    }
                }
                Err(error) => {
                    eprintln!("could not read owned supervisor evidence: {error}");
                    self.unconfirmed = true;
                }
            }
        }
        let Ok(files) = fs::read_dir(&directory) else {
            return;
        };
        for file in files.flatten() {
            if file.path().extension().and_then(|s| s.to_str()) != Some("sock") {
                continue;
            }
            let id = file
                .path()
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let Ok(mut client) = Connection::connect(&directory, &id) else {
                if file.path().exists() && !self.known_sockets.contains(&file.path()) {
                    eprintln!(
                        "unrecorded socket could not be connected: {:?}",
                        file.path()
                    );
                    self.unconfirmed = true;
                }
                continue;
            };
            if let Ok(ControlReply::State {
                supervisor_pid,
                pane_pid,
                ..
            }) = client.control(Control::Inspect, Duration::from_secs(3))
            {
                self.known_sockets.insert(file.path());
                for pid in [supervisor_pid, pane_pid] {
                    match observe_runtime_process(
                        &UnixCommandRunner,
                        u64::from(pid),
                        Instant::now() + Duration::from_secs(3),
                    ) {
                        Ok(
                            ProcessObservation::Live(process)
                            | ProcessObservation::Stopped(process),
                        ) => {
                            if !self.owned.contains(&process) {
                                self.owned.push(process);
                            }
                        }
                        Ok(ProcessObservation::Gone) => {}
                        other => {
                            eprintln!("owned PTY pid {pid} observation: {other:?}");
                            self.unconfirmed = true;
                        }
                    }
                }
            } else if file.path().exists() && !self.known_sockets.contains(&file.path()) {
                eprintln!(
                    "unrecorded socket returned no inspect state: {:?}",
                    file.path()
                );
                self.unconfirmed = true;
            }
        }
    }
    fn remember_programs(&mut self) {
        for name in ["alpha", "beta", "gamma", "alpha-peer"] {
            let Some(pid) = fs::read_to_string(self.root.join(format!("{name}.pid")))
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
            else {
                continue;
            };
            if let Ok(ProcessObservation::Live(process)) = observe_runtime_process(
                &UnixCommandRunner,
                pid,
                Instant::now() + Duration::from_secs(3),
            ) && !self.owned.contains(&process)
            {
                self.owned.push(process);
            }
        }
    }
    fn clean(&mut self) -> bool {
        for command in &mut self.commands {
            support::stop(command);
        }
        self.remember();
        self.remember_programs();
        if self.unconfirmed {
            return false;
        }
        for process in self.owned.iter().rev() {
            if matches!(observe_runtime_process(&UnixCommandRunner,process.pid(),Instant::now()+Duration::from_secs(3)),Ok(ProcessObservation::Live(ref now)|ProcessObservation::Stopped(ref now)) if now==process)
            {
                let _ = kill(Pid::from_raw(process.pid() as i32), Signal::SIGKILL);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let gone = self.owned.iter().all(|process| {
                match observe_runtime_process(&UnixCommandRunner, process.pid(), deadline) {
                    Ok(ProcessObservation::Gone) => true,
                    Ok(ProcessObservation::Live(now) | ProcessObservation::Stopped(now)) => {
                        now != *process
                    }
                    _ => false,
                }
            });
            if gone {
                return match fs::remove_dir_all(&self.root) {
                    Ok(()) => true,
                    Err(error) => error.kind() == std::io::ErrorKind::NotFound,
                };
            }
            if Instant::now() >= deadline {
                eprintln!("owned processes not confirmed gone: {:?}", self.owned);
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for World {
    fn drop(&mut self) {
        if !self.clean() {
            eprintln!("PTY cleanup unconfirmed; retained {}", self.root.display());
            assert!(std::thread::panicking(), "owned PTY processes survived")
        }
    }
}

#[test]
fn run_without_tmux_starts_independent_members_and_check_reads_causal_output() {
    let mut world = World::new();
    let agent = world.agent();
    world.json(&["identity", "create", "sender", "--json"]);
    let executable = env!("CARGO_BIN_EXE_tmt").replace('\'', "'\\''");
    let script = format!(
        r#"printf 'provider-ready\n'; while IFS= read -r line; do case "$line" in unbind-now) '{executable}' unbind --json > '__UNBOUND__';; *"tmt reply req_"*) set -- $line; while [ "$1" != tmt ]; do shift; done; '{executable}' whoami --json > '__WHOAMI__'; '{executable}' reply "$3" --receipt "$5" --message pty-processed; printf 'processed-request\n';; esac; done"#
    );
    for name in ["alpha", "beta"] {
        let script = script
            .replace(
                "__UNBOUND__",
                world.root.join("unbound.json").to_str().unwrap(),
            )
            .replace(
                "__WHOAMI__",
                world
                    .root
                    .join(format!("{name}.whoami.json"))
                    .to_str()
                    .unwrap(),
            );
        let script = format!(
            "printf '%s' \"$$\" > '{}'; {script}",
            world.root.join(format!("{name}.pid")).display()
        );
        let result = world.run(&["run", "--no-channel", "--save", name, &agent, "-c", &script]);
        assert!(result.status.success(), "{name}: {result:?}");
        // This launch has piped output and null input: it must return after
        // confirmation, without acquiring a terminal or waiting for the member.
        assert!(
            !result
                .stdout
                .windows(8)
                .any(|bytes| bytes == b"\x1b[?1049h")
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("host pty"),
            "{result:?}"
        );
    }
    let listed = world.json(&["ls", "--json"]);
    let rows = listed["identities"].as_array().expect("identity rows");
    assert!(rows.iter().any(|row| row["name"] == "alpha"));
    assert!(rows.iter().any(|row| row["name"] == "beta"));
    for row in rows
        .iter()
        .filter(|row| row["name"] == "alpha" || row["name"] == "beta")
    {
        assert_eq!(row["location"]["host"], "pty", "{row:#}");
    }
    for name in ["alpha", "beta"] {
        let result = world.run(&["check", name]);
        assert!(result.status.success(), "{name}: {result:?}");
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("provider-ready"),
            "{result:?}"
        );
    }
    for name in ["alpha", "beta"] {
        let reply = world.json(&[
            "talk",
            name,
            "hello from the private world",
            "--identity",
            "sender",
            "--timeout",
            "15",
            "--json",
        ]);
        assert_eq!(reply["response"], "pty-processed", "{reply:#}");
        let who: Value = serde_json::from_slice(
            &fs::read(world.root.join(format!("{name}.whoami.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(who["name"], name, "whoami packet: {who:#}");
        let checked = world.run(&["check", name]);
        assert!(checked.status.success(), "{checked:?}");
        assert!(
            String::from_utf8_lossy(&checked.stdout).contains("processed-request"),
            "{checked:?}"
        );
    }
    let listed = world.json(&["ls", "--json"]);
    let alpha = listed["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "alpha")
        .unwrap();
    let pane = alpha["pane"].as_str().unwrap().to_owned();
    let id = alpha["id"].as_str().unwrap().to_owned();
    let mut terminal =
        Connection::connect(&support::state_dir(&world.root).join("pty"), &pane).unwrap();
    terminal.input(b"unbind-now\r").unwrap();
    drop(terminal);
    let deadline = Instant::now() + Duration::from_secs(5);
    let unbound: Value = loop {
        if let Ok(bytes) = fs::read(world.root.join("unbound.json"))
            && let Ok(value) = serde_json::from_slice(&bytes)
        {
            break value;
        }
        assert!(Instant::now() < deadline, "caller unbind did not commit");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(unbound["id"], id);
    assert_eq!(unbound["retired"], false);
    let listed = world.json(&["ls", "--json"]);
    let alpha = listed["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "alpha")
        .unwrap();
    assert!(alpha["pane"].is_null());
    assert_eq!(alpha["id"], id);
    let removed = world.json(&["rm", "alpha", "--force", "--json"]);
    assert_eq!(removed["removed"], true);
    let beta = world.run(&["check", "beta"]);
    assert!(beta.status.success(), "{beta:?}");
    assert!(world.clean());
}

#[test]
fn stored_binding_reads_rename_and_force_remove_use_the_pty_host_without_tmux() {
    for tripwire in [false, true] {
        let mut world = World::new();
        // World clears the caller environment and exposes only ps and sh on PATH.
        assert!(!world.root.join("bin/tmux").exists());
        let tmux_called = world.root.join("tmux-called");
        if tripwire {
            // A refusing sentinel in place of an absent executable also catches
            // attempts whose spawn errors the command might otherwise ignore.
            let script = format!(
                "#!/bin/sh\nprintf called >> '{}'\nexit 99\n",
                tmux_called.display()
            );
            tmt_test_support::write_executable(
                &world.root.join("bin/tmux"),
                script.as_bytes(),
                0o755,
            )
            .unwrap();
        }
        let started = world.run(&[
            "run",
            "--host",
            "pty",
            "--no-channel",
            "--save",
            "alpha",
            "/bin/sh",
            "-c",
            "printf 'stored-pty-ready\n'; while IFS= read -r line; do :; done",
        ]);
        assert!(started.status.success(), "{started:?}");
        let named = world.json(&["ls", "alpha", "--json"]);
        assert_eq!(named["presence"], "active", "{named:#}");
        // The built-in host reports the member's live directory, as tmux reports a pane's;
        // the member was started from the world root (#5559).
        let cwd = named["pane"]["cwd"]
            .as_str()
            .expect("a built-in member reports its cwd");
        assert_eq!(
            fs::canonicalize(cwd).unwrap(),
            fs::canonicalize(&world.root).unwrap(),
            "{named:#}"
        );
        let id = named["identity"]["id"].as_str().unwrap().to_owned();
        let pane = named["pane"]["id"].as_str().unwrap().to_owned();
        let database = support::state_dir(&world.root).join("tmt.db");
        let original = tmt_adapters::storage::Storage::context_by_identity(&database, &id, 0)
            .unwrap()
            .unwrap()
            .entry
            .binding
            .unwrap();
        assert_eq!(original.server.host, tmt_core::host::HostKind::pty());
        let listed = world.json(&["ls", "--json"]);
        let rows = listed["identities"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{listed:#}");
        assert_eq!(rows[0]["id"], id);
        assert_eq!(rows[0]["presence"], "active");
        assert_eq!(rows[0]["location"]["host"], "pty");
        assert_eq!(rows[0]["pane"], pane);

        let renamed = world.json(&["rename", "alpha", "renamed", "--json"]);
        assert_eq!(renamed["renamed"], true, "{renamed:#}");
        assert_eq!(renamed["identity"]["id"], id);
        let current = tmt_adapters::storage::Storage::context_by_identity(&database, &id, 0)
            .unwrap()
            .unwrap()
            .entry;
        assert_eq!(current.identity.name, "renamed");
        assert_eq!(current.binding.as_ref().unwrap().id, original.id);
        let directory = support::state_dir(&world.root).join("pty");
        let mut terminal = Connection::connect(&directory, &pane).unwrap();
        let ControlReply::State { marker, .. } = terminal
            .control(Control::Inspect, Duration::from_secs(3))
            .unwrap()
        else {
            panic!("renamed supervisor did not return state");
        };
        let marker: Value = serde_json::from_str(marker.as_deref().unwrap()).unwrap();
        assert_eq!(marker["name"], "renamed");
        assert_eq!(marker["bindingId"], original.id);

        // A running member is never removed.
        let refused = world.run(&["rm", "renamed", "--force", "--json"]);
        assert_eq!(refused.status.code(), Some(5), "{refused:?}");
        let refused: Value = serde_json::from_slice(&refused.stdout).unwrap();
        assert_eq!(refused["error"]["code"], "AGENT_RUNNING", "{refused:#}");
        // A plain program has no recorded session for `tmt stop`; it is ended
        // in its own terminal, here by EOF, and then removes.
        terminal.input(b"\x04").unwrap();
        drop(terminal);
        let deadline = Instant::now() + WAIT;
        while world.json(&["ls", "renamed", "--json"])["presence"] == "active" {
            assert!(Instant::now() < deadline, "the program did not end");
            std::thread::sleep(Duration::from_millis(50));
        }
        let removed = world.json(&["rm", "renamed", "--force", "--json"]);
        assert_eq!(removed["removed"], true, "{removed:#}");
        assert_eq!(removed["identity"]["id"], id);
        assert!(
            tmt_adapters::storage::Storage::context_by_identity(&database, &id, 0)
                .unwrap()
                .is_none()
        );
        assert!(!tmux_called.exists(), "stored PTY operations invoked tmux");
        assert!(world.clean());
    }
}

#[test]
fn a_member_running_a_plain_program_receives_nothing_and_says_why() {
    let mut world = World::new();
    let started = world.run(&[
        "run",
        "--host",
        "pty",
        "--no-channel",
        "--save",
        "plain",
        "/bin/sh",
        "-c",
        "printf 'plain-ready\n'; while IFS= read -r line; do printf 'ran:%s\n' \"$line\"; done",
    ]);
    assert!(started.status.success(), "{started:?}");
    let listed = world.json(&["ls", "--json"]);
    let row = listed["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "plain")
        .unwrap()
        .clone();
    assert_eq!(row["presence"], "active", "{row:#}");
    assert_eq!(row["plainProgram"], "sh", "only the basename: {row:#}");
    let id = row["id"].as_str().unwrap().to_owned();

    let refused = world.run(&["talk", "plain", "do not run me", "--detach", "--json"]);
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let error: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(error["error"]["code"], "RECIPIENT_NOT_AGENT", "{error:#}");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("plain program (sh)") && message.contains("Nothing was sent"),
        "{message}"
    );
    // Nothing was stored for the recipient and nothing reached its terminal.
    let inbox = world.json(&["inbox", "--identity", &id, "--json"]);
    assert_eq!(
        inbox["items"].as_array().map(Vec::len),
        Some(0),
        "{inbox:#}"
    );
    let screen = world.run(&["check", "plain"]);
    let screen = String::from_utf8_lossy(&screen.stdout);
    assert!(screen.contains("plain-ready"), "{screen}");
    assert!(!screen.contains("do not run me"), "{screen}");
    assert!(world.clean());
}

#[test]
fn closing_a_built_in_member_waits_for_its_supervisor_and_touches_no_other_member() {
    use tmt_adapters::host::{Host, PaneClose};
    let mut world = World::new();
    let script = format!(
        r#"printf '%s' "$$" > '{root}/NAME.pid'; printf 'up\n'; while IFS= read -r line; do [ "$line" = quit ] && exit 0; done"#,
        root = world.root.display()
    );
    for name in ["alpha", "beta"] {
        let result = world.run(&[
            "run",
            "--no-channel",
            "--save",
            name,
            "/bin/sh",
            "-c",
            &script.replace("NAME", name),
        ]);
        assert!(result.status.success(), "{name}: {result:?}");
    }
    let listed = world.json(&["ls", "--json"]);
    let row = |name: &str| {
        listed["identities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap()
            .clone()
    };
    let database = support::state_dir(&world.root).join("tmt.db");
    let alpha = tmt_adapters::storage::Storage::context_by_identity(
        &database,
        row("alpha")["id"].as_str().unwrap(),
        0,
    )
    .unwrap()
    .unwrap()
    .entry;
    let server = alpha.binding.as_ref().unwrap().server.clone();
    let host = Host::for_server(&server);

    // Its program is still running: refused after the bound, and nothing was ended.
    let refused = host.session().close_opened_pane(&alpha);
    assert!(
        matches!(refused, Err(tmt_adapters::host::ActionError::Evidence(_))),
        "{refused:?}"
    );
    for name in ["alpha", "beta"] {
        let checked = world.run(&["check", name]);
        assert!(
            String::from_utf8_lossy(&checked.stdout).contains("up"),
            "{name} must still serve: {checked:?}"
        );
    }

    let pane = row("alpha")["pane"].as_str().unwrap().to_owned();
    let directory = support::state_dir(&world.root).join("pty");
    let socket = PathBuf::from(&server.socket_path);
    let supervisor_ended = || recorded_process_gone(server.server_pid);

    // A missing connection is not process death: keep the live supervisor behind
    // an unavailable socket and require the shutdown barrier to stay closed.
    let parked = socket.with_extension("parked");
    fs::rename(&socket, &parked).unwrap();
    assert!(Connection::connect(&directory, &pane).is_err());
    assert!(
        !supervisor_ended(),
        "a failed connection is not supervisor exit"
    );
    assert!(matches!(
        host.session().close_opened_pane(&alpha),
        Err(tmt_adapters::host::ActionError::Unverified)
    ));
    fs::rename(&parked, &socket).unwrap();

    // Its program ends on its own; the supervisor follows, and the close finds it gone.
    let mut terminal = Connection::connect(&directory, &pane).unwrap();
    terminal.input(b"quit\r").unwrap();
    drop(terminal);
    world.remember();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !supervisor_ended() {
        assert!(Instant::now() < deadline, "the supervisor did not end");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !socket.try_exists().unwrap(),
        "the supervisor left its socket"
    );
    assert!(
        !socket.with_extension("lock").try_exists().unwrap(),
        "the supervisor left its lock"
    );
    let closed = host.session().close_opened_pane(&alpha).unwrap();
    assert_eq!(closed, PaneClose::AlreadyGone);

    // The other member is untouched.
    let checked = world.run(&["check", "beta"]);
    assert!(
        String::from_utf8_lossy(&checked.stdout).contains("up"),
        "{checked:?}"
    );
    assert!(world.clean());
}

#[test]
fn the_built_in_launch_template_starts_a_background_member_without_tmux() {
    let mut world = World::new();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tmt"), world.root.join("bin/tmt")).unwrap();
    let templates = world.json(&["host", "templates", "--json"]);
    let hosts = templates["hosts"].as_array().unwrap();
    assert_eq!(hosts[0]["host"], "tmux", "{templates}");
    let pty = &hosts[1];
    assert_eq!(
        (pty["host"].as_str(), pty["state"].as_str()),
        (Some("pty"), Some("builtin"))
    );
    assert_eq!(
        pty["launch"]["placeholders"],
        serde_json::json!(["session", "command"])
    );
    let argv: Vec<String> = pty["launch"]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|word| word.as_str().unwrap().to_owned())
        .collect();
    let run = |world: &World, session: &str, command: &str| {
        let words: Vec<String> = argv
            .iter()
            .map(|word| match word.as_str() {
                "{session}" => session.to_owned(),
                "{command}" => command.to_owned(),
                _ => word.clone(),
            })
            .collect();
        // The caller's own environment: the world's, with the same PATH that has no tmux.
        let environment = world.command(&[]);
        let mut command = Command::new(&words[0]);
        command.args(&words[1..]).env_clear();
        if let Some(directory) = environment.get_current_dir() {
            command.current_dir(directory);
        }
        for (key, value) in environment.get_envs() {
            if let Some(value) = value {
                command.env(key, value);
            }
        }
        command.stdin(Stdio::null()).output().unwrap()
    };

    // The command is itself one built-in-host launch; the template runs it whole, so exactly
    // that member starts, and nothing is named after the session, which this host ignores (#5536).
    let started = run(
        &world,
        "ignored-session",
        "tmt run --host pty templated /bin/sh -c 'printf up; read l'",
    );
    assert!(started.status.success(), "{started:?}");
    world.remember();
    world.remember_programs();
    let seen = world.run(&["check", "templated"]);
    assert!(
        String::from_utf8_lossy(&seen.stdout).contains("up"),
        "{seen:?}"
    );
    // The session reaches the script only as data, and it is never run.
    let injected = world.root.join("injected");
    let hostile_session = format!("a b$(touch {})", injected.display());
    let plain = run(
        &world,
        &hostile_session,
        "tmt run --host pty plain /bin/sh -c 'printf x; read l'",
    );
    assert!(plain.status.success(), "{plain:?}");
    world.remember();
    world.remember_programs();
    assert!(!injected.exists(), "the session ran as script text");

    // A temporary member whose provider exits during startup reports why, and leaves no
    // identity behind: the dialog launches without --save for this reason.
    let provider = world.root.join("bin/early-provider");
    tmt_test_support::write_executable(
        &provider,
        b"#!/bin/sh\nprintf 'fixture-startup-refused\\n' >&2\nexit 23\n",
        0o755,
    )
    .unwrap();
    let early = run(
        &world,
        "ignored-session",
        "tmt run --host pty temp-early early-provider",
    );
    world.remember();
    world.remember_programs();
    assert_eq!(early.status.code(), Some(23), "{early:?}");
    assert!(
        String::from_utf8_lossy(&early.stderr).contains("fixture-startup-refused"),
        "{early:?}"
    );

    // An empty command is refused before `tmt run`, with no member started.
    let refused = run(&world, "other", "");
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("a command is required"));

    let listed = world.json(&["ls", "--all", "--json"]);
    let mut names: Vec<&str> = listed["identities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["plain", "templated"], "{listed}");
    assert!(world.clean());
}

#[test]
fn an_explicit_unavailable_host_never_falls_back_or_launches_a_member() {
    let mut world = World::new();
    let result = world.run(&["run", "--host", "tmux", "alpha", "/bin/sh", "-c", "exit 0"]);
    assert!(!result.status.success());
    assert!(!support::state_dir(&world.root).join("pty").exists());
    assert!(world.owned.is_empty());
}

#[test]
fn supervisor_loss_after_paste_is_uncertain_and_never_replayed() {
    let mut world = World::new();
    let agent = world.agent();
    world.json(&["identity", "create", "sender", "--json"]);
    world.json(&[
        "config",
        "set",
        "pasteEnterDelayMs",
        "10000",
        "--global",
        "--json",
    ]);
    for name in ["alpha", "beta"] {
        let script = format!(
            "printf '%s' \"$$\" > '{}'; printf 'provider-ready\\n'; while IFS= read -r line; do printf '%s\\n' \"$line\"; done",
            world.root.join(format!("{name}.pid")).display()
        );
        let started = world.run(&["run", "--no-channel", "--save", name, &agent, "-c", &script]);
        assert!(started.status.success(), "{started:?}");
    }
    let listed = world.json(&["ls", "--json"]);
    let alpha = listed["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "alpha")
        .unwrap();
    let pane = alpha["pane"].as_str().unwrap();
    let directory = support::state_dir(&world.root).join("pty");
    let mut observer = Connection::connect(&directory, pane).unwrap();
    let ControlReply::State { supervisor_pid, .. } = observer
        .control(Control::Inspect, Duration::from_secs(3))
        .unwrap()
    else {
        panic!("missing supervisor")
    };
    let owned = world
        .owned
        .iter()
        .find(|p| p.pid() == u64::from(supervisor_pid))
        .unwrap()
        .clone();
    let sending = world.start(&[
        "talk",
        "alpha",
        "interrupted-pty-delivery",
        "--identity",
        "sender",
        "--detach",
        "--json",
    ]);
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut bytes = Vec::new();
    // Terminal echo proves only that paste reached the PTY. It is an
    // interleaving barrier, never completion or provider-processing evidence.
    loop {
        if let Some(tmt_adapters::pty::protocol::ServerMessage::Output(output)) = observer
            .receive(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            bytes.extend(output);
        }
        if String::from_utf8_lossy(&bytes).contains("interrupted-pty-delivery") {
            break;
        }
        assert!(Instant::now() < deadline, "paste was not observed");
    }
    let db = rusqlite::Connection::open_with_flags(
        support::state_dir(&world.root).join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (request,status):(String,String)=db.query_row("SELECT request_id,wake_state FROM request_attempts WHERE message_text='interrupted-pty-delivery'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        status, "claimed",
        "the barrier must precede Enter settlement"
    );
    assert!(
        matches!(observe_runtime_process(&UnixCommandRunner,owned.pid(),Instant::now()+Duration::from_secs(3)),Ok(ProcessObservation::Live(now)) if now==owned)
    );
    kill(Pid::from_raw(supervisor_pid as i32), Signal::SIGKILL).unwrap();
    drop(observer);
    let failed = world.finish(sending);
    assert!(!failed.status.success(), "{failed:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&failed.stdout).unwrap()["error"]["code"],
        "DELIVERY_UNCERTAIN"
    );
    let before: (String, String, i64) = db
        .query_row(
            "SELECT status,wake_state,count(*) FROM request_attempts WHERE request_id=?",
            [&request],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(before, ("queued".into(), "uncertain".into(), 1));
    let beta = world.run(&["check", "beta"]);
    assert!(beta.status.success(), "{beta:?}");
    world.json(&["ls", "--json"]);
    let after: (String, String, i64) = db
        .query_row(
            "SELECT status,wake_state,count(*) FROM request_attempts WHERE request_id=?",
            [&request],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        after, before,
        "reads must not replay a possibly delivered request"
    );
    let program_pid = fs::read_to_string(world.root.join("alpha.pid"))
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let original = world.owned.iter().find(|p| p.pid() == program_pid).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match observe_runtime_process(&UnixCommandRunner, program_pid, deadline) {
            Ok(ProcessObservation::Gone) => break,
            Ok(ProcessObservation::Live(now) | ProcessObservation::Stopped(now))
                if now != *original =>
            {
                break;
            }
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "the owned program survived its supervisor; fixture cleanup must not hide it"
        );
        std::thread::yield_now();
    }
    drop(db);
    assert!(world.clean());
}

// Both resume scenarios release their single SessionStart only after exact Running
// provider/owner evidence, then wait for the committed session rather than hook exit.
fn record_provider_session(
    world: &World,
    previous_pane: Option<&str>,
    session: &str,
) -> (String, String, String) {
    let wait_hook = |world: &World, deadline: Instant, phase: &str| {
        while !world.root.join("hook.status").is_file() {
            assert!(
                Instant::now() < deadline,
                "{phase} hook did not publish its exit status; root={:?}; \
                 provider argv={:?}; provider pid={:?}; peer pid={:?}; \
                 hook stdout={:?}; hook stderr={:?}",
                world.root,
                fs::read_to_string(world.root.join("provider-argv")),
                fs::read_to_string(world.root.join("alpha.pid")),
                fs::read_to_string(world.root.join("alpha-peer.pid")),
                fs::read_to_string(world.root.join("hook.out")),
                fs::read_to_string(world.root.join("hook.err"))
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            fs::read_to_string(world.root.join("hook.status")).unwrap(),
            "0"
        );
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let db = rusqlite::Connection::open_with_flags(
        support::state_dir(&world.root).join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    #[derive(Debug)]
    struct LaunchRow {
        identity: String,
        pane: String,
        state: String,
        process_pid: Option<i64>,
        process_start: Option<String>,
        owner_pid: Option<i64>,
        owner_start: Option<String>,
        session: Option<String>,
    }
    // The fake provider emits one hook only. Hold it until the ordinary launch
    // has committed ownership, rather than racing that hook with launch's CAS.
    let wait_launch = |world: &World, previous_pane: Option<&str>, deadline: Instant| loop {
        let row = db
            .query_row(
                "SELECT i.id,b.pane_id,b.runtime_state,b.runtime_pid,b.runtime_start_identity, \
                 b.launch_owner_pid,b.launch_owner_start_identity,b.observed_provider_session_id \
                 FROM identities i JOIN bindings b ON b.identity_id=i.id \
                 WHERE i.canonical_name='alpha'",
                [],
                |r| {
                    Ok(LaunchRow {
                        identity: r.get(0)?,
                        pane: r.get(1)?,
                        state: r.get(2)?,
                        process_pid: r.get(3)?,
                        process_start: r.get(4)?,
                        owner_pid: r.get(5)?,
                        owner_start: r.get(6)?,
                        session: r.get(7)?,
                    })
                },
            )
            .unwrap();
        let expected_pid = fs::read_to_string(world.root.join("alpha.pid"))
            .ok()
            .and_then(|text| text.parse::<u64>().ok());
        if row.state == "running"
            && previous_pane.is_none_or(|pane| row.pane != pane)
            && let (Some(pid), Some(start), Some(owner), Some(owner_start)) = (
                row.process_pid.map(|pid| u64::try_from(pid).unwrap()),
                row.process_start.as_deref(),
                row.owner_pid.map(|pid| u64::try_from(pid).unwrap()),
                row.owner_start.as_deref(),
            )
            && Some(pid) == expected_pid
            && matches!(
                observe_runtime_process(&UnixCommandRunner, pid, deadline),
                Ok(ProcessObservation::Live(process)) if process.start_identity() == start
            )
            && matches!(
                observe_runtime_process(&UnixCommandRunner, owner, deadline),
                Ok(ProcessObservation::Live(process)) if process.start_identity() == owner_start
            )
        {
            break (row.identity, row.pane, row.session);
        }
        assert!(
            Instant::now() < deadline,
            "expected exact Running fixture launch; provider pid={expected_pid:?}; last row={row:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let release_hook = |world: &World, pane: &str| {
        let mut terminal =
            Connection::connect(&support::state_dir(&world.root).join("pty"), pane).unwrap();
        terminal.input(b"\r").unwrap();
    };
    // A zero-exit hook is best-effort; readiness is the committed session, not
    // the hook's status file. Each query owns a fresh read snapshot.
    let wait_session = |world: &World, previous_pane: Option<&str>, deadline: Instant| loop {
        let row: (String, String, Option<String>) = db
            .query_row(
                "SELECT i.id,b.pane_id,b.observed_provider_session_id FROM identities i \
                 JOIN bindings b ON b.identity_id=i.id WHERE i.canonical_name='alpha'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        if row.2.as_deref() == Some(session) && previous_pane.is_none_or(|pane| row.1 != pane) {
            break (row.0, row.1, row.2.unwrap());
        }
        assert!(
            Instant::now() < deadline,
            "expected committed session {session}, previous pane {previous_pane:?}, \
             last row {row:?}; hook diagnostics: {}",
            fs::read_to_string(world.root.join("hook.err")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let (identity, pane, before_session) = wait_launch(world, previous_pane, deadline);
    if previous_pane.is_none() {
        assert!(
            before_session.is_none(),
            "the held hook must cause session recording"
        );
    }
    assert!(!world.root.join("hook.status").exists());
    release_hook(world, &pane);
    wait_hook(world, deadline, "initial");
    let (recorded_identity, recorded_pane, recorded) = wait_session(world, previous_pane, deadline);
    assert_eq!(recorded_identity, identity);
    assert_eq!(recorded_pane, pane);
    assert_eq!(
        recorded,
        session,
        "hook diagnostics: {}",
        fs::read_to_string(world.root.join("hook.err")).unwrap()
    );
    (identity, pane, recorded)
}

#[test]
#[ignore = "flaky on hosted runners; see #4767"]
fn a_hook_recorded_session_resumes_in_a_new_pty_without_tmux() {
    let mut world = World::new();
    let example = std::path::Path::new(env!("CARGO_BIN_EXE_tmt"))
        .parent()
        .unwrap()
        .join("examples/provider-script-fixture");
    assert!(
        example.is_file(),
        "build the shared fixture first: cargo build --locked -p tmt-adapters --example provider-script-fixture"
    );
    let provider = world.root.join("bin/claude");
    tmt_test_support::write_executable(&provider, &fs::read(&example).unwrap(), 0o755).unwrap();
    let interpreter = world.root.join("fixture-interpreter");
    let tmt = env!("CARGO_BIN_EXE_tmt");
    let root = world.root.display();
    let session = "11111111-1111-4111-8111-111111111111";
    let script = format!(
        r#"#!/bin/sh
printf '%s' "$PPID" > '{root}/alpha.pid'
printf '%s' "$$" > '{root}/alpha-peer.pid'
printf '%s\n' "$@" >> '{root}/provider-argv'
IFS= read -r release_hook
printf '%s' '{{"hook_event_name":"SessionStart","session_id":"{session}","source":"startup","cwd":"{root}","model":"fixture-model"}}' | '{tmt}' __hook claude > '{root}/hook.out' 2> '{root}/hook.err'
printf '%s' "$?" > '{root}/hook.status.tmp'
/bin/mv '{root}/hook.status.tmp' '{root}/hook.status'
printf 'provider-ready\n'
IFS= read -r line
"#
    );
    tmt_test_support::write_executable(&interpreter, script.as_bytes(), 0o755).unwrap();
    world.interpreter = Some(interpreter);
    let started = world.run(&[
        "run",
        "--no-channel",
        "--save",
        "alpha",
        provider.to_str().unwrap(),
    ]);
    assert!(started.status.success(), "{started:?}");
    let (identity, pane, recorded) = record_provider_session(&world, None, session);
    assert_eq!(recorded, session);
    world.remember_programs();
    let old_program = fs::read_to_string(world.root.join("alpha.pid"))
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let mut terminal =
        Connection::connect(&support::state_dir(&world.root).join("pty"), &pane).unwrap();
    terminal.input(b"\r").unwrap();
    drop(terminal);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if matches!(
            observe_runtime_process(&UnixCommandRunner, old_program, deadline),
            Ok(ProcessObservation::Gone)
        ) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    fs::remove_file(world.root.join("hook.status")).unwrap();
    let resumed = world.run(&["resume", "--host", "pty", "--no-channel", "alpha"]);
    support::with_launch_failure_packet(
        &support::state_dir(&world.root).join("tmt.db"),
        &[old_program],
        || assert!(resumed.status.success(), "{resumed:?}"),
    );
    let (after_identity, after_pane, after_session) =
        record_provider_session(&world, Some(&pane), session);
    assert_eq!(after_identity, identity);
    assert_ne!(after_pane, pane);
    assert_eq!(after_session, session);
    let argv = fs::read_to_string(world.root.join("provider-argv")).unwrap();
    assert!(
        argv.lines()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|words| words == ["--resume", session]),
        "{argv}"
    );
    assert!(world.clean());
}

#[test]
fn a_fast_exit_reports_completion_and_a_missing_program_is_not_success() {
    let mut world = World::new();
    let fast = world.run(&[
        "run",
        "--no-channel",
        "--save",
        "alpha",
        "/bin/sh",
        "-c",
        "exit 0",
    ]);
    assert!(fast.status.success(), "{fast:?}");
    let completed = String::from_utf8_lossy(&fast.stderr);
    assert!(
        completed.contains("ended during startup (exit status 0)"),
        "{fast:?}"
    );
    assert!(!completed.contains("started alpha on host"), "{fast:?}");
    let failed = world.run(&[
        "run",
        "--no-channel",
        "--save",
        "beta",
        "/no-such-private-fixture-program",
    ]);
    assert!(!failed.status.success(), "{failed:?}");
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("Could not start the requested command"),
        "{failed:?}"
    );
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("not confirmed"));
    assert!(world.clean());
}

#[path = "pty_host/attach.rs"]
mod attach;
#[path = "pty_host/e2e.rs"]
mod e2e;
#[path = "pty_host/terminal_stream.rs"]
mod terminal_stream;

#[path = "pty_host/codex_launcher.rs"]
mod codex_launcher;

#[path = "pty_host/startup.rs"]
mod startup;

/// The newest built-in snapshot's members (the protected baseline by default), by name.
fn built_in_members(world: &mut World) -> Vec<(String, String)> {
    let shown = world.json(&["workspace", "show", "--host", "pty", "--json"]);
    assert_eq!(shown["host"], "pty", "{shown:#}");
    let mut members: Vec<_> = shown["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| {
            (
                member["name"].as_str().unwrap().to_owned(),
                member["action"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    members.sort();
    members
}

#[test]
fn built_in_members_are_recorded_leave_only_on_purpose_and_restore_never_touches_a_live_one() {
    let mut world = World::new();
    let store = support::state_dir(&world.root).join("workspace/pty");
    let empty = world.json(&["workspace", "ls", "--host", "pty", "--json"]);
    assert_eq!(
        empty,
        serde_json::json!({"version": 1, "host": "pty", "snapshots": []})
    );
    assert!(!store.exists(), "listing creates no store");
    let member = |world: &mut World, name: &str| {
        let script = format!(
            "printf '%s' \"$$\" > '{}'; printf 'ready\\n'; while IFS= read -r line; do :; done",
            world.root.join(format!("{name}.pid")).display()
        );
        let started = world.run(&[
            "run",
            "--host",
            "pty",
            "--no-channel",
            "--save",
            name,
            "/bin/sh",
            "-c",
            &script,
        ]);
        assert!(started.status.success(), "{name}: {started:?}");
    };
    // The launch alone publishes: no manual save, hook or backstop.
    member(&mut world, "alpha");
    assert!(store.join("generations.json").is_file());
    member(&mut world, "beta");
    assert_eq!(
        built_in_members(&mut world),
        [
            ("alpha".into(), "live".into()),
            ("beta".into(), "live".into())
        ]
    );
    // beta's program ends without an explicit stop (an exit, a crash or logout). The next
    // launch's capture lost beta but started gamma: gamma joins and beta stays.
    let beta: i32 = fs::read_to_string(world.root.join("beta.pid"))
        .unwrap()
        .parse()
        .unwrap();
    kill(Pid::from_raw(beta), Signal::SIGKILL).unwrap();
    let deadline = Instant::now() + WAIT;
    while world.json(&["ls", "beta", "--json"])["presence"] == "active" {
        assert!(Instant::now() < deadline, "beta's supervisor did not end");
        std::thread::sleep(Duration::from_millis(50));
    }
    member(&mut world, "gamma");
    let listed = world.json(&["workspace", "ls", "--host", "pty", "--json"]);
    // Later captures of the same live set may follow as shrinks; the protected one is the merge.
    let protected: Vec<_> = listed["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|snapshot| snapshot["protected"] == true)
        .collect();
    assert_eq!(protected.len(), 1, "{listed:#}");
    assert_eq!(protected[0]["shrink"], false, "{listed:#}");
    assert_eq!(protected[0]["agents"], 3, "{listed:#}");
    let baseline = built_in_members(&mut world);
    assert_eq!(baseline.len(), 3, "{baseline:?}");
    assert_eq!(baseline[0], ("alpha".into(), "live".into()));
    assert_eq!(baseline[1].0, "beta");
    assert_ne!(baseline[1].1, "live");
    assert_eq!(baseline[2], ("gamma".into(), "live".into()));
    // Removing beta on purpose takes it out of the default restore.
    let removed = world.run(&["rm", "beta", "--force", "--json"]);
    assert!(removed.status.success(), "{removed:?}");
    assert_eq!(
        built_in_members(&mut world),
        [
            ("alpha".into(), "live".into()),
            ("gamma".into(), "live".into())
        ]
    );
    // Restore never touches a running member and waits on one it cannot resume. alpha and
    // gamma run, so only gamma's record decides; a rerun finds the same and starts nothing.
    let alpha_pid = fs::read_to_string(world.root.join("alpha.pid")).unwrap();
    let gamma: i32 = fs::read_to_string(world.root.join("gamma.pid"))
        .unwrap()
        .parse()
        .unwrap();
    kill(Pid::from_raw(gamma), Signal::SIGKILL).unwrap();
    let deadline = Instant::now() + WAIT;
    while world.json(&["ls", "gamma", "--json"])["presence"] == "active" {
        assert!(Instant::now() < deadline, "gamma's supervisor did not end");
        std::thread::sleep(Duration::from_millis(50));
    }
    let report = support::state_dir(&world.root).join("restore-last-pty.json");
    for _ in 0..2 {
        let restored = world.run(&["workspace", "restore", "--host", "pty", "--json"]);
        assert_eq!(restored.status.code(), Some(0), "{restored:?}");
        let restored: Value = serde_json::from_slice(&restored.stdout).unwrap();
        assert_eq!(restored["host"], "pty", "{restored:#}");
        assert_eq!(restored["status"], "partial", "{restored:#}");
        let mut members: Vec<_> = restored["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| {
                (
                    member["name"].as_str().unwrap(),
                    member["status"].as_str().unwrap(),
                    member["reason"].as_str().unwrap(),
                )
            })
            .collect();
        // Recorded order is by identity, not by name.
        members.sort();
        assert_eq!(
            members,
            [
                ("alpha", "skipped", "live"),
                ("gamma", "needs_you", "no_remembered_session")
            ],
            "{restored:#}"
        );
        let gamma = restored["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|member| member["name"] == "gamma")
            .unwrap();
        let retry = gamma["retry"].as_str().unwrap();
        assert!(
            retry.ends_with("tmt resume --host pty --new -- gamma"),
            "{retry}"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&report).unwrap()).unwrap()["members"],
            restored["members"]
        );
        assert_eq!(
            fs::read_to_string(world.root.join("alpha.pid")).unwrap(),
            alpha_pid,
            "a running member is never restarted"
        );
        assert_eq!(world.json(&["ls", "alpha", "--json"])["presence"], "active");
    }
    // --fresh goes through ordinary resume --new; a member with no remembered driver is
    // refused there, and the refusal's own words come back with exit 1.
    let fresh = world.run(&["workspace", "restore", "--host", "pty", "--fresh", "--json"]);
    assert_eq!(fresh.status.code(), Some(1), "{fresh:?}");
    let fresh: Value = serde_json::from_slice(&fresh.stdout).unwrap();
    let member = |name: &str| {
        fresh["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|member| member["name"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(member("alpha")["status"], "skipped", "{fresh:#}");
    assert_eq!(member("gamma")["status"], "failed", "{fresh:#}");
    assert!(
        member("gamma")["tail"]
            .as_str()
            .unwrap()
            .contains("no remembered driver"),
        "{fresh:#}"
    );
    assert_eq!(world.json(&["ls", "alpha", "--json"])["presence"], "active");
}

/// A tmux store as a tmux user leaves it when their server ends.
fn leave_tmux_store(world: &World, socket: &str) {
    use tmt_core::workspace::*;
    let paths = tmt_adapters::config::ConfigPaths::resolve(
        &world.root,
        &world.root,
        Some(&support::state_dir(&world.root)),
        None,
    );
    let snapshot = WorkspaceSnapshot {
        captured_at_ms: 1,
        server: WorkspaceServer {
            socket: socket.into(),
            process: ProcessIncarnation::new(10, "native-start").unwrap(),
            id: None,
        },
        sessions: vec![WorkspaceSession {
            id: "$1".into(),
            name: "work".into(),
            windows: vec![WindowLink {
                index: 0,
                window: "@1".into(),
                active: true,
            }],
        }],
        windows: vec![WorkspaceWindow {
            id: "@1".into(),
            name: "shell".into(),
            layout: "abc,80x24,0,0,1".into(),
            visible_layout: "abc,80x24,0,0,1".into(),
            width: 80,
            height: 24,
            active_pane: "%1".into(),
        }],
        panes: vec![WorkspacePane {
            id: "%1".into(),
            window: "@1".into(),
            index: 0,
            left: 0,
            top: 0,
            width: 80,
            height: 24,
            cwd: "/tmp".into(),
            identity: None,
            command: None,
        }],
    };
    assert!(tmt_adapters::workspace::publish_event(&paths, socket, || Ok(snapshot)).unwrap());
}

#[test]
fn outside_tmux_workspace_commands_select_the_built_in_store_and_point_at_tmux_stores() {
    let mut world = World::new();
    // No tmux and no stores: an empty built-in listing, no hint.
    let empty = world.json(&["workspace", "ls", "--json"]);
    assert_eq!(
        empty,
        serde_json::json!({"version": 1, "host": "pty", "snapshots": []})
    );
    // A tmux user whose server ended: the built-in store is still selected, and the tmux
    // stores are named with the --socket form.
    leave_tmux_store(&world, "/tmp/ended-tmux.sock");
    let hinted = world.json(&["workspace", "ls", "--json"]);
    assert_eq!(hinted["host"], "pty", "{hinted:#}");
    assert_eq!(hinted["snapshots"], serde_json::json!([]), "{hinted:#}");
    assert_eq!(
        hinted["socketStores"],
        serde_json::json!(["/tmp/ended-tmux.sock"]),
        "{hinted:#}"
    );
    let human = world.run(&["workspace", "ls"]);
    assert!(human.status.success(), "{human:?}");
    assert!(
        String::from_utf8_lossy(&human.stdout)
            .contains("tmt workspace ls --socket '/tmp/ended-tmux.sock'"),
        "{human:?}"
    );
    for command in ["show", "restore"] {
        let refused = world.run(&["workspace", command, "--json"]);
        assert_eq!(refused.status.code(), Some(3), "{refused:?}");
        let refused: Value = serde_json::from_slice(&refused.stdout).unwrap();
        assert_eq!(refused["error"]["code"], "WORKSPACE_SNAPSHOT_NOT_FOUND");
        assert!(
            refused["error"]["suggestion"]
                .as_str()
                .unwrap()
                .contains(&format!(
                    "tmt workspace {command} --socket '/tmp/ended-tmux.sock'"
                )),
            "{refused:#}"
        );
        assert_eq!(
            refused["socketStores"],
            serde_json::json!(["/tmp/ended-tmux.sock"])
        );
    }
    // The explicit socket still reaches the tmux store.
    let tmux = world.json(&[
        "workspace",
        "ls",
        "--socket",
        "/tmp/ended-tmux.sock",
        "--json",
    ]);
    assert_eq!(tmux["host"], "tmux", "{tmux:#}");
    assert_eq!(tmux["snapshots"].as_array().unwrap().len(), 1, "{tmux:#}");
    // Once a member runs, the built-in store answers alone; the hint is gone.
    let started = world.run(&[
        "run",
        "--host",
        "pty",
        "--no-channel",
        "--save",
        "alpha",
        "/bin/sh",
        "-c",
        "printf 'ready\\n'; while IFS= read -r line; do :; done",
    ]);
    assert!(started.status.success(), "{started:?}");
    let listed = world.json(&["workspace", "ls", "--json"]);
    assert_eq!(listed["host"], "pty", "{listed:#}");
    assert!(listed.get("socketStores").is_none(), "{listed:#}");
    let shown = world.json(&["workspace", "show", "--json"]);
    assert_eq!(shown["members"][0]["name"], "alpha", "{shown:#}");
    assert_eq!(shown["members"][0]["action"], "live", "{shown:#}");
    let restored = world.json(&["workspace", "restore", "--json"]);
    assert_eq!(restored["host"], "pty", "{restored:#}");
    assert_eq!(restored["members"][0]["status"], "skipped", "{restored:#}");
}

#[test]
fn restore_checks_each_running_member_alone_and_skips_all_of_them() {
    let mut world = World::new();
    let names = ["alpha", "beta", "gamma", "delta"];
    for name in names {
        let started = world.run(&[
            "run",
            "--host",
            "pty",
            "--no-channel",
            "--save",
            name,
            "/bin/sh",
            "-c",
            "printf 'ready\\n'; while IFS= read -r line; do :; done",
        ]);
        assert!(started.status.success(), "{name}: {started:?}");
    }
    let restored = world.json(&["workspace", "restore", "--host", "pty", "--json"]);
    let mut members: Vec<_> = restored["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| {
            (
                member["name"].as_str().unwrap().to_owned(),
                member["status"].as_str().unwrap().to_owned(),
                member["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    members.sort();
    let mut expected: Vec<_> = names
        .iter()
        .map(|name| (name.to_string(), "skipped".to_owned(), "live".to_owned()))
        .collect();
    expected.sort();
    assert_eq!(members, expected, "{restored:#}");
    assert_eq!(restored["status"], "completed", "{restored:#}");
}
