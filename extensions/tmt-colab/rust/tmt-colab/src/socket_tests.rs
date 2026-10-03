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

struct ReaderMount {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    path: std::path::PathBuf,
}
impl Drop for ReaderMount {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        assert!(!self.path.exists(), "reader mount leaked its socket");
    }
}

/// Real mount workers plus the same sync lock used by owner management/events.
#[test]
fn mounted_link_reader_is_cut_off_by_narrowing_and_individual_revocation_twice() {
    use crate::readers::test_support::{DEVICE, Fixture, LINK, PAGE, SEED};
    use crate::transitions::ShareMode;
    use serde_json::json;
    use tmt_colab_model::values;
    use tungstenite::{Message, WebSocket, protocol::Role};
    for revoke in [false, true] {
        for _ in 0..2 {
            let mut f = Fixture::new();
            f.share(ShareMode::Link);
            f.add_link();
            let layout = Layout::open(&f.root).unwrap();
            let mount = MountSocket::bind(&layout, &f.key.space_id, Tunnels::PRODUCT)
                .unwrap()
                .with_registration(&layout, f.service.clone())
                .unwrap();
            f.server = mount.sync.as_ref().unwrap().clone();
            let path = mount.path.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let worker = thread::spawn(move || mount.run(&flag).unwrap());
            let _mount = ReaderMount {
                stop,
                worker: Some(worker),
                path: path.clone(),
            };
            let session = f.session(true);
            let token = session["token"].as_str().unwrap();
            let mut socket = UnixStream::connect(&path).unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            socket.write_all(format!("GET /sync HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: colab-sync-v1, colab-reader-v1.{token}\r\n\r\n").as_bytes()).unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut b = [0];
                socket.read_exact(&mut b).unwrap();
                head.push(b[0]);
            }
            let head = String::from_utf8(head).unwrap();
            assert!(head.starts_with("HTTP/1.1 101"));
            assert!(!head.contains(token));
            let mut peer = WebSocket::from_raw_socket(socket, Role::Client, None);
            let scope = f.scope();
            let frame = |kind: &str| json!({"version":1,"type":kind,"space":scope.space,"page":PAGE,"epoch":scope.epoch,"device":session["principal"],"membershipRevision":"0","cursors":[]});
            peer.send(Message::Text(frame("hello").to_string().into()))
                .unwrap();
            let mut completed = false;
            for _ in 0..8 {
                let p: serde_json::Value =
                    serde_json::from_str(peer.read().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(p["type"], "catchup");
                peer.send(Message::Text(json!({"version":1,"type":"ack","space":scope.space,"page":PAGE,"epoch":scope.epoch,"cursors":[]}).to_string().into())).unwrap();
                if p["more"] == false {
                    completed = true;
                    break;
                }
            }
            assert!(completed, "bounded reader catchup did not complete");
            if revoke {
                let body = json!({"type":"device.revoked","deviceId":DEVICE,"grantRevision":1})
                    .to_string();
                let mut event = UnixStream::connect(&path).unwrap();
                event
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                event.write_all(format!("POST {EVENTS} HTTP/1.1\r\ntmt-device-event: 1\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).unwrap();
                let mut response = String::new();
                event.read_to_string(&mut response).unwrap();
                assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            } else {
                f.share(ShareMode::Private);
            }
            // No further application data may be flushed after the owner commit.
            match peer.read() {
                Ok(Message::Close(_)) => {}
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {}
                Err(tungstenite::Error::Protocol(
                    tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                )) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                    ) => {}
                other => panic!("reader not cut off: {other:?}"),
            }
            let body=json!({"kind":"link","space":f.key.space_id,"page":PAGE,"chain":values::encode_binary(&f.chain)}).to_string();
            let mut denied = UnixStream::connect(&path).unwrap();
            denied
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            denied
                .write_all(
                    format!(
                        "POST {} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                        crate::readers::CHALLENGE_PATH,
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            let mut result = String::new();
            denied.read_to_string(&mut result).unwrap();
            assert!(result.starts_with("HTTP/1.1 403"), "{result}");
            // A retained seed can certify a fresh device only while its link survives.
            let chain = tmt_colab_model::certificate::Chain::from_json(&f.chain).unwrap();
            let mut cert = chain.certificate().unwrap();
            cert.device_id = "30000000-0000-4000-8000-000000000002";
            let keys = tmt_colab_model::link::Keys::derive(&SEED, &f.key.space_id, LINK).unwrap();
            let mut wire: serde_json::Value = serde_json::from_slice(&f.chain).unwrap();
            wire["deviceCertificate"] = json!(values::encode_binary(
                &tmt_colab_model::certificate::input(&cert).unwrap()
            ));
            wire["issuerSignature"] = json!(values::encode_binary(&keys.certify(&cert).unwrap()));
            let fresh=f.request(crate::readers::CHALLENGE_PATH,json!({"kind":"link","space":f.key.space_id,"page":PAGE,"chain":values::encode_binary(&serde_json::to_vec(&wire).unwrap())}));
            if revoke {
                assert!(
                    fresh.is_ok(),
                    "surviving seed fresh-device control: {fresh:?}"
                );
            } else {
                assert_eq!(fresh, Err(crate::registration::Code::Denied));
            }
        }
    }
}
