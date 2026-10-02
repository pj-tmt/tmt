use super::*;
use std::{
    net::{Ipv4Addr, TcpListener},
    thread,
    time::Duration,
};
use tmt_core::binding::session::ProviderSessionId;

#[derive(Clone, Copy)]
enum Reply {
    Accept,
    Internal,
    Archived,
    Deleted,
    Lost,
    WrongId,
}

#[allow(
    clippy::result_large_err,
    reason = "Tungstenite fixes the handshake callback error type."
)]
fn exercise(reply: Reply, version: &str) -> QueueOutcome {
    let user_agent = format!("tmt/{version} (fixture)");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let SocketAddr::V4(address) = listener.local_addr().unwrap() else {
        panic!("IPv4 fixture")
    };
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut socket = tungstenite::accept_hdr(
            stream,
            |request: &tungstenite::handshake::server::Request, response| {
                assert_eq!(
                    request.headers().get("authorization").unwrap(),
                    "Bearer fixture-capability"
                );
                Ok(response)
            },
        )
        .unwrap();
        let init: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(init["method"], "initialize");
        socket
            .send(Message::Text(
                json!({"id":init["id"],"result":{"userAgent":user_agent}})
                    .to_string()
                    .into(),
            ))
            .unwrap();
        let initialized: Value =
            serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(initialized["method"], "initialized");
        let queue: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(queue["method"], "thread/queue/add");
        assert_eq!(queue["params"]["threadId"], "owned-thread");
        let value = match reply {
            Reply::Accept => json!({"id":queue["id"], "result":{"queuedSubmission":{
                "id":"provider-submission", "clientUserMessageId":queue["params"]["clientUserMessageId"], "input":queue["params"]["input"]
            }}}),
            Reply::Internal => {
                json!({"id":queue["id"], "error":{"code":-32603,"message":"failed to serialize queued submission"}})
            }
            Reply::Archived => {
                json!({"id":queue["id"], "error":{"code":-32600,"message":"session owned-thread is archived. Run `codex unarchive owned-thread` to unarchive it first."}})
            }
            Reply::Deleted => {
                json!({"id":queue["id"], "error":{"code":-32603,"message":"failed to read thread: invalid thread-store request: no rollout found for thread id owned-thread"}})
            }
            Reply::WrongId => {
                json!({"id":"different-request", "error":{"code":-32603,"message":"internal error"}})
            }
            Reply::Lost => {
                drop(socket);
                return listener;
            }
        };
        socket
            .send(Message::Text(value.to_string().into()))
            .unwrap();
        // Client must close after its single attempt, including every error.
        // A second frame on this connection fails this assertion.
        assert!(socket.read().is_err());
        listener
    });
    let endpoint = Endpoint::new(address, "fixture-capability".into()).unwrap();
    let client = Client::connect(&endpoint, Instant::now() + Duration::from_secs(2)).unwrap();
    let result = client.queue(
        &QueueRequest::new(ProviderSessionId::new("owned-thread").unwrap(), "tiny").unwrap(),
    );
    let listener = server.join().unwrap();
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "no second connection/resend"
    );
    result
}

#[test]
fn acceptance_is_one_delivery_write_with_a_provider_receipt() {
    for version in ["0.159.2", "0.159.3", "0.160.0"] {
        assert_eq!(
            exercise(Reply::Accept, version),
            QueueOutcome::Accepted {
                submission_id: "provider-submission".into()
            }
        );
    }
}

#[test]
fn internal_error_lost_receipt_and_wrong_correlation_never_resend() {
    for reply in [Reply::Internal, Reply::Lost, Reply::WrongId] {
        assert_eq!(exercise(reply, "0.160.0"), QueueOutcome::Uncertain);
    }
}

#[test]
fn explicit_archived_and_deleted_refusal_never_resends() {
    assert_eq!(
        exercise(Reply::Archived, "0.160.0"),
        QueueOutcome::Refused { code: -32600 }
    );
    assert_eq!(
        exercise(Reply::Deleted, "0.160.0"),
        QueueOutcome::Refused { code: -32603 }
    );
}

