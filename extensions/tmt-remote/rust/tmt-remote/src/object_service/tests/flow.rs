//! Origins over a real door: a browser-side TCP client upgrades through `Mounts` to a
//! fixture extension on its owner-only socket, which also answers the private object
//! channel. The extension sees exactly what Remote forwards, and its bus is told, in
//! order, when each tunnel is established and closed. The registry's own rules (terminal
//! close, bounds, lock discipline) are tested directly.
use super::super::dispatch::Pause;
use super::super::origins::{Events, Next};
use super::*;
use crate::{
    canonical::{self, Envelope},
    session::DoorSessions,
    state::MachineKey,
    store::{Grant, SUPPORTED_SCOPES, Store, uuid_v4},
};
use crate::{
    http::{Door, Handler},
    mount::{Admitted, DeviceContext, SessionState, Sessions},
    routes::Routes,
    site::Site,
};
use ed25519_dalek::{Signer, SigningKey};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
};
use tmt_extension_objects::{
    Admit, Decision, ErrorCode, OriginPhase, OriginState, ResultFrame, Success, accept_head,
};

const OWNER: &str = "tmt_door=owner";
const ENDED: &str = "tmt_door=ended";
const ROUTE: &str = "GET /.tmt/remote/object-channel-v1 ";

/// Cookies to owner sessions: one shared live session for `OWNER`, one already ended.
struct Tabs(Mutex<HashMap<&'static str, Arc<SessionState>>>);
impl Tabs {
    fn new() -> Arc<Self> {
        let ended = Arc::new(SessionState::default());
        ended.end();
        Arc::new(Self(Mutex::new(HashMap::from([
            (OWNER, Arc::new(SessionState::default())),
            (ENDED, ended),
        ]))))
    }
}
impl Sessions for Tabs {
    fn context(&self, cookie: Option<&str>) -> Option<Admitted> {
        let state = self.0.lock().unwrap().get(cookie?)?.clone();
        Some(Admitted {
            context: DeviceContext {
                device_id: "00000000-0000-4000-8000-000000000004".into(),
                kind: "browser".into(),
                origin: "http://127.0.0.1:1".into(),
                name: "Laptop".into(),
                public_key: [7; 32],
                grant_revision: 3,
            },
            session: state,
        })
    }
}

/// A live door whose mounts report their origins to `origins`.
struct Live {
    addr: SocketAddr,
    origin: String,
    prefix: String,
    site: Arc<Site>,
    stop: Arc<AtomicBool>,
    door: Option<JoinHandle<()>>,
}
impl Live {
    fn new(env: &Env, origins: &Origins) -> Self {
        Self::with(env, origins, |_| Tabs::new()).0
    }
    /// A door whose mounts resolve owner sessions with what `sessions` builds from the
    /// door's origin; the resolver is returned too.
    fn with<S: Sessions + 'static>(
        env: &Env,
        origins: &Origins,
        sessions: impl FnOnce(&str) -> Arc<S>,
    ) -> (Self, Arc<S>) {
        Self::with_hook(env, origins, sessions, None)
    }
    fn with_hook<S: Sessions + 'static>(
        env: &Env,
        origins: &Origins,
        sessions: impl FnOnce(&str) -> Arc<S>,
        hook: Option<Arc<dyn crate::mount::ActivationSink>>,
    ) -> (Self, Arc<S>) {
        let routes = Routes::new(1024, "/r/k7qxm4tz2pbwn6rh".into()).unwrap();
        let prefix = routes.prefix().to_owned();
        let door = Door::bind(0).unwrap();
        let (addr, origin) = (door.socket_addr().unwrap(), door.origin.clone());
        let resolver = sessions(&origin);
        let mut mounts = Mounts::with_extensions(
            env.root.clone(),
            &origin,
            &prefix,
            Arc::clone(&resolver) as Arc<dyn Sessions>,
            &ALPHA,
        )
        .with_origins(Arc::new(origins.clone()));
        if let Some(hook) = hook {
            mounts = mounts.with_activation(hook);
        }
        let site = Arc::new(Site {
            routes,
            mounts: Arc::new(mounts),
            pages: None,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, served) = (Arc::clone(&stop), Arc::clone(&site));
        let door = thread::spawn(move || door.run(&flag, served as Arc<dyn Handler>).unwrap());
        let live = Self {
            addr,
            origin,
            prefix,
            site,
            stop,
            door: Some(door),
        };
        (live, resolver)
    }
    fn mounts(&self) -> &Mounts {
        &self.site.mounts
    }
    fn expect(&self) -> Expect {
        Expect {
            host: self.addr.to_string(),
            mount: format!("{}/x/alpha/", self.prefix),
        }
    }
    /// Send `request` and return the connection and the reply head.
    fn send(&self, request: &str) -> (TcpStream, String) {
        let mut client = TcpStream::connect(self.addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        client.write_all(request.as_bytes()).unwrap();
        let mut head = Vec::new();
        let mut byte = [0; 1];
        while !head.ends_with(b"\r\n\r\n") {
            client.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        (client, String::from_utf8(head).unwrap())
    }
    /// A websocket upgrade to the extension's sync route.
    fn upgrade(&self, cookie: &str, extra: &str) -> (TcpStream, String) {
        self.upgrade_at("sync", cookie, extra)
    }
    fn upgrade_at(&self, route: &str, cookie: &str, extra: &str) -> (TcpStream, String) {
        self.send(&format!(
            "GET {}/x/alpha/{route} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nCookie: {cookie}\r\n{extra}\r\n",
            self.prefix, self.addr, self.origin
        ))
    }
    fn page(&self) -> String {
        self.send(&format!(
            "GET {}/x/alpha/page HTTP/1.1\r\nHost: {}\r\n\r\n",
            self.prefix, self.addr
        ))
        .1
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(door) = self.door.take() {
            door.join().unwrap();
        }
    }
}

/// The fixture extension: object channel and websocket routes on one `door.sock`.
struct Ext {
    attempts: Arc<AtomicUsize>,
    path: PathBuf,
    stop: Arc<AtomicBool>,
    heads: Arc<Mutex<Vec<String>>>,
    buses: Arc<Mutex<Vec<Bus>>>,
    thread: Option<JoinHandle<()>>,
}
impl Ext {
    fn start(env: &Env, live: &Live) -> Self {
        Self::launch(env, live, false)
    }
    /// An extension that drops its end of the object channel as soon as it is set up.
    fn start_closing(env: &Env, live: &Live) -> Self {
        Self::launch(env, live, true)
    }
    fn launch(env: &Env, live: &Live, closing: bool) -> Self {
        Self::launch_mode(env, live, closing, false)
    }
    fn launch_mode(env: &Env, live: &Live, closing: bool, held_setup: bool) -> Self {
        let path = env.directory("alpha").join("door.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let counting = Arc::clone(&attempts);
        let heads = Arc::new(Mutex::new(Vec::new()));
        let buses = Arc::new(Mutex::new(Vec::new()));
        let expect = live.expect();
        let (flag, seen, held) = (Arc::clone(&stop), Arc::clone(&heads), Arc::clone(&buses));
        let thread = thread::spawn(move || {
            let mut connections = Vec::new();
            while let Ok((stream, _)) = listener.accept() {
                if flag.load(Ordering::Acquire) {
                    break;
                }
                let (expect, seen, held) = (expect.clone(), Arc::clone(&seen), Arc::clone(&held));
                let counting = Arc::clone(&counting);
                connections.push(thread::spawn(move || {
                    serve_connection(
                        stream, closing, held_setup, &counting, &expect, &seen, &held,
                    );
                }));
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            attempts,
            path,
            stop,
            heads,
            buses,
            thread: Some(thread),
        }
    }
    /// The nth head the extension was sent on a mounted route, once it has arrived.
    fn head(&self, index: usize) -> String {
        let started = Instant::now();
        loop {
            if let Some(head) = self.heads.lock().unwrap().get(index) {
                return head.clone();
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "no head {index}"
            );
            thread::yield_now();
        }
    }
    fn heads(&self) -> usize {
        self.heads.lock().unwrap().len()
    }
    fn bus<T>(&self, f: impl FnOnce(&Bus) -> T) -> T {
        self.bus_at(0, f)
    }
    fn bus_at<T>(&self, index: usize, f: impl FnOnce(&Bus) -> T) -> T {
        let started = Instant::now();
        loop {
            if let Some(bus) = self.buses.lock().unwrap().get(index) {
                return f(bus);
            }
            assert!(started.elapsed() < Duration::from_secs(10), "no bus");
            thread::yield_now();
        }
    }
    /// The next origin-state notice on the object channel.
    /// Hear notices until every origin in `origins` is established. The browser has its 101
    /// before Remote establishes an origin, so a request sent earlier would be denied;
    /// notices for other origins (an earlier tab's close) are not what is waited on.
    fn await_established(&self, origins: &[Uuid4]) {
        let mut waiting = origins.to_vec();
        while !waiting.is_empty() {
            let notice = self.notice();
            if notice.phase == OriginPhase::Established {
                waiting.retain(|origin| *origin != notice.origin_id);
            }
        }
    }
    fn notice(&self) -> OriginState {
        match self.bus(|bus| bus.recv(Some(Instant::now() + Duration::from_secs(10)))) {
            Ok(Frame::OriginState(state)) => state,
            other => panic!("expected an origin notice, heard {other:?}"),
        }
    }
}
impl Drop for Ext {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.path);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
        let _ = fs::remove_file(&self.path);
    }
}
fn serve_connection(
    mut stream: UnixStream,
    closing: bool,
    held_setup: bool,
    attempts: &AtomicUsize,
    expect: &Expect,
    seen: &Mutex<Vec<String>>,
    held: &Mutex<Vec<Bus>>,
) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if !matches!(stream.read(&mut byte), Ok(1)) {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    if text.starts_with(ROUTE) {
        attempts.fetch_add(1, Ordering::SeqCst);
        if held_setup {
            // Read until Remote's absolute setup budget closes its candidate. No reply,
            // fixed sleep or unowned timer stands in for a hung extension listener.
            while matches!(stream.read(&mut byte), Ok(1)) {}
            return;
        }
        let setup = Instant::now() + Duration::from_secs(5);
        if let Ok(link) = accept_head(stream, &head, &[], expect, &Budgets::contract(), setup) {
            let bus = Bus::start(link, Budgets::contract(), Caps::contract(), None).unwrap();
            if closing {
                drop(bus);
            } else {
                held.lock().unwrap().push(bus);
            }
        }
        return;
    }
    let upgrade = text.to_ascii_lowercase().contains("upgrade: websocket");
    // The route `/refuse` is an extension that refuses the upgrade; every other one accepts.
    let refuses = text.contains(" /refuse ");
    seen.lock().unwrap().push(text);
    match (upgrade, refuses) {
        (true, false) => {
            let _ = stream.write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nconnection: upgrade\r\nupgrade: websocket\r\n\r\n",
            );
            // Hold the tunnel until the door closes it.
            let mut sink = [0; 256];
            while matches!(stream.read(&mut sink), Ok(count) if count > 0) {}
        }
        (true, true) => {
            let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n");
        }
        (false, _) => {
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
        }
    }
}

/// Every value of the header `name` in a request head, lowercased name.
fn values(head: &str, name: &str) -> Vec<String> {
    head.lines()
        .filter_map(|line| line.split_once(": "))
        .filter(|(have, _)| have.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.to_owned())
        .collect()
}
fn origin_of(head: &str) -> Uuid4 {
    let found = values(head, "tmt-origin");
    assert_eq!(found.len(), 1, "exactly one tmt-origin: {head}");
    Uuid4::parse(&found[0]).expect("a lowercase canonical UUIDv4")
}

fn world<'e>(env: &'e Env) -> (ObjectService<'e>, Origins, Live, Ext) {
    let origins = Origins::default();
    let service = env.service_with(&ALPHA, bounds(), origins.clone()).unwrap();
    let live = Live::new(env, &origins);
    let ext = Ext::start(env, &live);
    (service, origins, live, ext)
}

#[test]
fn an_upgrade_gets_one_origin_and_the_extension_is_told_in_order() {
    let env = Env::new();
    let (service, origins, live, ext) = world(&env);
    let generation = service.activate(live.mounts(), "alpha").unwrap();
    let (client, head) = live.upgrade(OWNER, "tmt-origin: forged\r\n");
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    let sent = ext.head(0);
    let origin = origin_of(&sent);
    // Remote's own value alone: a client's tmt-origin and device context are replaced or dropped.
    assert!(!sent.contains("forged"));
    assert_eq!(values(&sent, "tmt-device-context").len(), 1);
    let established = ext.notice();
    assert_eq!(
        (
            established.origin_id,
            established.phase,
            established.generation
        ),
        (origin, OriginPhase::Established, generation)
    );
    let kept = origins.established(origin, "alpha", generation).unwrap();
    // The owner session the upgrade was admitted under is retained for later checks.
    let owner = kept.owner.expect("an owner binding");
    assert_eq!(owner.device_id, "00000000-0000-4000-8000-000000000004");
    assert_eq!(owner.grant_revision, 3);
    drop(client);
    let closed = ext.notice();
    assert_eq!(
        (closed.origin_id, closed.phase),
        (origin, OriginPhase::Closed)
    );
    assert!(origins.established(origin, "alpha", generation).is_none());
    assert_eq!(origins.count(), 0);
}

#[test]
fn only_an_upgrade_with_an_active_channel_carries_an_origin() {
    let env = Env::new();
    let (service, origins, live, ext) = world(&env);
    // Declared but no channel yet: the upgrade works and carries no origin.
    let (idle, head) = live.upgrade(OWNER, "");
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(values(&ext.head(0), "tmt-origin").is_empty());
    assert_eq!(origins.count(), 0);
    drop(idle);
    // With a channel, a page request still carries none.
    service.activate(live.mounts(), "alpha").unwrap();
    assert!(live.page().starts_with("HTTP/1.1 200"));
    assert!(values(&ext.head(1), "tmt-origin").is_empty());
    assert_eq!(origins.count(), 0);
}

#[test]
fn a_failed_adoption_is_never_established() {
    let env = Env::new();
    let (service, origins, live, ext) = world(&env);
    let generation = service.activate(live.mounts(), "alpha").unwrap();
    // The extension refuses the upgrade.
    let (_, head) = live.upgrade_at("refuse", OWNER, "");
    assert!(head.starts_with("HTTP/1.1 403"), "{head}");
    assert_eq!(origins.count(), 0);
    // The owner session ended before the upgrade could attach: the extension saw the
    // upgrade and answered 101, but Remote refuses the browser and never establishes.
    let (_, head) = live.upgrade(ENDED, "");
    assert!(head.starts_with("HTTP/1.1 404"), "{head}");
    assert_eq!(ext.heads(), 2);
    assert_eq!(origins.count(), 0);
    // Neither attempt was announced: the next notice is the first real tunnel's.
    let (_client, _) = live.upgrade(OWNER, "");
    let notice = ext.notice();
    assert_eq!(notice.phase, OriginPhase::Established);
    assert_eq!(notice.origin_id, origin_of(&ext.head(2)));
    assert!(
        origins
            .established(notice.origin_id, "alpha", generation)
            .is_some()
    );
}

#[test]
fn two_tabs_of_one_session_have_two_origins_and_each_closes_alone() {
    let env = Env::new();
    let (service, origins, live, ext) = world(&env);
    let generation = service.activate(live.mounts(), "alpha").unwrap();
    let (first, _) = live.upgrade(OWNER, "");
    let (second, _) = live.upgrade(OWNER, "");
    let (one, two) = (origin_of(&ext.head(0)), origin_of(&ext.head(1)));
    assert_ne!(one, two);
    let mut opened = [ext.notice(), ext.notice()];
    opened.sort_by_key(|notice| notice.origin_id.to_string());
    let mut expected = [one, two];
    expected.sort_by_key(Uuid4::to_string);
    assert_eq!(opened.map(|notice| notice.origin_id), expected);
    drop(first);
    let closed = ext.notice();
    assert_eq!((closed.origin_id, closed.phase), (one, OriginPhase::Closed));
    assert!(origins.established(one, "alpha", generation).is_none());
    assert!(origins.established(two, "alpha", generation).is_some());
    drop(second);
    assert_eq!(ext.notice().origin_id, two);
}

#[test]
fn ending_a_channel_removes_every_origin_and_a_successor_inherits_none() {
    let env = Env::new();
    let (service, origins, live, ext) = world(&env);
    let old = service.activate(live.mounts(), "alpha").unwrap();
    let (_client, _) = live.upgrade(OWNER, "");
    let origin = origin_of(&ext.head(0));
    assert_eq!(ext.notice().phase, OriginPhase::Established);
    let new = service.activate(live.mounts(), "alpha").unwrap();
    assert_ne!(old, new);
    // The replaced channel's origins are gone and cannot be named on the successor.
    assert_eq!(origins.count(), 0);
    assert!(origins.established(origin, "alpha", old).is_none());
    assert!(origins.established(origin, "alpha", new).is_none());
    service.shutdown();
    assert_eq!(origins.count(), 0);
}

fn attached(origins: &Origins, limit: usize) -> (Uuid4, Arc<Events>) {
    let generation = Uuid4::parse("5c1f0a3e-9d7b-4c2a-8f61-3b0e7d5a9c24").unwrap();
    let events = Arc::new(Events::new(limit));
    origins.attach("alpha", generation, Arc::clone(&events));
    (generation, events)
}
fn drained(events: &Events) -> Vec<(Uuid4, OriginPhase)> {
    let mut found = Vec::new();
    while let Next::Notice(origin, phase) = events.next(Duration::ZERO) {
        found.push((origin, phase));
    }
    found
}
fn drain_ignored(events: &Events) {
    drained(events);
}
fn ticket(origins: &Origins) -> Arc<dyn crate::mount::OriginTicket> {
    crate::mount::OriginSink::pending(origins, "alpha", None).expect("an attached extension")
}

#[test]
fn a_close_wins_over_a_later_establish_and_every_phase_change_is_announced_once() {
    let origins = Origins::default();
    let (generation, events) = attached(&origins, 8);
    // Closed while Pending: no notice, and a late establish does nothing.
    let early = ticket(&origins);
    early.close();
    early.establish();
    assert!(drained(&events).is_empty());
    assert_eq!(origins.count(), 0);
    // Dropped while Pending: nothing was ever announced.
    drop(ticket(&origins));
    assert!(drained(&events).is_empty());
    // Established once, closed twice (and dropped): one of each, in order.
    let live = ticket(&origins);
    let id = Uuid4::parse(&live.id()).unwrap();
    live.establish();
    live.establish();
    assert!(origins.established(id, "alpha", generation).is_some());
    live.close();
    live.close();
    drop(live);
    assert_eq!(
        drained(&events),
        vec![(id, OriginPhase::Established), (id, OriginPhase::Closed)]
    );
    // An origin stands only on its own extension and channel generation.
    let again = ticket(&origins);
    let again_id = Uuid4::parse(&again.id()).unwrap();
    again.establish();
    let other = Uuid4::parse("0b2d4f60-1a3c-4e5f-9a7b-8c9d0e1f2a3b").unwrap();
    assert!(origins.established(again_id, "alpha", generation).is_some());
    assert!(origins.established(again_id, "alpha", other).is_none());
    assert!(origins.established(again_id, "beta", generation).is_none());
    again.close();
    drain_ignored(&events);
    // No channel for the extension: no origin at all.
    assert!(crate::mount::OriginSink::pending(&origins, "beta", None).is_none());
    origins.detach("alpha", generation);
    assert!(crate::mount::OriginSink::pending(&origins, "alpha", None).is_none());
}

#[test]
fn the_notice_backlog_is_bounded_and_overflow_is_reported() {
    let origins = Origins::default();
    let (_, events) = attached(&origins, 2);
    for _ in 0..2 {
        let each = ticket(&origins);
        each.establish();
        each.close();
    }
    // Four notices were produced for a bound of two.
    assert!(matches!(events.next(Duration::ZERO), Next::Overflow));
}

#[test]
fn origins_hold_no_lock_across_a_session_or_a_frame_and_survive_contention() {
    // The module takes only its own locks: no session, bus or frame in its code.
    let source = include_str!("../origins.rs");
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        ".send(",
        ".recv(",
        "Sessions",
        "SessionState",
        ".context(",
        "TcpStream",
        "UnixStream",
    ] {
        assert!(
            !code.contains(forbidden),
            "origins.rs must not use {forbidden}"
        );
    }
    // Establishing, closing, looking up, attaching and detaching from many threads at once
    // finishes: the order Origins then Events never inverts.
    let origins = Origins::default();
    let (generation, events) = attached(&origins, 1 << 16);
    let started = Instant::now();
    thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                for _ in 0..500 {
                    let each = ticket(&origins);
                    let id = Uuid4::parse(&each.id()).unwrap();
                    each.establish();
                    let _ = origins.established(id, "alpha", generation);
                    each.close();
                }
            });
        }
        scope.spawn(|| {
            while started.elapsed() < Duration::from_millis(300) {
                let _ = events.next(Duration::ZERO);
            }
        });
    });
    assert!(started.elapsed() < Duration::from_secs(30));
    assert_eq!(origins.count(), 0);
}

