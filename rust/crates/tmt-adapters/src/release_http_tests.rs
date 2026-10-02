use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use rcgen::generate_simple_self_signed;
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use ureq::{
    Agent,
    tls::{Certificate, RootCerts, TlsConfig},
};

use super::{Https, MAX_REDIRECTS, MAX_RESPONSE_HEADER, USER_AGENT};

const SERVER_WAIT: Duration = Duration::from_secs(2);

struct Response {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
    declared_length: Option<usize>,
    emit_length: bool,
    hold_body: bool,
    header_delay: Duration,
}

impl Response {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            location: None,
            body: body.into(),
            declared_length: None,
            emit_length: true,
            hold_body: false,
            header_delay: Duration::ZERO,
        }
    }

    fn not_found() -> Self {
        Self {
            status: 404,
            ..Self::ok("missing")
        }
    }

    fn redirect(location: String) -> Self {
        Self {
            status: 302,
            location: Some(location),
            ..Self::ok(Vec::new())
        }
    }

    fn truncated(declared_length: usize, body: impl Into<Vec<u8>>) -> Self {
        Self {
            declared_length: Some(declared_length),
            ..Self::ok(body)
        }
    }

    fn without_length(body: impl Into<Vec<u8>>) -> Self {
        Self {
            emit_length: false,
            ..Self::ok(body)
        }
    }

    fn held(declared_length: usize) -> Self {
        Self {
            declared_length: Some(declared_length),
            hold_body: true,
            ..Self::ok(Vec::new())
        }
    }

    fn delayed_headers(mut self, delay: Duration) -> Self {
        self.header_delay = delay;
        self
    }
}

struct TestServer {
    address: SocketAddr,
    release: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<io::Result<()>>>,
    accepted: Option<Arc<AtomicUsize>>,
}

impl TestServer {
    fn spawn(config: Arc<ServerConfig>, response: Response) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local TLS listener");
        listener
            .set_nonblocking(true)
            .expect("set local listener nonblocking");
        let address = listener.local_addr().expect("read local listener address");
        let (release, released) = mpsc::channel();
        let thread = thread::spawn(move || serve(listener, config, response, released));
        Self {
            address,
            release: Some(release),
            thread: Some(thread),
            accepted: None,
        }
    }

    fn spawn_redirect_loop(config: Arc<ServerConfig>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local TLS listener");
        listener
            .set_nonblocking(true)
            .expect("set local listener nonblocking");
        let address = listener.local_addr().expect("read local listener address");
        let (release, released) = mpsc::channel();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&accepted);
        let thread =
            thread::spawn(move || serve_redirect_loop(listener, config, address, released, count));
        Self {
            address,
            release: Some(release),
            thread: Some(thread),
            accepted: Some(accepted),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("https://{}{}", self.address, path)
    }

    fn accepted(&self) -> usize {
        self.accepted
            .as_ref()
            .map_or(0, |count| count.load(Ordering::SeqCst))
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    response: Response,
    released: mpsc::Receiver<()>,
) -> io::Result<()> {
    let stream = accept(&listener, &released)?;
    serve_connection(stream, config, response, &released)
}

fn serve_redirect_loop(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    address: SocketAddr,
    released: mpsc::Receiver<()>,
    accepted: Arc<AtomicUsize>,
) -> io::Result<()> {
    for _ in 0..=MAX_REDIRECTS {
        let stream = accept(&listener, &released)?;
        accepted.fetch_add(1, Ordering::SeqCst);
        serve_connection(
            stream,
            Arc::clone(&config),
            Response::redirect(format!("https://{address}/loop")),
            &released,
        )?;
    }
    if let Ok(stream) = accept(&listener, &released) {
        accepted.fetch_add(1, Ordering::SeqCst);
        drop(stream);
    }
    Ok(())
}

fn serve_connection(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    response: Response,
    released: &mpsc::Receiver<()>,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(SERVER_WAIT))?;
    stream.set_write_timeout(Some(SERVER_WAIT))?;
    let connection = ServerConnection::new(config).map_err(io::Error::other)?;
    let mut stream = StreamOwned::new(connection, stream);
    read_request(&mut stream)?;
    thread::sleep(response.header_delay);

    write!(
        stream,
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason(response.status)
    )?;
    if let Some(location) = &response.location {
        write!(stream, "Location: {location}\r\n")?;
    }
    if response.emit_length {
        let length = response.declared_length.unwrap_or(response.body.len());
        write!(stream, "Content-Length: {length}\r\n")?;
    }
    write!(stream, "Connection: close\r\n\r\n")?;
    stream.flush()?;

    if response.hold_body {
        let _ = released.recv_timeout(SERVER_WAIT);
    } else {
        stream.write_all(&response.body)?;
        stream.flush()?;
        let _ = stream.get_mut().shutdown(Shutdown::Write);
    }
    Ok(())
}

fn accept(listener: &TcpListener, released: &mpsc::Receiver<()>) -> io::Result<TcpStream> {
    let deadline = Instant::now() + SERVER_WAIT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if released.try_recv().is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "test server released",
                    ));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "test server accept",
                    ));
                }
                thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
}

fn read_request(stream: &mut impl Read) -> io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "test request ended",
            ));
        }
        request.extend_from_slice(&buffer[..count]);
        if request.len() > 16 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "test request too large",
            ));
        }
    }
    Ok(())
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        404 => "Not Found",
        _ => "Status",
    }
}

