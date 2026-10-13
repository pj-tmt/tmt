//! Private loopback wire scenarios; no Firebase project or credential is used.
use super::{AuthErrorKind, Cancellation, Clock, Http, Operation, Request, SystemClock, UreqHttp};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

fn fixture(reply: Vec<u8>) -> (String, std::thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let join = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "wire fixture did not receive a connection"
                    );
                    std::thread::park_timeout(Duration::from_millis(2));
                }
                Err(error) => panic!("wire fixture accept failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut incoming = Vec::new();
        loop {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            incoming.push(byte[0]);
            if incoming.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let header = String::from_utf8(incoming.clone()).unwrap();
        let length = header
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        incoming.extend(body);
        if reply.is_empty() {
            assert_eq!(
                stream.read(&mut [0]).unwrap(),
                0,
                "deadline must close the in-flight connection"
            );
        } else {
            stream.write_all(&reply).unwrap();
        }
        incoming
    });
    (format!("http://{address}/fixture"), join)
}
fn request(url: String, cap: usize) -> Request {
    Request::firestore(
        Operation::Commit,
        url,
        b"{\"fixture\":true}".to_vec(),
        "fixture-token".into(),
        cap,
    )
}
fn response(status: &str, extra: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
#[test]
fn wire_preserves_status_retry_after_and_bounded_body_without_resending() {
    let clock = Arc::new(SystemClock::default());
    let http = UreqHttp::new(clock.clone());
    let cancel = AtomicBool::new(false);
    let (url, join) = fixture(response(
        "429 Too Many Requests",
        "Retry-After: 7\r\n",
        b"{}",
    ));
    let reply = http
        .send(&request(url, 2), clock.now().monotonic_ms + 2000, &cancel)
        .unwrap();
    assert_eq!(
        (reply.status, reply.retry_after_ms, reply.body),
        (429, Some(7000), b"{}".to_vec())
    );
    let wire = String::from_utf8(join.join().unwrap()).unwrap();
    assert!(wire.starts_with("POST /fixture HTTP/1.1\r\n"));
    assert!(
        wire.to_ascii_lowercase()
            .contains("authorization: bearer fixture-token\r\n")
    );
    assert!(wire.ends_with("{\"fixture\":true}"));
}
#[test]
fn next_byte_over_the_response_cap_is_rejected_without_exposing_body() {
    let clock = Arc::new(SystemClock::default());
    let http = UreqHttp::new(clock.clone());
    let (url, join) = fixture(response("200 OK", "", b"secret"));
    let error = http
        .send(
            &request(url, 5),
            clock.now().monotonic_ms + 2000,
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(error.kind, AuthErrorKind::InvalidReply);
    assert!(!error.to_string().contains("secret"));
    join.join().unwrap();
}
#[test]
fn redirect_returns_to_owner_without_following_the_location() {
    let clock = Arc::new(SystemClock::default());
    let http = UreqHttp::new(clock.clone());
    let (url, join) = fixture(response(
        "307 Temporary Redirect",
        "Location: http://127.0.0.1:1/forbidden\r\n",
        b"{}",
    ));
    let reply = http
        .send(
            &request(url, 2),
            clock.now().monotonic_ms + 2000,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(reply.status, 307);
    join.join().unwrap();
}
#[test]
fn cancelled_or_expired_calls_never_open_a_connection() {
    let clock = Arc::new(SystemClock::default());
    let http = UreqHttp::new(clock.clone());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let request = request(
        format!("http://{}/fixture", listener.local_addr().unwrap()),
        2,
    );
    let cancelled = AtomicBool::new(true);
    assert!(cancelled.cancelled());
    assert_eq!(
        http.send(&request, clock.now().monotonic_ms + 100, &cancelled)
            .unwrap_err()
            .kind,
        AuthErrorKind::Cancelled
    );
    assert_eq!(
        http.send(&request, 0, &AtomicBool::new(false))
            .unwrap_err()
            .kind,
        AuthErrorKind::Unavailable
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn an_in_flight_request_is_bounded_by_the_absolute_deadline_and_is_uncertain() {
    let clock = Arc::new(SystemClock::default());
    let http = UreqHttp::new(clock.clone());
    // The server consumes the request, then waits for the deadline to close
    // the socket instead of replying. No fixture sleep or retry is involved.
    let (url, join) = fixture(vec![]);
    let start = std::time::Instant::now();
    let error = http
        .send(
            &request(url, 2),
            clock.now().monotonic_ms + 200,
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(error.kind, AuthErrorKind::Unavailable);
    assert!(error.may_have_effect());
    assert!(start.elapsed() < Duration::from_secs(2));
    drop(http);
    assert!(
        join.join()
            .unwrap()
            .starts_with(b"POST /fixture HTTP/1.1\r\n")
    );
}
