//! The service over real Unix sockets: a fixture extension listens on its owner-only
//! `door.sock` under an isolated data root and answers the handshake with the leaf's
//! acceptor, so the offered `Host` and mount values, the one-active-one-candidate rule,
//! the installation bound, stop and the absence of leaked sockets are all observed from
//! the extension's side. Nothing infers an effect from a sleep.
use super::dispatch::Pause;
use super::*;
use crate::{
    limits,
    mount::{EXTENSIONS, NoSessions},
    objects::system_clock,
    state::Layout,
};
use std::{
    fs,
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};
use tmt_extension_objects::{
    Admission, Admit, AdmitInput, BeginInput, Bytes32, Call, Checkpoint, Chunk, Config,
    ConfigInput, Context, Counter, Decision, Disclosure, ErrorCode, Expect, Frame, Limits, Method,
    Origin, Outcome, Policy, Projection, ReadInput, Reason, Request, ResultFrame, Sha256Hex,
    StatusInput, Success, TransferBounds, TransferInput, accept,
};

const ORIGIN: &str = "http://127.0.0.1:4100";
const HOST: &str = "127.0.0.1:4100";
const PREFIX: &str = "/r/abcdefghijklmnop";
static NEVER: AtomicBool = AtomicBool::new(false);

const fn declared(name: &'static str, objects: ObjectDeclaration) -> Extension {
    Extension {
        name,
        body_bytes: 64 * 1024,
        reply_bytes: 1024,
        tunnels: 4,
        tunnel_idle: Duration::from_secs(5),
        objects,
    }
}
static ALPHA: [Extension; 1] = [declared("alpha", ObjectDeclaration::Local)];
static MIXED: [Extension; 2] = [
    declared("alpha", ObjectDeclaration::Local),
    declared("beta", ObjectDeclaration::Disabled),
];
static TRIO: [Extension; 3] = [
    declared("alpha", ObjectDeclaration::Local),
    declared("beta", ObjectDeclaration::Local),
    declared("gamma", ObjectDeclaration::Local),
];

fn io() -> IoBudget<'static> {
    IoBudget {
        deadline: Instant::now() + Duration::from_secs(60),
        cancelled: &NEVER,
    }
}
fn bounds() -> ServiceBounds {
    ServiceBounds {
        setup: Duration::from_secs(5),
        ..ServiceBounds::contract()
    }
}
fn mount_of(name: &str) -> String {
    format!("{PREFIX}/x/{name}/")
}