fn tls_material() -> (Arc<ServerConfig>, Vec<u8>) {
    let certified = generate_simple_self_signed(vec!["127.0.0.1".to_owned()])
        .expect("generate test TLS certificate");
    let certificate = certified.cert.der().as_ref().to_vec();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certified.signing_key.serialize_der(),
    ));
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(certificate.clone())], key)
        .expect("build test TLS server config");
    (Arc::new(config), certificate)
}

fn client(root: &[u8]) -> Https {
    let roots = [Certificate::from_der(root).to_owned()];
    let config = Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .max_response_header_size(MAX_RESPONSE_HEADER)
        .user_agent(USER_AGENT)
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::new_with_certs(&roots))
                .build(),
        )
        .build();
    Https::with_test_agent(Agent::new_with_config(config))
}

#[test]
fn valid_platform_like_task_trust_fetches_over_real_tls() {
    let (config, certificate) = tls_material();
    let server = TestServer::spawn(config, Response::ok("native"));
    let bytes = client(&certificate)
        .get(
            &server.url("/release"),
            "application/json",
            64,
            Instant::now() + Duration::from_secs(1),
        )
        .expect("trusted TLS request");
    assert_eq!(bytes, b"native");
}

#[test]
fn wrong_task_trust_rejects_real_tls() {
    let (config, _server_certificate) = tls_material();
    let (_wrong_config, wrong_certificate) = tls_material();
    let server = TestServer::spawn(config, Response::ok("secret"));
    let error = client(&wrong_certificate)
        .get(
            &server.url("/release"),
            "application/json",
            64,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("wrong TLS trust must fail");
    assert_eq!(error.kind(), io::ErrorKind::Other);
}

#[test]
fn http_and_unapproved_redirects_are_rejected_before_following() {
    let production = Https::new();
    let error = production
        .get(
            "http://api.github.com/releases",
            "application/json",
            64,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("HTTP URL must be rejected");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

    let (config, certificate) = tls_material();
    let server = TestServer::spawn(
        config,
        Response::redirect("https://example.com/release".to_owned()),
    );
    let error = client(&certificate)
        .get(
            &server.url("/redirect"),
            "application/json",
            64,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("unapproved redirect must be rejected");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn oversized_body_is_rejected_before_unbounded_read() {
    let (config, certificate) = tls_material();
    let server = TestServer::spawn(config, Response::without_length(vec![b'x'; 5]));
    let error = client(&certificate)
        .get(
            &server.url("/oversized"),
            "application/octet-stream",
            4,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("oversized body must fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn content_length_overflow_is_rejected_before_body_read() {
    let (config, certificate) = tls_material();
    let server = TestServer::spawn(config, Response::truncated(5, Vec::new()));
    let error = client(&certificate)
        .get(
            &server.url("/declared-oversized"),
            "application/octet-stream",
            4,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("oversized Content-Length must fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn not_found_status_maps_to_not_found() {
    let (config, certificate) = tls_material();
    let server = TestServer::spawn(config, Response::not_found());
    let error = client(&certificate)
        .get(
            &server.url("/missing"),
            "application/json",
            64,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("404 must fail");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[test]
fn truncated_body_is_rejected() {
    let (config, certificate) = tls_material();
    let server = TestServer::spawn(config, Response::truncated(5, "abc"));
    let error = client(&certificate)
        .get(
            &server.url("/truncated"),
            "application/octet-stream",
            64,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("truncated body must fail");
    assert_eq!(error.kind(), io::ErrorKind::Other);
}

#[test]
fn one_deadline_covers_redirect_and_body_wait() {
    let (config, certificate) = tls_material();
    let second = TestServer::spawn(config.clone(), Response::held(1));
    let first = TestServer::spawn(
        config,
        Response::redirect(second.url("/slow")).delayed_headers(Duration::from_millis(1_400)),
    );
    let deadline = Instant::now() + Duration::from_millis(1_600);
    let (done, result) = mpsc::channel();
    let first_url = first.url("/redirect");
    let thread = thread::spawn(move || {
        let result = client(&certificate).get(&first_url, "application/octet-stream", 64, deadline);
        done.send(result).unwrap();
    });
    let received = result.recv_timeout(SERVER_WAIT);
    // Release both fixture servers and join the client even when the deadline
    // assertion is about to fail; a failed test must not detach its worker.
    drop(first);
    drop(second);
    thread.join().unwrap();
    let error = received
        .expect("global deadline should finish before fixture cleanup")
        .expect_err("body wait must share the redirect deadline");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn real_tls_redirect_loop_stops_at_bound_without_external_contact() {
    let (config, certificate) = tls_material();
    let server = TestServer::spawn_redirect_loop(config);
    let error = client(&certificate)
        .get(
            &server.url("/loop"),
            "application/octet-stream",
            64,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("redirect loop must hit the local bound");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(server.accepted(), MAX_REDIRECTS + 1);
}

#[test]
fn request_errors_retain_causes_without_echoing_rejected_credentials() {
    let refused = super::map_ureq_error(ureq::Error::Io(io::Error::new(
        io::ErrorKind::ConnectionRefused,
        "connection refused by fixture",
    )));
    assert_eq!(refused.kind(), io::ErrorKind::Other);
    assert!(refused.to_string().contains("ConnectionRefused"));
    assert!(
        refused
            .to_string()
            .contains("connection refused by fixture")
    );
    let dns = super::map_ureq_error(ureq::Error::HostNotFound);
    assert!(dns.to_string().contains("host"));
    for error in [
        ureq::Error::BadUri("https://user:secret@invalid".into()),
        ureq::Error::ConnectProxyFailed("secret proxy response".into()),
    ] {
        let message = super::map_ureq_error(error).to_string();
        assert!(message.contains("HTTPS request failed:"));
        assert!(!message.contains("secret"));
    }
}
