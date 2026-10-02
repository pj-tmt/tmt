use super::*;

#[test]
fn response_drains_input_sent_after_its_fin_until_peer_eof() {
    let (mut socket, mut peer) = UnixStream::pair().unwrap();
    peer.set_read_timeout(Some(limits::RESPONSE)).unwrap();
    peer.set_write_timeout(Some(limits::RESPONSE)).unwrap();
    socket.set_read_timeout(Some(limits::RESPONSE)).unwrap();
    let worker = thread::spawn(move || {
        response(&mut socket, 429, b"CAPACITY", false).unwrap();
        socket.set_nonblocking(false).unwrap();
        assert_eq!(socket.read(&mut [0; 1]).unwrap(), 0, "input not drained");
    });
    let mut reply = String::new();
    peer.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 429"));
    assert!(reply.ends_with("CAPACITY"));
    // The server has already sent FIN; the request tail is intentionally late.
    peer.write_all(&vec![b'x'; limits::HTTP_BODY_BYTES])
        .unwrap();
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    worker.join().unwrap();
}

#[test]
fn websocket_keys_are_exactly_sixteen_base64_bytes() {
    // RFC 6455 section 1.3 sample key.
    assert!(websocket_key("dGhlIHNhbXBsZSBub25jZQ=="));
    assert!(websocket_key("AAAAAAAAAAAAAAAAAAAAAA=="));
    for invalid in [
        "",
        "dGhlIHNhbXBsZSBub25jZQ",
        "dGhlIHNhbXBsZSBub25jZQ=",
        "dGhlIHNhbXBsZSBub25jZR==",
        "dGhlIHNhbXBsZSBub25jZQ===",
        "dGhlIHNhbXBsZSBub25j-Q==",
        "dGhlIHNhbXBsZSBub25jZQ==\r",
        "AAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        assert!(!websocket_key(invalid), "{invalid:?}");
    }
}
