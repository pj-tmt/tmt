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
    peer.shutdown(Shutdown::Write).unwrap();
    worker.join().unwrap();
}
