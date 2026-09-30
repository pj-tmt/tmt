use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tmt_remote::http::{Door, ServeOptions};
struct Running {
    addr: std::net::SocketAddr,
    prefix: String,
    stop: Arc<AtomicBool>,
    child: Option<thread::JoinHandle<()>>,
}
impl Running {
    fn new(window: Duration) -> Self {
        let door = Door::bind(ServeOptions {
            port: 0,
            window,
            input_limit: 1024,
        })
        .unwrap();
        let addr = door.socket_addr().unwrap();
        assert!(addr.ip().is_loopback() && addr.is_ipv4());
        let prefix = door
            .address
            .split_once(&addr.to_string())
            .unwrap()
            .1
            .to_owned();
        assert_eq!(prefix.len(), 35);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let child = thread::spawn(move || door.run(&flag).unwrap());
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
            .set_read_timeout(Some(Duration::from_secs(6)))
            .unwrap();
        stream.write_all(bytes.as_bytes()).unwrap();
        let mut result = String::new();
        loop {
            let mut bytes = [0; 256];
            let count = stream.read(&mut bytes).unwrap();
            assert!(count > 0, "incomplete HTTP response: {result}");
            result.push_str(std::str::from_utf8(&bytes[..count]).unwrap());
            if result.ends_with("\r\n\r\n{}") {
                break;
            }
            assert!(result.len() < 1024, "response is bounded");
        }
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
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(child) = self.child.take() {
            child.join().unwrap();
        }
        assert!(TcpStream::connect(self.addr).is_err());
    }
}
#[test]
fn framing_positive_controls_and_isolated_refusals() {
    let door = Running::new(Duration::from_secs(30));
    let valid = door.post("/append", "", "{}");
    for suffix in ["/append", "/subscribe", "/ack", "/pair"] {
        let response = door.request(&door.post(suffix, "", "{}"));
        assert!(response.starts_with("HTTP/1.1 404"));
        assert!(response.ends_with("\r\n\r\n{}"));
        assert!(!response.contains("Access-Control"));
    }
    for origin in [
        "null",
        "https://evil.example",
        "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert!(
            door.request(&door.post("/append", &format!("Origin: {origin}\r\n"), "{}"))
                .starts_with("HTTP/1.1 404")
        );
    }
    for (request, status) in [
        (
            valid.replace(&format!("Host: {}", door.addr), "Host: localhost:80"),
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
            valid.replace("Content-Length: 2", "Transfer-Encoding: chunked"),
            400,
        ),
        (valid.replace("application/json", "text/plain"), 400),
        (valid.replace("/append", "/append?x=1"), 404),
        (valid.replace("/append", "/%61ppend"), 404),
        (valid.replace("POST", "GET"), 404),
        (door.post("/append", "Cookie: x=y\r\n", "{}"), 400),
        (
            door.post("/append", "Origin: cli\r\nOrigin: null\r\n", "{}"),
            400,
        ),
        (
            valid.replace("Content-Length: 2", "Content-Length: 999999999"),
            413,
        ),
    ] {
        assert!(
            door.request(&request)
                .starts_with(&format!("HTTP/1.1 {status}"))
        );
    }
    assert!(
        Door::bind(ServeOptions {
            port: door.addr.port(),
            window: Duration::from_secs(1),
            input_limit: 1
        })
        .is_err()
    );
    assert!(
        Door::bind(ServeOptions {
            port: 0,
            window: Duration::ZERO,
            input_limit: 1
        })
        .is_err()
    );
}
#[test]
fn finite_lifecycle_rate_and_acquisition_cleanup() {
    // Run lifecycle acceptance twice, including retained incomplete sockets.
    for _ in 0..2 {
        let mut door = Running::new(Duration::from_millis(200));
        let mut partial = TcpStream::connect(door.addr).unwrap();
        partial.write_all(b"POST /r/").unwrap();
        door.child.take().unwrap().join().unwrap();
        assert!(
            !door.stop.load(Ordering::Relaxed),
            "hard deadline, not a stop flag"
        );
        assert!(TcpStream::connect(door.addr).is_err());
        drop(door);
        drop(partial);
    }
    let door = Running::new(Duration::from_secs(30));
    for _ in 0..20 {
        assert!(
            door.request(&door.post("/ack", "", "{}"))
                .starts_with("HTTP/1.1 404")
        );
    }
    assert!(
        door.request(&door.post("/ack", "", "{}"))
            .starts_with("HTTP/1.1 429")
    );
    drop(door);
    let door = Running::new(Duration::from_secs(30));
    let mut retained = Vec::new();
    for _ in 0..20 {
        let mut stream = TcpStream::connect(door.addr).unwrap();
        stream.write_all(b"POST /r/").unwrap();
        retained.push(stream);
    }
    assert!(
        door.request(&door.post("/ack", "", "{}"))
            .starts_with("HTTP/1.1 429")
    );
    drop(door);
    for mut stream in retained {
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let result = stream.read(&mut [0; 1]);
        assert!(
            matches!(result, Ok(0))
                || matches!(result, Err(ref e) if matches!(e.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted)),
            "retained connection leaked"
        );
    }
    let door = Running::new(Duration::from_secs(30));
    for request in [
        "POST /r/",
        door.post("/append", "", "{}").trim_end_matches("{}"),
    ] {
        let response = door.request(request);
        assert!(response.starts_with("HTTP/1.1 400"));
    }
    let large = "x".repeat(8193);
    assert!(door.request(&large).starts_with("HTTP/1.1 413"));
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    assert!(
        Door::bind(ServeOptions {
            port: occupied.local_addr().unwrap().port(),
            window: Duration::from_secs(1),
            input_limit: 1
        })
        .is_err()
    );
}
