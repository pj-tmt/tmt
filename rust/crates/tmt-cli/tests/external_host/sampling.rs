//! The foreground consumption sampler on an approved external host (#3581):
//! a real `tmt run` owns a stand-in Claude in the fake pane, and the test
//! starts `tmt __consumption-sample` itself, as the run's own tick does.
//!
//! The driver's `gate` refuses every sampler but the one the test starts,
//! so no automatic tick can commit an observation around a baseline. The
//! host server is a separate process the test owns, so its recorded
//! incarnation can end while the run, provider and pane stay live.
use super::*;
use std::{
    io::Write,
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, ExitStatus},
    time::{Duration, Instant},
};

const SESSION: &str = "4f8c2a7e-0d3b-4c61-9a52-7b1e6d0c9f13";
const RECORDS: &str =
    include_str!("../../../tmt-adapters/src/runtime/fixtures/claude-usage-sequence.jsonl");

/// One request of the pinned sequence: its records, in order, by message ID.
fn request(message: &str) -> String {
    RECORDS
        .lines()
        .filter(|line| line.contains(&format!(r#""id": "{message}""#)))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// Consumption after `msg_fixture0002`, summed once by hand from the pinned
/// records: input 2 + 289 + 355111, cached 355111, output 546.
const AFTER_SECOND: [u64; 3] = [355_402, 355_111, 546];
/// And after `msg_fixture0003` too: input + 2 + 672 + 355400, cached
/// + 355400, output + 324.
const AFTER_THIRD: [u64; 3] = [711_476, 710_511, 870];

/// The host server: a process apart from the test, the run and the pane,
/// which runs until `stop-server` exists. Dropping it, also after a failed
/// assertion, stops and reaps it within a bound.
struct HostServer {
    child: Option<Child>,
    stop: PathBuf,
}

impl HostServer {
    fn start(fixture: &Fixture) -> Self {
        let stop = fixture.root.join("stop-server");
        let child = std::process::Command::new("/bin/sh")
            .args(["-c", "while [ ! -e \"$0\" ]; do sleep 0.05; done"])
            .arg(&stop)
            .env_clear()
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            child: Some(child),
            stop,
        }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    /// Ends the server and reaps it; its recorded incarnation is then gone.
    fn stop(&mut self) {
        if self.child.is_none() {
            return;
        }
        fs::write(&self.stop, "").unwrap();
        support::wait(&mut self.child, Duration::from_secs(10))
            .wait()
            .unwrap();
    }
}

impl Drop for HostServer {
    fn drop(&mut self) {
        let _ = fs::write(&self.stop, "");
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_secs(5);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A direct sampler the test owns from its spawn: the leader of its own
/// process group, with the attempt's budget. Until the leader is reaped its
/// pid cannot name another process, so only then is its group signalled.
/// `finish` and `Drop` both end the group, reap the leader and wait for the
/// group to be empty, so no sampler or driver outlives the test's fixture.
/// (The repository's bounded runner cannot be reused here: it takes no
/// environment and writes the request itself, so there is no gate barrier.)
struct DirectSampler {
    child: Option<Child>,
    budget: Instant,
}

impl DirectSampler {
    fn spawn(mut command: std::process::Command, budget: Instant) -> Self {
        let child = command.process_group(0).spawn().unwrap();
        Self {
            child: Some(child),
            budget,
        }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    fn group(&self) -> nix::unistd::Pid {
        nix::unistd::Pid::from_raw(i32::try_from(self.pid()).unwrap())
    }

    fn request(&mut self, request: &[u8]) {
        let mut stdin = self.child.as_mut().unwrap().stdin.take().unwrap();
        stdin.write_all(request).unwrap();
    }

    /// Waits until the leader exits, or kills its group when the budget
    /// ends; then reaps it. Returns its status and whether it was killed.
    fn finish(&mut self) -> (ExitStatus, bool) {
        let pid = u64::from(self.pid());
        let killed = loop {
            if Instant::now() >= self.budget {
                break true;
            }
            // An exited leader stays a zombie until it is reaped here, so
            // its pid names no other process; the observation calls that
            // zombie gone.
            if let Ok(ProcessObservation::UnreapedZombie(_) | ProcessObservation::Gone) =
                observe_runtime_process(&UnixCommandRunner, pid, self.budget)
            {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let status = self.stop();
        (status, killed)
    }

    /// Ends what is left of the group while its leader is unreaped, reaps
    /// the leader within 3 s, then waits, read only, for the group to be
    /// empty.
    fn stop(&mut self) -> ExitStatus {
        use nix::sys::signal::{Signal::SIGKILL, killpg};
        let group = self.group();
        let mut child = self.child.take().unwrap();
        let _ = killpg(group, SIGKILL);
        let reaped = Instant::now() + Duration::from_secs(3);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < reaped, "the sampler was not reaped");
            std::thread::sleep(Duration::from_millis(10));
        };
        let empty = Instant::now() + Duration::from_secs(3);
        while !matches!(killpg(group, None), Err(nix::errno::Errno::ESRCH)) {
            assert!(Instant::now() < empty, "the sampler's group is not empty");
            std::thread::sleep(Duration::from_millis(10));
        }
        status
    }
}

impl Drop for DirectSampler {
    fn drop(&mut self) {
        if self.child.is_none() {
            return;
        }
        if std::thread::panicking() {
            // The same cleanup; a second panic while unwinding would abort.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.stop()));
        } else {
            self.stop();
        }
    }
}

/// A run with consumption sampling: the server, the run and its admitted
/// provider, with the transcript at its first request. Fields drop in
/// order: the run, then the server, then the fixture's files.
struct Sampling {
    run: Option<Launched>,
    server: HostServer,
    fixture: Fixture,
    transcript: PathBuf,
    request: Value,
}

impl Sampling {
    fn start(name: &str) -> Self {
        register();
        let fixture = Fixture::new(name);
        let server = HostServer::start(&fixture);
        let driver = fixture.root.join("driver");
        fs::write(
            driver.join("server"),
            format!(
                r#"{{"ok":{{"server":{{"socket":"/tmp/fake.sock","pid":{},"startTime":"driver-says"}}}}}}"#,
                server.pid()
            ),
        )
        .unwrap();
        fs::write(driver.join("sampling"), "").unwrap();
        // No sampler is answered until the test names one.
        fs::write(driver.join("gate"), "").unwrap();
        fixture.approve();

        let tmt = env!("CARGO_BIN_EXE_tmt");
        let home = fixture.root.join("home");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(
            home.join(".claude/settings.json"),
            serde_json::json!({"hooks": {"Stop": [{"hooks": [{"type": "command",
                "command": format!("'{tmt}' __hook claude"), "timeout": 3}]}]}})
            .to_string(),
        )
        .unwrap();
        let projects = home.join(".claude/projects/-src");
        fs::create_dir_all(&projects).unwrap();
        let transcript = projects.join(format!("{SESSION}.jsonl"));
        fs::write(&transcript, request("msg_fixture0001")).unwrap();
        for (file, event) in [("start", "SessionStart"), ("prompt", "UserPromptSubmit")] {
            let payload = serde_json::json!({"hook_event_name": event, "session_id": SESSION,
                "source": "startup", "cwd": "/src", "model": "claude-test", "prompt": "go",
                "transcript_path": transcript});
            fs::write(
                fixture.root.join(format!("{file}.json")),
                payload.to_string(),
            )
            .unwrap();
        }

        // The stand-in is named `claude`, so it is the hooks' provider. It
        // reports its start and first prompt, then runs until `release`.
        let bin = fixture.root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let claude = bin.join("claude");
        std::os::unix::fs::symlink("/bin/sh", &claude).unwrap();
        let provider = "echo $$ > agent.pid
            \"$0\" __hook claude < start.json > start.out 2> start.err || exit 11
            \"$0\" __hook claude < prompt.json > prompt.out 2> prompt.err || exit 12
            touch ready
            trap 'touch acked; exit 3' TERM HUP
            while [ ! -f release ]; do sleep 0.05; done; exit 7";
        let mut run = Launched::start(
            &fixture,
            fixture.command_in(
                Some("f1"),
                &[
                    "run",
                    "--no-channel",
                    "worker",
                    claude.to_str().unwrap(),
                    "-c",
                    provider,
                    tmt,
                ],
            ),
        );
        run.adopt("agent.pid", false);
        let ready = fixture.root.join("ready");
        wait_until("the provider's start and prompt hooks", || ready.exists());
        let mut sampling = Self {
            fixture,
            server,
            run: Some(run),
            transcript,
            request: Value::Null,
        };
        for file in ["start", "prompt"] {
            let err = fs::read_to_string(sampling.fixture.root.join(format!("{file}.err")));
            assert_eq!(err.unwrap(), "", "{file} hook");
        }
        sampling.request = sampling.sampler_request();
        sampling
    }

    /// The tick's request for this run, from the stored binding: its exact
    /// launch owner, while the provider's session is running.
    fn sampler_request(&self) -> Value {
        let database = self.fixture.state().join("tmt.db");
        let identity: String = rusqlite::Connection::open(&database)
            .unwrap()
            .query_row(
                "SELECT id FROM identities WHERE name = 'worker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let now = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let stored = Storage::context_by_identity(&database, &identity, now)
            .unwrap()
            .unwrap();
        let binding = stored.entry.binding.unwrap();
        assert_eq!(
            binding.session.state,
            tmt_core::binding::session::RuntimeState::Running
        );
        assert_eq!(
            binding
                .session
                .key
                .unwrap()
                .provider_session
                .unwrap()
                .as_str(),
            SESSION
        );
        assert_eq!(binding.server.server_pid, u64::from(self.server.pid()));
        let owner = binding.session.launch_owner.unwrap();
        assert_eq!(owner.pid(), u64::from(self.run_pid()));
        serde_json::json!({"identityId": identity, "bindingId": binding.id,
            "ownerPid": owner.pid(), "ownerStart": owner.start_identity()})
    }

    fn run_pid(&self) -> u32 {
        self.run.as_ref().unwrap().child.as_ref().unwrap().id()
    }

    fn append(&self, message: &str) {
        let mut transcript = fs::OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .unwrap();
        transcript.write_all(request(message).as_bytes()).unwrap();
    }

    /// The snapshot log, one line per call.
    fn snapshots(&self) -> Vec<String> {
        fs::read_to_string(self.fixture.root.join("driver/snapshots"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The stored consumption counts: input, cached input and output.
    fn consumption(&self) -> Option<[u64; 3]> {
        let latest: Option<String> =
            rusqlite::Connection::open(self.fixture.state().join("tmt.db"))
                .unwrap()
                .query_row("SELECT latest FROM consumption_sources", [], |row| {
                    row.get(0)
                })
                .unwrap();
        let latest: Value = serde_json::from_str(&latest?).unwrap();
        let counts = &latest["consumption"];
        Some(
            ["inputTokens", "cachedInputTokens", "outputTokens"]
                .map(|field| counts[field].as_u64().unwrap()),
        )
    }

    fn sampled_at(&self) -> Option<i64> {
        rusqlite::Connection::open(self.fixture.state().join("tmt.db"))
            .unwrap()
            .query_row("SELECT sampled_at_ms FROM consumption_sources", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    /// Starts `tmt __consumption-sample` as the tick does, in its own
    /// process group, with only this worker answered by the driver.
    fn sample(&self) -> Sampled {
        self.sample_after(|_| {})
    }

    /// [`Self::sample`], with `before_request` run once the sampler is
    /// spawned and named in the gate, before it gets its request. The tick's
    /// 2 s budget governs the whole attempt, setup included. After it, the
    /// cleanup has up to 3 s to reap the sampler and up to 3 s more for its
    /// group to be empty, also when the attempt fails or unwinds.
    fn sample_after(&self, before_request: impl FnOnce(u32)) -> Sampled {
        let mut command = self.fixture.command_in(None, &["__consumption-sample"]);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut sampler = DirectSampler::spawn(command, Instant::now() + Duration::from_secs(2));
        let pid = sampler.pid();
        // It reads its request first, so it is live and has called no
        // driver yet.
        let observed =
            observe_runtime_process(&UnixCommandRunner, u64::from(pid), sampler.budget).unwrap();
        assert!(
            matches!(observed, ProcessObservation::Live(_)),
            "the sampler {pid} is not live: {observed:?}"
        );
        let gate = self.fixture.root.join("driver/gate");
        fs::write(gate.with_extension("next"), pid.to_string()).unwrap();
        fs::rename(gate.with_extension("next"), &gate).unwrap();
        before_request(pid);
        sampler.request(self.request.to_string().as_bytes());
        let (status, killed) = sampler.finish();
        fs::write(&gate, "").unwrap();
        Sampled {
            pid,
            status,
            killed,
        }
    }

    /// The run and its provider are still the processes it started.
    fn assert_run_live(&self) {
        let run = self.run.as_ref().unwrap();
        assert!(
            run.owned.iter().all(|owned| owned.seen() == Seen::Live),
            "the provider ended"
        );
        let observed =
            observe_runtime_process(&UnixCommandRunner, u64::from(self.run_pid()), deadline())
                .unwrap();
        assert!(
            matches!(observed, ProcessObservation::Live(_)),
            "the run ended: {observed:?}"
        );
    }

    /// Waits for an automatic tick that the gate refused, after `since`
    /// snapshot calls, and returns its sampler's pid.
    fn denied_tick(&self, since: usize) -> String {
        let mut found = None;
        let ticks = Instant::now() + Duration::from_secs(15);
        while found.is_none() {
            assert!(Instant::now() < ticks, "no tick: {:?}", self.snapshots());
            found = self.snapshots()[since..]
                .iter()
                .find(|line| line.contains("caller=sampler denied"))
                .cloned();
            std::thread::sleep(Duration::from_millis(20));
        }
        let line = found.unwrap();
        field(&line, "ppid").to_owned()
    }

    /// Lets the provider exit, reaps the run, stops the server, and checks
    /// that no fixture process is left.
    fn finish(mut self) {
        let run = self.run.take().unwrap();
        run.finish();
        self.server.stop();
        assert_fixture_processes_gone(&self.fixture, &self.snapshots());
    }
}

/// One `key=value` field of a snapshot log line.
fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split(' ')
        .find_map(|part| part.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("{key} in {line}"))
}

struct Sampled {
    pid: u32,
    status: ExitStatus,
    killed: bool,
}

impl Sampled {
    /// It failed: a nonzero exit, or the SIGKILL of its own group abort.
    fn assert_failed(&self) {
        assert!(!self.killed, "the sampler outlived its budget");
        assert!(
            !self.status.success()
                && (self.status.code().is_some() || self.status.signal() == Some(9)),
            "{:?}",
            self.status
        );
    }

    fn calls<'a>(&self, snapshots: &'a [String]) -> Vec<&'a String> {
        let pid = self.pid.to_string();
        snapshots
            .iter()
            .filter(|line| field(line, "ppid") == pid)
            .collect()
    }
}

/// No process whose command names this fixture is left (drivers and their
/// children do), and no sampler that called the driver is still a sampler.
/// The test process is the pane shell and the approval's driver parent, so
/// it is never counted, and nothing here signals anything. A `ps` that
/// fails is a failure, never an empty list.
fn assert_fixture_processes_gone(fixture: &Fixture, snapshots: &[String]) {
    let root = fixture.root.to_str().unwrap().to_owned();
    let samplers: Vec<String> = snapshots
        .iter()
        .filter(|line| line.contains("caller=sampler"))
        .map(|line| field(line, "ppid").to_owned())
        .collect();
    wait_until("no fixture process is left", || {
        let listed = std::process::Command::new("/bin/ps")
            .args(["-axo", "pid=,command="])
            .output()
            .unwrap();
        assert!(listed.status.success(), "{listed:?}");
        let listed = String::from_utf8(listed.stdout).unwrap();
        let mine = std::process::id().to_string();
        listed.lines().all(|line| {
            let line = line.trim_start();
            let (pid, command) = line.split_once(' ').unwrap_or((line, ""));
            pid == mine
                || !(command.contains(&root)
                    || (samplers.iter().any(|sampler| sampler == pid)
                        && command.contains("__consumption-sample")))
        })
    });
}

/// With its driver approved, the sampler reads the pane through it and
/// records the newly appended request before any Stop: the counts advance
/// by exactly that request, in storage and in `ls`. The gate answered only
/// this sampler, and a tick it refused afterwards changed nothing, so the
/// advance is this sampler's.
#[test]
fn an_approved_external_host_samples_new_consumption_before_stop() {
    let sampling = Sampling::start("sampling-advances");
    // The start and prompt hooks set the source at the first request.
    assert_eq!(sampling.consumption(), Some([0, 0, 0]));
    let sampled = sampling.sampled_at();
    sampling.append("msg_fixture0002");

    let calls = sampling.snapshots().len();
    let worker = sampling.sample();
    assert_eq!(sampling.consumption(), Some(AFTER_SECOND));
    assert!(worker.status.success(), "{:?}", worker.status);
    assert!(sampling.sampled_at() > sampled);
    let snapshots = sampling.snapshots();
    let mine = worker.calls(&snapshots[calls..]);
    assert_eq!(mine.len(), 1, "{snapshots:?}");
    assert!(
        mine[0].ends_with(r#"caller=sampler answered panes=["fake-1"]"#),
        "{mine:?}"
    );
    assert!(
        snapshots[calls..]
            .iter()
            .all(|line| !line.contains("caller=sampler answered") || mine.contains(&line)),
        "{snapshots:?}"
    );
    let row = worker_row(&sampling.fixture);
    let listed = &row["resume"]["consumption"];
    assert_eq!(
        ["inputTokens", "cachedInputTokens", "outputTokens"]
            .map(|field| listed[field].as_u64().unwrap()),
        AFTER_SECOND,
        "{row:#}"
    );
    assert_eq!(listed["complete"], true, "{row:#}");
    assert_eq!(row["session"]["activity"]["state"], "working", "{row:#}");
    assert!(!sampling.fixture.root.join("acked").exists());

    // A tick reaches the driver and is refused there, with new consumption
    // to sample: it changes no stored row.
    let baseline = stored_rows(&sampling.fixture);
    sampling.append("msg_fixture0003");
    sampling.denied_tick(sampling.snapshots().len());
    assert_eq!(stored_rows(&sampling.fixture), baseline);
    sampling.finish();
}

/// A failure after the sampler is spawned, before its normal end, still
/// stops and reaps it: the test process is still running, and the
/// sampler's pid names no process and its group is empty. The run and
/// provider continue, and no consumption is observed.
#[test]
fn a_direct_sampler_is_stopped_and_reaped_when_its_attempt_fails() {
    let sampling = Sampling::start("sampling-unwinds");
    let baseline = stored_rows(&sampling.fixture);
    sampling.append("msg_fixture0002");
    let spawned = std::cell::Cell::new(0);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sampling.sample_after(|pid| {
            spawned.set(pid);
            panic!("injected failure after the sampler started");
        })
    }));
    assert!(unwound.is_err());
    let pid = spawned.get();
    assert_ne!(pid, 0);
    // Reaped: not even a zombie answers for the pid, and its group is empty.
    let pid = nix::unistd::Pid::from_raw(i32::try_from(pid).unwrap());
    assert_eq!(
        nix::sys::signal::kill(pid, None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert_eq!(
        nix::sys::signal::killpg(pid, None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert_eq!(stored_rows(&sampling.fixture), baseline);
    sampling.assert_run_live();
    sampling.finish();
}

/// Without the gate, the run's own tick samples the external host too.
#[test]
fn the_runs_own_tick_samples_an_approved_external_host() {
    let sampling = Sampling::start("sampling-tick");
    let since = sampling.snapshots().len();
    fs::remove_file(sampling.fixture.root.join("driver/gate")).unwrap();
    sampling.append("msg_fixture0002");
    sampling.append("msg_fixture0003");
    let tick = Instant::now() + Duration::from_secs(15);
    while sampling.consumption() != Some(AFTER_THIRD) {
        assert!(
            Instant::now() < tick,
            "no tick sampled: {:?} {:?}",
            sampling.consumption(),
            sampling.snapshots()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        sampling.snapshots()[since..]
            .iter()
            .any(|line| line.contains(r#"caller=sampler answered panes=["fake-1"]"#)),
        "{:?}",
        sampling.snapshots()
    );
    sampling.finish();
}

/// Once its driver approval is removed, the sampler makes no consumption
/// observation and the run and provider continue.
#[test]
fn an_unapproved_external_host_makes_no_consumption_observation() {
    let sampling = Sampling::start("sampling-unapproved");
    let removed = sampling.fixture.tmt(&["driver", "rm", "fake"]);
    assert!(removed.status.success(), "{removed:?}");
    let drivers = |fixture: &Fixture| {
        let listed = fixture.json(&["driver", "ls", "--json"]);
        listed.to_string()
    };
    assert!(!drivers(&sampling.fixture).contains(r#""fake""#));
    let baseline = stored_rows(&sampling.fixture);
    sampling.append("msg_fixture0002");

    let calls = sampling.snapshots().len();
    let worker = sampling.sample();
    assert_eq!(stored_rows(&sampling.fixture), baseline);
    worker.assert_failed();
    assert!(worker.calls(&sampling.snapshots()[calls..]).is_empty());
    sampling.assert_run_live();
    assert!(!drivers(&sampling.fixture).contains(r#""fake""#));
    sampling.finish();
}

/// Once the recorded server ends, while the run, provider and pane stay
/// live and the driver still answers for the pane, the sampler makes no
/// consumption observation: it never samples against a missing or replaced
/// recorded server incarnation.
#[test]
fn a_missing_recorded_server_makes_no_consumption_observation() {
    let mut sampling = Sampling::start("sampling-server-gone");
    sampling.server.stop();
    let baseline = stored_rows(&sampling.fixture);
    sampling.append("msg_fixture0002");

    let calls = sampling.snapshots().len();
    let worker = sampling.sample();
    assert_eq!(stored_rows(&sampling.fixture), baseline);
    worker.assert_failed();
    sampling.assert_run_live();
    // The driver was reached and answered for the pane: the refusal is the
    // server observation's.
    let snapshots = sampling.snapshots();
    let mine = worker.calls(&snapshots[calls..]);
    assert_eq!(mine.len(), 1, "{snapshots:?}");
    assert!(
        mine[0].ends_with(r#"caller=sampler answered panes=["fake-1"]"#),
        "{mine:?}"
    );
    sampling.finish();
}

fn worker_row(fixture: &Fixture) -> Value {
    worker(&fixture.json(&["ls", "--json"])).clone()
}
