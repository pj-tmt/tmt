//! Real-socket mount acceptance: a fixture extension serves an owner-only Unix
//! socket under an isolated data root; the door forwards `<prefix>/x/colab/`
//! to it.
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tmt_remote::{
    devices::{Devices, event_json},
    http::{Door, Handler, Head},
    limits,
    mount::{
        Admitted, DeviceContext, EXTENSIONS, Extension, Mounts, NoSessions, ObjectDeclaration,
        Sessions,
    },
    pages::Pages,
    routes::Routes,
    site::Site,
    state::Layout,
    store::{DEFAULT_SCOPES, Grant, Store},
};

const OWNER: &str = "tmt_door=owner";
/// Test session resolver: one cookie value is an owner device.
struct OneOwner;
impl Sessions for OneOwner {
    fn context(&self, cookie: Option<&str>) -> Option<Admitted> {
        (cookie == Some(OWNER)).then(|| Admitted {
            context: DeviceContext {
                device_id: "00000000-0000-4000-8000-000000000004".into(),
                kind: "browser".into(),
                origin: "http://127.0.0.1:1".into(),
                name: "Laptop \u{e9}\"\n".into(),
                public_key: [7; 32],
                grant_revision: 3,
            },
            session: Arc::default(),
        })
    }
}

type Behavior = Arc<dyn Fn(UnixStream, &Recorder) + Send + Sync>;
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Vec<u8>>>>);
impl Recorder {
    fn push(&self, bytes: Vec<u8>) {
        self.0.lock().unwrap().push(bytes);
    }
    fn all(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .collect()
    }
}
/// Fixture extension process stand-in on a real Unix socket.
struct Fixture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    seen: Recorder,
}
impl Fixture {
    fn serve(socket: &Path, behavior: Behavior) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        fs::set_permissions(socket, fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Recorder::default();
        let (flag, record) = (Arc::clone(&stop), seen.clone());
        // One thread per connection, so concurrent tunnels are served at once.
        let thread = thread::spawn(move || {
            let mut connections = Vec::new();
            while !flag.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let (behavior, record) = (Arc::clone(&behavior), record.clone());
                        connections.push(thread::spawn(move || behavior(stream, &record)));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            stop,
            thread: Some(thread),
            seen,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}
/// Read one request head plus its declared body.
fn request(stream: &mut UnixStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .map_or(0, |v| v.parse::<usize>().unwrap());
            if bytes.len() >= end + 4 + length {
                return bytes;
            }
        }
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0, "request ended early");
        bytes.extend_from_slice(&chunk[..n]);
    }
}
fn replying(reply: &'static [u8]) -> Behavior {
    Arc::new(move |mut stream, seen| {
        seen.push(request(&mut stream));
        let _ = stream.write_all(reply);
    })
}
const PAGE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Security-Policy: default-src 'self'\r\nSet-Cookie: shadow=1; Path=/\r\nX-Ext: yes\r\nContent-Length: 5\r\n\r\nhello";

fn grant() -> Grant {
    Grant {
        client_id: "00000000-0000-4000-8000-000000000004".into(),
        public_key: [7; 32],
        kind: "cli".into(),
        origin: "cli".into(),
        name: "Laptop é".into(),
        agents: "all".into(),
        scopes: DEFAULT_SCOPES.iter().map(|s| (*s).into()).collect(),
        mode: "direct".into(),
        issued_at_ms: 1,
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    }
}
fn received(events: &mpsc::Receiver<String>) -> serde_json::Value {
    let wire = events
        .recv_timeout(Duration::from_secs(10))
        .expect("device event missing");
    let (head, body) = wire.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST /.tmt/remote/device-events HTTP/1.1\r\n"));
    assert!(head.contains("\r\ntmt-device-event: 1\r\n"));
    assert!(head.contains("\r\nContent-Type: application/json\r\n"));
    assert!(!head.to_ascii_lowercase().contains("cookie:"));
    serde_json::from_str(body).unwrap()
}

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Mounted {
    root: PathBuf,
    addr: SocketAddr,
    origin: String,
    prefix: String,
    mounts: Arc<Mounts>,
    stop: Arc<AtomicBool>,
    door: Option<JoinHandle<()>>,
}
impl Mounted {
    fn new(sessions: Arc<dyn Sessions>) -> Self {
        Self::with(sessions, &EXTENSIONS)
    }
    fn with(sessions: Arc<dyn Sessions>, extensions: &'static [Extension]) -> Self {
        // Short absolute root: Unix socket paths are limited to about 100 bytes.
        let root = PathBuf::from(format!(
            "/tmp/tmt-1039-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("colab")).unwrap();
        fs::set_permissions(root.join("colab"), fs::Permissions::from_mode(0o700)).unwrap();
        let routes = Routes::new(1024, "/r/k7qxm4tz2pbwn6rh".into()).unwrap();
        let prefix = routes.prefix().to_owned();
        let door = Door::bind(0).unwrap();
        let addr = door.socket_addr().unwrap();
        let origin = door.origin.clone();
        let site = Arc::new(Site {
            routes,
            mounts: Arc::new(Mounts::with_extensions(
                root.clone(),
                &origin,
                &prefix,
                sessions,
                extensions,
            )),
            pages: Some(Pages::new(
                &origin,
                "fixture-machine".into(),
                "fixture-window".into(),
                &prefix,
            )),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let mounts = Arc::clone(&site.mounts);
        let flag = Arc::clone(&stop);
        let door = thread::spawn(move || door.run(&flag, site as Arc<dyn Handler>).unwrap());
        Self {
            root,
            addr,
            origin,
            prefix,
            mounts,
            stop,
            door: Some(door),
        }
    }
    fn socket(&self) -> PathBuf {
        self.root.join("colab/door.sock")
    }
    fn extension(&self, behavior: Behavior) -> Fixture {
        Fixture::serve(&self.socket(), behavior)
    }
    /// `path` inside the mount space, `<prefix><path>`.
    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.prefix)
    }
    fn get(&self, path: &str, headers: &str) -> String {
        format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\n{headers}\r\n",
            self.addr
        )
    }
    fn send(&self, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(self.addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        stream.write_all(request).unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).unwrap();
        String::from_utf8_lossy(&reply).into_owned()
    }
    fn status(&self, request: &str) -> u16 {
        self.send(request.as_bytes())[9..12].parse().unwrap()
    }
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(door) = self.door.take() {
            door.join().unwrap();
        }
        assert!(TcpStream::connect(self.addr).is_err(), "listener leaked");
    }
}
impl Drop for Mounted {
    fn drop(&mut self) {
        self.stop();
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn forwards_to_owner_socket_without_device_context_or_cookie() {
    let door = Mounted::new(Arc::new(NoSessions));
    let extension = door.extension(replying(PAGE));
    let reply = door.send(
        door.get(
            &door.at("/x/colab/app/index.html"),
            &format!("Accept: text/html\r\nCookie: {OWNER}\r\nTMT-Device-Context: forged\r\nTMT-Device-Event: 1\r\nTMT-Mount: /x/evil/\r\nX-Other: dropped\r\n"),
        )
        .as_bytes(),
    );
    let (head, body) = reply.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"));
    assert_eq!(body, "hello");
    // The extension owns its content type, CSP and own headers; remote fills only absent defaults.
    assert!(head.contains("content-type: text/html\r\n"));
    assert!(head.contains("content-security-policy: default-src 'self'\r\n"));
    assert!(!head.contains("default-src 'none'"));
    assert!(head.contains("x-ext: yes\r\n"));
    assert!(head.contains("x-content-type-options: nosniff\r\n"));
    assert!(!head.to_ascii_lowercase().contains("set-cookie"));
    let seen = extension.seen.all();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0],
        format!(
            "GET /app/index.html HTTP/1.1\r\nhost: {}\r\ntmt-mount: {}/x/colab/\r\naccept: text/html\r\nconnection: close\r\ncontent-length: 0\r\n\r\n",
            door.addr, door.prefix
        )
    );
}