// ---- Commit B: the current-session check and mounted `config` answers, over real sockets
// and a real `DoorSessions` with a paired-device grant and a controllable idle clock.

/// A paired browser device and the clock its sessions' idle time follows.
struct Device {
    sessions: Arc<DoorSessions>,
    client_id: String,
    key: SigningKey,
    machine_id: String,
    window_id: String,
    origin: String,
    clock: Arc<Mutex<Instant>>,
}
impl Device {
    fn pair(env: &Env, origin: &str, idle: Duration) -> Self {
        Self::pair_with_key(env, origin, idle, SigningKey::from_bytes(&[5; 32]))
    }
    fn pair_with_key(env: &Env, origin: &str, idle: Duration, key: SigningKey) -> Self {
        let mut store = Store::open(&env.serving).unwrap();
        let machine = store.machine().unwrap();
        let client_id = uuid_v4().unwrap();
        store
            .insert_grant(&Grant {
                client_id: client_id.clone(),
                public_key: key.verifying_key().to_bytes(),
                kind: "browser".into(),
                origin: origin.into(),
                name: "Laptop".into(),
                agents: "all".into(),
                scopes: SUPPORTED_SCOPES.iter().map(|s| (*s).into()).collect(),
                mode: "direct".into(),
                issued_at_ms: 1,
                expires_at_ms: None,
                revision: 1,
                disabled: false,
            })
            .unwrap();
        let window_id = uuid_v4().unwrap();
        let base = Instant::now();
        let clock = Arc::new(Mutex::new(base));
        let ticking = Arc::clone(&clock);
        let sessions = Arc::new(
            DoorSessions::new(
                machine.id.clone(),
                window_id.clone(),
                origin.into(),
                format!("{}/x/", machine.route_prefix),
                MachineKey::open(env.serving.layout()).unwrap(),
                Arc::new(Mutex::new(store)),
                idle,
            )
            .with_clock(Arc::new(move || *ticking.lock().unwrap())),
        );
        Self {
            sessions,
            client_id,
            key,
            machine_id: machine.id,
            window_id,
            origin: origin.into(),
            clock,
        }
    }
    /// Open a session as the device and return its `Cookie` header value.
    fn open(&self) -> String {
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let payload = serde_json::json!({
            "clientNonce": nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
        })
        .to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let id = uuid_v4().unwrap();
        let signed = canonical::envelope(&Envelope {
            kind: "control",
            id: &id,
            correlation_id: None,
            machine_id: &self.machine_id,
            window_id: &self.window_id,
            client_id: &self.client_id,
            session_id: "new",
            sequence: "0",
            timestamp_ms: now,
            origin: &self.origin,
            operation: "session.open",
            payload: payload.as_bytes(),
        })
        .unwrap();
        let wire = serde_json::json!({
            "version": 1, "profile": "local-v1", "kind": "control", "id": id,
            "correlationId": null, "machineId": self.machine_id, "windowId": self.window_id,
            "clientId": self.client_id, "sessionId": "new", "sequence": "0",
            "timestampMs": now, "origin": self.origin, "operation": "session.open",
            "payload": canonical::base64url(payload.as_bytes()),
            "signature": canonical::base64url(&self.key.sign(&signed).to_bytes()),
        });
        let opened = self
            .sessions
            .open(Some(&self.origin), wire.to_string().as_bytes())
            .expect("the paired device opens a session");
        let cookie = opened.cookie.expect("a browser cookie");
        cookie.split(';').next().unwrap().to_owned()
    }
    fn advance(&self, by: Duration) {
        *self.clock.lock().unwrap() += by;
    }
}

