//! Real-socket acceptance of the owner browser reads (`x/-/ops`, `x/-/digest`): the composed
//! shell's admission, the exact query grammar, and that a rejected request costs no read.
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tmt_remote::{
    http::{Door, Handler},
    mount::{Admitted, DeviceContext, EXTENSIONS, Mounts, NoSessions, Sessions},
    pages::Pages,
    reads::Reads,
    routes::Routes,
    site::Site,
};
#[path = "support/executable_fixture.rs"]
mod executable_fixture;

const OWNER: &str = "tmt_door=owner";
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn context(revision: u64) -> DeviceContext {
    DeviceContext {
        device_id: "00000000-0000-4000-8000-000000000004".into(),
        kind: "browser".into(),
        origin: "http://127.0.0.1:1".into(),
        name: "Laptop".into(),
        public_key: [7; 32],
        grant_revision: revision,
    }
}
/// One cookie is the owner; `change_after` makes the Nth lookup report a changed grant, and
/// `end_after` makes later lookups report no session, as a revocation during the read would.
struct Owner {
    lookups: AtomicUsize,
    change_after: Option<usize>,
    end_after: Option<usize>,
}
impl Owner {
    fn steady() -> Arc<Self> {
        Arc::new(Self {
            lookups: AtomicUsize::new(0),
            change_after: None,
            end_after: None,
        })
    }
}
impl Sessions for Owner {
    fn context(&self, cookie: Option<&str>) -> Option<Admitted> {
        if cookie != Some(OWNER) {
            return None;
        }
        let seen = self.lookups.fetch_add(1, Ordering::SeqCst);
        if self.end_after.is_some_and(|n| seen >= n) {
            return None;
        }
        let revision = if self.change_after.is_some_and(|n| seen >= n) {
            4
        } else {
            3
        };
        Some(Admitted {
            context: context(revision),
            session: Arc::default(),
        })
    }
}

struct Fixture {
    root: PathBuf,
    addr: SocketAddr,
    prefix: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Fixture {
    fn new(sessions: Arc<dyn Sessions>, serve_reads: bool) -> Self {
        let root = PathBuf::from(format!(
            "/tmp/tmt-4325-door-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        executable_fixture::write_executable(
            &root.join("tmt"),
            &format!(
                "cd '{0}'\necho \"$*\" >> calls\n\
                 case \"$*\" in\n\
                 'ops squad ls --json') printf '{{\"squads\":[]}}';;\n\
                 'ops squad cron ls --json') printf '{{\"jobs\":[]}}';;\n\
                 'ops limits --json') printf '{{\"version\":1,\"limitHistory\":[{{\"provider\":\"codex\"}}]}}';;\n\
                 'digest usage report --since 7d --json') printf '{{\"version\":1,\"since\":\"7d\"}}';;\n\
                 'digest usage report --since 24h --json') printf '{{\"version\":1,\"since\":\"24h\"}}';;\n\
                 *) exit 3;;\nesac\n",
                root.display()
            ),
        )
        .unwrap();
        let routes = Routes::new(1024, "/r/k7qxm4tz2pbwn6rh".into()).unwrap();
        let prefix = routes.prefix().to_owned();
        let door = Door::bind(0).unwrap();
        let addr = door.socket_addr().unwrap();
        let origin = door.origin.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let mut mounts =
            Mounts::with_extensions(root.clone(), &origin, &prefix, sessions, &EXTENSIONS);
        if serve_reads {
            mounts = mounts.with_reads(Reads::new(root.join("tmt"), Arc::clone(&stop)));
        }
        let site = Arc::new(Site::new(
            routes,
            Arc::new(mounts),
            Some(Pages::new(
                &origin,
                "machine".into(),
                "window".into(),
                &prefix,
            )),
        ));
        let flag = Arc::clone(&stop);
        let thread = thread::spawn(move || door.run(&flag, site as Arc<dyn Handler>).unwrap());
        Self {
            root,
            addr,
            prefix,
            stop,
            thread: Some(thread),
        }
    }
    fn send(&self, method: &str, path: &str, headers: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(self.addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        write!(
            stream,
            "{method} {}{path} HTTP/1.1\r\nHost: {}\r\n{headers}\r\n",
            self.prefix, self.addr
        )
        .unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).unwrap();
        let reply = String::from_utf8_lossy(&reply).into_owned();
        let status = reply[9..12].parse().unwrap();
        let body = reply
            .split_once("\r\n\r\n")
            .map_or(String::new(), |(_, b)| b.to_owned());
        (status, body)
    }
    fn get(&self, path: &str) -> (u16, String) {
        self.send("GET", path, &format!("Cookie: {OWNER}\r\n"))
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("calls"))
            .map(|calls| calls.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn an_owner_session_reads_the_board_and_the_requested_usage_range() {
    let door = Fixture::new(Owner::steady(), true);
    let (status, body) = door.get("/x/-/ops");
    assert_eq!(status, 200, "{body}");
    let board: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (board["version"].clone(), board["state"].clone()),
        (1.into(), "ok".into())
    );
    assert_eq!(board["squads"], serde_json::json!([]));
    let (status, body) = door.get("/x/-/limits");
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["limitHistory"][0]["provider"],
        "codex"
    );
    let (status, body) = door.get("/x/-/digest/24h");
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["since"],
        "24h"
    );
    let (status, body) = door.get("/x/-/digest/7d");
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["since"],
        "7d"
    );
}