struct Env {
    root: PathBuf,
    serving: Serving,
}
impl Env {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = PathBuf::from(format!(
            "/tmp/t1894-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        Self { root, serving }
    }
    fn mounts(&self, extensions: &'static [Extension]) -> Mounts {
        Mounts::with_extensions(
            self.root.clone(),
            ORIGIN,
            PREFIX,
            Arc::new(NoSessions),
            extensions,
        )
    }
    fn service(
        &self,
        extensions: &'static [Extension],
        bounds: ServiceBounds,
    ) -> Option<ObjectService<'_>> {
        self.service_with(extensions, bounds, Origins::default())
    }
    fn service_with(
        &self,
        extensions: &'static [Extension],
        bounds: ServiceBounds,
        origins: Origins,
    ) -> Option<ObjectService<'_>> {
        ObjectService::open(
            &self.serving,
            extensions,
            Quotas::contract(),
            system_clock(),
            bounds,
            origins,
            &io(),
        )
        .unwrap()
    }
    fn ledger(&self) -> PathBuf {
        self.root.join("remote").join("objects.db")
    }
    fn directory(&self, name: &str) -> PathBuf {
        let directory = self.root.join(name);
        if !directory.exists() {
            fs::create_dir(&directory).unwrap();
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        directory
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// How the fixture extension answers a connection.
#[derive(Clone, Copy)]
enum Answer {
    /// Accept the handshake an offer for `mount` must satisfy.
    Expect,
    /// Hold the connection until released, then accept.
    Gated,
    /// Expect another mount, so the offer is refused.
    Refuse,
}

/// A fixture extension: its listener, the buses it accepted and what it saw.
struct Peer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    accepted: Arc<AtomicUsize>,
    buses: Arc<Mutex<Vec<Bus>>>,
    arrived: mpsc::Receiver<()>,
    release: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl Peer {
    fn start(env: &Env, name: &str, answer: Answer) -> Self {
        let path = env.directory(name).join("door.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicUsize::new(0));
        let buses = Arc::new(Mutex::new(Vec::new()));
        let (arrive, arrived) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let mount = match answer {
            Answer::Refuse => mount_of("other"),
            _ => mount_of(name),
        };
        let expect = Expect {
            host: HOST.into(),
            mount,
        };
        let (flag, count, held) = (Arc::clone(&stop), Arc::clone(&accepted), Arc::clone(&buses));
        let thread = thread::spawn(move || {
            while let Ok((stream, _)) = listener.accept() {
                if flag.load(Ordering::Acquire) {
                    break;
                }
                count.fetch_add(1, Ordering::AcqRel);
                let _ = arrive.send(());
                if matches!(answer, Answer::Gated) {
                    // A release, or the end of the test, lets the handshake go on.
                    let _ = released.recv();
                }
                let Ok(link) = accept(
                    stream,
                    &expect,
                    &Budgets::contract(),
                    Instant::now() + Duration::from_secs(5),
                ) else {
                    continue;
                };
                let bus = Bus::start(link, Budgets::contract(), Caps::contract(), None).unwrap();
                held.lock().unwrap().push(bus);
            }
        });
        Self {
            path,
            stop,
            accepted,
            buses,
            arrived,
            release: Some(release),
            thread: Some(thread),
        }
    }
    /// Block until a connection reached the listener.
    fn connection(&self) {
        self.arrived
            .recv_timeout(Duration::from_secs(10))
            .expect("no connection arrived");
    }
    fn release(&self) {
        self.release.as_ref().unwrap().send(()).unwrap();
    }
    fn connections(&self) -> usize {
        self.accepted.load(Ordering::Acquire)
    }
    /// Wait until at least `count` buses completed their handshake on this side: the
    /// service's `activate` returns when it read the reply, a moment before the fixture
    /// has its own bus.
    fn established(&self, count: usize) -> usize {
        let started = Instant::now();
        loop {
            let have = self.buses.lock().unwrap().len();
            if have >= count {
                return have;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "bus never established"
            );
            thread::yield_now();
        }
    }
    /// Whether the `index`th accepted bus saw its socket close.
    fn saw_close(&self, index: usize) -> bool {
        self.established(index + 1);
        let buses = self.buses.lock().unwrap();
        buses[index].recv(Some(Instant::now() + Duration::from_secs(10))) == Err(Fault::Closed)
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        drop(self.release.take());
        // Wake the blocking accept so the thread can end.
        let _ = UnixStream::connect(&self.path);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn sources(directory: &Path, found: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push((path.clone(), fs::read_to_string(&path).unwrap()));
        }
    }
}

#[test]
fn production_declares_no_object_storage() {
    let production: Vec<_> = EXTENSIONS
        .iter()
        .map(|extension| (extension.name, extension.objects))
        .collect();
    assert_eq!(
        production,
        [("colab", ObjectDeclaration::Disabled)],
        "enable an extension's objects only together with its real adapter readiness"
    );
    // Enabling is written only in tests: no production source names it as a field value.
    let mut found = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    for (path, text) in found {
        let tests = path.file_name().is_some_and(|name| name == "tests.rs")
            || path.components().any(|part| part.as_os_str() == "tests");
        assert!(
            tests || !text.contains("objects: ObjectDeclaration::Local"),
            "{} enables object storage outside a test",
            path.display()
        );
    }
    // The service itself names no extension.
    let own = include_str!("../object_service.rs").to_ascii_lowercase();
    assert!(
        !own.contains("colab"),
        "the service must not name an extension"
    );
}

#[test]
fn a_disabled_declaration_prepares_nothing_and_connects_nowhere() {
    let env = Env::new();
    // Production: nothing declares objects, so no ledger is created.
    assert!(env.service(&EXTENSIONS, bounds()).is_none());
    assert!(!env.ledger().exists());
    // A disabled entry beside an enabled one is neither handled nor connectable.
    let peer = Peer::start(&env, "beta", Answer::Expect);
    let mounts = env.mounts(&MIXED);
    let service = env.service(&MIXED, bounds()).unwrap();
    assert!(env.ledger().exists());
    assert!(service.handle("alpha").is_some());
    assert!(service.handle("beta").is_none());
    assert!(service.handle("unknown").is_none());
    assert_eq!(
        service.activate(&mounts, "beta"),
        Err(ActivateError::NotDeclared)
    );
    assert_eq!(
        service.activate(&mounts, "unknown"),
        Err(ActivateError::NotDeclared)
    );
    // The door's own socket check refuses it too, whatever a caller asks.
    let error = mounts
        .open_object_channel("beta", Instant::now() + Duration::from_secs(1))
        .err()
        .map(|error| error.kind());
    assert_eq!(error, Some(io::ErrorKind::NotFound));
    assert_eq!(peer.connections(), 0);
}

#[test]
fn readiness_refuses_storage_that_cannot_settle_and_never_resets_it() {
    let env = Env::new();
    fs::create_dir_all(env.root.join("remote")).unwrap();
    let damaged = b"not a ledger".repeat(100);
    fs::write(env.ledger(), &damaged).unwrap();
    let opened = ObjectService::open(
        &env.serving,
        &ALPHA,
        Quotas::contract(),
        system_clock(),
        bounds(),
        Origins::default(),
        &io(),
    );
    assert!(opened.is_err());
    assert_eq!(fs::read(env.ledger()).unwrap(), damaged);
    // A spent budget creates and changes nothing.
    let clean = Env::new();
    let cancelled = AtomicBool::new(true);
    let spent = IoBudget {
        deadline: Instant::now() + Duration::from_secs(60),
        cancelled: &cancelled,
    };
    let opened = ObjectService::open(
        &clean.serving,
        &ALPHA,
        Quotas::contract(),
        system_clock(),
        bounds(),
        Origins::default(),
        &spent,
    );
    assert!(opened.is_err());
    assert!(!clean.ledger().exists());
}

#[test]
fn the_offer_carries_the_values_the_door_sets_on_every_request_it_sends() {
    let env = Env::new();
    let peer = Peer::start(&env, "alpha", Answer::Expect);
    let mounts = env.mounts(&ALPHA);
    let service = env.service(&ALPHA, bounds()).unwrap();
    assert_eq!(service.active("alpha"), None);
    // The fixture accepts only Host 127.0.0.1:4100 and the mount `<prefix>/x/alpha/`.
    let generation = service.activate(&mounts, "alpha").unwrap();
    assert_eq!(service.active("alpha"), Some(generation));
    assert_eq!(service.ended("alpha"), None);
    assert_eq!(peer.connections(), 1);
    assert_eq!(peer.established(1), 1);
    assert_eq!(peer.buses.lock().unwrap()[0].generation(), generation);
}

#[test]
fn a_setup_that_fails_is_reported_once_and_never_retried() {
    let env = Env::new();
    let mounts = env.mounts(&ALPHA);
    let service = env.service(&ALPHA, bounds()).unwrap();
    // No socket yet, then an unsafe one, then a refused offer.
    let connect = |service: &ObjectService<'_>| service.activate(&mounts, "alpha");
    assert_eq!(
        connect(&service),
        Err(ActivateError::Connect(io::ErrorKind::NotFound))
    );
    let directory = env.directory("alpha");
    let real = directory.join("real.sock");
    let listener = UnixListener::bind(&real).unwrap();
    symlink(&real, directory.join("door.sock")).unwrap();
    assert_eq!(
        connect(&service),
        Err(ActivateError::Connect(io::ErrorKind::NotFound))
    );
    fs::remove_file(directory.join("door.sock")).unwrap();
    drop(listener);
    fs::remove_file(&real).unwrap();
    let open = Peer::start(&env, "alpha", Answer::Expect);
    fs::set_permissions(&open.path, fs::Permissions::from_mode(0o660)).unwrap();
    assert_eq!(
        connect(&service),
        Err(ActivateError::Connect(io::ErrorKind::NotFound))
    );
    assert_eq!(open.connections(), 0);
    drop(open);
    fs::remove_file(directory.join("door.sock")).unwrap();
    let refusing = Peer::start(&env, "alpha", Answer::Refuse);
    assert_eq!(
        connect(&service),
        Err(ActivateError::Channel(Fault::Truncated))
    );
    // One attempt, no fallback, and the slot is free for an explicit later call.
    assert_eq!(refusing.connections(), 1);
    assert_eq!(service.active("alpha"), None);
    assert_eq!(
        connect(&service),
        Err(ActivateError::Channel(Fault::Truncated))
    );
    assert_eq!(refusing.connections(), 2);
}

#[test]
fn one_candidate_per_extension_and_the_new_bus_replaces_the_active_one() {
    let env = Env::new();
    let peer = Peer::start(&env, "alpha", Answer::Gated);
    let mounts = env.mounts(&ALPHA);
    let service = env.service(&ALPHA, bounds()).unwrap();
    thread::scope(|scope| {
        let first = scope.spawn(|| service.activate(&mounts, "alpha"));
        peer.connection();
        // The candidate is mid-handshake: a second one for the same extension is refused.
        assert_eq!(service.activate(&mounts, "alpha"), Err(ActivateError::Busy));
        assert_eq!(peer.connections(), 1);
        peer.release();
        let one = first.join().unwrap().unwrap();
        assert_eq!(service.active("alpha"), Some(one));
        // A later call opens a new candidate; once established it replaces the active bus.
        let second = scope.spawn(|| service.activate(&mounts, "alpha"));
        peer.connection();
        assert_eq!(service.active("alpha"), Some(one));
        peer.release();
        let two = second.join().unwrap().unwrap();
        assert_ne!(one, two);
        assert_eq!(service.active("alpha"), Some(two));
    });
    assert_eq!(peer.established(2), 2);
    // The replaced bus was closed and joined: the extension saw its socket end.
    assert!(peer.saw_close(0));
    assert_eq!(service.ended("alpha"), None);
}

#[test]
fn the_installation_never_holds_more_buses_than_its_bound() {
    let env = Env::new();
    let peers: Vec<_> = ["alpha", "beta", "gamma"]
        .map(|name| Peer::start(&env, name, Answer::Expect))
        .into();
    let mounts = env.mounts(&TRIO);
    let service = env
        .service(
            &TRIO,
            ServiceBounds {
                buses: 2,
                ..bounds()
            },
        )
        .unwrap();
    service.activate(&mounts, "alpha").unwrap();
    service.activate(&mounts, "beta").unwrap();
    // A third extension, and even a replacement (its candidate is a bus), need a free slot.
    assert_eq!(
        service.activate(&mounts, "gamma"),
        Err(ActivateError::Capacity)
    );
    assert_eq!(
        service.activate(&mounts, "alpha"),
        Err(ActivateError::Capacity)
    );
    assert_eq!(peers[2].connections(), 0);
    assert_eq!(peers[0].connections(), 1);
    // Stopping releases every slot's socket.
    service.shutdown();
    assert!(peers[0].saw_close(0));
    assert!(peers[1].saw_close(0));
}

#[test]
fn stop_closes_every_bus_refuses_later_activation_and_ends_a_setup_in_progress() {
    let env = Env::new();
    let alpha = Peer::start(&env, "alpha", Answer::Expect);
    let beta = Peer::start(&env, "beta", Answer::Gated);
    let mounts = env.mounts(&TRIO);
    let service = env.service(&TRIO, bounds()).unwrap();
    service.activate(&mounts, "alpha").unwrap();
    thread::scope(|scope| {
        let pending = scope.spawn(|| service.activate(&mounts, "beta"));
        beta.connection();
        service.shutdown();
        assert_eq!(service.active("alpha"), None);
        assert_eq!(
            service.activate(&mounts, "gamma"),
            Err(ActivateError::Stopped)
        );
        // The candidate finishes its handshake after the stop and is closed, not kept.
        beta.release();
        assert_eq!(pending.join().unwrap(), Err(ActivateError::Stopped));
    });
    assert!(alpha.saw_close(0));
    assert_eq!(beta.established(1), 1);
    assert!(beta.saw_close(0));
    assert_eq!(service.active("beta"), None);
}

#[test]
fn dropping_the_service_closes_its_sockets() {
    let env = Env::new();
    let peer = Peer::start(&env, "alpha", Answer::Expect);
    let mounts = env.mounts(&ALPHA);
    let service = env.service(&ALPHA, bounds()).unwrap();
    service.activate(&mounts, "alpha").unwrap();
    drop(service);
    assert!(peer.saw_close(0));
}

// ---- Commit 2: the dispatcher, workers and `objects.config` answers. The test plays the
// extension: it sends requests on its end of the bus and answers the callbacks it receives.

fn with_bus<T>(peer: &Peer, f: impl FnOnce(&Bus) -> T) -> T {
    peer.established(1);
    f(&peer.buses.lock().unwrap()[0])
}
fn generation_of(peer: &Peer) -> Uuid4 {
    with_bus(peer, Bus::generation)
}
fn say(peer: &Peer, frame: Frame) {
    with_bus(peer, |bus| bus.send(&frame).unwrap());
}
/// The next frame the service sent, which must arrive within a generous bound.
fn heard(peer: &Peer) -> Frame {
    with_bus(peer, |bus| {
        bus.recv(Some(Instant::now() + Duration::from_secs(10)))
            .expect("the service sent nothing")
    })
}
fn counter(value: u64) -> Counter {
    Counter::new(value).unwrap()
}
fn config_input() -> ConfigInput {
    ConfigInput {
        namespace: Bytes32::from_bytes([7; 32]),
        policy: Policy::new(vec![1, 2, 3]).unwrap(),
    }
}
fn ask_config(peer: &Peer, id: u64, origin: Origin) {
    request(peer, id, origin, Call::Config(config_input()));
}
fn request(peer: &Peer, id: u64, origin: Origin, call: Call) {
    say(
        peer,
        Frame::Request(Request {
            generation: generation_of(peer),
            request_id: counter(id),
            origin,
            call,
        }),
    );
}
fn decide(peer: &Peer, callback: u64, request: u64, decision: Decision) {
    say(
        peer,
        Frame::Admission(Admission {
            generation: generation_of(peer),
            callback_id: counter(callback),
            request_id: counter(request),
            decision,
        }),
    );
}
/// The callback the service sent next; the test fails on any other frame.
fn callback(peer: &Peer) -> Admit {
    match heard(peer) {
        Frame::Admit(admit) => admit,
        other => panic!("expected a callback, heard {other:?}"),
    }
}
fn result(peer: &Peer) -> ResultFrame {
    match heard(peer) {
        Frame::Result(result) => result,
        other => panic!("expected a result, heard {other:?}"),
    }
}
fn bounds_of() -> TransferBounds {
    TransferBounds {
        payload_bytes: limits::OBJECT_PAYLOAD_BYTES,
        chunk_bytes: limits::OBJECT_CHUNK_BYTES,
    }
}
fn local_config() -> Config {
    let quotas = Quotas::contract();
    Config {
        backend_id: "local-fs".into(),
        immutable_create: true,
        chunked_read: true,
        recover_by_original_id: true,
        limits: Limits::Local {
            bounds: bounds_of(),
            namespace_bytes: quotas.namespace_bytes,
            extension_bytes: quotas.extension_bytes,
            installation_bytes: quotas.installation_bytes,
        },
    }
}
fn config_result(peer: &Peer, id: u64, outcome: Outcome) -> ResultFrame {
    ResultFrame {
        generation: generation_of(peer),
        request_id: counter(id),
        method: Method::Config,
        transfer_id: None,
        outcome,
    }
}
/// Why the channel to `name` ended, once the service has recorded it.
fn ended_soon(service: &ObjectService<'_>, name: &str) -> ChannelEnd {
    let started = Instant::now();
    loop {
        if let Some(ended) = service.ended(name) {
            return ended;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "channel never ended"
        );
        thread::yield_now();
    }
}
/// Every file and directory name under `directory`, for before/after comparison.
fn tree(directory: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = vec![directory.to_owned()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            names.push(path.strip_prefix(directory).unwrap().display().to_string());
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    names.sort();
    names
}

/// A service with one running channel to a fixture extension.
fn running<'a>(env: &'a Env, bounds: ServiceBounds) -> (ObjectService<'a>, Peer) {
    let peer = Peer::start(env, "alpha", Answer::Expect);
    let service = env.service(&ALPHA, bounds).unwrap();
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    (service, peer)
}

#[test]
fn local_config_is_answered_after_acquire_and_disclose_admission_and_changes_nothing() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let (ledger, files) = (
        fs::read(env.ledger()).unwrap(),
        tree(&env.root.join("alpha")),
    );
    ask_config(&peer, 1, Origin::LocalExtension);
    let acquire = callback(&peer);
    assert_eq!(acquire.boundary, Checkpoint::Acquire);
    assert_eq!(acquire.context, Context::LocalExtension);
    assert_eq!(acquire.request_id, counter(1));
    assert_eq!(acquire.callback_id, counter(1));
    assert_eq!(acquire.operation.input, AdmitInput::Config(config_input()));
    assert_eq!(acquire.operation.disclosure, None);
    decide(&peer, 1, 1, Decision::Allow);
    let disclose = callback(&peer);
    assert_eq!(disclose.boundary, Checkpoint::Disclose);
    assert_eq!(disclose.callback_id, counter(2));
    assert_eq!(
        disclose.operation.disclosure,
        Some(Disclosure::Config {
            projection: Projection::Local
        })
    );
    decide(&peer, 2, 1, Decision::Allow);
    let expected = config_result(&peer, 1, Outcome::Success(Success::Config(local_config())));
    assert_eq!(result(&peer), expected);
    // Reading the configuration created, reserved and changed nothing.
    assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
    assert_eq!(tree(&env.root.join("alpha")), files);
    assert_eq!(service.ended("alpha"), None);
}

#[test]
fn the_browser_projection_carries_the_bounds_without_the_owner_quotas() {
    let source = ConfigSource {
        backend_id: "local-fs",
        caps: crate::objects::BackendCaps {
            max_payload_bytes: limits::OBJECT_PAYLOAD_BYTES,
            chunk_bytes: limits::OBJECT_CHUNK_BYTES,
        },
        quotas: Quotas::contract(),
    };
    let browser = source.project(Projection::Browser);
    assert_eq!(browser.limits, Limits::Browser(bounds_of()));
    assert_eq!(source.project(Projection::Local), local_config());
    assert_eq!(
        Config {
            limits: local_config().limits,
            ..browser
        },
        local_config()
    );
}

#[test]
fn every_request_asks_afresh_and_a_refusal_is_never_an_allow() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    // (acquire decision, disclose decision if reached, expected outcome)
    let rows: [(&str, Decision, Option<Decision>, Outcome); 6] = [
        (
            "allowed twice",
            Decision::Allow,
            Some(Decision::Allow),
            Outcome::Success(Success::Config(local_config())),
        ),
        (
            "acquire denied after an allow",
            Decision::Deny,
            None,
            Outcome::Failure(ErrorCode::Denied),
        ),
        (
            "acquire unavailable",
            Decision::Unavailable,
            None,
            Outcome::Failure(ErrorCode::Unavailable),
        ),
        (
            "disclose denied",
            Decision::Allow,
            Some(Decision::Deny),
            Outcome::Failure(ErrorCode::Denied),
        ),
        (
            "disclose unavailable",
            Decision::Allow,
            Some(Decision::Unavailable),
            Outcome::Failure(ErrorCode::Unavailable),
        ),
        (
            "allowed again after refusals",
            Decision::Allow,
            Some(Decision::Allow),
            Outcome::Success(Success::Config(local_config())),
        ),
    ];
    let mut issued = 0;
    for (index, (name, acquire, disclose, outcome)) in rows.into_iter().enumerate() {
        let id = index as u64 + 1;
        ask_config(&peer, id, Origin::LocalExtension);
        let first = callback(&peer);
        assert_eq!(first.boundary, Checkpoint::Acquire, "{name}");
        issued += 1;
        assert_eq!(first.callback_id, counter(issued), "{name}");
        decide(&peer, issued, id, acquire);
        if let Some(disclose) = disclose {
            let second = callback(&peer);
            assert_eq!(second.boundary, Checkpoint::Disclose, "{name}");
            issued += 1;
            assert_eq!(second.callback_id, counter(issued), "{name}");
            decide(&peer, issued, id, disclose);
        }
        // The result comes next: no further callback was sent after a refusal.
        assert_eq!(result(&peer), config_result(&peer, id, outcome), "{name}");
    }
    assert_eq!(service.ended("alpha"), None);
}

#[test]
fn every_method_denies_an_unestablished_mounted_origin_without_a_callback() {
    let env = Env::new();
    let (_service, peer) = running(&env, bounds());
    let transfer = Uuid4::parse("5c1f0a3e-9d7b-4c2a-8f61-3b0e7d5a9c24").unwrap();
    let origin = Uuid4::parse("0b2d4f60-1a3c-4e5f-9a7b-8c9d0e1f2a3b").unwrap();
    let namespace = Bytes32::from_bytes([7; 32]);
    let policy = Policy::new(vec![1, 2, 3]).unwrap();
    let digest = Sha256Hex::from_bytes([3; 32]);
    let calls: Vec<(Method, Option<Uuid4>, Call)> = vec![
        (
            Method::Begin,
            Some(transfer),
            Call::Begin(BeginInput {
                transfer_id: transfer,
                namespace,
                opaque_key: Bytes32::from_bytes([9; 32]),
                policy: policy.clone(),
                payload_sha256: digest,
                payload_bytes: 5,
            }),
        ),
        (
            Method::Part,
            Some(transfer),
            Call::Part(tmt_extension_objects::PartInput {
                transfer_id: transfer,
                index: 0,
                bytes: Chunk::new(vec![1, 2, 3]).unwrap(),
            }),
        ),
        (
            Method::Commit,
            Some(transfer),
            Call::Commit(TransferInput {
                transfer_id: transfer,
            }),
        ),
        (
            Method::Status,
            Some(transfer),
            Call::Status(StatusInput {
                transfer_id: transfer,
                namespace,
                policy: policy.clone(),
            }),
        ),
        (
            Method::Read,
            None,
            Call::Read(ReadInput {
                namespace,
                opaque_key: Bytes32::from_bytes([9; 32]),
                policy,
                payload_sha256: digest,
                payload_bytes: 5,
                offset: 0,
                count: 5,
            }),
        ),
        (
            Method::Discard,
            Some(transfer),
            Call::Discard(TransferInput {
                transfer_id: transfer,
            }),
        ),
    ];
    let mut id = 0;
    // Nothing mounted is established, so every mounted request is denied.
    for (method, transfer_id, call) in calls.iter().cloned() {
        id += 1;
        request(&peer, id, Origin::Mounted(origin), call);
        let expected = ResultFrame {
            generation: generation_of(&peer),
            request_id: counter(id),
            method,
            transfer_id,
            outcome: Outcome::Failure(ErrorCode::Denied),
        };
        assert_eq!(result(&peer), expected, "mounted {method:?}");
    }
    id += 1;
    ask_config(&peer, id, Origin::Mounted(origin));
    assert_eq!(
        result(&peer),
        config_result(&peer, id, Outcome::Failure(ErrorCode::Denied))
    );
}

#[test]
fn eight_requests_wait_together_while_the_reader_keeps_delivering_decisions() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    for id in 1..=8 {
        ask_config(&peer, id, Origin::LocalExtension);
    }
    // Two workers each hold a request open, so the first two callbacks arrive together;
    // answer them in reverse order and the rest as they come.
    let (first, second) = (callback(&peer), callback(&peer));
    assert_eq!(first.boundary, Checkpoint::Acquire);
    assert!(first.callback_id < second.callback_id);
    decide(
        &peer,
        second.callback_id.get(),
        second.request_id.get(),
        Decision::Allow,
    );
    decide(
        &peer,
        first.callback_id.get(),
        first.request_id.get(),
        Decision::Allow,
    );
    let mut done = Vec::new();
    let mut highest = second.callback_id.get();
    while done.len() < 8 {
        match heard(&peer) {
            Frame::Admit(admit) => {
                assert!(admit.callback_id.get() > highest, "strictly increasing");
                highest = admit.callback_id.get();
                decide(&peer, highest, admit.request_id.get(), Decision::Allow);
            }
            Frame::Result(result) => {
                assert_eq!(
                    result.outcome,
                    Outcome::Success(Success::Config(local_config()))
                );
                done.push(result.request_id.get());
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    done.sort_unstable();
    assert_eq!(done, (1..=8).collect::<Vec<_>>());
    assert_eq!(highest, 16, "two callbacks for each of the eight requests");
    assert_eq!(service.ended("alpha"), None);
}

#[test]
fn a_callback_left_unanswered_ends_the_channel_and_is_never_an_allow() {
    let env = Env::new();
    let quick = ServiceBounds {
        callback: Duration::from_millis(200),
        ..bounds()
    };
    let (service, peer) = running(&env, quick);
    ask_config(&peer, 1, Origin::LocalExtension);
    assert_eq!(callback(&peer).boundary, Checkpoint::Acquire);
    // No decision: the channel ends at the callback bound and no result is sent, because
    // a result is refused while a callback is outstanding.
    assert!(peer.saw_close(0));
    assert_eq!(service.ended("alpha"), Some(ChannelEnd::CallbackTimeout));
    assert_eq!(service.active("alpha"), None);
    // The dead channel holds no slot: an explicit later activation opens a new one.
    let again = service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    assert_eq!(service.active("alpha"), Some(again));
    assert_eq!(service.ended("alpha"), None);
    assert_eq!(peer.established(2), 2);
}

#[test]
fn an_extension_that_closes_its_end_finishes_the_channel() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let bus = service.bus("alpha").unwrap();
    peer.established(1);
    let theirs = peer.buses.lock().unwrap().remove(0);
    theirs.close();
    // The dispatcher sees the end, every thread ends, and the last one drops the bus.
    let started = Instant::now();
    while service.ended("alpha").is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "channel never ended"
        );
        thread::yield_now();
    }
    assert!(matches!(service.ended("alpha"), Some(ChannelEnd::Bus(_))));
    service.shutdown();
    assert!(bus.upgrade().is_none());
}