/// One side of a conversation on the extension's `index`th bus.
struct Talk<'a>(&'a Ext, usize);
impl Talk<'_> {
    fn say(&self, frame: Frame) {
        self.0.bus_at(self.1, |bus| bus.send(&frame).unwrap());
    }
    fn heard(&self) -> Frame {
        self.0.bus_at(self.1, |bus| {
            bus.recv(Some(Instant::now() + Duration::from_secs(10)))
                .expect("nothing heard")
        })
    }
    /// The next frame that is not a lifecycle notice.
    fn next(&self) -> Frame {
        loop {
            match self.heard() {
                Frame::OriginState(_) => {}
                other => return other,
            }
        }
    }
    fn generation(&self) -> Uuid4 {
        self.0.bus_at(self.1, Bus::generation)
    }
    fn config(&self, id: u64, origin: Uuid4) {
        self.say(Frame::Request(Request {
            generation: self.generation(),
            request_id: counter(id),
            origin: Origin::Mounted(origin),
            call: Call::Config(config_input()),
        }));
    }
    fn callback(&self) -> Admit {
        match self.next() {
            Frame::Admit(admit) => admit,
            other => panic!("expected a callback, heard {other:?}"),
        }
    }
    fn result(&self) -> ResultFrame {
        match self.next() {
            Frame::Result(result) => result,
            other => panic!("expected a result, heard {other:?}"),
        }
    }
    fn decide(&self, admit: &Admit, decision: Decision) {
        self.say(Frame::Admission(Admission {
            generation: self.generation(),
            callback_id: admit.callback_id,
            request_id: admit.request_id,
            decision,
        }));
    }
    /// The result must be a denial with no callback before it.
    fn denied(&self, id: u64) {
        let result = self.result();
        assert_eq!(result.request_id, counter(id));
        assert_eq!(
            result.outcome,
            Outcome::Failure(ErrorCode::Denied),
            "request {id}"
        );
    }
}

/// Everything over a real `DoorSessions`.
struct Real<'e> {
    service: ObjectService<'e>,
    origins: Origins,
    live: Live,
    ext: Ext,
    device: Device,
}
fn real<'e>(env: &'e Env, idle: Duration, hook: dispatch::Hook) -> Real<'e> {
    let origins = Origins::default();
    let service = env.service_with(&ALPHA, bounds(), origins.clone()).unwrap();
    service.set_hook(hook);
    let device = std::cell::RefCell::new(None);
    let (live, _) = Live::with(env, &origins, |origin| {
        let paired = Device::pair(env, origin, idle);
        let sessions = Arc::clone(&paired.sessions);
        *device.borrow_mut() = Some(paired);
        sessions
    });
    let device = device.into_inner().unwrap();
    let ext = Ext::start(env, &live);
    service.activate(live.mounts(), "alpha").unwrap();
    Real {
        service,
        origins,
        live,
        ext,
        device,
    }
}
fn browser_config() -> Config {
    let mut config = local_config();
    config.limits = Limits::Browser(bounds_of());
    config
}