#[test]
fn the_reply_is_json_with_the_door_security_headers() {
    let door = Fixture::new(Owner::steady(), true);
    let mut stream = TcpStream::connect(door.addr).unwrap();
    write!(
        stream,
        "GET {}/x/-/ops HTTP/1.1\r\nHost: {}\r\nCookie: {OWNER}\r\n\r\n",
        door.prefix, door.addr
    )
    .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    let head = reply.split_once("\r\n\r\n").unwrap().0.to_ascii_lowercase();
    assert!(head.contains("content-type: application/json"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
    assert!(head.contains("x-content-type-options: nosniff"), "{head}");
}

#[test]
fn without_a_live_owner_session_every_read_is_the_generic_404_and_runs_nothing() {
    let door = Fixture::new(Arc::new(NoSessions), true);
    for path in [
        "/x/-/ops",
        "/x/-/digest/24h",
        "/x/-/digest/7d",
        "/x/-/digest/bad",
    ] {
        let (status, body) = door.send("GET", path, &format!("Cookie: {OWNER}\r\n"));
        assert_eq!((status, body.as_str()), (404, "{}"), "{path}");
        let (status, _) = door.send("GET", path, "");
        assert_eq!(status, 404, "{path}");
    }
    assert!(door.calls().is_empty(), "{:?}", door.calls());
}

#[test]
fn cross_site_and_non_get_requests_are_refused_before_any_read() {
    let door = Fixture::new(Owner::steady(), true);
    let (status, _) = door.send(
        "GET",
        "/x/-/ops",
        &format!("Cookie: {OWNER}\r\nSec-Fetch-Site: cross-site\r\n"),
    );
    assert_eq!(status, 403);
    let (status, _) = door.send(
        "GET",
        "/x/-/ops",
        &format!("Cookie: {OWNER}\r\nOrigin: http://evil.example\r\n"),
    );
    assert_eq!(status, 403);
    for method in ["POST", "PUT", "DELETE", "PATCH"] {
        let (status, _) = door.send(
            method,
            "/x/-/ops",
            &format!("Cookie: {OWNER}\r\nContent-Length: 0\r\n"),
        );
        assert_eq!(status, 404, "{method}");
    }
    let (status, _) = door.send(
        "GET",
        "/x/-/ops",
        &format!("Cookie: {OWNER}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n"),
    );
    assert_eq!(status, 404);
    assert!(door.calls().is_empty(), "{:?}", door.calls());
}

#[test]
fn an_unknown_range_is_the_generic_404_and_a_query_is_the_edge_framing_refusal() {
    let door = Fixture::new(Owner::steady(), true);
    for path in ["/x/-/digest", "/x/-/digest/1h", "/x/-/digest/7d/x"] {
        let (status, body) = door.get(path);
        assert_eq!((status, body.as_str()), (404, "{}"), "{path}");
    }
    // The edge target grammar allows no query outside its two fixed forms, before any session.
    for path in ["/x/-/digest?since=7d", "/x/-/ops?x=1"] {
        assert_eq!(door.get(path).0, 400, "{path}");
        assert_eq!(door.send("GET", path, "").0, 400, "{path}");
    }
    assert!(door.calls().is_empty(), "{:?}", door.calls());
}

#[test]
fn a_session_revoked_or_regranted_during_the_read_gets_the_generic_404_not_the_data() {
    for (change_after, end_after) in [(None, Some(1)), (Some(1), None)] {
        let door = Fixture::new(
            Arc::new(Owner {
                lookups: AtomicUsize::new(0),
                change_after,
                end_after,
            }),
            true,
        );
        let (status, body) = door.get("/x/-/ops");
        assert_eq!(
            (status, body.as_str()),
            (404, "{}"),
            "{change_after:?} {end_after:?}"
        );
        // The read itself ran once; only its answer was withheld.
        assert_eq!(door.calls().len(), 2, "{:?}", door.calls());
    }
}

#[test]
fn without_a_configured_read_source_the_routes_do_not_exist_and_other_mounts_are_unchanged() {
    let door = Fixture::new(Owner::steady(), false);
    for path in ["/x/-/ops", "/x/-/digest/24h", "/x/-/digest/7d"] {
        assert_eq!(door.get(path).0, 404, "{path}");
    }
    assert_eq!(door.get("/x/-/shell").0, 200);
    assert!(door.calls().is_empty());
}

#[test]
fn only_the_named_reads_exist_under_the_reserved_segment() {
    let door = Fixture::new(Owner::steady(), true);
    for path in [
        "/x/-/limits/",
        "/x/-/Limits",
        "/x/-/ops/",
        "/x/-/digest/",
        "/x/-/Ops",
        "/x/-/",
        "/x/-",
    ] {
        assert_eq!(door.get(path).0, 404, "{path}");
    }
    assert!(door.calls().is_empty(), "{:?}", door.calls());
}