#[test]
fn owner_session_forwards_exact_device_context_and_body() {
    let door = Mounted::new(Arc::new(OneOwner));
    let extension = door.extension(replying(PAGE));
    let body = "{\"page\":1}";
    let post = format!(
        "POST {}/x/colab/api HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nCookie: {OWNER}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        door.prefix,
        door.addr,
        door.origin,
        body.len()
    );
    assert_eq!(door.status(&post), 200);
    let seen = extension.seen.all();
    let context = concat!(
        r#"{"deviceId":"00000000-0000-4000-8000-000000000004","kind":"browser","#,
        r#""origin":"http://127.0.0.1:1","name":"Laptop \u00e9\"\n","#,
        r#""publicKey":"BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc","owner":true,"#,
        r#""grantRevision":3}"#
    );
    assert_eq!(
        seen[0],
        format!(
            "POST /api HTTP/1.1\r\nhost: {}\r\ntmt-mount: {}/x/colab/\r\norigin: {}\r\ncontent-type: application/json\r\ntmt-device-context: {context}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
            door.addr,
            door.prefix,
            door.origin,
            body.len()
        )
    );
    let value: serde_json::Value = serde_json::from_str(context).unwrap();
    assert_eq!(value["name"], "Laptop \u{e9}\"\n");
    // A non-owner request on the same mount carries no context.
    assert_eq!(door.status(&post.replace(OWNER, "tmt_door=other")), 200);
    assert!(!extension.seen.all()[1].contains("tmt-device-context"));
}

#[test]
fn reserved_device_events_never_forward_even_with_a_forged_marker() {
    let door = Mounted::new(Arc::new(OneOwner));
    let extension = door.extension(replying(PAGE));
    for path in [
        "/.tmt",
        "/.tmt/",
        "/.tmt/remote/device-events",
        "/.tmt/other",
        "/%2Etmt/remote/device-events",
        "/%2etmt/remote/device-events",
        "//.tmt/remote/device-events",
        "/./.tmt/remote/device-events",
        "/app/../other",
        "/app/%2e%2E/other",
    ] {
        for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
            let wire = format!(
                "{method} {}/x/colab{path} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nCookie: {OWNER}\r\nTMT-Device-Event: 1\r\nContent-Length: 2\r\n\r\n{{}}",
                door.prefix, door.addr, door.origin
            );
            let target = door.at(&format!("/x/colab{path}"));
            let head = Head {
                method,
                path: &target,
                origin: Some(&door.origin),
                cookie: Some(OWNER),
                content_type: None,
                content_length: Some(2),
                upgrade: false,
            };
            assert_eq!(door.mounts.admit(&head).unwrap_err().status, 404);
            // The edge also rejects encoded, empty and dot segments as invalid
            // framing; the mount guard must independently refuse these paths.
            let status = if path.contains('%')
                || path.contains("//")
                || path.contains("/./")
                || path.contains("/../")
            {
                400
            } else {
                404
            };
            assert_eq!(door.status(&wire), status);
        }
    }
    assert!(extension.seen.all().is_empty());
    // Positive control: the marker is stripped on an otherwise admitted request.
    assert_eq!(
        door.status(&door.get(&door.at("/x/colab/app"), "TMT-Device-Event: 1\r\n")),
        200
    );
    assert!(
        !extension.seen.all()[0]
            .to_ascii_lowercase()
            .contains("tmt-device-event")
    );
}