#[test]
fn a_mounted_owner_session_gets_the_reduced_config_after_both_admissions() {
    let env = Env::new();
    let world = real(&env, Duration::from_secs(600), None);
    let cookie = world.device.open();
    let (_tab, head) = world.live.upgrade(&cookie, "");
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    let origin = origin_of(&world.ext.head(0));
    assert_eq!(world.ext.notice().phase, OriginPhase::Established);
    let talk = Talk(&world.ext, 0);
    talk.config(1, origin);
    let acquire = talk.callback();
    let owner = Context::OwnerSession {
        origin_id: origin,
        device_id: Uuid4::parse(&world.device.client_id).unwrap(),
        grant_revision: 1,
    };
    assert_eq!(
        (acquire.boundary, acquire.context),
        (Checkpoint::Acquire, owner)
    );
    talk.decide(&acquire, Decision::Allow);
    let disclose = talk.callback();
    assert_eq!(
        (disclose.boundary, disclose.context),
        (Checkpoint::Disclose, owner)
    );
    assert_eq!(
        disclose.operation.disclosure,
        Some(Disclosure::Config {
            projection: Projection::Browser
        })
    );
    talk.decide(&disclose, Decision::Allow);
    let result = talk.result();
    // Reduced for a mounted origin: the bounds, never the owner's quotas.
    assert_eq!(
        result.outcome,
        Outcome::Success(Success::Config(browser_config()))
    );
}

#[test]
fn a_mounted_connection_without_an_owner_session_is_asked_as_mounted_and_sees_the_same_projection()
{
    let env = Env::new();
    let world = real(&env, Duration::from_secs(600), None);
    let (_tab, head) = world.live.send(&format!(
        "GET {}/x/alpha/sync HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        world.live.prefix, world.live.addr, world.live.origin
    ));
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    let origin = origin_of(&world.ext.head(0));
    assert_eq!(world.ext.notice().phase, OriginPhase::Established);
    let talk = Talk(&world.ext, 0);
    talk.config(1, origin);
    let acquire = talk.callback();
    assert_eq!(acquire.context, Context::Mounted { origin_id: origin });
    talk.decide(&acquire, Decision::Allow);
    let disclose = talk.callback();
    talk.decide(&disclose, Decision::Allow);
    assert_eq!(
        talk.result().outcome,
        Outcome::Success(Success::Config(browser_config()))
    );
}

#[test]
fn a_request_is_denied_without_a_callback_unless_its_origin_stands() {
    let env = Env::new();
    let world = real(&env, Duration::from_secs(600), None);
    let cookie = world.device.open();
    let (tab, _) = world.live.upgrade(&cookie, "");
    let closing = origin_of(&world.ext.head(0));
    assert_eq!(world.ext.notice().phase, OriginPhase::Established);
    let talk = Talk(&world.ext, 0);
    // An origin nobody issued, one still Pending, and one whose tunnel has closed.
    let unknown = Uuid4::parse("0b2d4f60-1a3c-4e5f-9a7b-8c9d0e1f2a3b").unwrap();
    let pending = ticket(&world.origins);
    let waiting = Uuid4::parse(&pending.id()).unwrap();
    drop(tab);
    assert_eq!(world.ext.notice().phase, OriginPhase::Closed);
    for (id, origin) in [(1, unknown), (2, waiting), (3, closing)] {
        talk.config(id, origin);
        talk.denied(id);
    }
    // After the pending one establishes it is served, but only on this channel: a
    // successor bus does not know it.
    pending.establish();
    talk.config(4, waiting);
    let acquire = talk.callback();
    assert_eq!(acquire.context, Context::Mounted { origin_id: waiting });
    talk.decide(&acquire, Decision::Deny);
    assert_eq!(talk.result().outcome, Outcome::Failure(ErrorCode::Denied));
    world
        .service
        .activate(world.live.mounts(), "alpha")
        .unwrap();
    let successor = Talk(&world.ext, 1);
    successor.config(1, waiting);
    successor.denied(1);
}

#[test]
fn two_tabs_are_each_asked_as_themselves_and_a_closed_tab_is_denied() {
    let env = Env::new();
    let world = real(&env, Duration::from_secs(600), None);
    let cookie = world.device.open();
    let (first, _) = world.live.upgrade(&cookie, "");
    let (_second, _) = world.live.upgrade(&cookie, "");
    let (one, two) = (origin_of(&world.ext.head(0)), origin_of(&world.ext.head(1)));
    world.ext.await_established(&[one, two]);
    let talk = Talk(&world.ext, 0);
    for (id, origin) in [(1, one), (2, two)] {
        talk.config(id, origin);
        let acquire = talk.callback();
        assert!(
            matches!(acquire.context, Context::OwnerSession { origin_id, .. } if origin_id == origin)
        );
        talk.decide(&acquire, Decision::Deny);
        assert_eq!(talk.result().outcome, Outcome::Failure(ErrorCode::Denied));
    }
    drop(first);
    let closed = loop {
        let notice = world.ext.notice();
        if notice.phase == OriginPhase::Closed {
            break notice;
        }
    };
    assert_eq!(closed.origin_id, one);
    talk.config(3, one);
    talk.denied(3);
    talk.config(4, two);
    let acquire = talk.callback();
    assert!(matches!(acquire.context, Context::OwnerSession { origin_id, .. } if origin_id == two));
    talk.decide(&acquire, Decision::Deny);
    assert_eq!(talk.result().outcome, Outcome::Failure(ErrorCode::Denied));
}

#[test]
fn a_session_that_ends_is_denied_before_acquire_between_admissions_and_before_the_result() {
    let env = Env::new();
    let sessions: Arc<Mutex<Option<Arc<DoorSessions>>>> = Arc::new(Mutex::new(None));
    let client = Arc::new(Mutex::new(String::new()));
    let at_result = Arc::new(AtomicBool::new(false));
    let hook: dispatch::Hook = {
        let (sessions, client, at_result) = (
            Arc::clone(&sessions),
            Arc::clone(&client),
            Arc::clone(&at_result),
        );
        Some(Arc::new(move |pause: Pause, _| {
            if pause == Pause::BeforeResult && at_result.load(Ordering::Acquire) {
                let live = sessions.lock().unwrap().clone().unwrap();
                live.end_device(&client.lock().unwrap());
            }
        }))
    };
    let world = real(&env, Duration::from_secs(600), hook);
    *sessions.lock().unwrap() = Some(Arc::clone(&world.device.sessions));
    *client.lock().unwrap() = world.device.client_id.clone();
    let talk = Talk(&world.ext, 0);
    let tab = |expected: u64| {
        let cookie = world.device.open();
        let (tab, _) = world.live.upgrade(&cookie, "");
        let origin = origin_of(&world.ext.head(expected as usize));
        world.ext.await_established(&[origin]);
        (tab, origin)
    };
    // (a) Revoked before the request arrives: denied, no callback.
    let (_a, origin) = tab(0);
    world.device.sessions.end_device(&world.device.client_id);
    talk.config(1, origin);
    talk.denied(1);
    // (b) Revoked while the acquire callback is out: the disclose callback is never sent.
    let (_b, origin) = tab(1);
    talk.config(2, origin);
    let acquire = talk.callback();
    world.device.sessions.end_device(&world.device.client_id);
    talk.decide(&acquire, Decision::Allow);
    talk.denied(2);
    // (c) Revoked after the extension allowed disclosure and before the result leaves:
    // no configuration is sent.
    let (_c, origin) = tab(2);
    at_result.store(true, Ordering::Release);
    talk.config(3, origin);
    let acquire = talk.callback();
    talk.decide(&acquire, Decision::Allow);
    let disclose = talk.callback();
    talk.decide(&disclose, Decision::Allow);
    talk.denied(3);
}

#[test]
fn an_idle_expired_session_is_denied() {
    let env = Env::new();
    let world = real(&env, Duration::from_secs(60), None);
    let cookie = world.device.open();
    let (_tab, _) = world.live.upgrade(&cookie, "");
    let origin = origin_of(&world.ext.head(0));
    world.ext.await_established(&[origin]);
    let talk = Talk(&world.ext, 0);
    talk.config(1, origin);
    let acquire = talk.callback();
    talk.decide(&acquire, Decision::Deny);
    assert_eq!(talk.result().outcome, Outcome::Failure(ErrorCode::Denied));
    // The session's idle time passes on the injected clock; the tunnel is still open.
    world.device.advance(Duration::from_secs(61));
    talk.config(2, origin);
    talk.denied(2);
}

#[test]
fn currentness_follows_the_live_session_its_device_and_its_revision() {
    let env = Env::new();
    let origins = Origins::default();
    let (_live, device) = {
        let mut made = None;
        let (live, _) = Live::with(&env, &origins, |origin| {
            let paired = Device::pair(&env, origin, Duration::from_secs(600));
            let sessions = Arc::clone(&paired.sessions);
            made = Some(paired);
            sessions
        });
        (live, made.unwrap())
    };
    let cookie = device.open();
    let admitted = device
        .sessions
        .context(Some(&cookie))
        .expect("a live session");
    let binding = crate::mount::OwnerBinding {
        session: admitted.session,
        device_id: admitted.context.device_id,
        grant_revision: admitted.context.grant_revision,
    };
    assert!(device.sessions.current(&binding));
    let mut other_device = binding.clone();
    other_device.device_id = uuid_v4().unwrap();
    let mut other_revision = binding.clone();
    other_revision.grant_revision += 1;
    let stranger = crate::mount::OwnerBinding {
        session: Arc::new(SessionState::default()),
        ..binding.clone()
    };
    for (name, wrong) in [
        ("another device", other_device),
        ("another grant revision", other_revision),
        ("a session this door never opened", stranger),
    ] {
        assert!(!device.sessions.current(&wrong), "{name}");
    }
    // Idle expiry and revocation each end it, and nothing brings it back.
    device.advance(Duration::from_secs(601));
    assert!(!device.sessions.current(&binding));
    let again = device.open();
    let fresh = device.sessions.context(Some(&again)).unwrap();
    let revoked = crate::mount::OwnerBinding {
        session: fresh.session,
        device_id: fresh.context.device_id,
        grant_revision: fresh.context.grant_revision,
    };
    assert!(device.sessions.current(&revoked));
    device.sessions.end_device(&device.client_id);
    assert!(!device.sessions.current(&revoked));
    assert!(!device.sessions.current(&binding));
}

