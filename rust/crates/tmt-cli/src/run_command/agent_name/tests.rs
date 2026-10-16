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
fn held(name: &str) -> BindingEntry {
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

#[derive(Debug, Clone, Copy)]
struct Failure {
    refusal: AgentNameRefusal,
    cleanup_failed: bool,
}

impl CallFailure for Failure {
    fn cleanup_failed(&self) -> bool {
        self.cleanup_failed
    }

    fn refusal(&self) -> AgentNameRefusal {
        self.refusal
    }
}

fn refused(refusal: AgentNameRefusal) -> Result<AgentNaming, Failure> {
    Err(Failure {
        refusal,
        cleanup_failed: false,
    })
}

type Step<T> = Box<dyn FnMut(&Calls) -> T + Send>;
type Alive = Box<dyn FnMut(&Calls, &ProcessIncarnation) -> bool + Send>;

/// The effects one test scripts, and the calls they received in order.
struct Fake {
    calls: Calls,
    entry: Step<Option<BindingEntry>>,
    alive: Alive,
    probe: Step<Result<Probe, Failure>>,
    set: Step<Result<AgentNaming, Failure>>,
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

#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<String>>>);

impl Calls {
    fn push(&self, call: impl Into<String>) {
        self.0.lock().unwrap().push(call.into());
    }

    fn list(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    fn sets(&self) -> Vec<String> {
        self.list()
            .into_iter()
            .filter(|call| call.starts_with("set "))
            .collect()
    }
}

impl Default for Fake {
    /// A held binding, both processes alive, an agent without a name, and a
    /// host that confirms the name.
    fn default() -> Self {
        Self {
            calls: Calls::default(),
            entry: Box::new(|_| Some(held("worker"))),
            alive: Box::new(|_, _| true),
            probe: Box::new(|_| Ok(Probe::Active(None))),
            set: Box::new(|_| Ok(AgentNaming::Done)),
            ended: Ended::default(),
        }
    }
}

impl Ports for Fake {
    type Error = Failure;

    fn entry(&mut self) -> Option<BindingEntry> {
        self.calls.push("entry");
        (self.entry)(&self.calls)
    }

    fn alive(&mut self, process: &ProcessIncarnation) -> bool {
        self.calls.push(format!("alive {}", process.pid()));
        (self.alive)(&self.calls, process)
    }

    fn probe(&mut self, _: &BindingEntry) -> Result<Probe, Failure> {
        self.calls.push("probe");
        (self.probe)(&self.calls)
    }

    fn name_agent(
        &mut self,
        binding: &Binding,
        identity: &Identity,
    ) -> Result<AgentNaming, Failure> {
        assert_eq!(binding.id, "binding");
        self.calls.push(format!("set {}", identity.name));
        (self.set)(&self.calls)
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

fn run(fake: &mut Fake) -> (Outcome, Vec<Duration>) {
    let clock = Clock::default();
    let started = Instant::now();
    let outcome = name_launched_agent(&launch(), fake, &clock, started);
    let waits = clock
        .waits
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|at| at - started)
        .collect();
    (outcome, waits)
}

/// Waits, with a bound, until `ended` is set.
fn wait_for(ended: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ended.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the thread ends");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The attempts follow the schedule and stop after the fifth, so a host
/// that never has an agent gets exactly five calls.
#[test]
fn a_host_without_an_agent_gets_five_attempts_on_schedule() {
    let mut fake = Fake {
        set: Box::new(|_| refused(AgentNameRefusal::NoAgent)),
        ..Fake::default()
    };
    let (outcome, waits) = run(&mut fake);
    assert_eq!(outcome, Outcome::NoAgent);
    assert_eq!(waits, ATTEMPTS);
    assert_eq!(fake.calls.sets(), vec!["set worker"; 5]);
}

/// Only "no agent yet" and "not ready" lead to another attempt; the first
/// attempt the host confirms is the last.
#[test]
fn only_no_agent_and_not_ready_try_again() {
    let mut answers = vec![
        refused(AgentNameRefusal::NotReady),
        refused(AgentNameRefusal::NoAgent),
        Ok(AgentNaming::Done),
    ]
    .into_iter();
    let mut fake = Fake {
        set: Box::new(move |_| answers.next().unwrap()),
        ..Fake::default()
    };
    assert_eq!(run(&mut fake).0, Outcome::Named);
    assert_eq!(fake.calls.sets().len(), 3);

    for (answer, outcome) in [
        (Ok(AgentNaming::NotLinked), Outcome::NotLinked),
        (
            refused(AgentNameRefusal::Refused),
            Outcome::Failed {
                cleanup_failed: false,
            },
        ),
        // A failed or timed-out call, or an output limit: the name may be set.
        (
            refused(AgentNameRefusal::Uncertain),
            Outcome::Failed {
                cleanup_failed: false,
            },
        ),
        (
            Err(Failure {
                refusal: AgentNameRefusal::Uncertain,
                cleanup_failed: true,
            }),
            Outcome::Failed {
                cleanup_failed: true,
            },
        ),
    ] {
        let mut fake = Fake {
            set: Box::new(move |_| answer),
            ..Fake::default()
        };
        assert_eq!(run(&mut fake).0, outcome);
        assert_eq!(fake.calls.sets().len(), 1, "{outcome:?}");
    }
    assert!(
        Outcome::Failed {
            cleanup_failed: true
        }
        .cleanup_failed()
    );
    assert!(!Outcome::Named.cleanup_failed());
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
        let mut fake = Fake {
            entry: Box::new(move |_| {
                let mut entry = held("worker");
                change(&mut entry);
                Some(entry)
            }),
            ..Fake::default()
        };
        assert_eq!(run(&mut fake).0, Outcome::Ended, "case {case}");
        assert_eq!(fake.calls.list(), ["entry"], "case {case}");
    }

    // Storage that can't be read is not a held binding.
    let mut fake = Fake {
        entry: Box::new(|_| None),
        ..Fake::default()
    };
    assert_eq!(run(&mut fake).0, Outcome::Ended);
    assert_eq!(fake.calls.list(), ["entry"]);

    // A dead owner or child, or one whose evidence is unknown.
    for gone in [100, 200] {
        let mut fake = Fake {
            alive: Box::new(move |_, process| process.pid() != gone),
            ..Fake::default()
        };
        assert_eq!(run(&mut fake).0, Outcome::Ended, "{gone}");
        assert!(fake.calls.sets().is_empty(), "{gone}");
        assert!(!fake.calls.list().contains(&"probe".to_owned()), "{gone}");
    }
}

/// An agent that already has a name keeps it, the identity's or another:
/// the automatic naming never replaces a name. A pane that is not this
/// binding's ends it, and a failed probe asks nothing more.
#[test]
fn an_agent_with_any_name_or_an_unbound_pane_is_left_alone() {
    for (probe, outcome) in [
        (Ok(Probe::Active(Some("worker".into()))), Outcome::Kept),
        (Ok(Probe::Active(Some("ann".into()))), Outcome::Kept),
        (Ok(Probe::NotActive), Outcome::Ended),
        (
            Err(Failure {
                refusal: AgentNameRefusal::Uncertain,
                cleanup_failed: true,
            }),
            Outcome::Failed {
                cleanup_failed: true,
            },
        ),
    ] {
        let mut fake = Fake {
            probe: Box::new(move |_| probe.clone()),
            ..Fake::default()
        };
        assert_eq!(run(&mut fake).0, outcome);
        assert!(fake.calls.sets().is_empty(), "{outcome:?}");
    }
}

/// A stop observed after any blocking step, or right before the set,
/// starts no set.
#[test]
fn a_stop_after_any_step_starts_no_set() {
    for step in ["entry", "alive 100", "alive 200", "probe"] {
        let clock = Clock::default();
        let stop = clock.stop.clone();
        let raise = move |calls: &Calls| {
            if calls.list().last().map(String::as_str) == Some(step) {
                stop.store(true, Ordering::SeqCst);
            }
        };
        let (on_entry, on_alive, on_probe) = (raise.clone(), raise.clone(), raise);
        let mut fake = Fake {
            entry: Box::new(move |calls| {
                on_entry(calls);
                Some(held("worker"))
            }),
            alive: Box::new(move |calls, _| {
                on_alive(calls);
                true
            }),
            probe: Box::new(move |calls| {
                on_probe(calls);
                Ok(Probe::Active(None))
            }),
            ..Fake::default()
        };
        let outcome = name_launched_agent(&launch(), &mut fake, &clock, Instant::now());
        assert_eq!(outcome, Outcome::Stopped, "{step}");
        assert!(fake.calls.sets().is_empty(), "{step}");
        assert_eq!(fake.calls.list().last().unwrap(), step);
    }

    // A stop before the first attempt asks nothing at all.
    let clock = Clock::default();
    clock.stop.store(true, Ordering::SeqCst);
    let mut fake = Fake::default();
    let outcome = name_launched_agent(&launch(), &mut fake, &clock, Instant::now());
    assert_eq!(outcome, Outcome::Stopped);
    assert!(fake.calls.list().is_empty());
}

/// A second `tmt run` takes the binding while the first run's set is in
/// flight: the first run's next attempt sees another owner and child, and
/// asks the host nothing more.
#[test]
fn a_second_run_ends_the_first_runs_naming() {
    let taken = Arc::new(AtomicBool::new(false));
    let read = taken.clone();
    let mut fake = Fake {
        entry: Box::new(move |_| {
            let mut entry = held("worker");
            if read.load(Ordering::SeqCst) {
                let session = &mut entry.binding.as_mut().unwrap().session;
                session.launch_owner = Some(incarnation(300));
                session.key.as_mut().unwrap().incarnation = incarnation(400);
            }
            Some(entry)
        }),
        set: Box::new(move |_| {
            // The second run commits while this call is in flight.
            taken.store(true, Ordering::SeqCst);
            refused(AgentNameRefusal::NotReady)
        }),
        ..Fake::default()
    };
    assert_eq!(run(&mut fake).0, Outcome::Ended);
    assert_eq!(fake.calls.sets(), ["set worker"]);
    assert_eq!(fake.calls.list().last().unwrap(), "entry");
}

/// An unbind while the set is in flight: the next attempt sees no binding
/// and stops. The thread never takes a name away, so the name the set may
/// have given stays: the documented best-effort result.
#[test]
fn an_unbind_during_a_set_ends_the_naming_without_a_clear() {
    let unbound = Arc::new(AtomicBool::new(false));
    let read = unbound.clone();
    let mut fake = Fake {
        entry: Box::new(move |_| {
            let mut entry = held("worker");
            if read.load(Ordering::SeqCst) {
                entry.binding = None;
            }
            Some(entry)
        }),
        set: Box::new(move |_| {
            unbound.store(true, Ordering::SeqCst);
            refused(AgentNameRefusal::NoAgent)
        }),
        ..Fake::default()
    };
    assert_eq!(run(&mut fake).0, Outcome::Ended);
    assert_eq!(fake.calls.sets(), ["set worker"]);
}

/// A `tmt mv` while the set is in flight: the set carries the name read
/// before it and is the only one; there is no corrective call. The rename's
/// own call decides the final name (best effort, documented).
#[test]
fn a_rename_during_a_set_gets_no_corrective_call() {
    let renamed = Arc::new(AtomicBool::new(false));
    let read = renamed.clone();
    let mut fake = Fake {
        entry: Box::new(move |_| {
            Some(held(if read.load(Ordering::SeqCst) {
                "other"
            } else {
                "worker"
            }))
        }),
        set: Box::new(move |_| {
            renamed.store(true, Ordering::SeqCst);
            Ok(AgentNaming::Done)
        }),
        ..Fake::default()
    };
    assert_eq!(run(&mut fake).0, Outcome::Named);
    assert_eq!(fake.calls.sets(), ["set worker"]);
}

fn spawn(work: Work) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new().spawn(work)
}

/// `finish` stops a waiting thread at once and returns the outcome of a
/// thread that ended.
#[test]
fn finish_stops_a_waiting_thread_and_returns_its_outcome() {
    let fake = Fake::default();
    let ended = fake.ended.0.clone();
    let naming = Naming::start_with(launch(), fake, spawn).expect("a thread");
    assert_eq!(naming.finish(FINISH_WITHIN), Some(Outcome::Stopped));
    assert!(ended.load(Ordering::SeqCst));
}

/// Dropping the handle, as every early return of `tmt run` does, stops the
/// thread too.
#[test]
fn dropping_the_handle_stops_the_thread() {
    let fake = Fake::default();
    let ended = fake.ended.0.clone();
    drop(Naming::start_with(launch(), fake, spawn).expect("a thread"));
    wait_for(&ended);
}

/// A thread blocked in a step past the bound is left to finish on its own:
/// `finish` returns without an outcome, so a cleanup failure it might
/// report later is lost.
#[test]
fn a_thread_blocked_past_the_bound_is_detached() {
    let (entered, inside) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let fake = Fake {
        entry: Box::new(move |_| {
            entered.send(()).unwrap();
            // A storage read that does not return until the test says so.
            let _ = released.lock().unwrap().recv();
            Some(held("worker"))
        }),
        set: Box::new(|_| {
            Err(Failure {
                refusal: AgentNameRefusal::Uncertain,
                cleanup_failed: true,
            })
        }),
        ..Fake::default()
    };
    let ended = fake.ended.0.clone();
    let calls = fake.calls.clone();
    let naming = Naming::start_with(launch(), fake, spawn).expect("a thread");
    inside
        .recv_timeout(Duration::from_secs(10))
        .expect("the first attempt starts");
    assert_eq!(naming.finish(Duration::from_millis(50)), None);
    assert!(!ended.load(Ordering::SeqCst), "the thread still runs");
    // Test-owned cleanup: release the read; the stop it then observes ends
    // the thread before any set.
    release.send(()).unwrap();
    wait_for(&ended);
    assert!(calls.sets().is_empty());
}

/// A panic in the thread gives no outcome and nothing to report.
#[test]
fn a_panicking_thread_gives_no_outcome() {
    let (entered, inside) = mpsc::channel();
    let fake = Fake {
        entry: Box::new(move |_| {
            entered.send(()).unwrap();
            panic!("a broken read")
        }),
        ..Fake::default()
    };
    let ended = fake.ended.0.clone();
    let naming = Naming::start_with(launch(), fake, spawn).expect("a thread");
    inside
        .recv_timeout(Duration::from_secs(10))
        .expect("the first attempt starts");
    assert_eq!(naming.finish(FINISH_WITHIN), None);
    assert!(
        ended.load(Ordering::SeqCst),
        "the unwind dropped the effects"
    );
}

/// No thread, no naming, no output: the launch goes on as before.
#[test]
fn a_thread_that_cannot_start_is_silent() {
    let fake = Fake::default();
    let ended = fake.ended.0.clone();
    let naming = Naming::start_with(launch(), fake, |_| Err(io::Error::other("no threads")));
    assert!(naming.is_none());
    assert!(ended.load(Ordering::SeqCst), "the work was dropped unrun");
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
