//! Colab's owner-only socket `<dataRoot>/colab/door.sock`, reached only
//! through the remote door's mount. Remote owns Host, Origin, cookies and the
//! browser-facing framing; colab trusts `tmt-device-context` because only
//! this user can connect to the socket, and answers its own pages and
//! WebSocket upgrades. Relocated from colab's former loopback door: the
//! bounded single-request reader and the drained reply are unchanged.
use crate::{Result, keyring::Layout, limits};
use nix::poll::{PollFd, PollFlags, poll};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    os::{
        fd::AsFd,
        unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const SOCKET: &str = "door.sock";
/// The longest socket path every supported platform accepts (`sun_path`
/// holds 104 bytes on macOS, including the terminating NUL).
const SOCKET_PATH_BYTES: usize = 103;
/// The device context header the remote door sets for an owner session.
const CONTEXT_HEADER: &str = "tmt-device-context";
const PROTOCOL: &str = "colab-sync-v1";
const POLICY: &str = "default-src 'none'; base-uri 'none'; frame-ancestors 'none'";

/// Tunnel bounds; colab caps its own tunnels even though the door does too.
#[derive(Clone, Copy)]
pub struct Tunnels {
    pub cap: usize,
    /// A tunnel with no bytes for this long closes, until colab-sync-v1
    /// heartbeats exist.
    pub idle: Duration,
}
impl Tunnels {
    pub const PRODUCT: Self = Self {
        cap: limits::TUNNELS,
        idle: limits::TUNNEL_IDLE,
    };
}

/// Why the socket could not be bound.
#[derive(Debug)]
pub enum SocketFault {
    PathTooLong(PathBuf),
    /// Something other than this user's socket holds the path.
    Occupied(PathBuf),
}
impl SocketFault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PathTooLong(_) => "COLAB_SOCKET_PATH_TOO_LONG",
            Self::Occupied(_) => "COLAB_STATE_UNSAFE",
        }
    }
}
impl std::fmt::Display for SocketFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathTooLong(path) => write!(
                f,
                "Colab socket path {} is longer than {SOCKET_PATH_BYTES} bytes; choose a shorter data root.",
                path.display()
            ),
            Self::Occupied(path) => write!(
                f,
                "{} exists and is not this user's socket; remove it to serve.",
                path.display()
            ),
        }
    }
}
impl std::error::Error for SocketFault {}