#[test]
fn a_channel_that_ends_right_after_setup_leaves_no_origin_to_issue() {
    let env = Env::new();
    let origins = Origins::default();
    let service = env.service_with(&ALPHA, bounds(), origins.clone()).unwrap();
    let live = Live::new(&env, &origins);
    let ext = Ext::start_closing(&env, &live);
    // Whether the channel ends before or after setup finishes varies from run to run; in
    // every case it takes its registration with it and none is left behind.
    for _ in 0..25 {
        service.activate(live.mounts(), "alpha").unwrap();
        assert!(matches!(ended_soon(&service, "alpha"), ChannelEnd::Bus(_)));
        assert!(!origins.attached("alpha"));
    }
    let (_tab, head) = live.upgrade(OWNER, "");
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(values(&ext.head(0), "tmt-origin").is_empty());
    assert_eq!(origins.count(), 0);
}

fn observation_tab(live: &Live, ext: &Ext, index: usize) -> (TcpStream, Uuid4) {
    let (tab, head) = live.send(&format!(
        "GET {}/x/alpha/sync HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n", live.prefix, live.addr, live.origin
    ));
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    let origin = origin_of(&ext.head(index));
    ext.await_established(&[origin]);
    (tab, origin)
}

/// Send an observational request on the actual extension channel. A peer close
/// is an asserted result here, never an unwrap-write in a connection worker.
fn observe_request(talk: &Talk<'_>, id: u64, origin: Uuid4, call: Call) {
    talk.0.bus_at(talk.1, |bus| {
        assert!(
            bus.send(&Frame::Request(Request {
                generation: bus.generation(),
                request_id: counter(id),
                origin: Origin::Mounted(origin),
                call,
            }))
            .is_ok()
        )
    });
}
fn observe_allowed(talk: &Talk<'_>, context: Context) -> Outcome {
    let acquire = talk.callback();
    assert_eq!(
        (acquire.boundary, acquire.context),
        (Checkpoint::Acquire, context)
    );
    talk.decide(&acquire, Decision::Allow);
    let disclose = talk.callback();
    assert_eq!(
        (disclose.boundary, disclose.context),
        (Checkpoint::Disclose, context)
    );
    talk.decide(&disclose, Decision::Allow);
    talk.result().outcome
}

#[test]
fn original_observation_owner_scope_survives_a_session_but_not_another_device() {
    use super::observe::{read_input, seed, spec, status_input};
    let env = Env::new();
    let world = real(&env, Duration::from_secs(600), None);
    let cookie = world.device.open();
    let (_a, _) = world.live.upgrade(&cookie, "");
    let origin = origin_of(&world.ext.head(0));
    world.ext.await_established(&[origin]);
    let device_id = Uuid4::parse(&world.device.client_id).unwrap();
    let context = Context::OwnerSession {
        origin_id: origin,
        device_id,
        grant_revision: 1,
    };
    let original = spec(context, 7, b"hello");
    seed(&world.service, &original, b"hello", true);
    let talk = Talk(&world.ext, 0);
    observe_request(&talk, 1, origin, status_input(&original));
    assert!(matches!(
        observe_allowed(&talk, context),
        Outcome::Success(Success::Committed { .. })
    ));
    observe_request(&talk, 2, origin, read_input(&original, 0, 5));
    assert!(matches!(
        observe_allowed(&talk, context),
        Outcome::Success(Success::Read { .. })
    ));
    // New session, same actual device: original identity remains the same.
    world.device.sessions.end_device(&world.device.client_id);
    let cookie = world.device.open();
    let (_b, _) = world.live.upgrade(&cookie, "");
    let next = origin_of(&world.ext.head(1));
    world.ext.await_established(&[next]);
    observe_request(&talk, 3, next, status_input(&original));
    assert!(matches!(
        observe_allowed(
            &talk,
            Context::OwnerSession {
                origin_id: next,
                device_id,
                grant_revision: 1
            }
        ),
        Outcome::Success(Success::Committed { .. })
    ));
    let mut other = Device::pair_with_key(
        &env,
        &world.device.origin,
        Duration::from_secs(600),
        SigningKey::from_bytes(&[6; 32]),
    );
    other.sessions = Arc::clone(&world.device.sessions);
    other.window_id = world.device.window_id.clone();
    let cookie = other.open();
    let (_c, _) = world.live.upgrade(&cookie, "");
    let foreign = origin_of(&world.ext.head(2));
    world.ext.await_established(&[foreign]);
    observe_request(&talk, 4, foreign, status_input(&original));
    assert_eq!(
        observe_allowed(
            &talk,
            Context::OwnerSession {
                origin_id: foreign,
                device_id: Uuid4::parse(&other.client_id).unwrap(),
                grant_revision: 1
            }
        ),
        Outcome::Success(Success::State(tmt_extension_objects::State::NotObserved))
    );
}

#[test]
fn original_observation_mounted_connections_and_local_owner_scopes_are_distinct() {
    use super::observe::{read_input, seed, spec, status_input};
    let env = Env::new();
    let (service, _origins, live, ext) = world(&env);
    service.activate(live.mounts(), "alpha").unwrap();
    let (_a, a) = observation_tab(&live, &ext, 0);
    let (_b, b) = observation_tab(&live, &ext, 1);
    let context = Context::Mounted { origin_id: a };
    let original = spec(context, 7, b"hello");
    seed(&service, &original, b"hello", true);
    let talk = Talk(&ext, 0);
    observe_request(&talk, 1, a, status_input(&original));
    assert!(matches!(
        observe_allowed(&talk, context),
        Outcome::Success(Success::Committed { .. })
    ));
    observe_request(&talk, 2, b, status_input(&original));
    assert_eq!(
        observe_allowed(&talk, Context::Mounted { origin_id: b }),
        Outcome::Success(Success::State(tmt_extension_objects::State::NotObserved))
    );
    // Reads have current reference policy, not the original upload policy.
    observe_request(&talk, 3, a, read_input(&original, 1, 3));
    assert_eq!(
        observe_allowed(&talk, context),
        Outcome::Success(Success::Read {
            offset: 1,
            total_bytes: 5,
            bytes: Chunk::new(b"ell".to_vec()).unwrap()
        })
    );
    let local = spec(Context::LocalExtension, 7, b"hello").intent;
    let owner = spec(
        Context::OwnerSession {
            origin_id: a,
            device_id: a,
            grant_revision: 1,
        },
        7,
        b"hello",
    )
    .intent;
    assert_ne!(local, owner);
    assert_ne!(original.intent, local);
    assert_ne!(original.intent, owner);
    assert_ne!(
        original.intent,
        spec(Context::Mounted { origin_id: b }, 7, b"hello").intent
    );
    assert_ne!(
        original.intent,
        super::super::original::original_id(
            &crate::objects::ExtensionId::new("beta").unwrap(),
            context,
            super::observe::transfer()
        )
    );
}

#[test]
fn original_observation_revoke_during_acquire_disclose_or_actual_io_suppresses_data() {
    use super::observe::{read_input, seed, spec};
    for phase in 0..4 {
        let env = Env::new();
        let world = real(&env, Duration::from_secs(600), None);
        let cookie = world.device.open();
        let (_tab, _) = world.live.upgrade(&cookie, "");
        let origin = origin_of(&world.ext.head(0));
        world.ext.await_established(&[origin]);
        let context = Context::OwnerSession {
            origin_id: origin,
            device_id: Uuid4::parse(&world.device.client_id).unwrap(),
            grant_revision: 1,
        };
        let original = spec(context, 7, b"hello");
        seed(&world.service, &original, b"hello", true);
        if phase == 2 {
            let sessions = Arc::clone(&world.device.sessions);
            let device = world.device.client_id.clone();
            world.service.storage.observe(Arc::new(move |point| {
                if point == crate::objects::Milestone::ReadBytes {
                    sessions.end_device(&device);
                }
                true
            }));
        }
        let talk = Talk(&world.ext, 0);
        observe_request(&talk, 1, origin, read_input(&original, 0, 5));
        let acquire = talk.callback();
        if phase == 0 {
            world.device.sessions.end_device(&world.device.client_id);
        }
        if phase == 3 {
            Store::open(&env.serving)
                .unwrap()
                .rename(&world.device.client_id, "Changed binding")
                .unwrap();
        }
        talk.decide(&acquire, Decision::Allow);
        if phase == 1 {
            let disclose = talk.callback();
            world.device.sessions.end_device(&world.device.client_id);
            talk.decide(&disclose, Decision::Allow);
        }
        assert_eq!(talk.result().outcome, Outcome::Failure(ErrorCode::Denied));
    }
}

