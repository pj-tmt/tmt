//! Origins over a real door: a browser-side TCP client upgrades through `Mounts` to a
//! fixture extension on its owner-only socket, which also answers the private object
//! channel. The extension sees exactly what Remote forwards, and its bus is told, in
//! order, when each tunnel is established and closed. The registry's own rules (terminal
//! close, bounds, lock discipline) are tested directly.
use super::super::origins::{Events, Next};
use super::*;
use crate::{
    http::{Door, Handler},
    mount::{Admitted, DeviceContext, SessionState, Sessions},
    routes::Routes,
    site::Site,
};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
};
use tmt_extension_objects::{OriginPhase, OriginState, accept_head};

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
        let routes = Routes::new(1024, "/r/k7qxm4tz2pbwn6rh".into()).unwrap();
        let prefix = routes.prefix().to_owned();
        let door = Door::bind(0).unwrap();
        let (addr, origin) = (door.socket_addr().unwrap(), door.origin.clone());
        let mounts =
            Mounts::with_extensions(env.root.clone(), &origin, &prefix, Tabs::new(), &ALPHA)
                .with_origins(Arc::new(origins.clone()));
        let site = Arc::new(Site {
            routes,
            mounts: Arc::new(mounts),
            pages: None,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, served) = (Arc::clone(&stop), Arc::clone(&site));
        let door = thread::spawn(move || door.run(&flag, served as Arc<dyn Handler>).unwrap());
        Self {
            addr,
            origin,
            prefix,
            site,
            stop,
            door: Some(door),
        }
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
    path: PathBuf,
    stop: Arc<AtomicBool>,
    heads: Arc<Mutex<Vec<String>>>,
    buses: Arc<Mutex<Vec<Bus>>>,
    thread: Option<JoinHandle<()>>,
}
impl Ext {
    fn start(env: &Env, live: &Live) -> Self {
        let path = env.directory("alpha").join("door.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
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
                connections.push(thread::spawn(move || {
                    serve_connection(stream, &expect, &seen, &held);
                }));
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
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
        let started = Instant::now();
        loop {
            if let Some(bus) = self.buses.lock().unwrap().first() {
                return f(bus);
            }
            assert!(started.elapsed() < Duration::from_secs(10), "no bus");
            thread::yield_now();
        }
    }
    /// The next origin-state notice on the object channel.
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
        let setup = Instant::now() + Duration::from_secs(5);
        if let Ok(link) = accept_head(stream, &head, &[], expect, &Budgets::contract(), setup) {
            let bus = Bus::start(link, Budgets::contract(), Caps::contract(), None).unwrap();
            held.lock().unwrap().push(bus);
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
