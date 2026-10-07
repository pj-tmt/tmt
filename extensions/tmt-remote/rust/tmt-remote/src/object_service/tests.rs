//! The service over real Unix sockets: a fixture extension listens on its owner-only
//! `door.sock` under an isolated data root and answers the handshake with the leaf's
//! acceptor, so the offered `Host` and mount values, the one-active-one-candidate rule,
//! the installation bound, stop and the absence of leaked sockets are all observed from
//! the extension's side. Nothing infers an effect from a sleep.
use super::*;
use crate::{
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
use tmt_extension_objects::{Expect, accept};

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
        ObjectService::open(
            &self.serving,
            extensions,
            Quotas::contract(),
            system_clock(),
            bounds,
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
    /// How many buses completed their handshake.
    fn established(&self) -> usize {
        self.buses.lock().unwrap().len()
    }
    /// Whether the `index`th accepted bus saw its socket close.
    fn saw_close(&self, index: usize) -> bool {
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
        let tests = path.file_name().is_some_and(|name| name == "tests.rs");
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
    assert_eq!(service.fault("alpha"), None);
    assert_eq!(peer.connections(), 1);
    assert_eq!(peer.established(), 1);
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
    assert_eq!(peer.established(), 2);
    // The replaced bus was closed and joined: the extension saw its socket end.
    assert!(peer.saw_close(0));
    assert_eq!(service.fault("alpha"), None);
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
    assert_eq!(beta.established(), 1);
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