#[test]
fn original_observation_closed_origin_at_the_io_boundary_cannot_disclose() {
    use super::observe::{read_input, seed, spec};
    let env = Env::new();
    let (service, origins, live, ext) = world(&env);
    service.activate(live.mounts(), "alpha").unwrap();
    let (tab, origin) = observation_tab(&live, &ext, 0);
    let original = spec(Context::Mounted { origin_id: origin }, 7, b"hello");
    seed(&service, &original, b"hello", true);
    let held = Arc::new(Mutex::new(Some(tab)));
    let closing = Arc::clone(&held);
    let generation = service.active("alpha").unwrap();
    service.storage.observe(Arc::new(move |point| {
        if point == crate::objects::Milestone::ReadBytes {
            drop(closing.lock().unwrap().take());
            let limit = Instant::now() + Duration::from_secs(5);
            while origins.established(origin, "alpha", generation).is_some()
                && Instant::now() < limit
            {
                thread::yield_now();
            }
        }
        true
    }));
    let talk = Talk(&ext, 0);
    observe_request(&talk, 1, origin, read_input(&original, 0, 5));
    let acquire = talk.callback();
    talk.decide(&acquire, Decision::Allow);
    assert_eq!(talk.result().outcome, Outcome::Failure(ErrorCode::Denied));
}

/// The upload's exact captured principal is the same at all three decisions.
fn upload_allowed(talk: &Talk<'_>, context: Context) -> Outcome {
    for boundary in [
        Checkpoint::Acquire,
        Checkpoint::Effect,
        Checkpoint::Disclose,
    ] {
        let admit = talk.callback();
        assert_eq!((admit.boundary, admit.context), (boundary, context));
        assert_eq!(
            admit.operation.disclosure.is_some(),
            boundary == Checkpoint::Disclose
        );
        talk.decide(&admit, Decision::Allow);
    }
    talk.result().outcome
}

#[test]
fn original_upload_owner_reconnect_recovers_same_original_other_principals_cannot() {
    use super::observe::{spec, status_input};
    use super::upload::{begin, commit, part};
    let env = Env::new();
    let world = real(&env, Duration::from_secs(600), None);
    let cookie = world.device.open();
    let (tab, _) = world.live.upgrade(&cookie, "");
    let a = origin_of(&world.ext.head(0));
    world.ext.await_established(&[a]);
    let owner = Context::OwnerSession {
        origin_id: a,
        device_id: Uuid4::parse(&world.device.client_id).unwrap(),
        grant_revision: 1,
    };
    let original = spec(owner, 7, b"hello");
    let talk = Talk(&world.ext, 0);
    observe_request(&talk, 1, a, begin(&original));
    assert!(matches!(
        upload_allowed(&talk, owner),
        Outcome::Success(Success::Pending { .. })
    ));
    drop(tab);
    assert_eq!(world.ext.notice().phase, OriginPhase::Closed);
    let cookie = world.device.open();
    let (_tab, _) = world.live.upgrade(&cookie, "");
    let b = origin_of(&world.ext.head(1));
    world.ext.await_established(&[b]);
    let next = Context::OwnerSession {
        origin_id: b,
        device_id: Uuid4::parse(&world.device.client_id).unwrap(),
        grant_revision: 1,
    };
    observe_request(&talk, 2, b, part(b"hello", 0));
    assert!(matches!(
        upload_allowed(&talk, next),
        Outcome::Success(Success::Progress { received: 5, .. })
    ));
    observe_request(&talk, 3, b, commit());
    assert!(matches!(
        upload_allowed(&talk, next),
        Outcome::Success(Success::Committed {
            payload_bytes: 5,
            ..
        })
    ));
    observe_request(&talk, 4, b, status_input(&original));
    assert!(matches!(
        observe_allowed(&talk, next),
        Outcome::Success(Success::Committed { .. })
    ));
    let mut other = Device::pair_with_key(
        &env,
        &world.live.origin,
        Duration::from_secs(600),
        SigningKey::from_bytes(&[8; 32]),
    );
    other.sessions = Arc::clone(&world.device.sessions);
    other.window_id = world.device.window_id.clone();
    let (_foreign, _) = world.live.upgrade(&other.open(), "");
    let c = origin_of(&world.ext.head(2));
    world.ext.await_established(&[c]);
    observe_request(&talk, 5, c, part(b"hello", 0));
    assert_eq!(
        talk.result().outcome,
        Outcome::Failure(ErrorCode::Unavailable)
    );
    observe_request(&talk, 6, b, status_input(&original));
    assert!(matches!(
        observe_allowed(&talk, next),
        Outcome::Success(Success::Committed { .. })
    ));
}

#[test]
fn original_upload_connection_scope_is_charged_after_close_and_cannot_reconnect() {
    use super::observe::{spec, status_input};
    use super::upload::{begin, part};
    let env = Env::new();
    let (service, _origins, live, ext) = world(&env);
    service.activate(live.mounts(), "alpha").unwrap();
    let (tab, a) = observation_tab(&live, &ext, 0);
    let context = Context::Mounted { origin_id: a };
    let original = spec(context, 7, b"hello");
    let talk = Talk(&ext, 0);
    observe_request(&talk, 1, a, begin(&original));
    assert!(matches!(
        upload_allowed(&talk, context),
        Outcome::Success(Success::Pending { .. })
    ));
    let before = service.handle("alpha").unwrap().usage(None, &io()).unwrap();
    assert_eq!(
        (
            before.entries,
            before.active_uploads,
            before.retained_identities
        ),
        (1, 1, 1)
    );
    drop(tab);
    assert_eq!(ext.notice().phase, OriginPhase::Closed);
    let (_tab, b) = observation_tab(&live, &ext, 1);
    observe_request(&talk, 2, b, status_input(&original));
    assert_eq!(
        observe_allowed(&talk, Context::Mounted { origin_id: b }),
        Outcome::Success(Success::State(tmt_extension_objects::State::NotObserved))
    );
    observe_request(&talk, 3, b, part(b"hello", 0));
    assert_eq!(
        talk.result().outcome,
        Outcome::Failure(ErrorCode::Unavailable)
    );
    assert_eq!(
        service.handle("alpha").unwrap().usage(None, &io()).unwrap(),
        before
    );
    // The abandoned connection original still saturates the active-intent ceiling.
    service.shutdown();
    drop(service);
    let origins = Origins::default();
    let service = ObjectService::open(
        &env.serving,
        &ALPHA,
        Quotas {
            active_intents: 1,
            ..Quotas::contract()
        },
        system_clock(),
        bounds(),
        origins,
        &io(),
    )
    .unwrap()
    .unwrap();
    // Reuse the same installed extension's actual fixture channel, not another opener.
    service.activate(live.mounts(), "alpha").unwrap();
    ext.bus_at(1, |_| ());
    let successor = Talk(&ext, 1);
    successor.say(Frame::Request(Request {
        generation: successor.generation(),
        request_id: counter(1),
        origin: Origin::LocalExtension,
        call: begin(&super::observe::spec(Context::LocalExtension, 8, b"hello")),
    }));
    for boundary in [Checkpoint::Acquire, Checkpoint::Effect] {
        let admit = successor.callback();
        assert_eq!(admit.boundary, boundary);
        successor.decide(&admit, Decision::Allow);
    }
    assert_eq!(
        successor.result().outcome,
        Outcome::Failure(ErrorCode::Capacity(
            tmt_extension_objects::Limit::ActiveIntents
        ))
    );
}

#[test]
fn original_upload_revoke_or_binding_change_before_effect_allocates_nothing() {
    use super::observe::spec;
    use super::upload::begin;
    for phase in 0..3 {
        let env = Env::new();
        let world = real(&env, Duration::from_secs(600), None);
        let cookie = world.device.open();
        let (_tab, _) = world.live.upgrade(&cookie, "");
        let origin = origin_of(&world.ext.head(0));
        world.ext.await_established(&[origin]);
        let context = Context::OwnerSession {
            origin_id: origin,
            device_id: Uuid4::parse(&world.device.client_id).unwrap(),
            grant_revision: 1,
        };
        let original = spec(context, 7, b"hello");
        let before = fs::read(env.ledger()).unwrap();
        let talk = Talk(&world.ext, 0);
        observe_request(&talk, 1, origin, begin(&original));
        let acquire = talk.callback();
        let held = if phase == 0 {
            acquire
        } else {
            talk.decide(&acquire, Decision::Allow);
            let effect = talk.callback();
            assert_eq!(effect.boundary, Checkpoint::Effect);
            effect
        };
        if phase == 2 {
            Store::open(&env.serving)
                .unwrap()
                .rename(&world.device.client_id, "Changed binding")
                .unwrap();
        } else {
            world.device.sessions.end_device(&world.device.client_id);
        }
        talk.decide(&held, Decision::Allow);
        assert_eq!(talk.result().outcome, Outcome::Failure(ErrorCode::Denied));
        assert_eq!(fs::read(env.ledger()).unwrap(), before);
        assert_eq!(
            world
                .service
                .handle("alpha")
                .unwrap()
                .usage(None, &io())
                .unwrap(),
            crate::objects::Usage::default()
        );
    }
}

