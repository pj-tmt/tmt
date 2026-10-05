//! Bounded loopback HTTP workers, relocated from the colab door.
//! The door owns sockets, strict framing, Host admission, bounds and shutdown;
//! a [`Handler`] owns routes, Origin policy and authority.
use crate::{error::RemoteError, limits};
use nix::poll::{PollFd, PollFlags, poll};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream},
    os::fd::AsFd,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

/// Framed request head, offered to the handler before any body byte is read.
pub struct Head<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub origin: Option<&'a str>,
    pub cookie: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub content_length: Option<usize>,
    pub upgrade: bool,
}
pub struct Request {
    pub method: String,
    pub path: String,
    pub origin: Option<String>,
    pub cookie: Option<String>,
    pub upgrade: bool,
    /// Every admitted header with a lowercase name, in arrival order.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Held until the request is dropped, so handling counts against the budget.
    _reserved: Reservation,
}
/// Door-wide in-flight body budget shared by all workers.
#[derive(Clone, Default)]
struct Budget(Arc<AtomicUsize>);
struct Reservation {
    budget: Budget,
    bytes: usize,
}
impl Budget {
    fn reserve(&self, bytes: usize) -> Option<Reservation> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= limits::IN_FLIGHT_BODY_BYTES)
            })
            .ok()?;
        Some(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.0.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
/// One complete reply. The door adds framing headers and fills any absent
/// [`SECURITY_DEFAULTS`]; the reply's own headers (for example a mounted
/// extension's CSP) are never overridden.
pub struct Reply {
    pub status: u16,
    /// Lowercase names; framing headers are owned by the door.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
pub const SECURITY_DEFAULTS: [(&str, &str); 4] = [
    ("cache-control", "no-store"),
    (
        "content-security-policy",
        "default-src 'none'; base-uri 'none'; frame-ancestors 'none'",
    ),
    ("referrer-policy", "no-referrer"),
    ("x-content-type-options", "nosniff"),
];
/// Hop-by-hop and framing headers the door writes itself.
pub const FRAMING: [&str; 5] = [
    "connection",
    "content-length",
    "keep-alive",
    "transfer-encoding",
    "upgrade",
];
impl Reply {
    /// Delivery-only refusal: the status reports framing, never authority.
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: b"{}".to_vec(),
        }
    }
}
/// Application owns routes and authority. The door admits framing and Host first.
pub trait Handler: Send + Sync {
    /// Admit a head and return its body limit, or refuse before reading the body.
    fn admit(&self, head: &Head<'_>) -> Result<usize, Reply>;
    /// Return a reply for the door to write, or `None` after taking over the
    /// client socket (an upgraded tunnel). Door shutdown closes that socket.
    fn handle(&self, request: Request, client: &mut TcpStream) -> Option<Reply>;
    /// Called once when the door stops, before its workers are joined, to end
    /// anything the handler took over.
    fn shutdown(&self) {}
    fn maintain(&self) -> Result<(), RemoteError> {
        Ok(())
    }
}

pub struct Door {
    listener: TcpListener,
    /// Exact `http://127.0.0.1:<port>` origin; also the only admitted Host authority.
    pub origin: String,
    host: String,
    budget: Budget,
}
struct Worker {
    socket: TcpStream,
    handle: JoinHandle<()>,
}
impl Door {
    pub fn bind(port: u16) -> Result<Self, RemoteError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AddrInUse {
                RemoteError::new(
                    "REMOTE_PORT_BUSY",
                    &format!("Loopback port {port} is busy; choose another with --port, or use --port 0 for a free port."),
                )
            } else {
                error.into()
            }
        })?;
        listener.set_nonblocking(true)?;
        let host = listener.local_addr()?.to_string();
        Ok(Self {
            listener,
            origin: format!("http://{host}"),
            host,
            budget: Budget::default(),
        })
    }
    pub fn socket_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    /// Serve until `stop`; shutdown closes retained sockets before joining workers.
    pub fn run(self, stop: &AtomicBool, handler: Arc<dyn Handler>) -> Result<(), RemoteError> {
        let mut workers: Vec<Worker> = Vec::new();
        let result = (|| -> Result<(), RemoteError> {
            while !stop.load(Ordering::Acquire) {
                handler.maintain()?;
                for i in (0..workers.len()).rev() {
                    if workers[i].handle.is_finished() {
                        workers
                            .swap_remove(i)
                            .handle
                            .join()
                            .map_err(|_| worker_failure())?;
                    }
                }
                let mut events = [PollFd::new(self.listener.as_fd(), PollFlags::POLLIN)];
                match poll(&mut events, 100u16) {
                    Ok(_) => {}
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(e) => return Err(std::io::Error::from(e).into()),
                }
                for _ in 0..limits::SOCKETS {
                    let (mut socket, _) = match self.listener.accept() {
                        Ok(c) => c,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e.into()),
                    };
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    socket.set_nonblocking(false)?;
                    if workers.len() == limits::SOCKETS {
                        let _ = response(&mut socket, &Reply::empty(429));
                        continue;
                    }
                    let retained = socket.try_clone()?;
                    let host = self.host.clone();
                    let budget = self.budget.clone();
                    let handler = Arc::clone(&handler);
                    let handle =
                        thread::Builder::new()
                            .name("remote-http".into())
                            .spawn(move || {
                                let reply =
                                    match acquire(&mut socket, &host, &budget, handler.as_ref()) {
                                        Ok(request) => handler.handle(request, &mut socket),
                                        Err(reply) => Some(reply),
                                    };
                                if let Some(reply) = reply {
                                    let _ = response(&mut socket, &reply);
                                }
                            })?;
                    workers.push(Worker {
                        socket: retained,
                        handle,
                    });
                }
            }
            Ok(())
        })();
        drop(self.listener);
        handler.shutdown();
        // Close retained handles before joining, interrupting blocked reads/writes.
        for worker in &workers {
            let _ = worker.socket.shutdown(Shutdown::Both);
        }
        let mut panicked = false;
        for worker in workers {
            panicked |= worker.handle.join().is_err();
        }
        if panicked {
            return Err(worker_failure());
        }
        result
    }
}
fn worker_failure() -> RemoteError {
    RemoteError::new("REMOTE_IO", "HTTP worker cleanup failed.")
}
/// Serialize a response head with door-owned framing and absent security defaults.
pub fn head(status: u16, headers: &[(String, String)], length: Option<usize>) -> String {
    let mut text = format!("HTTP/1.1 {status} Response\r\n");
    for (name, value) in headers {
        if !FRAMING.contains(&name.as_str()) {
            text.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    for (name, value) in SECURITY_DEFAULTS {
        if !headers.iter().any(|(present, _)| present == name) {
            text.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    match length {
        Some(length) => text.push_str(&format!(
            "content-length: {length}\r\nconnection: close\r\n\r\n"
        )),
        None => text.push_str("connection: upgrade\r\nupgrade: websocket\r\n\r\n"),
    }
    text
}
fn response(socket: &mut TcpStream, reply: &Reply) -> std::io::Result<()> {
    let deadline = Instant::now() + limits::RESPONSE;
    let status = reply.status;
    let body = reply.body.as_slice();
    let bytes = head(status, &reply.headers, Some(body.len()));
    for mut bytes in [bytes.as_bytes(), body] {
        while !bytes.is_empty() {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or(std::io::ErrorKind::TimedOut)?;
            socket.set_write_timeout(Some(remaining))?;
            match socket.write(bytes) {
                Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }
    socket.shutdown(Shutdown::Write)?;
    // FIN lets the peer finish reading the response before closing its write half.
    // Wait for that EOF: immediately readable bytes alone omit input in flight,
    // and a final close with unread input can reset a fully written capacity reply.
    // The write and drain share one deadline and the drain has a byte budget.
    socket.set_nonblocking(true)?;
    let mut discarded = [0; 1024];
    let mut remaining_bytes = limits::DRAIN_BYTES;
    while remaining_bytes != 0 {
        let Some(remaining_time) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let size = remaining_bytes.min(discarded.len());
        match socket.read(&mut discarded[..size]) {
            Ok(0) => break,
            Ok(n) => remaining_bytes -= n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let mut events = [PollFd::new(socket.as_fd(), PollFlags::POLLIN)];
                let timeout = remaining_time.as_millis().min(u16::MAX as u128) as u16;
                match poll(&mut events, timeout) {
                    Ok(0) => break,
                    Ok(_) | Err(nix::errno::Errno::EINTR) => continue,
                    Err(_) => break,
                }
            }
            Err(_) => break,
        }
    }
    Ok(())
}
/// Origin-form target with only the transport session query; no fragment, escapes, dot segments or empty
/// inner segments; only the last segment may be empty (a directory such as
/// `<prefix>/x/colab/`). Isolation between the operation routes and the mount
/// space relies on it.
fn target(path: &str) -> bool {
    let path = if let Some((path, id)) = path.split_once("?tmt-session=") {
        if !crate::canonical::is_core_id(id)
            || id.as_bytes()[14] != b'4'
            || !matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
        {
            return false;
        }
        path
    } else {
        path
    };
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    let segments: Vec<&str> = rest.split('/').collect();
    !path.contains(['?', '#', '%', '\\'])
        && segments
            .iter()
            .enumerate()
            .all(|(i, s)| (!s.is_empty() || i + 1 == segments.len()) && *s != "." && *s != "..")
}
/// One request only; no pipelining, forwarded authority or HTTP transfer encoding.
fn acquire(
    socket: &mut TcpStream,
    host: &str,
    budget: &Budget,
    handler: &dyn Handler,
) -> Result<Request, Reply> {
    let deadline = Instant::now() + limits::ACQUISITION;
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    let end = loop {
        if bytes.len() >= limits::HEADER_BYTES {
            return Err(Reply::empty(413));
        }
        read(socket, &mut bytes, &mut chunk, deadline)?;
        if let Some(p) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
    };
    if end > limits::HEADER_BYTES {
        return Err(Reply::empty(413));
    }
    if bytes[..end].iter().enumerate().any(|(i, b)| {
        (*b == b'\n' && (i == 0 || bytes[i - 1] != b'\r'))
            || (*b == b'\r' && bytes.get(i + 1) != Some(&b'\n'))
    }) {
        return Err(Reply::empty(400));
    }
    let mut fields = [httparse::EMPTY_HEADER; limits::HEADER_FIELDS];
    let mut request = httparse::Request::new(&mut fields);
    if request
        .parse(&bytes[..end])
        .map_err(|_| Reply::empty(400))?
        != httparse::Status::Complete(end)
        || request.version != Some(1)
    {
        return Err(Reply::empty(400));
    }
    let mut seen = BTreeSet::new();
    let mut actual_host = None;
    let mut origin = None;
    let mut cookie = None;
    let mut content_type = None;
    let mut size = None;
    let mut upgrade = false;
    let mut headers = Vec::new();
    for header in request.headers.iter() {
        let name = header.name.to_ascii_lowercase();
        if !seen.insert(name.clone())
            || [
                "transfer-encoding",
                "authorization",
                "forwarded",
                "x-forwarded-host",
                "x-forwarded-proto",
            ]
            .contains(&name.as_str())
        {
            return Err(Reply::empty(400));
        }
        let value = std::str::from_utf8(header.value).map_err(|_| Reply::empty(400))?;
        headers.push((name.clone(), value.to_owned()));
        match name.as_str() {
            "host" => actual_host = Some(value),
            "origin" => origin = Some(value),
            "cookie" => cookie = Some(value),
            "content-type" => content_type = Some(value),
            "upgrade" => upgrade = true,
            "connection" => {
                upgrade |= value
                    .split(',')
                    .any(|v| v.trim().eq_ignore_ascii_case("upgrade"))
            }
            "content-length" => {
                if value.is_empty()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                    || (value.len() > 1 && value.starts_with('0'))
                {
                    return Err(Reply::empty(400));
                }
                size = Some(value.parse::<usize>().map_err(|_| Reply::empty(413))?);
            }
            _ => {}
        }
    }
    // Exact numeric authority defeats DNS rebinding; no alias is admitted.
    if actual_host != Some(host) {
        return Err(Reply::empty(400));
    }
    let method = request.method.ok_or_else(|| Reply::empty(400))?;
    let path = request.path.ok_or_else(|| Reply::empty(400))?;
    if !target(path) {
        return Err(Reply::empty(400));
    }
    let limit = handler.admit(&Head {
        method,
        path,
        origin,
        cookie,
        content_type,
        content_length: size,
        upgrade,
    })?;
    let size = size.unwrap_or(0);
    // A handler can narrow, never widen, the door-owned body bound.
    if size > limit.min(limits::BODY_BYTES) {
        return Err(Reply::empty(413));
    }
    // Reserve before reading so concurrent unauthenticated bodies stay bounded.
    let reserved = budget.reserve(size).ok_or_else(|| Reply::empty(429))?;
    let request = Request {
        method: method.to_owned(),
        path: path.to_owned(),
        origin: origin.map(str::to_owned),
        cookie: cookie.map(str::to_owned),
        upgrade,
        headers,
        body: Vec::new(),
        _reserved: reserved,
    };
    // Header borrows end here; the body is acquired only after admission.
    while bytes.len() < end + size {
        read(socket, &mut bytes, &mut chunk, deadline)?;
    }
    if bytes.len() != end + size {
        return Err(Reply::empty(400));
    }
    Ok(Request {
        body: bytes.split_off(end),
        ..request
    })
}
fn read(
    socket: &mut TcpStream,
    bytes: &mut Vec<u8>,
    chunk: &mut [u8],
    deadline: Instant,
) -> Result<(), Reply> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| Reply::empty(408))?;
    socket
        .set_read_timeout(Some(remaining))
        .map_err(|_| Reply::empty(400))?;
    let n = socket.read(chunk).map_err(|_| Reply::empty(408))?;
    if n == 0 {
        return Err(Reply::empty(400));
    }
    bytes.extend_from_slice(&chunk[..n]);
    Ok(())
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
