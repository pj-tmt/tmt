//! Real-socket mount acceptance: a fixture extension serves an owner-only Unix
//! socket under an isolated data root; the door forwards `/x/colab/` to it.
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
    http::{Door, Handler},
    limits,
    mount::{Admitted, DeviceContext, EXTENSIONS, Extension, Mounts, NoSessions, Sessions},
    routes::Routes,
    site::Site,
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

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Mounted {
    root: PathBuf,
    addr: SocketAddr,
    origin: String,
    prefix: String,
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
        let routes = Routes::new(1024, "/r/00112233445566778899aabbccddeeff".into()).unwrap();
        let prefix = routes.prefix().to_owned();
        let door = Door::bind(0).unwrap();
        let addr = door.socket_addr().unwrap();
        let origin = door.origin.clone();
        let site = Arc::new(Site {
            routes,
            mounts: Mounts::with_extensions(root.clone(), &origin, sessions, extensions),
            pages: None,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let door = thread::spawn(move || door.run(&flag, site as Arc<dyn Handler>).unwrap());
        Self {
            root,
            addr,
            origin,
            prefix,
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
            "/x/colab/app/index.html",
            &format!("Accept: text/html\r\nCookie: {OWNER}\r\nTMT-Device-Context: forged\r\nTMT-Mount: /x/evil/\r\nX-Other: dropped\r\n"),
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
            "GET /app/index.html HTTP/1.1\r\nhost: {}\r\ntmt-mount: /x/colab/\r\naccept: text/html\r\nconnection: close\r\ncontent-length: 0\r\n\r\n",
            door.addr
        )
    );
}

#[test]
fn owner_session_forwards_exact_device_context_and_body() {
    let door = Mounted::new(Arc::new(OneOwner));
    let extension = door.extension(replying(PAGE));
    let body = "{\"page\":1}";
    let post = format!(
        "POST /x/colab/api HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nCookie: {OWNER}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
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
            "POST /api HTTP/1.1\r\nhost: {}\r\ntmt-mount: /x/colab/\r\norigin: {}\r\ncontent-type: application/json\r\ntmt-device-context: {context}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
            door.addr,
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
fn origin_method_and_prefix_isolation_refuse_before_forwarding() {
    let mut door = Mounted::new(Arc::new(OneOwner));
    let extension = door.extension(replying(PAGE));
    let origin = format!("Origin: {}\r\n", door.origin);
    let post = |headers: &str| {
        format!(
            "POST /x/colab/api HTTP/1.1\r\nHost: {}\r\n{headers}Content-Length: 2\r\n\r\n{{}}",
            door.addr
        )
    };
    assert_eq!(door.status(&post(&origin)), 200);
    for (request, status) in [
        (post(""), 403),
        (post("Origin: null\r\n"), 403),
        (post("Origin: http://localhost:1\r\n"), 403),
        (
            door.get("/x/colab/", "Origin: https://evil.invalid\r\n"),
            403,
        ),
        (door.get("/x/colab", ""), 404),
        (door.get("/x/Colab/", ""), 404),
        (door.get("/x/other/", ""), 404),
        (door.get("/x/", ""), 404),
        (door.get("/x/colab/../../r/x/append", ""), 400),
        (door.get("/x/colab//a", ""), 400),
        (
            format!(
                "OPTIONS /x/colab/ HTTP/1.1\r\nHost: {}\r\n{origin}\r\n",
                door.addr
            ),
            404,
        ),
        (
            format!(
                "POST /x/colab/api HTTP/1.1\r\nHost: {}\r\n{origin}Content-Length: 65537\r\n\r\n",
                door.addr
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
        door.status(&door.get(&format!("/x/colab{}/append", door.prefix), "")),
        200
    );
    let seen = extension.seen.all();
    assert_eq!(
        seen.len(),
        2,
        "only the two admitted mount requests forwarded"
    );
    assert!(seen[1].starts_with(&format!("GET {}/append HTTP/1.1", door.prefix)));
    drop(extension);
    door.stop();
}

#[test]
fn unsafe_or_missing_sockets_are_not_mounted() {
    let door = Mounted::new(Arc::new(NoSessions));
    let get = door.get("/x/colab/", "");
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
            door.status(&door.get("/x/colab/", "")),
            502,
            "{}",
            String::from_utf8_lossy(reply)
        );
    }
    let door = Mounted::new(Arc::new(NoSessions));
    let _extension = door.extension(replying(b"HTTP/1.1 204 No Content\r\n\r\n"));
    assert_eq!(door.status(&door.get("/x/colab/", "")), 204);
    // After the head is streamed, an early extension close yields a body
    // shorter than its declared length, which the client detects.
    let door = Mounted::new(Arc::new(NoSessions));
    let _extension = door.extension(replying(
        b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nshort",
    ));
    let reply = door.send(door.get("/x/colab/", "").as_bytes());
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
        .write_all(door.get("/x/colab/blob", "").as_bytes())
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
fn websocket_bytes_pass_through_unchanged_and_close_with_either_side() {
    let (closed, ended) = mpsc::channel();
    let mut door = Mounted::new(Arc::new(NoSessions));
    let extension = door.extension(echo(closed));
    // Unauthenticated-origin upgrades are refused before any forwarding.
    assert_eq!(door.status(&door.get("/x/colab/sync", UPGRADE)), 403);
    let origin = format!("Origin: {}\r\n{UPGRADE}", door.origin);
    assert_eq!(
        door.status(&door.get("/x/colab/sync", &origin.replace("websocket", "h2c"))),
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
            .write_all(door.get("/x/colab/sync", &origin).as_bytes())
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
}];
/// Upgrade a new client and return it after the 101 head.
fn tunnel(door: &Mounted) -> TcpStream {
    let mut client = TcpStream::connect(door.addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let origin = format!("Origin: {}\r\n{UPGRADE}", door.origin);
    client
        .write_all(door.get("/x/colab/sync", &origin).as_bytes())
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
    let refused = door.send(door.get("/x/colab/sync", &origin).as_bytes());
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
        door.status(&door.get("/x/colab/", "Sec-Fetch-Site: cross-site\r\n")),
        403
    );
    for site in ["same-origin", "none"] {
        assert_eq!(
            door.status(&door.get("/x/colab/", &format!("Sec-Fetch-Site: {site}\r\n"))),
            200
        );
    }
}