#[test]
fn stopping_with_a_callback_outstanding_joins_every_thread_and_closes_the_socket() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let bus = service.bus("alpha").unwrap();
    ask_config(&peer, 1, Origin::LocalExtension);
    assert_eq!(callback(&peer).boundary, Checkpoint::Acquire);
    service.shutdown();
    // shutdown returned, so the dispatcher and both workers were joined, and the last of
    // them dropped the bus: nothing holds the socket.
    assert!(bus.upgrade().is_none());
    assert!(peer.saw_close(0));
    assert_eq!(service.active("alpha"), None);
    assert_eq!(
        service.activate(&env.mounts(&ALPHA), "alpha"),
        Err(ActivateError::Stopped)
    );
}

#[test]
fn the_installation_budget_is_shared_by_every_channel() {
    let env = Env::new();
    let alpha = Peer::start(&env, "alpha", Answer::Expect);
    let beta = Peer::start(&env, "beta", Answer::Expect);
    let tight = ServiceBounds {
        installation: Caps {
            requests: 1,
            callbacks: 8,
        },
        ..bounds()
    };
    static PAIR: [Extension; 2] = [
        declared("alpha", ObjectDeclaration::Local),
        declared("beta", ObjectDeclaration::Local),
    ];
    let service = env.service(&PAIR, tight).unwrap();
    let mounts = env.mounts(&PAIR);
    service.activate(&mounts, "alpha").unwrap();
    service.activate(&mounts, "beta").unwrap();
    // One request fits the whole installation; it stays open until its callback is answered.
    ask_config(&alpha, 1, Origin::LocalExtension);
    assert_eq!(callback(&alpha).boundary, Checkpoint::Acquire);
    // A request on the other channel is over the installation's bound and ends that channel.
    ask_config(&beta, 1, Origin::LocalExtension);
    assert!(beta.saw_close(0));
    // The bus shuts the socket down before the dispatcher records why it ended.
    let ended = ended_soon(&service, "beta");
    assert_eq!(ended, ChannelEnd::Bus(Fault::Correlation(Reason::Capacity)));
    assert_eq!(service.ended("alpha"), None);
}

