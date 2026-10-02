use super::*;
use std::net::SocketAddr;

#[test]
fn response_drains_input_sent_after_its_fin_until_peer_eof() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    peer.set_read_timeout(Some(limits::RESPONSE)).unwrap();
    peer.set_write_timeout(Some(limits::RESPONSE)).unwrap();
    let (mut socket, _) = listener.accept().unwrap();
    socket.set_read_timeout(Some(limits::RESPONSE)).unwrap();
    let worker = thread::spawn(move || {
        response(&mut socket, &Reply::empty(429)).unwrap();
        socket.set_nonblocking(false).unwrap();
        assert_eq!(socket.read(&mut [0; 1]).unwrap(), 0, "input not drained");
    });
    let mut reply = String::new();
    peer.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 429"));
    assert!(reply.ends_with("\r\n\r\n{}"));
    // The server has already sent FIN; the request tail is intentionally late.
    peer.write_all(&vec![b'x'; limits::DRAIN_BYTES]).unwrap();
    peer.shutdown(Shutdown::Write).unwrap();
    worker.join().unwrap();
}

#[test]
fn target_grammar_isolates_prefixes() {
    for valid in ["/", "/r/ab/append", "/x/colab/", "/x/colab/app.js"] {
        assert!(target(valid), "{valid}");
    }
    for invalid in [
        "",
        "r/x",
        "http://evil.invalid/",
        "/r/ab/append?x=1",
        "/r/ab/%61ppend",
        "/x/colab/../../r/ab/append",
        "/x/colab/./a",
        "/x//colab/",
        "/x/colab//a",
        "/x/colab/a#b",
        "/x\\colab",
    ] {
        assert!(!target(invalid), "{invalid}");
    }
}

/// Admits everything with an unbounded body, so only door-owned bounds apply.
struct Permissive;
impl Handler for Permissive {
    fn admit(&self, _: &Head<'_>) -> Result<usize, Reply> {
        Ok(usize::MAX)
    }
    fn handle(&self, _: Request) -> Reply {
        Reply::empty(200)
    }
}
fn head(addr: SocketAddr, size: usize) -> String {
    format!("POST /a HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {size}\r\n\r\n")
}
fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while !condition() {
        assert!(Instant::now() < deadline, "condition not reached");
        thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn door_clamps_handler_limits_and_bounds_in_flight_bodies() {
    let door = Door::bind(0).unwrap();
    let addr = door.socket_addr().unwrap();
    let budget = door.budget.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let server = thread::spawn(move || door.run(&flag, Arc::new(Permissive)).unwrap());
    let status = |stream: &mut TcpStream| {
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply[9..12].parse::<u16>().unwrap()
    };
    let send = |text: &str| {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(8)))
            .unwrap();
        stream.write_all(text.as_bytes()).unwrap();
        stream
    };
    // Positive control: a small body passes the clamp and the budget.
    assert_eq!(status(&mut send(&format!("{}{{}}", head(addr, 2)))), 200);
    // The handler's unbounded limit is clamped to the door maximum.
    assert_eq!(status(&mut send(&head(addr, limits::BODY_BYTES + 1))), 413);
    // One large body holds its reservation while the next would exceed the budget.
    let large = limits::IN_FLIGHT_BODY_BYTES / 2 + 1;
    let first = send(&head(addr, large));
    wait_until(|| budget.0.load(Ordering::Acquire) == large);
    assert_eq!(status(&mut send(&head(addr, large))), 429);
    // Releasing the first request returns its bytes to the budget.
    drop(first);
    wait_until(|| budget.0.load(Ordering::Acquire) == 0);
    let second = send(&head(addr, large));
    wait_until(|| budget.0.load(Ordering::Acquire) == large);
    drop(second);
    wait_until(|| budget.0.load(Ordering::Acquire) == 0);
    stop.store(true, Ordering::Release);
    server.join().unwrap();
}
