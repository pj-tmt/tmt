//! The upgrade over real Unix sockets: a valid handshake in both roles, every
//! refusal as a one-token change to an accepted twin, and every bound on the clock.
use super::super::tests::{GEN, expect, generation, idle, offer, quick, setup};
use super::*;
use std::{
    io::{Read, Write},
    os::unix::net::UnixListener,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

/// The request the initiator must write, byte for byte.
fn request() -> String {
    format!(
        "GET /.tmt/remote/object-channel-v1 HTTP/1.1\r\nHost: 127.0.0.1:4100\r\nConnection: Upgrade\r\nUpgrade: tmt-object-channel-v1\r\nContent-Length: 0\r\ntmt-mount: colab\r\ntmt-object-channel: 1\r\ntmt-object-generation: {GEN}\r\n\r\n"
    )
}
/// The reply the acceptor must write, byte for byte.
fn reply() -> String {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tmt-object-channel-v1\r\ntmt-object-generation: {GEN}\r\n\r\n"
    )
}
/// `text` with its one occurrence of `from` replaced by `to`.
fn mutate(text: &str, from: &str, to: &str) -> String {
    assert_eq!(text.matches(from).count(), 1, "{from} must occur once");
    text.replacen(from, to, 1)
}
fn refused(refusal: Refusal) -> Fault {
    Fault::Handshake(refusal)
}

/// Send `bytes` to an acceptor and return its result and whatever it replied.
fn offered(bytes: &[u8]) -> (Result<Link, Fault>, Vec<u8>) {
    let (mut client, server) = UnixStream::pair().unwrap();
    let accepted = thread::spawn(move || accept(server, &expect(), &quick(), setup()));
    client.write_all(bytes).unwrap();
    let result = accepted.join().unwrap();
    // A refused link was dropped, so the client reads to EOF; an accepted one is
    // still open, so read only what is there.
    let mut replied = Vec::new();
    if result.is_err() {
        client.read_to_end(&mut replied).unwrap();
    } else {
        client.set_nonblocking(true).unwrap();
        let mut chunk = [0u8; 1024];
        if let Ok(count) = client.read(&mut chunk) {
            replied.extend_from_slice(&chunk[..count]);
        }
    }
    (result, replied)
}

#[test]
fn the_acceptor_replies_exactly_once_to_the_canonical_request() {
    let (result, replied) = offered(request().as_bytes());
    let link = result.unwrap();
    assert_eq!(link.generation(), generation(GEN));
    assert_eq!(String::from_utf8(replied).unwrap(), reply());
}

#[test]
fn the_acceptor_refuses_every_one_token_change_and_replies_nothing() {
    let pad = "a".repeat(9000);
    let many: String = (0..40).map(|n| format!("X-N{n}: 1\r\n")).collect();
    let after = "tmt-object-channel: 1\r\n";
    #[rustfmt::skip]
    let rows: Vec<(&str, String, Refusal)> = vec![
        ("method", mutate(&request(), "GET /", "POST /"), Refusal::Method),
        ("lowercase method", mutate(&request(), "GET /", "get /"), Refusal::Method),
        ("query", mutate(&request(), "channel-v1 HTTP", "channel-v1?x=1 HTTP"), Refusal::Path),
        ("other route", mutate(&request(), "object-channel-v1 HTTP", "object-channel-v2 HTTP"), Refusal::Path),
        ("route case", mutate(&request(), "/remote/", "/Remote/"), Refusal::Path),
        ("http 1.0", mutate(&request(), "HTTP/1.1\r\n", "HTTP/1.0\r\n"), Refusal::Version),
        ("host", mutate(&request(), "Host: 127.0.0.1:4100", "Host: 127.0.0.1:4101"), Refusal::BadValue("host")),
        ("mount", mutate(&request(), "tmt-mount: colab", "tmt-mount: other"), Refusal::BadValue("tmt-mount")),
        ("connection", mutate(&request(), "Connection: Upgrade", "Connection: close"), Refusal::BadValue("connection")),
        ("upgrade", mutate(&request(), "tmt-object-channel-v1\r\nContent", "websocket\r\nContent"), Refusal::BadValue("upgrade")),
        ("body length", mutate(&request(), "Content-Length: 0", "Content-Length: 1"), Refusal::BadValue("content-length")),
        ("padded length", mutate(&request(), "Content-Length: 0", "Content-Length: 00"), Refusal::BadValue("content-length")),
        ("channel version", mutate(&request(), after, "tmt-object-channel: 2\r\n"), Refusal::BadValue("tmt-object-channel")),
        ("uppercase generation", mutate(&request(), GEN, &GEN.to_uppercase()), Refusal::BadValue("tmt-object-generation")),
        ("version 1 generation", mutate(&request(), "-4b86-", "-1b86-"), Refusal::BadValue("tmt-object-generation")),
        ("missing mount", mutate(&request(), "tmt-mount: colab\r\n", ""), Refusal::MissingHeader("tmt-mount")),
        ("missing generation", mutate(&request(), &format!("tmt-object-generation: {GEN}\r\n"), ""), Refusal::MissingHeader("tmt-object-generation")),
        ("repeated field", mutate(&request(), after, &format!("{after}{after}")), Refusal::DuplicateHeader),
        ("repeated field in another case", mutate(&request(), after, &format!("{after}TMT-Object-Channel: 1\r\n")), Refusal::DuplicateHeader),
        ("cookie", mutate(&request(), after, &format!("{after}Cookie: a=b\r\n")), Refusal::UnknownHeader),
        ("transfer encoding", mutate(&request(), after, &format!("{after}Transfer-Encoding: chunked\r\n")), Refusal::UnknownHeader),
        ("websocket key", mutate(&request(), after, &format!("{after}Sec-WebSocket-Key: x\r\n")), Refusal::UnknownHeader),
        ("device context", mutate(&request(), after, &format!("{after}tmt-device-context: {{}}\r\n")), Refusal::UnknownHeader),
        ("origin", mutate(&request(), after, &format!("{after}Origin: http://x\r\n")), Refusal::UnknownHeader),
        ("head over 8 KiB", mutate(&request(), "tmt-mount: colab\r\n", &format!("tmt-mount: colab\r\nX-Pad: {pad}\r\n")), Refusal::Head),
        ("over 32 fields", mutate(&request(), after, &format!("{after}{many}")), Refusal::Head),
        ("garbage", "GARBAGE\r\n\r\n".into(), Refusal::Head),
        ("bytes before the reply", format!("{}{{\"x\":1}}", request()), Refusal::Pipelined),
    ];
    // The accepted twin of every row.
    assert!(offered(request().as_bytes()).0.is_ok());
    for (name, text, refusal) in rows {
        let (result, replied) = offered(text.as_bytes());
        assert_eq!(result.err(), Some(refused(refusal)), "{name}");
        assert!(replied.is_empty(), "{name} was answered");
    }
}