#[test]
fn only_2xx_device_replies_acknowledge_and_unsafe_sockets_receive_nothing() {
    let door = Mounted::new(Arc::new(NoSessions));
    assert!(!door.mounts.device_event(&event_json(&grant())));
    for reply in [
        b"HTTP/1.1 204 No Content\r\n\r\n".as_slice(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".as_slice(),
        b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n".as_slice(),
        b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".as_slice(),
        b"invalid reply\r\n\r\n".as_slice(),
    ] {
        let extension = door.extension(replying(reply));
        assert_eq!(
            door.mounts.device_event(&event_json(&grant())),
            reply.starts_with(b"HTTP/1.1 2")
        );
        let count = extension.seen.all().len();
        fs::set_permissions(door.socket(), fs::Permissions::from_mode(0o666)).unwrap();
        assert!(!door.mounts.device_event(&event_json(&grant())));
        assert_eq!(extension.seen.all().len(), count);
        fs::set_permissions(door.socket(), fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(door.root.join("colab"), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!door.mounts.device_event(&event_json(&grant())));
        assert_eq!(extension.seen.all().len(), count);
        fs::set_permissions(door.root.join("colab"), fs::Permissions::from_mode(0o700)).unwrap();
        drop(extension);
        fs::remove_file(door.socket()).unwrap();
    }
}

#[test]
fn durable_device_events_retry_rename_revoke_and_replay_after_restarts() {
    for _ in 0..2 {
        let door = Mounted::new(Arc::new(NoSessions));
        let layout = Layout::open(&door.root).unwrap();
        let serving = layout.serve_lock().unwrap();
        let store = Arc::new(Mutex::new(Store::open(&serving).unwrap()));
        let original = grant();
        store.lock().unwrap().insert_grant(&original).unwrap();
        let devices = Arc::new(Devices::new(Arc::clone(&store), None));
        let (sent, events) = mpsc::channel();
        let attempts = Arc::new(AtomicUsize::new(0));
        let extension = door.extension(Arc::new(move |mut stream, seen| {
            let wire = request(&mut stream);
            seen.push(wire.clone());
            let reply = if attempts.fetch_add(1, Ordering::Relaxed) == 0 {
                "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n"
            } else {
                "HTTP/1.1 204 No Content\r\n\r\n"
            };
            stream.write_all(reply.as_bytes()).unwrap();
            sent.send(String::from_utf8(wire).unwrap()).unwrap();
        }));
        let worker = devices.start_events(Arc::clone(&door.mounts)).unwrap();
        let initial = serde_json::json!({"type":"device.renamed", "deviceId":original.client_id,"grantRevision":1,"name":"Laptop é"});
        assert_eq!(received(&events), initial);
        assert_eq!(received(&events), initial); // 503 must retry, not acknowledge.
        devices.rename(&original.client_id, "Travel").unwrap();
        assert_eq!(
            received(&events),
            serde_json::json!({"type":"device.renamed","deviceId":original.client_id,"grantRevision":2,"name":"Travel"})
        );
        devices.revoke(&original.client_id).unwrap();
        let revoked = serde_json::json!({"type":"device.revoked","deviceId":original.client_id,"grantRevision":3});
        assert_eq!(received(&events), revoked);
        assert_eq!(devices.revoke(&original.client_id).unwrap().revision, 3);
        assert_eq!(received(&events), revoked);
        drop(worker);
        drop(extension);
        fs::remove_file(door.socket()).unwrap();
        // Extension restart alone recovers through periodic replay, with no mutation.
        let (sent, events) = mpsc::channel();
        let extension = door.extension(Arc::new(move |mut stream, _| {
            let wire = request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
                .unwrap();
            sent.send(String::from_utf8(wire).unwrap()).unwrap();
        }));
        let worker = devices.start_events(Arc::clone(&door.mounts)).unwrap();
        assert_eq!(received(&events), revoked);
        drop(extension);
        fs::remove_file(door.socket()).unwrap();
        let (sent, events) = mpsc::channel();
        let extension = door.extension(Arc::new(move |mut stream, _| {
            let wire = request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
                .unwrap();
            sent.send(String::from_utf8(wire).unwrap()).unwrap();
        }));
        assert_eq!(received(&events), revoked);
        drop(worker);
        drop(devices);
        drop(store);
        // No journal is needed when remote reopens its actual database.
        let devices = Arc::new(Devices::new(
            Arc::new(Mutex::new(Store::open(&serving).unwrap())),
            None,
        ));
        let worker = devices.start_events(Arc::clone(&door.mounts)).unwrap();
        assert_eq!(received(&events), revoked);
        drop(worker);
        drop(extension);
    }
}

#[test]
fn an_unresponsive_extension_cannot_block_mutation_or_worker_shutdown() {
    for _ in 0..2 {
        let door = Mounted::new(Arc::new(NoSessions));
        let layout = Layout::open(&door.root).unwrap();
        let serving = layout.serve_lock().unwrap();
        let store = Arc::new(Mutex::new(Store::open(&serving).unwrap()));
        let original = grant();
        store.lock().unwrap().insert_grant(&original).unwrap();
        let devices = Arc::new(Devices::new(Arc::clone(&store), None));
        let (sent, events) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let gate = Mutex::new(gate);
        let extension = door.extension(Arc::new(move |mut stream, _| {
            let wire = request(&mut stream);
            sent.send(String::from_utf8(wire).unwrap()).unwrap();
            gate.lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }));
        let worker = devices.start_events(Arc::clone(&door.mounts)).unwrap();
        received(&events); // Callback is held, with no response yet.
        assert!(store.try_lock().is_ok(), "callback retained the grant lock");
        assert_eq!(
            devices
                .rename(&original.client_id, "Travel")
                .unwrap()
                .revision,
            2
        );
        assert_eq!(devices.revoke(&original.client_id).unwrap().revision, 3);
        assert!(devices.list().unwrap()[0].disabled);
        let began = Instant::now();
        drop(worker);
        assert!(
            began.elapsed() < Duration::from_secs(3),
            "worker leaked behind reply/backoff"
        );
        release.send(()).unwrap();
        drop(extension);
    }
}

#[test]
fn origin_method_and_prefix_isolation_refuse_before_forwarding() {
    let mut door = Mounted::new(Arc::new(OneOwner));
    let extension = door.extension(replying(PAGE));
    let origin = format!("Origin: {}\r\n", door.origin);
    let post = |headers: &str| {
        format!(
            "POST {}/x/colab/api HTTP/1.1\r\nHost: {}\r\n{headers}Content-Length: 2\r\n\r\n{{}}",
            door.prefix, door.addr
        )
    };
    assert_eq!(door.status(&post(&origin)), 200);
    for (request, status) in [
        (post(""), 403),
        (post("Origin: null\r\n"), 403),
        (post("Origin: http://localhost:1\r\n"), 403),
        (
            door.get(&door.at("/x/colab/"), "Origin: https://evil.invalid\r\n"),
            403,
        ),
        (door.get(&door.at("/x/colab"), ""), 404),
        (door.get(&door.at("/x/Colab/"), ""), 404),
        (door.get(&door.at("/x/other/"), ""), 404),
        (door.get(&door.at("/x/"), ""), 404),
        // The old root mount space is gone, with no redirect revealing the prefix.
        (door.get("/x/colab/", ""), 404),
        (door.get("/x/", ""), 404),
        (door.get(&door.at("/x/colab/../../r/x/append"), ""), 400),
        (door.get(&door.at("/x/colab//a"), ""), 400),
        (
            format!(
                "OPTIONS {}/x/colab/ HTTP/1.1\r\nHost: {}\r\n{origin}\r\n",
                door.prefix, door.addr
            ),
            404,
        ),
        (
            format!(
                "POST {}/x/colab/api HTTP/1.1\r\nHost: {}\r\n{origin}Content-Length: 65537\r\n\r\n",
                door.prefix, door.addr
            ),
            413,
        ),
    ] {
        assert_eq!(door.status(&request), status, "{request:.80}");
    }
    // The remote binding never reaches the extension, and /x/ never reaches /r/.
    let binding = format!(
        "POST {}/append HTTP/1.1\r\nHost: {}\r\n{origin}Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}",
        door.prefix, door.addr
    );
    assert_eq!(door.status(&binding), 404);
    assert_eq!(
        door.status(&door.get(&door.at(&format!("/x/colab{}/append", door.prefix)), "")),
        200
    );
    let seen = extension.seen.all();
    assert_eq!(
        seen.len(),
        2,
        "only the two admitted mount requests forwarded"
    );
    assert!(seen[1].starts_with(&format!("GET {}/append HTTP/1.1", door.prefix)));
    // Under the prefix, operation routes and the mount space stay disjoint:
    // an operation route still refuses any cookie, and reaches no extension.
    let with_cookie = binding.replace("Content-Type", &format!("Cookie: {OWNER}\r\nContent-Type"));
    assert_eq!(door.status(&with_cookie), 400);
    assert_eq!(extension.seen.all().len(), 2);
    drop(extension);
    door.stop();
}

#[test]
fn unsafe_or_missing_sockets_are_not_mounted() {
    let door = Mounted::new(Arc::new(NoSessions));
    let get = door.get(&door.at("/x/colab/"), "");
    assert_eq!(door.status(&get), 404, "no socket");
    {
        let _extension = door.extension(replying(PAGE));
        assert_eq!(door.status(&get), 200, "positive control");
        fs::set_permissions(door.socket(), fs::Permissions::from_mode(0o660)).unwrap();
        assert_eq!(door.status(&get), 404, "group-accessible socket");
        fs::set_permissions(door.socket(), fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(door.root.join("colab"), fs::Permissions::from_mode(0o750)).unwrap();
        assert_eq!(door.status(&get), 404, "group-accessible directory");
        fs::set_permissions(door.root.join("colab"), fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(door.status(&get), 200, "restored");
    }
    // A stale socket file with no listener is refused as unavailable transport.
    assert_eq!(door.status(&get), 503);
    fs::remove_file(door.socket()).unwrap();
    // A symlinked socket or directory is never followed.
    let elsewhere = door.root.join("real");
    fs::create_dir(&elsewhere).unwrap();
    fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o700)).unwrap();
    let _extension = Fixture::serve(&elsewhere.join("door.sock"), replying(PAGE));
    std::os::unix::fs::symlink(elsewhere.join("door.sock"), door.socket()).unwrap();
    assert_eq!(door.status(&get), 404, "symlinked socket");
    fs::remove_file(door.socket()).unwrap();
    fs::remove_dir(door.root.join("colab")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, door.root.join("colab")).unwrap();
    assert_eq!(door.status(&get), 404, "symlinked directory");
    fs::remove_file(door.root.join("colab")).unwrap();
    fs::create_dir(door.root.join("colab")).unwrap();
}

#[test]
fn malformed_extension_replies_are_bad_gateway() {
    for reply in [
        b"HTTP/1.1 200 OK\r\nCache-Control: public\r\nCache-Control: private\r\nContent-Length: 1\r\n\r\nx",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".as_slice(),
        b"HTTP/1.1 200 OK\r\n\r\nunframed",
        b"HTTP/1.1 100 Continue\r\nContent-Length: 0\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx",
        b"HTTP/1.1 200 OK\r\nContent-Length: 16777217\r\n\r\n",
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n",
        b"garbage\r\n\r\n",
        b"",
    ] {
        let door = Mounted::new(Arc::new(NoSessions));
        let _extension = door.extension(replying(reply));
        assert_eq!(
            door.status(&door.get(&door.at("/x/colab/"), "")),
            502,
            "{}",
            String::from_utf8_lossy(reply)
        );
    }
    let door = Mounted::new(Arc::new(NoSessions));
    let _extension = door.extension(replying(b"HTTP/1.1 204 No Content\r\n\r\n"));
    assert_eq!(door.status(&door.get(&door.at("/x/colab/"), "")), 204);
    // After the head is streamed, an early extension close yields a body
    // shorter than its declared length, which the client detects.
    let door = Mounted::new(Arc::new(NoSessions));
    let _extension = door.extension(replying(
        b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nshort",
    ));
    let reply = door.send(door.get(&door.at("/x/colab/"), "").as_bytes());
    assert!(reply.contains("content-length: 9\r\n"));
    assert!(reply.ends_with("\r\n\r\nshort"));
}

#[test]
fn large_replies_stream_exactly() {
    let door = Mounted::new(Arc::new(NoSessions));
    let body: Vec<u8> = (0..=255u8).cycle().take(1024 * 1024 + 7).collect();
    let expected = body.clone();
    let _extension = door.extension(Arc::new(move |mut stream, seen| {
        seen.push(request(&mut stream));
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&body);
    }));
    let mut stream = TcpStream::connect(door.addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .write_all(door.get(&door.at("/x/colab/blob"), "").as_bytes())
        .unwrap();
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).unwrap();
    let end = reply.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    assert_eq!(reply.len() - end, expected.len(), "streamed length");
    assert!(&reply[end..] == expected.as_slice(), "streamed bytes");
}

const UPGRADE: &str = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: colab-sync-v1\r\n";
const SWITCH: &[u8] = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\nSec-WebSocket-Protocol: colab-sync-v1\r\n\r\n";
/// Echo extension: switch protocols, then return every byte it receives.
fn echo(closed: mpsc::Sender<Vec<u8>>) -> Behavior {
    Arc::new(move |mut stream, seen| {
        seen.push(request(&mut stream));
        stream.write_all(SWITCH).unwrap();
        stream.write_all(b"early").unwrap();
        let mut received = Vec::new();
        let mut chunk = [0; 4096];
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        while let Ok(n) = stream.read(&mut chunk) {
            if n == 0 {
                break;
            }
            received.extend_from_slice(&chunk[..n]);
            if stream.write_all(&chunk[..n]).is_err() {
                break;
            }
        }
        closed.send(received).unwrap();
    })
}

#[test]
fn websocket_subprotocol_offer_and_selected_value_pass_through_unchanged() {
    const READER: &str = "colab-reader-v1.Token_012-AbC";
    // Preserve internal whitespace as well as every offered value and its order.
    let offer = format!("colab-sync-v1,  {READER}");
    for selected in ["colab-sync-v1", READER] {
        let door = Mounted::new(Arc::new(NoSessions));
        let extension = door.extension(Arc::new(move |mut stream, seen| {
            seen.push(request(&mut stream));
            let reply = String::from_utf8(SWITCH.to_vec())
                .unwrap()
                .replace("colab-sync-v1", selected);
            stream.write_all(reply.as_bytes()).unwrap();
        }));
        let upgrade = UPGRADE.replace("colab-sync-v1", &offer);
        let reply = door.send(
            door.get(
                &door.at("/x/colab/sync"),
                &format!("Origin: {}\r\n{upgrade}", door.origin),
            )
            .as_bytes(),
        );
        let (head, _) = reply.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 101"));
        let returned: Vec<_> = head
            .split("\r\n")
            .filter_map(|line| line.strip_prefix("sec-websocket-protocol: "))
            .collect();
        assert_eq!(returned, [selected], "only the extension's selected value");
        let seen = extension.seen.all();
        assert_eq!(seen.len(), 1);
        let forwarded: Vec<_> = seen[0]
            .split("\r\n")
            .filter_map(|line| line.strip_prefix("sec-websocket-protocol: "))
            .map(str::as_bytes)
            .collect();
        assert_eq!(forwarded, [offer.as_bytes()], "byte-identical offer");
    }
}

#[test]
fn websocket_upgrade_strips_forged_door_headers_with_or_without_owner_session() {
    for cookie in ["tmt_door=other", OWNER] {
        let door = Mounted::new(Arc::new(OneOwner));
        let extension = door.extension(replying(SWITCH));
        let reply = door.send(
            door.get(
                &door.at("/x/colab/sync"),
                &format!(
                    "Origin: {}\r\nCookie: {cookie}\r\n{UPGRADE}TMT-Device-Context: forged\r\nTMT-Device-Event: 1\r\nTMT-Mount: /x/evil/\r\n",
                    door.origin
                ),
            )
            .as_bytes(),
        );
        assert!(reply.starts_with("HTTP/1.1 101"), "upgrade admitted");
        let seen = extension.seen.all();
        assert_eq!(seen.len(), 1);
        assert!(!seen[0].contains("forged"));
        assert!(!seen[0].contains("/x/evil/"));
        assert!(!seen[0].contains("tmt-device-event:"));
        assert!(!seen[0].contains("cookie:"));
        let mounts: Vec<_> = seen[0]
            .split("\r\n")
            .filter_map(|line| line.strip_prefix("tmt-mount: "))
            .collect();
        assert_eq!(mounts, [format!("{}/x/colab/", door.prefix)]);
        let contexts: Vec<_> = seen[0]
            .split("\r\n")
            .filter_map(|line| line.strip_prefix("tmt-device-context: "))
            .collect();
        if cookie == OWNER {
            assert_eq!(contexts.len(), 1, "only the door's owner context");
            let context: serde_json::Value = serde_json::from_str(contexts[0]).unwrap();
            assert_eq!(context["owner"], true);
            assert_eq!(context["deviceId"], "00000000-0000-4000-8000-000000000004");
            assert_eq!(context["grantRevision"], 3);
        } else {
            assert!(contexts.is_empty(), "no context without an owner session");
        }
    }
}

#[test]
fn websocket_bytes_pass_through_unchanged_and_close_with_either_side() {
    let (closed, ended) = mpsc::channel();
    let mut door = Mounted::new(Arc::new(NoSessions));
    let extension = door.extension(echo(closed));
    // Unauthenticated-origin upgrades are refused before any forwarding.
    assert_eq!(
        door.status(&door.get(&door.at("/x/colab/sync"), UPGRADE)),
        403
    );
    let origin = format!("Origin: {}\r\n{UPGRADE}", door.origin);
    assert_eq!(
        door.status(&door.get(
            &door.at("/x/colab/sync"),
            &origin.replace("websocket", "h2c")
        )),
        400
    );
    assert!(extension.seen.all().is_empty());
    let payload: Vec<u8> = (0..=255u8).cycle().take(200_000).collect();
    for round in 0..2 {
        let mut client = TcpStream::connect(door.addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        client
            .write_all(door.get(&door.at("/x/colab/sync"), &origin).as_bytes())
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        let end = loop {
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
            let n = client.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&bytes[..end]).into_owned();
        assert!(head.starts_with("HTTP/1.1 101"));
        assert!(head.contains("sec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
        assert!(head.contains("sec-websocket-protocol: colab-sync-v1\r\n"));
        let mut received = bytes.split_off(end);
        let writer = {
            let mut client = client.try_clone().unwrap();
            let payload = payload.clone();
            thread::spawn(move || client.write_all(&payload).unwrap())
        };
        while received.len() < 5 + payload.len() {
            let n = client.read(&mut chunk).unwrap();
            assert!(n > 0, "tunnel closed early");
            received.extend_from_slice(&chunk[..n]);
        }
        writer.join().unwrap();
        assert_eq!(&received[..5], b"early");
        assert_eq!(&received[5..], payload.as_slice(), "exact bytes both ways");
        if round == 0 {
            // Client close reaches the extension.
            client.shutdown(std::net::Shutdown::Both).unwrap();
            assert_eq!(ended.recv_timeout(Duration::from_secs(5)).unwrap(), payload);
        } else {
            // Door shutdown closes the live tunnel at both ends and joins its worker.
            let start = Instant::now();
            door.stop();
            assert!(start.elapsed() < Duration::from_secs(3));
            assert_eq!(ended.recv_timeout(Duration::from_secs(5)).unwrap(), payload);
            match client.read(&mut chunk) {
                Ok(0) => {}
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
                other => panic!("client tunnel leaked: {other:?}"),
            }
        }
    }
    let seen = extension.seen.all();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].contains("connection: upgrade\r\nupgrade: websocket\r\n"));
    assert!(seen[0].contains("sec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\n"));
    assert!(seen[0].contains(&format!("origin: {}\r\n", door.origin)));
}

/// More tunnels than edge sockets, with a short idle bound for the test.
static WIDE: [Extension; 1] = [Extension {
    name: "colab",
    body_bytes: 64 * 1024,
    reply_bytes: 1024,
    tunnels: limits::SOCKETS + 2,
    tunnel_idle: Duration::from_millis(600),
    objects: ObjectDeclaration::Disabled,
}];
/// Upgrade a new client and return it after the 101 head.
fn tunnel(door: &Mounted) -> TcpStream {
    let mut client = TcpStream::connect(door.addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let origin = format!("Origin: {}\r\n{UPGRADE}", door.origin);
    client
        .write_all(door.get(&door.at("/x/colab/sync"), &origin).as_bytes())
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        client.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 101"));
    let mut early = [0; 5];
    client.read_exact(&mut early).unwrap();
    assert_eq!(&early, b"early");
    client
}
fn closed(client: &mut TcpStream) -> bool {
    matches!(client.read(&mut [0; 1]), Ok(0))
        || matches!(client.read(&mut [0; 1]), Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset)
}

#[test]
fn tunnels_have_their_own_cap_and_idle_bound() {
    let (sender, _ended) = mpsc::channel();
    let door = Mounted::with(Arc::new(NoSessions), &WIDE);
    let _extension = door.extension(echo(sender));
    // More live tunnels than door edge sockets: they no longer occupy them.
    let mut clients: Vec<_> = (0..WIDE[0].tunnels).map(|_| tunnel(&door)).collect();
    let binding = format!(
        "POST {}/append HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}",
        door.prefix, door.addr
    );
    assert_eq!(door.status(&binding), 404, "/r/ keeps its edge sockets");
    // A full pool refuses the next upgrade with a retry hint, before reaching the extension.
    let origin = format!("Origin: {}\r\n{UPGRADE}", door.origin);
    let refused = door.send(door.get(&door.at("/x/colab/sync"), &origin).as_bytes());
    assert!(refused.starts_with("HTTP/1.1 503"));
    assert!(refused.contains(&format!(
        "retry-after: {}\r\n",
        tmt_remote::mount::RETRY_AFTER_SECONDS
    )));
    // An active tunnel outlives the idle bound; idle ones close and free their slots.
    let active = &mut clients[0];
    let start = Instant::now();
    while start.elapsed() < WIDE[0].tunnel_idle * 2 {
        active.write_all(b"ping").unwrap();
        let mut echo = [0; 4];
        active.read_exact(&mut echo).unwrap();
        assert_eq!(&echo, b"ping");
        thread::sleep(WIDE[0].tunnel_idle / 4);
    }
    for client in &mut clients[1..] {
        assert!(closed(client), "idle tunnel left open");
    }
    let mut replacement = tunnel(&door);
    replacement.write_all(b"again").unwrap();
    let mut echo = [0; 5];
    replacement.read_exact(&mut echo).unwrap();
    assert_eq!(&echo, b"again");
}

#[test]
fn cross_site_fetch_metadata_is_refused() {
    let door = Mounted::new(Arc::new(NoSessions));
    let _extension = door.extension(replying(PAGE));
    assert_eq!(
        door.status(&door.get(&door.at("/x/colab/"), "Sec-Fetch-Site: cross-site\r\n")),
        403
    );
    for site in ["same-origin", "none"] {
        assert_eq!(
            door.status(&door.get(
                &door.at("/x/colab/"),
                &format!("Sec-Fetch-Site: {site}\r\n")
            )),
            200
        );
    }
}

#[test]
fn mounted_replies_keep_only_a_referrer_policy_that_hides_the_prefix() {
    for (policy, sent) in [
        (None, "no-referrer"),
        (Some("unsafe-url"), "no-referrer"),
        (Some("origin"), "no-referrer"),
        (Some("same-origin"), "same-origin"),
        (Some("no-referrer"), "no-referrer"),
    ] {
        let door = Mounted::new(Arc::new(NoSessions));
        let header = policy
            .map(|p| format!("Referrer-Policy: {p}\r\n"))
            .unwrap_or_default();
        let reply: &'static [u8] = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{header}Content-Length: 2\r\n\r\nok"
        )
        .into_bytes()
        .leak();
        let _extension = door.extension(replying(reply));
        let response = door.send(door.get(&door.at("/x/colab/"), "").as_bytes());
        let (head, _) = response.split_once("\r\n\r\n").unwrap();
        let policies: Vec<&str> = head
            .lines()
            .filter_map(|l| l.strip_prefix("referrer-policy: "))
            .collect();
        assert_eq!(policies, [sent], "{policy:?}");
    }
}

struct NoPublicSessionLookup;
impl Sessions for NoPublicSessionLookup {
    fn context(&self, _: Option<&str>) -> Option<Admitted> {
        panic!("public entry consulted the session resolver");
    }
}
#[derive(Default)]
struct PublicEntrySessionCalls(AtomicUsize);
impl Sessions for PublicEntrySessionCalls {
    fn context(&self, _: Option<&str>) -> Option<Admitted> {
        self.0.fetch_add(1, Ordering::Relaxed);
        None
    }
}
#[test]
fn short_public_entries_forward_exact_paths_without_authority_or_redirect() {
    let sessions = Arc::new(PublicEntrySessionCalls::default());
    let door = Mounted::new(Arc::clone(&sessions) as Arc<dyn Sessions>);
    let extension = door.extension(replying(PAGE));
    for (path, target) in [
        ("/colab", "/"),
        ("/colab/", "/"),
        ("/p/aB_0", "/p/aB_0"),
        ("/read/Z9_-", "/read/Z9_-"),
    ] {
        let reply = door.send(door.get(path, &format!("Cookie: {OWNER}\r\nTMT-Device-Context: forged\r\nTMT-Origin: forged\r\nTMT-Mount: /evil/\r\nTMT-Device-Event: 1\r\nAccept: text/html\r\n")).as_bytes());
        let (head, body) = reply.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 200"), "{path}: {head}");
        assert_eq!(body, "hello");
        assert!(!head.contains("location:"));
        assert!(!head.contains("set-cookie:"));
        assert!(head.contains("content-security-policy: default-src 'self'\r\n"));
        assert!(head.contains("cache-control: no-store\r\n"));
        let seen = extension.seen.all();
        assert_eq!(
            seen.last().unwrap(),
            &format!(
                "GET {target} HTTP/1.1\r\nhost: {}\r\ntmt-mount: {}/x/colab/\r\naccept: text/html\r\nconnection: close\r\ncontent-length: 0\r\n\r\n",
                door.addr, door.prefix
            )
        );
    }
    assert_eq!(
        sessions.0.load(Ordering::Relaxed),
        0,
        "public session lookup"
    );
    let paths: std::collections::BTreeSet<_> = extension
        .seen
        .all()
        .iter()
        .map(|head| head.split_whitespace().nth(1).unwrap().to_owned())
        .collect();
    assert_eq!(
        paths,
        ["/", "/p/aB_0", "/read/Z9_-"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
}

fn generic_public_refusal(reply: &str, status: u16) {
    let (head, body) = reply.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with(&format!("HTTP/1.1 {status} ")), "{head}");
    assert_eq!(body, include_str!("../assets/error.html"));
    assert!(!head.contains("location:"));
    assert!(!head.contains("set-cookie:"));
}

#[test]
fn short_public_entries_refuse_non_entries_and_admission_failures_before_connect() {
    let door = Mounted::new(Arc::new(NoPublicSessionLookup));
    let extension = door.extension(replying(PAGE));
    for path in [
        "/p",
        "/p/",
        "/p/abc",
        "/p/abcd/",
        "/p/abcd/other",
        "/p/a.b_",
        "/read",
        "/read/",
        "/read/abc",
        "/read/abcd/",
        "/p/.tmt",
        "/read/.tmt",
        "/p/abcd?tmt-session=00000000-0000-4000-8000-000000000004",
        "/colab?tmt-session=00000000-0000-4000-8000-000000000004",
    ] {
        generic_public_refusal(&door.send(door.get(path, "").as_bytes()), 404);
    }
    for path in [
        "/p/../abcd",
        "/read/../abcd",
        "/p/ab%63d",
        "/read/ab%63d",
        "/p//abcd",
        "/read/abcd?cap=secret",
        "/colab/#secret",
        "/p/abcd\\other",
    ] {
        assert_eq!(door.status(&door.get(path, "")), 400, "{path}");
    }
    for path in [
        "/colab/other",
        "/.tmt/colab",
        "/x/colab/",
        "/r/abcd",
        "/other/abcd",
    ] {
        assert_eq!(door.status(&door.get(path, "")), 404, "{path}");
    }
    for path in ["/p/abcd", "/read/abcd", "/colab", "/colab/"] {
        for method in ["POST", "HEAD", "PUT", "PATCH", "DELETE", "OPTIONS"] {
            generic_public_refusal(
                &door.send(
                    format!("{method} {path} HTTP/1.1\r\nHost: {}\r\n\r\n", door.addr).as_bytes(),
                ),
                404,
            );
        }
        for headers in [
            "Origin: https://example.com\r\n",
            "Origin: null\r\n",
            "Sec-Fetch-Site: cross-site\r\n",
        ] {
            generic_public_refusal(&door.send(door.get(path, headers).as_bytes()), 403);
        }
        generic_public_refusal(
            &door.send(
                door.get(path, "Connection: Upgrade\r\nUpgrade: websocket\r\n")
                    .as_bytes(),
            ),
            404,
        );
        generic_public_refusal(
            &door.send(
                door.get(path, "Connection: Upgrade\r\nUpgrade: other\r\n")
                    .as_bytes(),
            ),
            404,
        );
        assert_eq!(
            &door.send(
                format!(
                    "GET {path} HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
                    door.addr.port()
                )
                .as_bytes()
            )[9..12],
            "400"
        );
    }
    for family in ["p", "read"] {
        for id in ["a".repeat(65), "éabc".into()] {
            generic_public_refusal(
                &door.send(door.get(&format!("/{family}/{id}"), "").as_bytes()),
                404,
            );
        }
    }
    assert!(
        extension.seen.all().is_empty(),
        "a refused entry reached Colab"
    );
    // Boundary positives reach exactly the public entry, not arbitrary root paths.
    for family in ["p", "read"] {
        for id in ["aB_0".to_owned(), "Z9_-".repeat(16)] {
            for site in ["none", "same-origin"] {
                assert_eq!(
                    door.status(&door.get(
                        &format!("/{family}/{id}"),
                        &format!("Origin: {}\r\nSec-Fetch-Site: {site}\r\n", door.origin)
                    )),
                    200
                );
            }
        }
    }
    assert_eq!(extension.seen.all().len(), 8);
}

#[test]
fn short_public_unavailable_statuses_are_generic_and_id_independent() {
    for path in ["/p/known000", "/p/unknown0", "/read/known000", "/colab/"] {
        let door = Mounted::new(Arc::new(NoPublicSessionLookup));
        generic_public_refusal(&door.send(door.get(path, "").as_bytes()), 404);
        let door = Mounted::with(Arc::new(NoPublicSessionLookup), &[]);
        let extension = door.extension(replying(PAGE));
        generic_public_refusal(&door.send(door.get(path, "").as_bytes()), 404);
        assert!(extension.seen.all().is_empty());
        assert!(door.mounts.extension_of(path).is_none());
        let door = Mounted::new(Arc::new(NoPublicSessionLookup));
        let listener = UnixListener::bind(door.socket()).unwrap();
        fs::set_permissions(door.socket(), fs::Permissions::from_mode(0o600)).unwrap();
        drop(listener); // Existing, safe socket but no listener: connect fails.
        generic_public_refusal(&door.send(door.get(path, "").as_bytes()), 503);
    }
    let door = Mounted::new(Arc::new(NoPublicSessionLookup));
    let extension = door.extension(replying(PAGE));
    fs::set_permissions(door.root.join("colab"), fs::Permissions::from_mode(0o755)).unwrap();
    generic_public_refusal(&door.send(door.get("/p/abcd", "").as_bytes()), 404);
    fs::set_permissions(door.root.join("colab"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(door.socket(), fs::Permissions::from_mode(0o666)).unwrap();
    generic_public_refusal(&door.send(door.get("/p/abcd", "").as_bytes()), 404);
    assert!(extension.seen.all().is_empty());
    fs::set_permissions(door.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let actual = door.root.join("colab/actual.sock");
    fs::rename(door.socket(), &actual).unwrap();
    std::os::unix::fs::symlink(&actual, door.socket()).unwrap();
    generic_public_refusal(&door.send(door.get("/p/abcd", "").as_bytes()), 404);
    assert!(extension.seen.all().is_empty());
    fs::remove_file(door.socket()).unwrap();
    fs::rename(actual, door.socket()).unwrap();
}

#[test]
fn short_public_replies_keep_extension_policy_but_cannot_cache_or_upgrade() {
    for policy in ["unsafe-url", "origin", "same-origin", "no-referrer"] {
        let door = Mounted::new(Arc::new(NoPublicSessionLookup));
        let reply: &'static [u8] = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Security-Policy: default-src 'self'\r\nSet-Cookie: shadow=1; Path=/\r\nCache-Control: public, max-age=1000\r\nReferrer-Policy: {policy}\r\nContent-Length: 2\r\n\r\nok").into_bytes().leak();
        let _extension = door.extension(replying(reply));
        let response = door.send(door.get("/read/abcd", "").as_bytes());
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        assert_eq!(body, "ok");
        assert_eq!(
            head.lines()
                .filter(|line| line.starts_with("cache-control:"))
                .collect::<Vec<_>>(),
            ["cache-control: no-store"]
        );
        assert!(!head.contains("set-cookie:"));
        assert!(head.contains("content-security-policy: default-src 'self'"));
        assert!(head.contains(if policy == "same-origin" {
            "referrer-policy: same-origin"
        } else {
            "referrer-policy: no-referrer"
        }));
    }
    for reply in [
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n".as_slice(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 16777217\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx",
        b"HTTP/1.1 200 OK\r\nCache-Control: public\r\nCache-Control: private\r\nContent-Length: 1\r\n\r\nx",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        b"garbage\r\n\r\n",
        b"",
    ] {
        let door = Mounted::new(Arc::new(NoPublicSessionLookup));
        let _extension = door.extension(replying(reply));
        generic_public_refusal(&door.send(door.get("/p/abcd", "").as_bytes()), 502);
    }
    let door = Mounted::new(Arc::new(NoPublicSessionLookup));
    let _extension = door.extension(replying(
        b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nshort",
    ));
    let reply = door.send(door.get("/p/abcd", "").as_bytes());
    assert!(reply.contains("content-length: 9\r\n"));
    assert!(reply.ends_with("\r\n\r\nshort"));
}

#[test]
fn short_public_reply_deadline_closes_the_owned_extension_connection() {
    let door = Mounted::new(Arc::new(NoPublicSessionLookup));
    let (closed, observed) = mpsc::channel();
    let _extension = door.extension(Arc::new(move |mut stream, seen| {
        seen.push(request(&mut stream));
        stream
            .set_read_timeout(Some(limits::MOUNT_RESPONSE + Duration::from_secs(5)))
            .unwrap();
        assert_eq!(
            stream.read(&mut [0]).unwrap(),
            0,
            "door did not close after the reply deadline"
        );
        closed.send(()).unwrap();
    }));
    generic_public_refusal(&door.send(door.get("/p/abcd", "").as_bytes()), 502);
    observed
        .recv_timeout(Duration::from_secs(5))
        .expect("extension connection leaked");
}