pub struct MountSocket {
    listener: UnixListener,
    pub path: PathBuf,
    /// The bound socket's inode, so cleanup never removes a replacement.
    identity: (u64, u64),
    space_id: String,
    tunnels: Tunnels,
}
struct Worker {
    socket: UnixStream,
    handle: JoinHandle<()>,
}
impl MountSocket {
    /// Bind under the held serve lock. An existing socket owned by this user is
    /// a stale leftover of an earlier serve and is replaced; anything else
    /// refuses.
    pub fn bind(layout: &Layout, space_id: &str, tunnels: Tunnels) -> Result<Self> {
        let path = layout.directory.join(SOCKET);
        if path.as_os_str().len() > SOCKET_PATH_BYTES {
            return Err(SocketFault::PathTooLong(path).into());
        }
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_socket() && m.uid() == nix::unistd::getuid().as_raw() => {
                fs::remove_file(&path)?
            }
            Ok(_) => return Err(SocketFault::Occupied(path).into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let metadata = fs::symlink_metadata(&path)?;
        Ok(Self {
            listener,
            identity: (metadata.dev(), metadata.ino()),
            path,
            space_id: space_id.to_owned(),
            tunnels,
        })
    }
    pub fn run(self, stop: &AtomicBool) -> Result<()> {
        let mut workers: Vec<Worker> = Vec::new();
        let live = Arc::new(AtomicUsize::new(0));
        let space_id: Arc<str> = self.space_id.as_str().into();
        let result = (|| -> Result<()> {
            while !stop.load(Ordering::Acquire) {
                for i in (0..workers.len()).rev() {
                    if workers[i].handle.is_finished() {
                        workers
                            .swap_remove(i)
                            .handle
                            .join()
                            .map_err(|_| "Socket worker panicked.")?;
                    }
                }
                let mut events = [PollFd::new(self.listener.as_fd(), PollFlags::POLLIN)];
                match poll(&mut events, 100u16) {
                    Ok(_) => {}
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(e) => return Err(e.into()),
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
                    let busy = workers.len().saturating_sub(live.load(Ordering::Acquire));
                    if busy >= limits::SOCKETS {
                        let _ = response(&mut socket, 429, b"CAPACITY", false);
                        continue;
                    }
                    let retained = socket.try_clone()?;
                    let (live, space_id, tunnels) =
                        (Arc::clone(&live), Arc::clone(&space_id), self.tunnels);
                    let handle = thread::Builder::new()
                        .name("colab-socket".into())
                        .spawn(move || serve(socket, &space_id, &live, tunnels))?;
                    workers.push(Worker {
                        socket: retained,
                        handle,
                    });
                }
            }
            Ok(())
        })();
        // Close retained handles before joining, interrupting blocked reads,
        // writes and held tunnels.
        for worker in &workers {
            let _ = worker.socket.shutdown(std::net::Shutdown::Both);
        }
        let mut panicked = false;
        for worker in workers {
            panicked |= worker.handle.join().is_err();
        }
        if panicked {
            return Err("Socket worker cleanup failed.".into());
        }
        result
    }
}
impl Drop for MountSocket {
    /// Remove the socket on exit, only while it is still the one bound here.
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
/// What one request asks for, after framing admission.
struct Request {
    path: String,
    method: String,
    /// The owner device's name, when remote forwarded an owner context.
    owner: Option<String>,
    upgrade: bool,
    key: Option<String>,
    protocols: Vec<String>,
}
fn serve(mut socket: UnixStream, space_id: &str, live: &AtomicUsize, tunnels: Tunnels) {
    let request = match acquire(&mut socket) {
        Ok(request) => request,
        Err(status) => {
            let _ = response(&mut socket, status, b"INVALID", false);
            return;
        }
    };
    if request.upgrade {
        let accepted = request.method == "GET"
            && request.path == "/sync"
            && request.protocols.iter().any(|p| p == PROTOCOL);
        let Some(key) = request.key.filter(|_| accepted) else {
            let _ = response(&mut socket, 400, b"INVALID", false);
            return;
        };
        // Non-owner principals authenticate with colab itself, which is later work.
        if request.owner.is_none() {
            let _ = response(&mut socket, 403, b"DENIED", false);
            return;
        }
        let reserved = live
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < tunnels.cap).then_some(n + 1)
            })
            .is_ok();
        if !reserved {
            let _ = response(&mut socket, 503, b"CAPACITY", false);
            return;
        }
        hold(&mut socket, &key, tunnels.idle);
        live.fetch_sub(1, Ordering::AcqRel);
        return;
    }
    if request.method != "GET" || request.path != "/" {
        let _ = response(&mut socket, 404, b"NOT FOUND", false);
        return;
    }
    let text = match &request.owner {
        Some(name) => format!(
            "Colab space {} is running. You are signed in as {}. Co-editing arrives with the next colab slice.",
            escape(space_id),
            escape(name)
        ),
        None => "This colab space is private. Open it from a browser paired with tmt remote pair, or use a share link.".into(),
    };
    let page = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>TMT Colab</title><h1>TMT Colab</h1><p>{text}</p></html>"
    );
    let _ = response(&mut socket, 200, page.as_bytes(), true);
}
/// Accept a colab-sync-v1 upgrade and hold the tunnel until either side
/// closes or no bytes arrive within `idle`. Frames are later work, so input
/// is read and discarded.
fn hold(socket: &mut UnixStream, key: &str, idle: Duration) {
    let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
    let head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: {PROTOCOL}\r\n\r\n"
    );
    if socket.write_all(head.as_bytes()).is_err() || socket.set_read_timeout(Some(idle)).is_err() {
        return;
    }
    let mut discarded = [0; 1024];
    while matches!(socket.read(&mut discarded), Ok(n) if n > 0) {}
    let _ = socket.shutdown(std::net::Shutdown::Both);
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn response(socket: &mut UnixStream, status: u16, body: &[u8], html: bool) -> std::io::Result<()> {
    let deadline = Instant::now() + limits::RESPONSE;
    let kind = if html {
        "text/html; charset=utf-8"
    } else {
        "text/plain; charset=utf-8"
    };
    let bytes = format!(
        "HTTP/1.1 {status} Response\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nContent-Security-Policy: {POLICY}\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    );
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
    socket.shutdown(std::net::Shutdown::Write)?;
    // FIN lets the peer finish reading the response before closing its write half.
    // Wait for that EOF: immediately readable bytes alone omit input in flight,
    // and a final close with unread input can reset a fully written capacity reply.
    // The write and drain share one deadline and the drain has a byte budget.
    socket.set_nonblocking(true)?;
    let mut discarded = [0; 1024];
    let mut remaining_bytes = limits::HEADER_BYTES + limits::HTTP_BODY_BYTES;
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
/// One request only; no pipelining, forwarded authority or HTTP transfer encoding.
fn acquire(socket: &mut UnixStream) -> std::result::Result<Request, u16> {
    let deadline = Instant::now() + limits::ACQUISITION;
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    let end = loop {
        if bytes.len() >= limits::HEADER_BYTES {
            return Err(413);
        }
        read(socket, &mut bytes, &mut chunk, deadline)?;
        if let Some(p) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
    };
    if end > limits::HEADER_BYTES {
        return Err(413);
    }
    if bytes[..end].iter().enumerate().any(|(i, b)| {
        (*b == b'\n' && (i == 0 || bytes[i - 1] != b'\r'))
            || (*b == b'\r' && bytes.get(i + 1) != Some(&b'\n'))
    }) {
        return Err(400);
    }
    let mut fields = [httparse::EMPTY_HEADER; limits::HEADER_FIELDS];
    let mut parsed = httparse::Request::new(&mut fields);
    if parsed.parse(&bytes[..end]).map_err(|_| 400u16)? != httparse::Status::Complete(end)
        || parsed.version != Some(1)
    {
        return Err(400);
    }
    let mut seen = BTreeSet::new();
    let mut request = Request {
        path: parsed.path.ok_or(400u16)?.to_owned(),
        method: parsed.method.ok_or(400u16)?.to_owned(),
        owner: None,
        upgrade: false,
        key: None,
        protocols: Vec::new(),
    };
    let mut size = 0;
    for header in parsed.headers.iter() {
        let name = header.name.to_ascii_lowercase();
        if !seen.insert(name.clone())
            || [
                "transfer-encoding",
                "forwarded",
                "x-forwarded-host",
                "x-forwarded-proto",
            ]
            .contains(&name.as_str())
        {
            return Err(400);
        }
        let value = std::str::from_utf8(header.value).map_err(|_| 400u16)?;
        match name.as_str() {
            "upgrade" => request.upgrade |= value.eq_ignore_ascii_case("websocket"),
            "sec-websocket-key" => request.key = Some(value.to_owned()),
            "sec-websocket-protocol" => {
                request.protocols = value.split(',').map(|p| p.trim().to_owned()).collect()
            }
            CONTEXT_HEADER => request.owner = Some(owner_name(value)?),
            "content-length" => {
                if value.is_empty()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                    || (value.len() > 1 && value.starts_with('0'))
                {
                    return Err(400);
                }
                size = value.parse::<usize>().map_err(|_| 413u16)?;
                if size > limits::HTTP_BODY_BYTES {
                    return Err(413);
                }
            }
            _ => {}
        }
    }
    if !request.path.starts_with('/') || request.path.contains(['?', '#', '%']) {
        return Err(400);
    }
    // Drop header borrows before acquiring the bounded body; no body is interpreted.
    while bytes.len() < end + size {
        read(socket, &mut bytes, &mut chunk, deadline)?;
    }
    if bytes.len() != end + size {
        return Err(400);
    }
    Ok(request)
}
/// The owner device name from the remote door's context; anything other than
/// an owner context with a device ID and name is malformed.
fn owner_name(value: &str) -> std::result::Result<String, u16> {
    let context: Value = serde_json::from_str(value).map_err(|_| 400u16)?;
    let valid = context["owner"] == true && context["deviceId"].is_string();
    match context["name"].as_str() {
        Some(name) if valid => Ok(name.to_owned()),
        _ => Err(400),
    }
}
fn read(
    socket: &mut UnixStream,
    bytes: &mut Vec<u8>,
    chunk: &mut [u8],
    deadline: Instant,
) -> std::result::Result<(), u16> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or(408u16)?;
    socket
        .set_read_timeout(Some(remaining))
        .map_err(|_| 400u16)?;
    let n = socket.read(chunk).map_err(|_| 408u16)?;
    if n == 0 {
        return Err(400);
    }
    bytes.extend_from_slice(&chunk[..n]);
    Ok(())
}
#[cfg(test)]
#[path = "socket_tests.rs"]
mod tests;
