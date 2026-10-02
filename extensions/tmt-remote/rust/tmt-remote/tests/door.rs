//! Real-socket door acceptance: framing, Host admission, bounds and shutdown.
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tmt_remote::{
    http::{Door, Handler},
    limits,
    routes::Routes,
};
const PREFIX: &str = "/r/0123456789abcdef0123456789abcdef";
struct Running {
    addr: SocketAddr,
    prefix: String,
    stop: Arc<AtomicBool>,
    child: Option<thread::JoinHandle<()>>,
}
impl Running {
    fn new() -> Self {
        let routes = Arc::new(Routes::new(1024, PREFIX.into()).unwrap());
        let prefix = routes.prefix().to_owned();
        assert_eq!(prefix.len(), 35);
        let door = Door::bind(0).unwrap();
        let addr = door.socket_addr().unwrap();
        assert!(addr.ip().is_loopback() && addr.is_ipv4());
        assert_eq!(door.origin, format!("http://{addr}"));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let child = thread::spawn(move || door.run(&flag, routes as Arc<dyn Handler>).unwrap());
        Self {
            addr,
            prefix,
            stop,
            child: Some(child),
        }
    }
    fn request(&self, bytes: &str) -> String {
        let mut stream = TcpStream::connect(self.addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        stream.write_all(bytes.as_bytes()).unwrap();
        let mut result = String::new();
        stream.read_to_string(&mut result).unwrap();
        assert!(result.ends_with("\r\n\r\n{}"), "bounded reply: {result}");
        result
    }
    fn post(&self, suffix: &str, extra: &str, body: &str) -> String {
        format!(
            "POST {}{suffix} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n{body}",
            self.prefix,
            self.addr,
            body.len()
        )
    }
    fn status(&self, request: &str) -> u16 {
        self.request(request)[9..12].parse().unwrap()
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(child) = self.child.take() {
            child.join().unwrap();
        }
        assert!(TcpStream::connect(self.addr).is_err(), "listener leaked");
    }
}
#[test]
fn remote_routes_refuse_with_isolated_framing_controls() {
    let door = Running::new();
    let valid = door.post("/append", "", "{}");
    for suffix in ["/append", "/subscribe", "/ack", "/pair"] {
        let response = door.request(&door.post(suffix, "", "{}"));
        assert!(response.starts_with("HTTP/1.1 404"));
        assert!(!response.contains("Access-Control"));
    }
    for origin in [
        "null",
        "https://evil.example",
        "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert_eq!(
            door.status(&door.post("/append", &format!("Origin: {origin}\r\n"), "{}")),
            404
        );
    }
    let fields = (0..29)
        .map(|i| format!("X-Field-{i}: value\r\n"))
        .collect::<String>();
    assert_eq!(door.status(&door.post("/append", &fields, "{}")), 404);
    let pair_limit = "x".repeat(limits::PAIR_BODY_BYTES);
    assert_eq!(door.status(&door.post("/pair", "", &pair_limit)), 404);
    for (request, status) in [
        (
            door.post("/append", &format!("{fields}X-Overflow: value\r\n"), "{}"),
            400,
        ),
        (
            valid.replace(
                "Content-Length: 2",
                "Content-Length: 2\r\nContent-Length: 2",
            ),
            400,
        ),
        (
            valid.replace("Content-Length: 2", "Content-Length: 02"),
            400,
        ),
        (
            valid.replace("Content-Length: 2", "Transfer-Encoding: chunked"),
            400,
        ),
        (valid.replace("Content-Length: 2\r\n", ""), 400),
        (valid.replace("application/json", "text/plain"), 400),
        (valid.replace("/append", "/append?x=1"), 400),
        (valid.replace("/append", "/%61ppend"), 400),
        (valid.replace("/append", "/./append"), 400),
        (valid.replace("/append", "//append"), 400),
        (valid.replace("POST", "GET"), 404),
        (
            valid.replace(&door.prefix, "/r/00000000000000000000000000000000"),
            404,
        ),
        (door.post("/append", "Cookie: x=y\r\n", "{}"), 400),
        (
            door.post("/append", "Authorization: Bearer x\r\n", "{}"),
            400,
        ),
        (door.post("/append", "Forwarded: host=evil\r\n", "{}"), 400),
        (
            door.post("/append", "X-Forwarded-Host: evil\r\n", "{}"),
            400,
        ),
        (
            door.post("/append", "Origin: cli\r\nOrigin: null\r\n", "{}"),
            400,
        ),
        (
            door.post(
                "/append",
                "Connection: Upgrade\r\nUpgrade: websocket\r\n",
                "{}",
            ),
            404,
        ),
        (valid.replace("\r\nHost", "\nHost"), 400),
        (
            valid.replace("Content-Length: 2", "Content-Length: 999999999"),
            413,
        ),
        (door.post("/pair", "", &format!("{pair_limit}x")), 413),
        (
            format!("GET / HTTP/1.1\r\nHost: {}\r\n\r\n", door.addr),
            404,
        ),
        (
            format!(
                "GET http://evil.invalid/ HTTP/1.1\r\nHost: {}\r\n\r\n",
                door.addr
            ),
            400,
        ),
        (
            format!("GET / HTTP/1.1\r\nX-Fill: {}", "x".repeat(8192)),
            413,
        ),
    ] {
        assert_eq!(door.status(&request), status, "{request:.120}");
    }
}
#[test]
fn exact_numeric_host_refuses_dns_rebinding_aliases() {
    let door = Running::new();
    let valid = door.post("/append", "", "{}");
    assert_eq!(door.status(&valid), 404);
    let port = door.addr.port();
    for host in [
        format!("localhost:{port}"),
        format!("attacker.invalid:{port}"),
        format!("127.0.0.1.evil:{port}"),
        "127.0.0.1".into(),
        format!("127.0.0.1:{port}\r\nHost: 127.0.0.1:{port}"),
    ] {
        let request = valid.replace(&format!("Host: {}", door.addr), &format!("Host: {host}"));
        assert_eq!(door.status(&request), 400, "{host}");
    }
    assert_eq!(
        door.status(&valid.replace(&format!("Host: {}\r\n", door.addr), "")),
        400
    );
}
#[test]
fn unauthenticated_attempts_are_rate_bounded() {
    let door = Running::new();
    for _ in 0..limits::ATTEMPTS_PER_MINUTE {
        assert_eq!(door.status(&door.post("/ack", "", "{}")), 404);
    }
    assert_eq!(door.status(&door.post("/ack", "", "{}")), 429);
}
#[test]
fn capacity_acquisition_and_shutdown_close_retained_sockets_twice() {
    for _ in 0..2 {
        let door = Running::new();
        let mut retained = Vec::new();
        for _ in 0..limits::SOCKETS {
            let mut socket = TcpStream::connect(door.addr).unwrap();
            socket.write_all(b"POST /r/").unwrap();
            retained.push(socket);
        }
        assert_eq!(door.status(&door.post("/ack", "", "{}")), 429);
        let start = Instant::now();
        drop(door);
        assert!(start.elapsed() < Duration::from_secs(1));
        for mut socket in retained {
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            match socket.read(&mut [0; 1]) {
                Ok(0) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ) => {}
                result => panic!("socket leaked: {result:?}"),
            }
        }
    }
    let door = Running::new();
    let partial = door.post("/append", "", "{}");
    for request in ["POST /r/", partial.trim_end_matches("{}")] {
        let start = Instant::now();
        assert_eq!(door.status(request), 408);
        assert!(start.elapsed() >= limits::ACQUISITION - Duration::from_millis(100));
    }
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = Door::bind(occupied.local_addr().unwrap().port())
        .err()
        .unwrap();
    assert_eq!(busy.code, "REMOTE_PORT_BUSY");
    assert!(Routes::new(0, PREFIX.into()).is_err());
    assert!(Routes::new(limits::CORE_INPUT_BYTES + 1, PREFIX.into()).is_err());
    for prefix in [
        "/r/0123456789abcdef0123456789ABCDEF",
        "/r/0123456789abcdef0123456789abcde",
        "/x/0123456789abcdef0123456789abcdef",
        "/r/0123456789abcdef0123456789abcdef/",
    ] {
        assert!(Routes::new(1024, prefix.into()).is_err(), "{prefix}");
    }
}