#[test]
fn original_upload_revoke_during_mutation_or_disclose_is_unknown_and_charged() {
    use super::observe::spec;
    use super::upload::begin;
    for phase in 0..3 {
        let env = Env::new();
        let world = real(&env, Duration::from_secs(600), None);
        let cookie = world.device.open();
        let (_tab, _) = world.live.upgrade(&cookie, "");
        let origin = origin_of(&world.ext.head(0));
        world.ext.await_established(&[origin]);
        let context = Context::OwnerSession {
            origin_id: origin,
            device_id: Uuid4::parse(&world.device.client_id).unwrap(),
            grant_revision: 1,
        };
        let original = spec(context, 7, b"hello");
        if phase == 0 {
            let sessions = Arc::clone(&world.device.sessions);
            let device = world.device.client_id.clone();
            world.service.storage.observe(Arc::new(move |point| {
                if point == crate::objects::Milestone::Adopted {
                    sessions.end_device(&device);
                }
                true
            }));
        }
        let talk = Talk(&world.ext, 0);
        observe_request(&talk, 1, origin, begin(&original));
        for boundary in [Checkpoint::Acquire, Checkpoint::Effect] {
            let admit = talk.callback();
            assert_eq!(admit.boundary, boundary);
            talk.decide(&admit, Decision::Allow);
        }
        if phase != 0 {
            let disclose = talk.callback();
            if phase == 1 {
                world.device.sessions.end_device(&world.device.client_id);
            } else {
                Store::open(&env.serving)
                    .unwrap()
                    .rename(&world.device.client_id, "Changed binding")
                    .unwrap();
            }
            talk.decide(&disclose, Decision::Allow);
        }
        assert_eq!(talk.result().outcome, Outcome::Failure(ErrorCode::Unknown));
        let charge = world
            .service
            .handle("alpha")
            .unwrap()
            .usage(None, &io())
            .unwrap();
        assert_eq!(
            (
                charge.entries,
                charge.active_uploads,
                charge.retained_identities
            ),
            (1, 1, 1)
        );
        assert_eq!(world.service.ended("alpha"), None);
    }
}

#[test]
fn original_upload_close_at_the_mutation_boundary_suppresses_disclosure() {
    use super::observe::spec;
    use super::upload::begin;
    let env = Env::new();
    let (service, origins, live, ext) = world(&env);
    service.activate(live.mounts(), "alpha").unwrap();
    let (tab, origin) = observation_tab(&live, &ext, 0);
    let original = spec(Context::Mounted { origin_id: origin }, 7, b"hello");
    let held = Arc::new(Mutex::new(Some(tab)));
    let closing = Arc::clone(&held);
    let generation = service.active("alpha").unwrap();
    service.storage.observe(Arc::new(move |point| {
        if point == crate::objects::Milestone::Adopted {
            drop(closing.lock().unwrap().take());
            let limit = Instant::now() + Duration::from_secs(5);
            while origins.established(origin, "alpha", generation).is_some()
                && Instant::now() < limit
            {
                thread::yield_now();
            }
            assert!(origins.established(origin, "alpha", generation).is_none());
        }
        true
    }));
    let talk = Talk(&ext, 0);
    observe_request(&talk, 1, origin, begin(&original));
    for boundary in [Checkpoint::Acquire, Checkpoint::Effect] {
        let admit = talk.callback();
        assert_eq!(admit.boundary, boundary);
        talk.decide(&admit, Decision::Allow);
    }
    assert_eq!(talk.result().outcome, Outcome::Failure(ErrorCode::Unknown));
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .usage(None, &io())
            .unwrap()
            .active_uploads,
        1
    );
}

#[test]
fn validated_upgrade_activates_a_late_listener_and_restart_replaces_origins() {
    let env = Env::new();
    let origins = Origins::default();
    let service = env.service_with(&ALPHA, bounds(), origins.clone()).unwrap();
    let hook = service.reactivation(Arc::new(AtomicBool::new(false)));
    let (live, _) = Live::with_hook(&env, &origins, |_| Tabs::new(), Some(hook.clone()));
    let ext = Ext::start(&env, &live);
    assert_eq!(service.readiness().snapshot()[0]["state"], "unavailable");
    service.with_reactivation(&hook, live.mounts(), || {
        assert!(
            live.upgrade(OWNER, "Sec-Fetch-Site: cross-site\r\n")
                .1
                .starts_with("HTTP/1.1 403")
        );
        assert!(
            live.upgrade_at(".tmt/remote/object-channel-v1", OWNER, "")
                .1
                .starts_with("HTTP/1.1 404")
        );
        assert_eq!(ext.attempts.load(Ordering::SeqCst), 0);
        // Ordinary page access is not an activation demand.
        assert!(live.page().starts_with("HTTP/1.1 200"));
        assert_eq!(service.readiness().snapshot()[0]["state"], "unavailable");
        let (first, response) = live.upgrade(OWNER, "");
        assert!(response.starts_with("HTTP/1.1 101"));
        let old = origin_of(&ext.head(1));
        ext.await_established(&[old]);
        assert_eq!(service.readiness().snapshot()[0]["state"], "ready");
        let generation = service.locked().slots[0]
            .active
            .as_ref()
            .unwrap()
            .generation();
        let original = super::observe::spec(Context::LocalExtension, 7, b"hello");
        let talk = Talk(&ext, 0);
        for (id, call) in [
            (1, super::upload::begin(&original)),
            (2, super::upload::part(b"hello", 0)),
            (3, super::upload::commit()),
        ] {
            talk.say(Frame::Request(Request {
                generation: talk.generation(),
                request_id: counter(id),
                origin: Origin::LocalExtension,
                call,
            }));
            assert!(matches!(
                upload_allowed(&talk, Context::LocalExtension),
                Outcome::Success(_)
            ));
        }
        let (old_bus, old_reader, old_writer) = {
            let state = service.locked();
            let running = state.slots[0].active.as_ref().unwrap();
            (running.bus(), running.view(), running.writer_view())
        };
        // A healthy upgrade does not replace the channel.
        let (second, response) = live.upgrade(OWNER, "");
        assert!(response.starts_with("HTTP/1.1 101"));
        let sibling = origin_of(&ext.head(2));
        ext.await_established(&[sibling]);
        assert_eq!(
            service.locked().slots[0]
                .active
                .as_ref()
                .unwrap()
                .generation(),
            generation
        );
        ext.buses.lock().unwrap().remove(0).close();
        let deadline = Instant::now() + Duration::from_secs(10);
        while service.readiness().snapshot()[0]["state"] == "ready" {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        assert_eq!(service.readiness().snapshot()[0]["reason"], "channel-ended");
        let (third, response) = live.upgrade(OWNER, "");
        assert!(response.starts_with("HTTP/1.1 101"));
        let current = origin_of(&ext.head(3));
        assert_ne!(old, current);
        assert!(origins.established(old, "alpha", generation).is_none());
        assert_ne!(
            service.locked().slots[0]
                .active
                .as_ref()
                .unwrap()
                .generation(),
            generation
        );
        assert_eq!(service.readiness().snapshot()[0]["state"], "ready");
        let successor = Talk(&ext, 0); // the fixture removed its closed first bus
        successor.say(Frame::Request(Request {
            generation: successor.generation(),
            request_id: counter(1),
            origin: Origin::LocalExtension,
            call: super::observe::status_input(&original),
        }));
        assert!(matches!(
            observe_allowed(&successor, Context::LocalExtension),
            Outcome::Success(Success::Committed {
                payload_bytes: 5,
                ..
            })
        ));
        assert!(old_bus.upgrade().is_none());
        assert!(old_reader.upgrade().is_none());
        assert!(old_writer.upgrade().is_none());
        drop((first, second, third));
    });
    service.shutdown();
    drop(live);
    drop(ext);
    assert!(!env.directory("alpha").join("door.sock").exists());
}

/// Every phase uses the same assertion, including the full-pool sensitivity probe.
fn assert_page_upgrade(response: &str, phase: &str, elapsed_ms: usize, ext: &Ext) {
    assert!(
        response.starts_with("HTTP/1.1 101"),
        "{phase}: clock={elapsed_ms}ms attempts={} page_heads={} status={:?} full_head={response:?}",
        ext.attempts.load(Ordering::SeqCst),
        ext.heads(),
        response.lines().next().unwrap_or("<empty>")
    );
}

fn await_page_tunnel_release(live: &Live) {
    let session = live
        .mounts()
        .sessions()
        .context(Some(OWNER))
        .unwrap()
        .session;
    // In Mounts::adopt, `_slot` is a closure-body local; `session` remains in
    // the consumed closure environment. Body locals drop before that environment,
    // so the last SessionTransport detach follows every fixture TunnelSlot release.
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.has_transport() {
        assert!(
            Instant::now() < deadline,
            "page tunnel teardown did not finish"
        );
        thread::yield_now();
    }
}

#[test]
fn a_full_page_tunnel_pool_refuses_the_next_upgrade_with_503() {
    let env = Env::new();
    let origins = Origins::default();
    let live = Live::new(&env, &origins);
    let ext = Ext::start(&env, &live);
    let mut clients = Vec::new();
    for _ in 0..ALPHA[0].tunnels {
        let (client, response) = live.upgrade(OWNER, "");
        assert_page_upgrade(&response, "fill pool", 0, &ext);
        clients.push(client);
    }
    for index in 0..ALPHA[0].tunnels {
        ext.head(index);
    }
    let (refused, response) = live.upgrade(OWNER, "");
    assert!(response.starts_with("HTTP/1.1 503"), "{response:?}");
    assert_eq!(ext.heads(), ALPHA[0].tunnels);
    assert_eq!(ext.attempts.load(Ordering::SeqCst), 0);
    drop((refused, clients));
    await_page_tunnel_release(&live);
    let (successor, response) = live.upgrade(OWNER, "");
    assert_page_upgrade(&response, "pool released", 0, &ext);
    drop(successor);
    live.mounts().shutdown();
    drop(live);
    drop(ext);
}

#[test]
fn hung_setup_is_single_flight_forwards_without_origin_and_cools_down_monotonically() {
    use crate::mount::ActivationSink;
    let env = Env::new();
    let origins = Origins::default();
    let service = env.service_with(&ALPHA, bounds(), origins.clone()).unwrap();
    let elapsed = Arc::new(AtomicUsize::new(0));
    let clock_elapsed = elapsed.clone();
    let anchor = Instant::now();
    let hook = super::super::activation::Reactivation::new(
        &service,
        Arc::new(AtomicBool::new(false)),
        Arc::new(move || {
            anchor + Duration::from_millis(clock_elapsed.load(Ordering::SeqCst) as u64)
        }),
    );
    let (live, _) = Live::with_hook(&env, &origins, |_| Tabs::new(), Some(hook.clone()));
    let ext = Ext::launch_mode(&env, &live, false, true);
    service.with_reactivation(&hook, live.mounts(), || {
        let clients = thread::scope(|scope| {
            let mut clients = Vec::new();
            for _ in 0..4 {
                clients.push(scope.spawn(|| live.upgrade(OWNER, "")));
            }
            let mut sockets = Vec::new();
            for client in clients {
                let (socket, response) = client.join().unwrap();
                assert_page_upgrade(
                    &response,
                    "initial single flight",
                    elapsed.load(Ordering::SeqCst),
                    &ext,
                );
                sockets.push(socket);
            }
            sockets
        });
        // Wait on the typed setup outcome, not elapsed fixture wall time: the real
        // held private head must have exhausted exactly the 250 ms demand deadline.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !hook.idle() {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        assert_eq!(
            service.locked().slots[0].failure,
            Some(ActivateError::Channel(Fault::Timeout(
                tmt_extension_objects::Stage::Head
            )))
        );
        assert_eq!(ext.attempts.load(Ordering::SeqCst), 1);
        for index in 0..4 {
            assert!(values(&ext.head(index), "tmt-origin").is_empty());
        }
        assert_eq!(origins.count(), 0);
        assert_eq!(service.readiness().snapshot()[0]["state"], "unavailable");
        // Closing clients alone does not synchronously join their splice owners.
        drop(clients);
        await_page_tunnel_release(&live);
        elapsed.store(999, Ordering::SeqCst);
        let (client, response) = live.upgrade(OWNER, "");
        assert_page_upgrade(
            &response,
            "inside cooldown",
            elapsed.load(Ordering::SeqCst),
            &ext,
        );
        drop(client);
        assert_eq!(ext.attempts.load(Ordering::SeqCst), 1);
        elapsed.store(1000, Ordering::SeqCst);
        let (client, response) = live.upgrade(OWNER, "");
        assert_page_upgrade(
            &response,
            "cooldown elapsed",
            elapsed.load(Ordering::SeqCst),
            &ext,
        );
        drop(client);
        assert_eq!(ext.attempts.load(Ordering::SeqCst), 2);
        hook.close();
        elapsed.store(2000, Ordering::SeqCst);
        hook.prepare("alpha", Instant::now() + limits::MOUNT_RESPONSE);
        assert_eq!(ext.attempts.load(Ordering::SeqCst), 2);
    });
    service.shutdown();
    drop(live);
    drop(ext);
}

