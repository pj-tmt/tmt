use super::*;

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
