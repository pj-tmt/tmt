//! Colab's owner-only socket on real Unix sockets: framing bounds, the device
//! context from the remote door, the colab-sync-v1 handshake, tunnel bounds,
//! capacity and shutdown.
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tmt_colab::{
    keyring::Layout,
    limits,
    socket::{MountSocket, Tunnels},
};

const SPACE: &str = "space-1";
const OWNER: &str = r#"tmt-device-context: {"deviceId":"00000000-0000-4000-8000-000000000004","kind":"browser","origin":"http://127.0.0.1:1","name":"<b>Laptop</b>","publicKey":"AA","owner":true,"grantRevision":1}"#;
const UPGRADE: &str = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: colab-sync-v1\r\n";

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Running {
    root: PathBuf,
    path: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Running {
    fn start(tunnels: Tunnels) -> Self {
        // Short absolute root: Unix socket paths are limited to about 100 bytes.
        let root = PathBuf::from(format!(
            "/tmp/tmt-1039-colab-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let layout = Layout::open(&root).unwrap();
        let socket = MountSocket::bind(&layout, SPACE, tunnels).unwrap();
        let path = socket.path.clone();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let worker = std::thread::spawn(move || socket.run(&flag).unwrap());
        Self {
            root,
            path,
            stop,
            worker: Some(worker),
        }
    }
    fn connect(&self) -> UnixStream {
        let socket = UnixStream::connect(&self.path).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
    }
    fn request(&self, request: &str) -> String {
        let mut socket = self.connect();
        socket.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    }
    fn get(path: &str, headers: &str) -> String {
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\n{headers}\r\n")
    }
    /// Upgrade and return the client after the 101 head.
    fn tunnel(&self) -> (UnixStream, String) {
        let mut socket = self.connect();
        socket
            .write_all(Self::get("/sync", &format!("{OWNER}\r\n{UPGRADE}")).as_bytes())
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0; 1];
        while !head.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        (socket, String::from_utf8(head).unwrap())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        assert!(!self.path.exists(), "socket left behind");
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn closed(socket: &mut UnixStream) -> bool {
    matches!(socket.read(&mut [0; 1]), Ok(0))
        || matches!(socket.read(&mut [0; 1]), Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset)
}

#[test]
fn pages_follow_the_forwarded_owner_context_within_the_door_bounds() {
    let server = Running::start(Tunnels::PRODUCT);
    let private = server.request(&Running::get("/", ""));
    assert!(private.starts_with("HTTP/1.1 200"));
    assert!(private.contains("This colab space is private. Open it from a browser paired with tmt remote pair, or use a share link."));
    assert!(private.contains("Referrer-Policy: no-referrer"));
    assert!(private.contains("Content-Security-Policy: default-src 'none'"));
    let owned = server.request(&Running::get("/", &format!("{OWNER}\r\n")));
    assert!(owned.contains(
        "Colab space space-1 is running. You are signed in as &lt;b&gt;Laptop&lt;/b&gt;. Co-editing arrives with the next colab slice."
    ));
    for context in [
        "tmt-device-context: {}\r\n",
        "tmt-device-context: not json\r\n",
        "tmt-device-context: {\"deviceId\":\"x\",\"name\":\"n\",\"owner\":false}\r\n",
    ] {
        assert!(
            server
                .request(&Running::get("/", context))
                .starts_with("HTTP/1.1 400"),
            "{context}"
        );
    }
    let fields = (0..31)
        .map(|i| format!("X-Field-{i}: value\r\n"))
        .collect::<String>();
    assert!(
        server
            .request(&Running::get("/", &fields))
            .starts_with("HTTP/1.1 200")
    );
    for (request, status) in [
        (
            Running::get("/", &format!("{fields}X-Overflow: value\r\n")),
            400,
        ),
        ("GET / HTTP/1.1\nHost: x\r\n\r\n".into(), 400),
        (Running::get("/", "Transfer-Encoding: chunked\r\n"), 400),
        (Running::get("/", "Forwarded: host=evil\r\n"), 400),
        (Running::get("/", "Content-Length: 00\r\n"), 400),
        (
            Running::get("/", "Content-Length: 0\r\nContent-Length: 0\r\n"),
            400,
        ),
        (Running::get("http://evil.invalid/", ""), 400),
        (Running::get("/?q=1", ""), 400),
        (Running::get("/api", "Content-Length: 65537\r\n"), 413),
        (
            format!("GET / HTTP/1.1\r\nX-Fill: {}", "x".repeat(8192)),
            413,
        ),
        (Running::get("/other", ""), 404),
    ] {
        assert!(
            server
                .request(&request)
                .starts_with(&format!("HTTP/1.1 {status}")),
            "{request:.60}"
        );
    }
    // A full bounded body is read and the request still answered.
    let body = "x".repeat(limits::HTTP_BODY_BYTES);
    assert!(
        server
            .request(&format!(
                "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ))
            .starts_with("HTTP/1.1 404")
    );
}

#[test]
fn owner_upgrades_are_accepted_capped_and_closed_when_idle() {
    let server = Running::start(Tunnels {
        cap: 2,
        idle: Duration::from_millis(500),
    });
    let (mut first, head) = server.tunnel();
    assert!(head.starts_with("HTTP/1.1 101"));
    // RFC 6455 section 1.3 sample key and accept value.
    assert!(head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
    assert!(head.contains("Sec-WebSocket-Protocol: colab-sync-v1\r\n"));
    let (_second, _) = server.tunnel();
    let refused = server.request(&Running::get("/sync", &format!("{OWNER}\r\n{UPGRADE}")));
    assert!(refused.starts_with("HTTP/1.1 503"), "{refused:.40}");
    // Frames are later work: input is accepted, nothing is answered.
    first.write_all(b"frame").unwrap();
    for (headers, status) in [
        (UPGRADE.to_owned(), 403),
        (
            format!("{OWNER}\r\n{}", UPGRADE.replace("colab-sync-v1", "chat")),
            400,
        ),
        (
            format!(
                "{OWNER}\r\n{}",
                UPGRADE.replace("Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n", "")
            ),
            400,
        ),
    ] {
        assert!(
            server
                .request(&Running::get("/sync", &headers))
                .starts_with(&format!("HTTP/1.1 {status}")),
            "{headers:.60}"
        );
    }
    // After the idle bound both tunnels close and the cap frees.
    let start = Instant::now();
    assert!(closed(&mut first));
    assert!(start.elapsed() < Duration::from_secs(3));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let (mut again, head) = server.tunnel();
        if head.starts_with("HTTP/1.1 101") {
            drop(again.write_all(b"x"));
            break;
        }
        assert!(Instant::now() < deadline, "cap never freed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn capacity_and_shutdown_close_retained_sockets_and_tunnels_twice() {
    for _ in 0..2 {
        let server = Running::start(Tunnels::PRODUCT);
        let (mut tunnel, head) = server.tunnel();
        assert!(head.starts_with("HTTP/1.1 101"));
        let mut retained = Vec::new();
        for _ in 0..limits::SOCKETS {
            let mut socket = server.connect();
            socket.write_all(b"GET / HTTP/1.1\r\n").unwrap();
            retained.push(socket);
        }
        // A held tunnel does not count against request workers.
        assert!(
            server
                .request(&Running::get("/", ""))
                .starts_with("HTTP/1.1 429")
        );
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(closed(&mut tunnel), "tunnel leaked");
        for mut socket in retained {
            assert!(closed(&mut socket), "socket leaked");
        }
    }
    let server = Running::start(Tunnels::PRODUCT);
    assert!(
        server
            .request("GET / HTTP/1.1\r\n")
            .starts_with("HTTP/1.1 408")
    );
}
