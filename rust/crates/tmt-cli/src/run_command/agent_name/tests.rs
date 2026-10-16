use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tmt_core::{
    binding::session::{BindingSessionState, ObservedSessionKey},
    endpoint::ServerEvidence,
    identity::Lifetime,
};

fn incarnation(pid: u64) -> ProcessIncarnation {
    ProcessIncarnation::new(pid, &format!("start-{pid}")).unwrap()
}

fn launch() -> Launch {
    Launch {
        database: PathBuf::from("/nonexistent/tmt.db"),
        identity_id: "identity".into(),
        binding_id: "binding".into(),
        owner: incarnation(100),
        child: incarnation(200),
    }
}

/// The entry this launch holds: its binding, its owner and child, running.
fn held_by_launch(name: &str) -> BindingEntry {
    BindingEntry {
        identity: Identity {
            id: "identity".into(),
            name: name.into(),
            canonical_name: name.to_lowercase(),
            lifetime: Lifetime::Saved,
            created_at: String::new(),
            updated_at: String::new(),
        },
        binding: Some(Binding {
            id: "binding".into(),
            identity_id: "identity".into(),
            server: ServerEvidence {
                host: tmt_core::host::HostKind::Tmux,
                server_id: "server".into(),
                socket_path: "/tmp/host".into(),
                server_pid: 1,
                server_start_time: "0".into(),
            },
            pane_id: "%1".into(),
            pane_pid: 10,
            pane_incarnation: None,
            session: BindingSessionState {
                last_transition: None,
                state: RuntimeState::Running,
                key: Some(ObservedSessionKey {
                    incarnation: incarnation(200),
                    provider_session: None,
                }),
                launch_owner: Some(incarnation(100)),
            },
        }),
    }
}

/// What the store holds now. Tests change it in the middle of a step to
/// stand for a concurrent command.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stored {
    Held,
    Renamed,
    SecondRun,
    Unbound,
}

impl Stored {
    fn entry(self) -> BindingEntry {
        let mut entry = held_by_launch(if self == Self::Renamed {
            "other"
        } else {
            "worker"
        });
        match self {
            Self::Held | Self::Renamed => {}
            Self::SecondRun => {
                let session = &mut entry.binding.as_mut().unwrap().session;
                session.launch_owner = Some(incarnation(300));
                session.key.as_mut().unwrap().incarnation = incarnation(400);
            }
            Self::Unbound => entry.binding = None,
        }
        entry
    }
}

fn error(refusal: AgentNameRefusal, cleanup_failed: bool) -> CallError {
    CallError {
        refusal,
        cleanup_failed,
    }
}

/// The calls the fake received, shared with a thread that owns the fake.
#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<String>>>);

impl Calls {
    fn list(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    fn sets(&self) -> Vec<String> {
        let mut sets = self.list();
        sets.retain(|call| call.starts_with("set "));
        sets
    }
}

type Hook = Box<dyn FnMut(&str) + Send>;

/// Scripted effects. `stored` is what `entry` reads; `hook` runs inside
/// each call, by name, before it answers, so a test can change the store,
/// raise a stop or block there.
struct Fake {
    calls: Calls,
    stored: Arc<Mutex<Stored>>,
    dead: Option<u64>,
    alive_error: Option<CallError>,
    probe: Result<Probe, CallError>,
    answers: Vec<Result<AgentNaming, CallError>>,
    hook: Hook,
    ended: Ended,
}

/// Set when the fake is dropped, which is when the thread that owns it ends.
#[derive(Default)]
struct Ended(Arc<AtomicBool>);

impl Drop for Ended {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Default for Fake {
    /// A held binding, both processes alive, an agent without a name, and a
    /// host that confirms the name.
    fn default() -> Self {
        Self {
            calls: Calls::default(),
            stored: Arc::new(Mutex::new(Stored::Held)),
            dead: None,
            alive_error: None,
            probe: Ok(Probe::Active(None)),
            answers: Vec::new(),
            hook: Box::new(|_| {}),
            ended: Ended::default(),
        }
    }
}

impl Fake {
    fn call(&mut self, call: String) {
        self.calls.0.lock().unwrap().push(call.clone());
        (self.hook)(&call);
    }
}

impl Ports for Fake {
    fn entry(&mut self) -> Option<BindingEntry> {
        self.call("entry".into());
        Some(self.stored.lock().unwrap().entry())
    }