#[test]
fn the_acceptor_ends_at_its_head_bound_and_at_the_setup_bound() {
    let whole = request();
    let half = &whole.as_bytes()[..40];
    for (head, outer, expected) in [
        (
            Duration::from_millis(150),
            Duration::from_secs(5),
            Duration::from_millis(150),
        ),
        (
            Duration::from_secs(5),
            Duration::from_millis(100),
            Duration::from_millis(100),
        ),
    ] {
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(half).unwrap();
        let budgets = Budgets { head, ..quick() };
        let started = Instant::now();
        let result = accept(server, &expect(), &budgets, started + outer);
        let took = started.elapsed();
        assert_eq!(result.err(), Some(Fault::Timeout(Stage::Head)));
        assert!(
            took >= expected - Duration::from_millis(5) && took < expected * 4,
            "{took:?}"
        );
        drop(client);
    }
    // A closed peer before the blank line is not a timeout.
    let (mut client, server) = UnixStream::pair().unwrap();
    client.write_all(half).unwrap();
    drop(client);
    assert_eq!(
        accept(server, &expect(), &quick(), setup()).err(),
        Some(Fault::Truncated)
    );
}

/// Fill the send buffer of `stream` so the next write has to wait.
fn fill(stream: &UnixStream) {
    let mut writer = stream.try_clone().unwrap();
    writer.set_nonblocking(true).unwrap();
    while writer.write(&[0u8; 4096]).is_ok() {}
}

#[test]
fn a_reply_the_peer_will_not_read_ends_at_the_reply_bound() {
    let (mut client, server) = UnixStream::pair().unwrap();
    fill(&server);
    client.write_all(request().as_bytes()).unwrap();
    let started = Instant::now();
    let result = accept(server, &expect(), &quick(), setup());
    assert_eq!(result.err(), Some(Fault::Timeout(Stage::Reply)));
    let took = started.elapsed();
    assert!(
        took >= quick().reply - Duration::from_millis(5) && took < quick().reply * 4,
        "{took:?}"
    );
}

/// Read one head from `server`, then answer with `reply`.
fn scripted(server: UnixStream, reply: String) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut server = server;
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            server.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        server.write_all(reply.as_bytes()).unwrap();
        String::from_utf8(head).unwrap()
    })
}

#[test]
fn the_initiator_writes_the_canonical_request_and_accepts_the_canonical_reply() {
    let (client, server) = UnixStream::pair().unwrap();
    let peer = scripted(server, reply());
    let link = initiate(client, &offer(), setup()).unwrap();
    assert_eq!(peer.join().unwrap(), request());
    assert_eq!(link.generation(), generation(GEN));
}

