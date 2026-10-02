use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tmt_colab::http::Door;
struct Running {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Running {
    fn start() -> Self {
        let door = Door::bind(0).unwrap();
        let address = door
            .address
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .parse()
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = std::thread::spawn(move || door.run(&flag).unwrap());
        Self {
            address,
            stop,
            worker: Some(worker),
        }
    }
    fn request(&self, request: &str) -> String {
        let mut socket = TcpStream::connect(self.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        socket.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    }
    fn wire(&self, path: &str, headers: &str) -> String {
        format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\n{headers}\r\n",
            self.address
        )
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        assert!(TcpStream::connect(self.address).is_err(), "listener leaked");
    }
}
#[test]
fn exact_host_origin_and_framing_have_real_socket_positive_controls() {
    let server = Running::start();
    let origin = format!("Origin: http://{}\r\n", server.address);
    let valid = server.wire("/", &origin);
    let page = server.request(&valid);
    assert!(page.starts_with("HTTP/1.1 200"));
    assert!(page.contains("TMT Colab"));
    assert!(!page.contains("Access-Control-Allow-Origin"));
    let fields = (0..31)
        .map(|i| format!("X-Field-{i}: value\r\n"))
        .collect::<String>();
    assert!(
        server
            .request(&server.wire("/", &fields))
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        server
            .request(&server.wire("/", &format!("{fields}X-Overflow: value\r\n")))
            .starts_with("HTTP/1.1 400")
    );
    for host in ["attacker.invalid:80", "127.0.0.1.evil:80", "localhost:80"] {
        assert!(
            server
                .request(&valid.replace(
                    &format!("Host: {}", server.address),
                    &format!("Host: {host}")
                ))
                .starts_with("HTTP/1.1 403")
        );
    }
    for bad in ["Origin: null\r\n", "Origin: https://evil.invalid\r\n", ""] {
        assert!(
            server
                .request(&server.wire("/sync", bad))
                .starts_with("HTTP/1.1 403")
        );
    }
    let upgrade = format!(
        "{origin}Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: colab-sync-v1\r\n"
    );
    assert!(
        server
            .request(&server.wire("/sync", &upgrade))
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        server
            .request(&server.wire("/sync", &format!("{upgrade}Cookie: session=forged\r\n")))
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        server
            .request(&format!("GET / HTTP/1.1\nHost: {}\r\n\r\n", server.address))
            .starts_with("HTTP/1.1 400")
    );
    for headers in [
        "Host: attacker.invalid\r\n",
        "Transfer-Encoding: chunked\r\n",
        "Forwarded: host=evil\r\n",
        "Content-Length: 00\r\n",
        "Content-Length: 0\r\nContent-Length: 0\r\n",
    ] {
        assert!(
            server
                .request(&server.wire("/", headers))
                .starts_with("HTTP/1.1 400")
        );
    }
    assert!(
        server
            .request(&server.wire("http://evil.invalid/", ""))
            .starts_with("HTTP/1.1 400")
    );
    assert!(
        server
            .request(&server.wire("/sync", &format!("{origin}Content-Length: 65537\r\n")))
            .starts_with("HTTP/1.1 413")
    );
    let body = "x".repeat(65536);
    assert!(
        server
            .request(&format!(
                "{}{}",
                server.wire("/api", &format!("{origin}Content-Length: 65536\r\n")),
                body
            ))
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        server
            .request(&format!("GET / HTTP/1.1\r\nX-Fill: {}", "x".repeat(8192)))
            .starts_with("HTTP/1.1 413")
    );
}
#[test]
fn acquisition_capacity_and_shutdown_close_all_retained_sockets_twice() {
    for _ in 0..2 {
        let server = Running::start();
        let mut retained = Vec::new();
        for _ in 0..tmt_colab::limits::SOCKETS {
            let mut socket = TcpStream::connect(server.address).unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\n").unwrap();
            retained.push(socket);
        }
        assert!(
            server
                .request(&server.wire("/", ""))
                .starts_with("HTTP/1.1 429")
        );
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_secs(1));
        for mut socket in retained {
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            match socket.read(&mut [0; 1]) {
                Ok(0) => {}
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
                result => panic!("socket leaked: {result:?}"),
            }
        }
    }
    let server = Running::start();
    assert!(
        server
            .request("GET / HTTP/1.1\r\n")
            .starts_with("HTTP/1.1 408")
    );
}
