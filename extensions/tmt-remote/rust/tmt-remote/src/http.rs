//! Finite single-owner loopback reactor; no core client or storage handle.
use crate::{
    error::RemoteError,
    transport::{LoopbackTransport, Transport},
};
use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
const HEADERS: usize = 8192;
const PAIR: usize = 16384;
const CONNECTIONS: usize = 32;
const ACQUISITION: Duration = Duration::from_secs(5);

pub struct ServeOptions {
    pub port: u16,
    pub window: Duration,
    pub input_limit: usize,
}
pub struct Door {
    listener: TcpListener,
    pub address: String,
    prefix: String,
    port: u16,
    window: Duration,
    body_limit: usize,
}
impl Door {
    pub fn bind(options: ServeOptions) -> Result<Self, RemoteError> {
        if options.window.is_zero()
            || options.window > Duration::from_secs(86400)
            || options.input_limit == 0
            || options.input_limit > 16 * 1024 * 1024
        {
            return Err(RemoteError::new(
                "REMOTE_INPUT_INVALID",
                "Invalid remote window or input bound.",
            ));
        }
        let mut entropy = [0; 16];
        getrandom::fill(&mut entropy)
            .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain route entropy."))?;
        let prefix = format!(
            "/r/{}",
            entropy
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, options.port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            address: format!("http://127.0.0.1:{port}{prefix}"),
            prefix,
            port,
            window: options.window,
            body_limit: 4 * (options.input_limit + 8192).div_ceil(3) + 8192,
        })
    }
    pub fn socket_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    /// Denied traffic never refreshes idle, and termination drops all sockets.
    pub fn run(self, stop: &AtomicBool) -> Result<(), RemoteError> {
        let transport = LoopbackTransport::default();
        let end = Instant::now() + self.window.min(Duration::from_secs(900));
        let mut reset = Instant::now() + Duration::from_secs(60);
        let mut attempts = 0;
        let mut pending: Vec<Connection> = Vec::new();
        while !stop.load(Ordering::Relaxed) && Instant::now() < end {
            if Instant::now() >= reset {
                attempts = 0;
                reset = Instant::now() + Duration::from_secs(60);
            }
            // Bound per-turn accepts as well as retained connections.
            for _ in 0..=CONNECTIONS {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(true)?;
                        attempts += 1;
                        if attempts > 20 || pending.len() == CONNECTIONS {
                            write_rejection(stream, 429);
                        } else {
                            pending.push(Connection {
                                stream,
                                bytes: Vec::new(),
                                deadline: Instant::now() + ACQUISITION,
                            });
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            for index in (0..pending.len()).rev() {
                let connection = &mut pending[index];
                let mut bytes = [0; 4096];
                let status = match connection.stream.read(&mut bytes) {
                    Ok(0) => Some(400),
                    Ok(count) => {
                        connection.bytes.extend_from_slice(&bytes[..count]);
                        frame(
                            &connection.bytes,
                            self.port,
                            &self.prefix,
                            self.body_limit,
                            &transport,
                        )
                    }
                    Err(e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::Interrupted =>
                    {
                        None
                    }
                    Err(_) => Some(400),
                };
                if let Some(status) =
                    status.or_else(|| (Instant::now() >= connection.deadline).then_some(400))
                {
                    write_rejection(pending.swap_remove(index).stream, status);
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}
struct Connection {
    stream: TcpStream,
    bytes: Vec<u8>,
    deadline: Instant,
}
fn write_rejection(mut stream: TcpStream, status: u16) {
    let reason = match status {
        400 => "Bad Request",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Not Found",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    // Nonblocking, bounded response; failed delivery never becomes authority.
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.shutdown(std::net::Shutdown::Write);
}
fn frame(
    bytes: &[u8],
    port: u16,
    prefix: &str,
    body_limit: usize,
    transport: &impl Transport,
) -> Option<u16> {
    let Some(header_end) = bytes
        .windows(4)
        .position(|x| x == b"\r\n\r\n")
        .map(|p| p + 4)
    else {
        return (bytes.len() > HEADERS).then_some(413);
    };
    if header_end > HEADERS {
        return Some(413);
    }
    let mut slots = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut slots);
    if !matches!(
        request.parse(&bytes[..header_end]),
        Ok(httparse::Status::Complete(_))
    ) || request.version != Some(1)
    {
        return Some(400);
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut host = None;
    let mut length = None;
    let mut content = None;
    for header in request.headers.iter() {
        let name = header.name.to_ascii_lowercase();
        if !seen.insert(name.clone())
            || [
                "transfer-encoding",
                "cookie",
                "authorization",
                "forwarded",
                "x-forwarded-host",
            ]
            .contains(&name.as_str())
        {
            return Some(400);
        }
        let Ok(value) = std::str::from_utf8(header.value) else {
            return Some(400);
        };
        match name.as_str() {
            "host" => host = Some(value),
            "content-length" => length = Some(value),
            "content-type" => content = Some(value),
            _ => {}
        }
    }
    if host != Some(format!("127.0.0.1:{port}").as_str()) {
        return Some(400);
    }
    let path = request.path.unwrap_or("");
    let suffix = path.strip_prefix(prefix).unwrap_or("");
    if request.method != Some("POST")
        || !["/append", "/subscribe", "/ack", "/pair"].contains(&suffix)
    {
        return Some(404);
    }
    if content != Some("application/json") {
        return Some(400);
    }
    let Some(length) = length
        .filter(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|x| x.parse::<usize>().ok())
    else {
        return Some(400);
    };
    if length > if suffix == "/pair" { PAIR } else { body_limit } {
        return Some(413);
    }
    let received = bytes.len() - header_end;
    if received > length {
        return Some(400);
    }
    if received < length {
        return None;
    }
    let body = &bytes[header_end..];
    let denied = match suffix {
        "/append" => transport.append(body),
        "/subscribe" => transport.subscribe(body),
        "/ack" => transport.ack(body),
        _ => return Some(404),
    };
    // No successful admission exists in this slice. Fail closed if it changes.
    let _ = denied;
    Some(404)
}