#[test]
fn the_initiator_refuses_every_one_token_change_to_the_reply() {
    let after = "tmt-object-generation";
    let pad = "a".repeat(9000);
    #[rustfmt::skip]
    let rows: Vec<(&str, String, Fault)> = vec![
        ("not an upgrade", mutate(&reply(), "101 Switching Protocols", "200 OK"), refused(Refusal::Status)),
        ("missing route", mutate(&reply(), "101 Switching Protocols", "404 Not Found"), refused(Refusal::Status)),
        ("other reason", mutate(&reply(), "101 Switching Protocols", "101 Upgrade"), refused(Refusal::Status)),
        ("http 1.0", mutate(&reply(), "HTTP/1.1 101", "HTTP/1.0 101"), refused(Refusal::Version)),
        ("connection", mutate(&reply(), "Connection: Upgrade", "Connection: close"), refused(Refusal::BadValue("connection"))),
        ("upgrade", mutate(&reply(), "Upgrade: tmt-object-channel-v1", "Upgrade: websocket"), refused(Refusal::BadValue("upgrade"))),
        ("another generation", mutate(&reply(), "3f14\r\n", "3f15\r\n"), refused(Refusal::Generation)),
        ("malformed generation", mutate(&reply(), GEN, &GEN.to_uppercase()), refused(Refusal::BadValue(after))),
        ("missing generation", mutate(&reply(), &format!("{after}: {GEN}\r\n"), ""), refused(Refusal::MissingHeader(after))),
        ("body length", mutate(&reply(), "Connection: Upgrade\r\n", "Connection: Upgrade\r\nContent-Length: 0\r\n"), refused(Refusal::UnknownHeader)),
        ("repeated field", mutate(&reply(), "Connection: Upgrade\r\n", "Connection: Upgrade\r\nConnection: Upgrade\r\n"), refused(Refusal::DuplicateHeader)),
        ("head over 8 KiB", mutate(&reply(), "Connection: Upgrade\r\n", &format!("Connection: Upgrade\r\nX-Pad: {pad}\r\n")), refused(Refusal::Head)),
        ("garbage", "NOT HTTP\r\n\r\n".into(), refused(Refusal::Head)),
        ("closed before the blank line", "HTTP/1.1 101".into(), Fault::Truncated),
    ];
    // The accepted twin of every row.
    let (client, server) = UnixStream::pair().unwrap();
    let peer = scripted(server, reply());
    assert!(initiate(client, &offer(), setup()).is_ok());
    peer.join().unwrap();
    for (name, text, fault) in rows {
        let (client, server) = UnixStream::pair().unwrap();
        let peer = scripted(server, text);
        let result = initiate(client, &offer(), setup());
        assert_eq!(result.err(), Some(fault), "{name}");
        peer.join().unwrap();
    }
}

#[test]
fn the_initiator_ends_at_the_setup_bound_and_never_sends_an_unsafe_offer() {
    let (client, server) = UnixStream::pair().unwrap();
    let started = Instant::now();
    let result = initiate(client, &offer(), started + Duration::from_millis(150));
    assert_eq!(result.err(), Some(Fault::Timeout(Stage::Head)));
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(145) && took < Duration::from_millis(600),
        "{took:?}"
    );
    drop(server);

    // A request the peer will not read.
    let (client, server) = UnixStream::pair().unwrap();
    fill(&client);
    let result = initiate(
        client,
        &offer(),
        Instant::now() + Duration::from_millis(150),
    );
    assert_eq!(result.err(), Some(Fault::Timeout(Stage::Reply)));
    drop(server);

    // A host that could split the head writes nothing.
    for host in ["a\r\nX: 1", "", "a b ", &"h".repeat(256)] {
        let (client, mut server) = UnixStream::pair().unwrap();
        let unsafe_offer = Offer {
            host: host.into(),
            ..offer()
        };
        assert_eq!(
            initiate(client, &unsafe_offer, setup()).err(),
            Some(refused(Refusal::BadValue("host"))),
            "{host:?}"
        );
        let mut seen = Vec::new();
        server.read_to_end(&mut seen).unwrap();
        assert!(seen.is_empty());
    }
}

static SOCKETS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn the_handshake_works_over_a_bound_socket_path() {
    let path = std::env::temp_dir().join(format!(
        "tmt-car-{}-{}.sock",
        std::process::id(),
        SOCKETS.fetch_add(1, Ordering::Relaxed)
    ));
    let listener = UnixListener::bind(&path).unwrap();
    let acceptor = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        accept(stream, &expect(), &quick(), setup())
    });
    let client = UnixStream::connect(&path).unwrap();
    let mut initiated = initiate(client, &offer(), setup()).unwrap();
    let mut accepted = acceptor.join().unwrap().unwrap();
    std::fs::remove_file(&path).unwrap();
    let frame = super::super::tests::admission(GEN, 3);
    initiated.write_frame(&frame, &quick()).unwrap();
    assert_eq!(accepted.read_frame(idle(), &quick()).unwrap(), frame);
}