#[test]
fn stop_during_demand_setup_wakes_joiners_and_cannot_publish_a_late_channel() {
    use crate::mount::ActivationSink;
    let env = Env::new();
    let origins = Origins::default();
    let service = env.service_with(&ALPHA, bounds(), origins.clone()).unwrap();
    let hook = service.reactivation(Arc::new(AtomicBool::new(false)));
    let live = Live::new(&env, &origins);
    let ext = Ext::launch_mode(&env, &live, false, true);
    let readiness = service.readiness();
    service.with_reactivation(&hook, live.mounts(), || {
        thread::scope(|scope| {
            let waiting =
                scope.spawn(|| hook.prepare("alpha", Instant::now() + limits::MOUNT_RESPONSE));
            let deadline = Instant::now() + Duration::from_secs(10);
            while ext.attempts.load(Ordering::SeqCst) == 0 {
                assert!(Instant::now() < deadline);
                thread::yield_now();
            }
            assert_eq!(readiness.snapshot()[0]["state"], "starting");
            hook.close();
            waiting.join().unwrap();
            assert_eq!(readiness.snapshot()[0]["state"], "unavailable");
        });
    });
    assert!(service.active("alpha").is_none());
    assert_eq!(origins.count(), 0);
    assert_eq!(ext.attempts.load(Ordering::SeqCst), 1);
    service.shutdown();
    drop(live);
    drop(ext);
    drop(service);
    // A retained status view has no worker or storage handle to keep mutating.
    assert_eq!(readiness.snapshot()[0]["state"], "unavailable");
}

#[test]
fn a_late_setup_candidate_never_supplies_an_origin_to_the_forwarded_upgrade() {
    use crate::mount::ActivationSink;
    let env = Env::new();
    let origins = Origins::default();
    let service = env.service_with(&ALPHA, bounds(), origins.clone()).unwrap();
    let hook = service.reactivation(Arc::new(AtomicBool::new(false)));
    let (live, _) = Live::with_hook(&env, &origins, |_| Tabs::new(), Some(hook.clone()));
    let ext = Ext::start(&env, &live);
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let held = Mutex::new(held);
    service.set_hook(Some(Arc::new(move |phase, deadline| {
        if phase == Pause::BeforeInstall {
            entered.send(deadline).unwrap();
            let released = held.lock().unwrap().recv_timeout(Duration::from_secs(10));
            assert!(
                !matches!(released, Err(mpsc::RecvTimeoutError::Timeout)),
                "setup hold exceeded its fixture bound"
            );
        }
    })));
    service.with_reactivation(&hook, live.mounts(), || {
        // Own the only sender in the scope: assertion unwinding disconnects the hold
        // before the setup worker is joined, just as an explicit release does.
        thread::scope(|scope| {
            let release = release;
            let waiting = scope.spawn(|| live.upgrade(OWNER, ""));
            let setup_deadline = observed.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(
                setup_deadline <= Instant::now() + Duration::from_millis(250),
                "demand setup renewed or exceeded its short absolute budget"
            );
            assert_eq!(service.readiness().snapshot()[0]["state"], "starting");
            let joining =
                scope.spawn(|| hook.prepare("alpha", Instant::now() + limits::MOUNT_RESPONSE));
            let (client, response) = waiting.join().unwrap();
            assert!(!joining.join().unwrap());
            assert_eq!(hook.queued(), 0, "a joiner queued a successor attempt");
            // The caller has spent the 250 ms demand budget, while the built candidate
            // remains held before its installation fence. Page sync still gets its 101.
            assert!(response.starts_with("HTTP/1.1 101"));
            assert!(values(&ext.head(0), "tmt-origin").is_empty());
            release.send(()).unwrap();
            drop(client);
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !hook.idle() {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        assert!(service.active("alpha").is_none());
        assert_eq!(origins.count(), 0);
        assert_eq!(
            service.locked().slots[0].failure,
            Some(ActivateError::Channel(Fault::Timeout(
                tmt_extension_objects::Stage::Head
            )))
        );
    });
    service.shutdown();
    drop(live);
    drop(ext);
}

#[test]
fn initial_absolute_deadline_is_captured_and_later_upgrade_reactivates() {
    let env = Env::new();
    let origins = Origins::default();
    let service = env.service_with(&ALPHA, bounds(), origins.clone()).unwrap();
    let hook = service.reactivation(Arc::new(AtomicBool::new(false)));
    let (live, _) = Live::with_hook(&env, &origins, |_| Tabs::new(), Some(hook.clone()));
    let held = Ext::launch_mode(&env, &live, false, true);
    let (captured, observed) = mpsc::sync_channel(1);
    service.set_hook(Some(Arc::new(move |phase, deadline| {
        if phase == Pause::BeforeSetup {
            captured.send(deadline).unwrap();
        }
    })));
    let deadline = Instant::now() + limits::OBJECT_REACTIVATION;
    assert_eq!(
        service.activate_until(live.mounts(), "alpha", deadline),
        Err(ActivateError::Channel(Fault::Timeout(
            tmt_extension_objects::Stage::Head
        )))
    );
    assert_eq!(
        observed.recv_timeout(Duration::from_secs(10)).unwrap(),
        deadline,
        "setup renewed the initial budget"
    );
    assert_eq!(held.attempts.load(Ordering::SeqCst), 1);
    assert!(service.active("alpha").is_none());
    assert_eq!(origins.count(), 0);
    service.set_hook(None);
    drop(held);
    let ext = Ext::start(&env, &live);
    service.with_reactivation(&hook, live.mounts(), || {
        let (client, reply) = live.upgrade(OWNER, "");
        assert!(reply.starts_with("HTTP/1.1 101"));
        let origin = origin_of(&ext.head(0));
        ext.await_established(&[origin]);
        assert_eq!(service.readiness().snapshot()[0]["state"], "ready");
        drop(client);
    });
    service.shutdown();
    drop(live);
    drop(ext);
    assert!(!env.directory("alpha").join("door.sock").exists());
}