    fn alive(&mut self, process: &ProcessIncarnation) -> Result<bool, CallError> {
        self.call(format!("alive {}", process.pid()));
        match self.alive_error {
            Some(error) => Err(error),
            None => Ok(self.dead != Some(process.pid())),
        }
    }

    fn probe(&mut self, _: &BindingEntry) -> Result<Probe, CallError> {
        self.call("probe".into());
        self.probe.clone()
    }

    fn name_agent(
        &mut self,
        binding: &Binding,
        identity: &Identity,
    ) -> Result<AgentNaming, CallError> {
        assert_eq!(binding.id, "binding");
        self.call(format!("set {}", identity.name));
        if self.answers.is_empty() {
            Ok(AgentNaming::Done)
        } else {
            self.answers.remove(0)
        }
    }
}

/// A clock that never sleeps: it records each wait and stops when asked.
#[derive(Default)]
struct Clock {
    waits: Mutex<Vec<Instant>>,
    stop: Arc<AtomicBool>,
}

impl Pacing for Clock {
    fn until(&self, at: Instant) -> bool {
        self.waits.lock().unwrap().push(at);
        !self.stopped()
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// Runs the attempts on `fake` with a clock that never sleeps.
fn run(fake: &mut Fake, clock: &Clock) -> Outcome {
    name_launched_agent(&launch(), fake, clock, Instant::now())
}

/// One full attempt that ends in a set.
const ATTEMPT: [&str; 8] = [
    "entry",
    "alive 100",
    "alive 200",
    "probe",
    "entry",
    "alive 100",
    "alive 200",
    "set worker",
];

/// The attempts follow the schedule and stop after the fifth, so a host
/// that never has an agent gets exactly five calls.
#[test]
fn a_host_without_an_agent_gets_five_attempts_on_schedule() {
    let mut fake = Fake {
        answers: vec![Err(error(AgentNameRefusal::NoAgent, false)); 5],
        ..Fake::default()
    };
    let clock = Clock::default();
    let started = Instant::now();
    let outcome = name_launched_agent(&launch(), &mut fake, &clock, started);
    assert!(!outcome.cleanup_failed);
    let waits: Vec<_> = clock
        .waits
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|at| at - started)
        .collect();
    assert_eq!(waits, ATTEMPTS);
    assert_eq!(fake.calls.list(), ATTEMPT.repeat(5));
}

/// Only "no agent yet" and "not ready" lead to another attempt; any other
/// answer is the last. Only a cleanup failure is reported.
#[test]
fn only_no_agent_and_not_ready_try_again() {
    use AgentNameRefusal::*;
    for (answers, sets, cleanup_failed) in [
        (
            vec![
                Err(error(NotReady, false)),
                Err(error(NoAgent, false)),
                Ok(AgentNaming::Done),
            ],
            3,
            false,
        ),
        (vec![Ok(AgentNaming::NotLinked)], 1, false),
        (vec![Err(error(Refused, false))], 1, false),
        // A failed or timed-out call, or an output limit: the name may be set.
        (vec![Err(error(Uncertain, false))], 1, false),
        (vec![Err(error(Uncertain, true))], 1, true),
        // A cleanup failure ends the attempts whatever the host said.
        (vec![Err(error(NoAgent, true))], 1, true),
    ] {
        let mut fake = Fake {
            answers,
            ..Fake::default()
        };
        let outcome = run(&mut fake, &Clock::default());
        assert_eq!(outcome.cleanup_failed, cleanup_failed, "{sets}");
        assert_eq!(fake.calls.sets().len(), sets, "{cleanup_failed}");
    }
}

/// Each attempt checks that this exact launch still holds the binding and
/// that both of its processes are alive; any difference ends the naming
/// before the host is asked anything.
#[test]
fn a_launch_that_no_longer_holds_its_binding_is_never_named() {
    let changed: [fn(&mut BindingEntry); 6] = [
        |entry| entry.binding = None,
        |entry| entry.binding.as_mut().unwrap().id = "other".into(),
        |entry| entry.binding.as_mut().unwrap().session.launch_owner = Some(incarnation(101)),
        |entry| {
            entry.binding.as_mut().unwrap().session.key = Some(ObservedSessionKey {
                incarnation: incarnation(201),
                provider_session: None,
            });
        },
        |entry| entry.binding.as_mut().unwrap().session.key = None,
        |entry| entry.binding.as_mut().unwrap().session.state = RuntimeState::Ended,
    ];
    for (case, change) in changed.into_iter().enumerate() {
        let mut entry = held_by_launch("worker");
        change(&mut entry);
        assert!(!launch().holds(&entry), "case {case}");
    }
    assert!(launch().holds(&held_by_launch("worker")));

    // The store as the attempts read it, and a dead or unknown process.
    for (stored, dead, calls) in [
        (Stored::SecondRun, None, &ATTEMPT[..1]),
        (Stored::Unbound, None, &ATTEMPT[..1]),
        (Stored::Held, Some(100), &ATTEMPT[..2]),
        (Stored::Held, Some(200), &ATTEMPT[..3]),
    ] {
        let mut fake = Fake {
            stored: Arc::new(Mutex::new(stored)),
            dead,
            ..Fake::default()
        };
        assert!(!run(&mut fake, &Clock::default()).cleanup_failed);
        assert_eq!(fake.calls.list(), calls, "{stored:?} {dead:?}");
    }
}

/// An agent that already has a name keeps it, the identity's or another:
/// the automatic naming never replaces a name. A pane that is not this
/// binding's ends it. Failed process or host evidence asks nothing more.
#[test]
fn a_named_agent_an_unbound_pane_or_failed_evidence_ends_the_naming() {
    for (probe, cleanup_failed) in [
        (Ok(Probe::Active(Some("worker".into()))), false),
        (Ok(Probe::Active(Some("ann".into()))), false),
        (Ok(Probe::NotActive), false),
        (Err(error(AgentNameRefusal::Uncertain, false)), false),
        (Err(error(AgentNameRefusal::Uncertain, true)), true),
    ] {
        let mut fake = Fake {
            probe: probe.clone(),
            ..Fake::default()
        };
        let outcome = run(&mut fake, &Clock::default());
        assert_eq!(outcome.cleanup_failed, cleanup_failed, "{probe:?}");
        assert_eq!(fake.calls.list(), &ATTEMPT[..4], "{probe:?}");
    }
    let mut fake = Fake {
        alive_error: Some(error(AgentNameRefusal::Uncertain, true)),
        ..Fake::default()
    };
    assert!(run(&mut fake, &Clock::default()).cleanup_failed);
    assert_eq!(fake.calls.list(), &ATTEMPT[..2]);
}

/// A stop observed after any step starts no further step and no set. A
/// cleanup failure of the probe that ran when the stop came is still
/// reported.
#[test]
fn a_stop_after_any_step_starts_no_further_step() {
    let mut cases: Vec<(usize, bool)> = (0..7).map(|at| (at, false)).collect();
    cases.push((3, true));
    for (at, probe_cleanup_failed) in cases {
        let clock = Clock::default();
        let stop = clock.stop.clone();
        let mut fake = Fake::default();
        if probe_cleanup_failed {
            fake.probe = Err(error(AgentNameRefusal::Uncertain, true));
        }
        let seen = fake.calls.clone();
        fake.hook = Box::new(move |_| {
            if seen.list().len() == at + 1 {
                stop.store(true, Ordering::SeqCst);
            }
        });
        let outcome = run(&mut fake, &clock);
        assert_eq!(fake.calls.list(), &ATTEMPT[..=at], "{at}");
        assert_eq!(outcome.cleanup_failed, probe_cleanup_failed, "{at}");
    }

    // A stop before the first attempt asks nothing at all.
    let clock = Clock::default();
    clock.stop.store(true, Ordering::SeqCst);
    let mut fake = Fake::default();
    assert!(!run(&mut fake, &clock).cleanup_failed);
    assert!(fake.calls.list().is_empty());
}

/// A concurrent command that commits while the probe or the set runs. The
/// launch key is read again after the probe, so a second run or an unbind
/// during the probe gets no set, and a rename during the probe sets the
/// name stored now. A change during a set is read at the next attempt: a
/// second run or an unbind ends the naming without a clear, and a rename is
/// carried by the next set; there is no corrective call.
#[test]
fn a_command_that_commits_during_a_probe_or_a_set_is_seen_at_the_next_read() {
    for (during, change, sets) in [
        ("probe", Stored::SecondRun, vec![]),
        ("probe", Stored::Unbound, vec![]),
        // Still held: each "not ready" set carries the name stored now.
        ("probe", Stored::Renamed, vec!["set other"; 5]),
        ("set worker", Stored::SecondRun, vec!["set worker"]),
        ("set worker", Stored::Unbound, vec!["set worker"]),
        // Still held: the later attempts carry the new name.
        (
            "set worker",
            Stored::Renamed,
            vec![
                "set worker",
                "set other",
                "set other",
                "set other",
                "set other",
            ],
        ),
    ] {
        let mut fake = Fake {
            // "Not ready", so only a change in the store ends the attempts.
            answers: vec![Err(error(AgentNameRefusal::NotReady, false)); 5],
            ..Fake::default()
        };
        let stored = fake.stored.clone();
        let mut done = false;
        fake.hook = Box::new(move |call| {
            if call == during && !done {
                *stored.lock().unwrap() = change;
                done = true;
            }
        });
        let outcome = run(&mut fake, &Clock::default());
        assert!(!outcome.cleanup_failed);
        assert_eq!(fake.calls.sets(), sets, "{during} {change:?}");
    }
}

fn spawn(work: Work) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new().spawn(work)
}

/// A probe that blocks in the thread until the test releases it, then
/// answers `probe`, or panics when `probe` is `None`.
fn blocked_probe(
    probe: Option<Result<Probe, CallError>>,
) -> (Fake, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered, inside) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let fake = Fake {
        probe: probe.clone().unwrap_or(Ok(Probe::Active(None))),
        hook: Box::new(move |call| {
            if call == "probe" {
                entered.send(()).unwrap();
                let _ = released.lock().unwrap().recv();
                assert!(probe.is_some(), "a broken probe");
            }
        }),
        ..Fake::default()
    };
    (fake, inside, release)
}

/// Waits, with a bound, until `ended` is set.
fn wait_for(ended: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ended.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the thread ends");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `finish` stops the thread and returns the outcome only of a thread that
/// ended: a waiting thread stops at once; a stop that comes during a probe
/// that then fails to clean up still reports it; a panic gives none; a
/// thread past the bound is detached without an outcome.
#[test]
fn finish_returns_the_outcome_of_an_ended_thread_only() {
    let fake = Fake::default();
    let ended = fake.ended.0.clone();
    let naming = Naming::start_with(launch(), fake, spawn).expect("a thread");
    assert_eq!(naming.finish(FINISH_WITHIN), Some(ENDED));
    assert!(ended.load(Ordering::SeqCst));

    for (probe, finished) in [
        (
            Some(Err(error(AgentNameRefusal::Uncertain, true))),
            Some(Outcome {
                cleanup_failed: true,
            }),
        ),
        (None, None),
    ] {
        let (fake, inside, release) = blocked_probe(probe);
        let ended = fake.ended.0.clone();
        let calls = fake.calls.clone();
        let naming = Naming::start_with(launch(), fake, spawn).expect("a thread");
        inside
            .recv_timeout(Duration::from_secs(10))
            .expect("the probe runs");
        naming.request_stop();
        release.send(()).unwrap();
        assert_eq!(naming.finish(FINISH_WITHIN), finished);
        assert!(ended.load(Ordering::SeqCst));
        assert!(calls.sets().is_empty());
    }

    let (fake, inside, release) = blocked_probe(Some(Ok(Probe::Active(None))));
    let ended = fake.ended.0.clone();
    let calls = fake.calls.clone();
    let naming = Naming::start_with(launch(), fake, spawn).expect("a thread");
    inside
        .recv_timeout(Duration::from_secs(10))
        .expect("the probe runs");
    assert_eq!(naming.finish(Duration::from_millis(50)), None);
    assert!(!ended.load(Ordering::SeqCst), "the thread still runs");
    release.send(()).unwrap();
    wait_for(&ended);
    assert!(
        calls.sets().is_empty(),
        "the stop it then saw starts no set"
    );
}

/// Dropping the handle, as every early return of `tmt run` does, stops the
/// thread; a thread that could not start is silent and its work never runs.
#[test]
fn dropping_the_handle_stops_the_thread_and_a_failed_start_is_silent() {
    let fake = Fake::default();
    let ended = fake.ended.0.clone();
    drop(Naming::start_with(launch(), fake, spawn).expect("a thread"));
    wait_for(&ended);

    let fake = Fake::default();
    let ended = fake.ended.0.clone();
    let calls = fake.calls.clone();
    let naming = Naming::start_with(launch(), fake, |_| Err(io::Error::other("no threads")));
    assert!(naming.is_none());
    assert!(ended.load(Ordering::SeqCst), "the work was dropped unrun");
    assert!(calls.list().is_empty());
}

/// The real pacing returns once the due time has passed, and a stop request
/// ends a wait at once.
#[test]
fn the_stop_signal_ends_a_wait() {
    let signal = Arc::new(StopSignal::default());
    assert!(signal.until(Instant::now()));
    assert!(!signal.stopped());
    let waiter = signal.clone();
    let waiting =
        std::thread::spawn(move || waiter.until(Instant::now() + Duration::from_secs(60)));
    signal.request();
    assert!(!waiting.join().unwrap());
    assert!(signal.stopped());
}

/// While the naming thread is blocked in a call that only the test can
/// release, the wait loop of `tmt run` still forwards TERM and HUP to its
/// child: the child ends by the signal while the thread is still blocked.
/// A fixture process runs the same composition as `run_bound`, so the
/// signals never reach this test process, and no production deadline is
/// involved.
#[test]
fn a_blocked_naming_thread_never_delays_signal_forwarding() {
    use tmt_adapters::process::{CommandRequest, CommandRunner, UnixCommandRunner};
    const SCRIPT: &str = r#"
set -eu
ulimit -c 0
export TMT_NAMING_FIXTURE_DIR="$2"
"$1" --exact run_command::agent_name::tests::naming_signal_fixture --nocapture --test-threads=1 &
wrapper=$!
while [ ! -s "$2/ready" ]; do
  kill -0 "$wrapper"
  sleep 0.01
done
kill -"$3" "$wrapper"
wait "$wrapper"
test "$(cat "$2/result")" = "$4"
"#;
    for (signal, expected) in [("TERM", "143 blocked"), ("HUP", "129 blocked")] {
        let directory =
            std::env::temp_dir().join(format!("tmt-naming-signal-{}-{signal}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let args = [
            "-c".into(),
            SCRIPT.into(),
            "naming-signal-fixture".into(),
            std::env::current_exe().unwrap().into_os_string(),
            directory.clone().into_os_string(),
            signal.into(),
            expected.into(),
        ];
        // The bounded runner owns the fixture's whole group, so a failure
        // leaves neither the fixture nor its child behind.
        let result = UnixCommandRunner.execute(CommandRequest {
            program: std::ffi::OsStr::new("/bin/sh"),
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(20),
            max_output_bytes: 65_536,
        });
        let _ = std::fs::remove_dir_all(&directory);
        result.unwrap_or_else(|error| panic!("{signal} fixture failed: {error:?}"));
    }
}

/// The fixture process: an interactive child, a naming thread blocked in
/// its probe, and the wait that forwards signals. It records the child's
/// status and whether the thread was still blocked when the wait returned.
#[test]
fn naming_signal_fixture() {
    let Some(directory) = std::env::var_os("TMT_NAMING_FIXTURE_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let child = tmt_adapters::process::interactive::InteractiveChild::start_with(
        std::ffi::OsStr::new("/bin/sleep"),
        &["60".into()],
        &[],
    )
    .unwrap();
    let (fake, inside, release) = blocked_probe(Some(Ok(Probe::Active(None))));
    let calls = fake.calls.clone();
    let naming = Naming::start_with(launch(), fake, spawn).expect("a thread");
    inside
        .recv_timeout(Duration::from_secs(10))
        .expect("the probe runs");
    std::fs::write(directory.join("ready.tmp"), "ready\n").unwrap();
    std::fs::rename(directory.join("ready.tmp"), directory.join("ready")).unwrap();
    let code = child
        .wait(|cause| panic!("unexpected degradation: {cause}"))
        .unwrap();
    // Nothing released the probe yet: its call is the last one.
    let blocked = calls.list().last().map(String::as_str) == Some("probe");
    std::fs::write(
        directory.join("result"),
        format!("{code} {}", if blocked { "blocked" } else { "released" }),
    )
    .unwrap();
    release.send(()).unwrap();
    assert!(naming.finish(FINISH_WITHIN).is_some());
}
