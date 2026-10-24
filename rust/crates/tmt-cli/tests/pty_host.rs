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
        runtime::{ProcessObservation, observe_runtime_process},
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
        for name in ["alpha", "beta", "alpha-peer"] {
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
        let result = world.run(&[
            "run",
            "--no-channel",
            "--save",
            name,
            "/bin/sh",
            "-c",
            &script,
        ]);
        assert!(result.status.success(), "{name}: {result:?}");
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
        let started = world.run(&[
            "run",
            "--no-channel",
            "--save",
            name,
            "/bin/sh",
            "-c",
            &script,
        ]);
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

#[test]
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
    let (identity, pane, before_session) = wait_launch(&world, None, deadline);
    assert!(
        before_session.is_none(),
        "the held hook must cause session recording"
    );
    assert!(!world.root.join("hook.status").exists());
    release_hook(&world, &pane);
    wait_hook(&world, deadline, "initial");
    let (recorded_identity, recorded_pane, recorded) = wait_session(&world, None, deadline);
    assert_eq!(recorded_identity, identity);
    assert_eq!(recorded_pane, pane);
    assert_eq!(
        recorded,
        session,
        "hook diagnostics: {}",
        fs::read_to_string(world.root.join("hook.err")).unwrap()
    );
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
    assert!(resumed.status.success(), "{resumed:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    let (resumed_identity, resumed_pane, _) = wait_launch(&world, Some(&pane), deadline);
    assert_eq!(resumed_identity, identity);
    release_hook(&world, &resumed_pane);
    wait_hook(&world, deadline, "resumed");
    let (after_identity, after_pane, after_session) = wait_session(&world, Some(&pane), deadline);
    assert_eq!(after_pane, resumed_pane);
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
    drop(db);
    assert!(world.clean());
}

#[test]
fn a_fast_exit_has_one_confirmed_startup_and_a_missing_program_is_not_success() {
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
    let failed = world.run(&[
        "run",
        "--no-channel",
        "--save",
        "beta",
        "/no-such-private-fixture-program",
    ]);
    assert!(!failed.status.success(), "{failed:?}");
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("launch"),
        "{failed:?}"
    );
    assert!(world.clean());
}

#[path = "pty_host/attach.rs"]
mod attach;