#[test]
fn connect_refusal_is_before_any_delivery_write() {
    use nix::sys::socket::{
        AddressFamily, SockFlag, SockType, SockaddrIn, bind, getsockname, socket,
    };
    use std::os::fd::{AsFd, AsRawFd};
    // Hold the allocation without listen(): dropping a listener first would let
    // another parallel test reuse the port before this connect attempt.
    let reserved = socket(
        AddressFamily::Inet,
        SockType::Stream,
        SockFlag::empty(),
        None,
    )
    .unwrap();
    nix::fcntl::fcntl(
        reserved.as_fd(),
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
    )
    .unwrap();
    bind(
        reserved.as_raw_fd(),
        &SockaddrIn::from(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
    )
    .unwrap();
    let address = SocketAddrV4::from(getsockname::<SockaddrIn>(reserved.as_raw_fd()).unwrap());
    assert_eq!(
        TcpListener::bind(address).unwrap_err().kind(),
        std::io::ErrorKind::AddrInUse,
        "the refusal fixture must retain exclusive ownership of its port"
    );
    let endpoint = Endpoint::new(address, "fixture".into()).unwrap();
    assert!(matches!(
        Client::connect(&endpoint, Instant::now() + Duration::from_secs(1)),
        Err(TransportError::Unreachable)
    ));
}

#[test]
fn version_comes_from_provider_build_prefix_not_client_suffix() {
    for version in [
        "tmt/0.159.2 (OS) (client; 999)",
        "codex_cli_rs/0.159.3 (OS)",
        "tmt/0.160.0 (OS) (client; 0.159.3)",
    ] {
        assert!(supported_version(version));
    }
    for version in [
        "tmt/0.159.4 (OS)",
        "tmt/0.160.1 (tmt; 0.160.0)",
        "tmt/0.160.999 (OS)",
        "tmt/0.161.0 (OS)",
        "tmt/0.160.0-dev",
        "tmt/0.160 (OS)",
        "tmt/0.160.0.1 (OS)",
        "tmt/0.159.1 (tmt; 0.159.3)",
        "0.159.3",
        "/0.159.3",
        "tmt/0.159.3-dev",
    ] {
        assert!(!supported_version(version));
    }
}

#[test]
fn slow_partial_handshake_cannot_renew_the_absolute_deadline() {
    use std::{
        io::{self, Read, Write},
        sync::mpsc,
    };
    struct Drip {
        stream: TcpStream,
        stop: mpsc::Receiver<()>,
    }
    impl Read for Drip {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.stream.read(bytes)
        }
    }
    impl Write for Drip {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            match self.stop.recv_timeout(Duration::from_millis(40)) {
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.stream.write(&bytes[..bytes.len().min(1)])
                }
                _ => Err(io::ErrorKind::BrokenPipe.into()),
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            self.stream.flush()
        }
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let SocketAddr::V4(address) = listener.local_addr().unwrap() else {
        panic!("IPv4 fixture")
    };
    let (stop, stopped) = mpsc::channel();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert!(
            tungstenite::accept(Drip {
                stream,
                stop: stopped
            })
            .is_err()
        );
    });
    let start = Instant::now();
    let outcome = Client::connect(
        &Endpoint::new(address, "fixture".into()).unwrap(),
        start + Duration::from_millis(150),
    );
    let elapsed = start.elapsed();
    let _ = stop.send(());
    server.join().unwrap();
    assert!(matches!(outcome, Err(TransportError::NotReady)));
    assert!(
        elapsed < Duration::from_millis(500),
        "handshake renewed budget: {elapsed:?}"
    );
}

#[test]
fn partial_delivery_receipt_expires_uncertain_without_resend() {
    use std::sync::mpsc;
    use tungstenite::protocol::frame::{
        Frame,
        coding::{Data, OpCode},
    };
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let SocketAddr::V4(address) = listener.local_addr().unwrap() else {
        panic!("IPv4 fixture")
    };
    let (stop, stopped) = mpsc::channel();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        let init: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        socket
            .send(Message::Text(
                json!({"id":init["id"], "result":{"userAgent":"tmt/0.159.3 (fixture)"}})
                    .to_string()
                    .into(),
            ))
            .unwrap();
        socket.read().unwrap(); // initialized
        let queue: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(queue["method"], "thread/queue/add");
        socket
            .send(Message::Frame(Frame::message(
                vec![b'{'],
                OpCode::Data(Data::Text),
                false,
            )))
            .unwrap();
        // Each fragment arrives within a per-read timeout. Only an absolute
        // stream deadline stops the unfinished message within the total budget.
        for _ in 0..25 {
            match stopped.recv_timeout(Duration::from_millis(40)) {
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if socket
                        .send(Message::Frame(Frame::message(
                            vec![b' '],
                            OpCode::Data(Data::Continue),
                            false,
                        )))
                        .is_err()
                    {
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(
            socket.read().is_err(),
            "a second delivery frame must not follow timeout"
        );
        listener
    });
    let start = Instant::now();
    let client = Client::connect(
        &Endpoint::new(address, "fixture".into()).unwrap(),
        start + Duration::from_millis(150),
    )
    .unwrap();
    let result = client.queue(
        &QueueRequest::new(ProviderSessionId::new("owned-thread").unwrap(), "tiny").unwrap(),
    );
    let elapsed = start.elapsed();
    let _ = stop.send(());
    let listener = server.join().unwrap();
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(result, QueueOutcome::Uncertain);
    assert!(
        elapsed < Duration::from_millis(500),
        "frame renewed budget: {elapsed:?}"
    );
}