#[test]
fn a_request_whose_time_is_spent_before_a_callback_is_unavailable_and_the_channel_lives() {
    let env = Env::new();
    // A zero request bound: the time is spent when a worker takes the request.
    let (service, peer) = running(
        &env,
        ServiceBounds {
            request: Duration::ZERO,
            ..bounds()
        },
    );
    for id in 1..=2 {
        ask_config(&peer, id, Origin::LocalExtension);
        // The first and only frame is the result: no callback was sent.
        assert_eq!(
            result(&peer),
            config_result(&peer, id, Outcome::Failure(ErrorCode::Unavailable))
        );
    }
    assert_eq!(service.ended("alpha"), None);
    assert!(service.active("alpha").is_some());
}

#[test]
fn time_spent_between_acquire_and_disclose_is_unavailable_with_no_second_callback() {
    let env = Env::new();
    let peer = Peer::start(&env, "alpha", Answer::Expect);
    let service = env
        .service(
            &ALPHA,
            ServiceBounds {
                request: Duration::from_millis(400),
                ..bounds()
            },
        )
        .unwrap();
    // Only the first request is held until its own deadline has passed.
    let first = Arc::new(AtomicBool::new(true));
    service.set_hook(Some(Arc::new(move |pause: Pause, deadline: Instant| {
        if pause == Pause::BetweenAdmissions && first.swap(false, Ordering::AcqRel) {
            while Instant::now() < deadline {
                thread::yield_now();
            }
        }
    })));
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    ask_config(&peer, 1, Origin::LocalExtension);
    assert_eq!(callback(&peer).boundary, Checkpoint::Acquire);
    decide(&peer, 1, 1, Decision::Allow);
    // The disclose callback is never sent: the next frame is the result.
    assert_eq!(
        result(&peer),
        config_result(&peer, 1, Outcome::Failure(ErrorCode::Unavailable))
    );
    // The channel keeps serving: the next request has its full time and is answered.
    ask_config(&peer, 2, Origin::LocalExtension);
    let acquire = callback(&peer);
    assert_eq!(
        (acquire.boundary, acquire.callback_id),
        (Checkpoint::Acquire, counter(2))
    );
    decide(&peer, 2, 2, Decision::Allow);
    let disclose = callback(&peer);
    assert_eq!(
        (disclose.boundary, disclose.callback_id),
        (Checkpoint::Disclose, counter(3))
    );
    decide(&peer, 3, 2, Decision::Allow);
    assert_eq!(
        result(&peer),
        config_result(&peer, 2, Outcome::Success(Success::Config(local_config())))
    );
    assert_eq!(service.ended("alpha"), None);
}

mod flow;
mod observe;

mod upload;
