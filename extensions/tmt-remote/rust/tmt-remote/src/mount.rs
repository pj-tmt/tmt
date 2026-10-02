//! `/x/<extension>/` mounts. Remote owns admission (Host, Origin, framing and
//! bounds) and forwards to an owner-only local socket in the extension's data
//! subtree, adding the authenticated device context only for owner sessions.
//! The extension owns its replies: content type, CSP and other headers.
use crate::{
    http::{self, Head, Reply, Request},
    limits,
};
use nix::poll::{PollFd, PollFlags, poll};
use serde_json::json;
use std::{
    fs,
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    os::{
        fd::AsFd,
        unix::{
            fs::{FileTypeExt, MetadataExt},
            net::UnixStream,
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// One owner-installed extension the door may mount, with its own request and
/// reply bounds.
pub struct Extension {
    pub name: &'static str,
    pub body_bytes: usize,
    pub reply_bytes: usize,
    /// Live upgraded tunnels. Tunnels run outside the door's edge sockets, so
    /// open pages cannot starve `/r/`, pairing or page loads.
    pub tunnels: usize,
    /// A tunnel with no bytes in either direction for this long is closed.
    pub tunnel_idle: Duration,
}
/// Slice 1 mounts exactly colab; a general enabled-extension registry is later work.
pub static EXTENSIONS: [Extension; 1] = [Extension {
    name: "colab",
    body_bytes: 64 * 1024,
    reply_bytes: 16 * 1024 * 1024,
    tunnels: 16,
    tunnel_idle: Duration::from_secs(120),
}];
/// Seconds a client should wait before retrying a refused upgrade.
pub const RETRY_AFTER_SECONDS: u32 = 5;
/// Socket file name inside `<dataRoot>/<extension>/`.
pub const SOCKET: &str = "door.sock";
/// Set only by remote; never copied from a client request.
pub const CONTEXT_HEADER: &str = "tmt-device-context";
/// The mount prefix the forwarded path was taken from, for example `/x/colab/`.
pub const MOUNT_HEADER: &str = "tmt-mount";
/// Client request headers an extension may see. Cookie (the remote door
/// session) and anything else are dropped.
const FORWARDED: [&str; 9] = [
    "accept",
    "accept-language",
    "content-type",
    "if-modified-since",
    "if-none-match",
    "sec-websocket-extensions",
    "sec-websocket-key",
    "sec-websocket-protocol",
    "sec-websocket-version",
];
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Authenticated owner-device context, as defined by the extension channel API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceContext {
    pub device_id: String,
    pub kind: String,
    pub origin: String,
    pub name: String,
    pub grant_revision: u64,
}
impl DeviceContext {
    /// Compact JSON with every non-ASCII or control character escaped, so the
    /// value is header-safe without a second encoding.
    pub fn header_value(&self) -> String {
        let text = json!({
            "deviceId": self.device_id,
            "kind": self.kind,
            "origin": self.origin,
            "name": self.name,
            "owner": true,
            "grantRevision": self.grant_revision,
        })
        .to_string();
        let mut value = String::with_capacity(text.len());
        for c in text.chars() {
            if c.is_ascii() && !c.is_ascii_control() {
                value.push(c);
            } else {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    value.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
        value
    }
}
/// Resolves a door session cookie to an owner device. Pairing supplies the
/// real implementation; until then no request carries a device context.
pub trait Sessions: Send + Sync {
    fn context(&self, cookie: Option<&str>) -> Option<DeviceContext>;
}
pub struct NoSessions;
impl Sessions for NoSessions {
    fn context(&self, _: Option<&str>) -> Option<DeviceContext> {
        None
    }
}

pub struct Mounts {
    root: PathBuf,
    origin: String,
    host: String,
    sessions: Arc<dyn Sessions>,
    extensions: &'static [Extension],
    /// Live tunnels per extension, indexed like `extensions`.
    active: Vec<Arc<AtomicUsize>>,
    tunnels: Mutex<Tunnels>,
}
#[derive(Default)]
struct Tunnels {
    closed: bool,
    running: Vec<(TcpStream, JoinHandle<()>)>,
}
/// One reserved tunnel slot, released when its splice ends.
struct Slot(Arc<AtomicUsize>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl Mounts {
    /// `root` is the absolute core data root; `origin` the door's exact origin.
    pub fn new(root: PathBuf, origin: &str, sessions: Arc<dyn Sessions>) -> Self {
        Self::with_extensions(root, origin, sessions, &EXTENSIONS)
    }
    /// As [`Mounts::new`] with an explicit allowlist and bounds.
    pub fn with_extensions(
        root: PathBuf,
        origin: &str,
        sessions: Arc<dyn Sessions>,
        extensions: &'static [Extension],
    ) -> Self {
        Self {
            root,
            host: origin.trim_start_matches("http://").to_owned(),
            origin: origin.to_owned(),
            sessions,
            extensions,
            active: extensions.iter().map(|_| Arc::default()).collect(),
            tunnels: Mutex::default(),
        }
    }
    /// Close every live tunnel and join its thread; later tunnels are refused.
    pub fn shutdown(&self) {
        let running = match self.tunnels.lock() {
            Ok(mut tunnels) => {
                tunnels.closed = true;
                std::mem::take(&mut tunnels.running)
            }
            Err(_) => return,
        };
        for (client, _) in &running {
            let _ = client.shutdown(Shutdown::Both);
        }
        for (_, thread) in running {
            let _ = thread.join();
        }
    }
    fn reserve(&self, index: usize) -> Option<Slot> {
        let active = &self.active[index];
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.extensions[index].tunnels).then_some(n + 1)
            })
            .ok()?;
        Some(Slot(Arc::clone(active)))
    }
    /// Run a splice on its own thread so it leaves the door's edge sockets.
    fn adopt(&self, client: TcpStream, extension: UnixStream, slot: Slot, idle: Duration) {
        let Ok(mut tunnels) = self.tunnels.lock() else {
            return;
        };
        tunnels.running.retain(|(_, thread)| !thread.is_finished());
        let Ok(retained) = client.try_clone() else {
            return;
        };
        if tunnels.closed {
            let _ = client.shutdown(Shutdown::Both);
            return;
        }
        let spawned = thread::Builder::new()
            .name("remote-tunnel".into())
            .spawn(move || {
                let _slot = slot;
                splice(client, extension, idle);
            });
        if let Ok(thread) = spawned {
            tunnels.running.push((retained, thread));
        }
    }
    /// `/x/<name>/<rest>` for an allowlisted name; `rest` keeps its leading slash.
    fn extension<'a>(&self, path: &'a str) -> Option<(usize, &'static Extension, &'a str)> {
        let (name, _) = path.strip_prefix("/x/")?.split_once('/')?;
        let grammar = name.len() <= 32
            && name.starts_with(|c: char| c.is_ascii_lowercase())
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        let extensions: &'static [Extension] = self.extensions;
        let index = extensions.iter().position(|e| grammar && e.name == name)?;
        Some((index, &extensions[index], &path[3 + name.len()..]))
    }
    /// The extension's socket, only if both it and its directory are owned by
    /// this user, grant nothing to group/other and are not symlinks. The 0700
    /// directory keeps other users from swapping the socket after the check.
    fn socket(&self, extension: &Extension) -> Option<PathBuf> {
        let uid = nix::unistd::getuid().as_raw();
        let private = |path: &Path, socket: bool| {
            fs::symlink_metadata(path).is_ok_and(|m| {
                let kind = m.file_type();
                (if socket {
                    kind.is_socket()
                } else {
                    kind.is_dir()
                }) && m.uid() == uid
                    && m.mode() & 0o077 == 0
            })
        };
        let directory = self.root.join(extension.name);
        let socket = directory.join(SOCKET);
        (private(&directory, false) && private(&socket, true)).then_some(socket)
    }
    pub fn admit(&self, head: &Head<'_>) -> Result<usize, Reply> {
        let Some((_, extension, _)) = self.extension(head.path) else {
            return Err(Reply::empty(404));
        };
        if !METHODS.contains(&head.method) || (head.upgrade && head.method != "GET") {
            return Err(Reply::empty(404));
        }
        // Browsers omit Origin on top-level navigation; every other request and
        // every upgrade must come from the door's own origin.
        match head.origin {
            Some(origin) if origin == self.origin => {}
            None if head.method == "GET" && !head.upgrade => {}
            _ => return Err(Reply::empty(403)),
        }
        Ok(extension.body_bytes)
    }
    pub fn handle(&self, request: Request, client: &mut TcpStream) -> Option<Reply> {
        let (index, extension, rest) = self.extension(&request.path)?;
        let websocket = request.upgrade
            && header(&request.headers, "upgrade")
                .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
        if request.upgrade && !websocket {
            return Some(Reply::empty(400));
        }
        // Fetch metadata, when the browser sends it, refuses other sites outright.
        if header(&request.headers, "sec-fetch-site") == Some("cross-site") {
            return Some(Reply::empty(403));
        }
        let Some(socket) = self.socket(extension) else {
            return Some(Reply::empty(404));
        };
        // Reserve before connecting, so a full tunnel pool never reaches the extension.
        let slot = match websocket.then(|| self.reserve(index)) {
            Some(None) => {
                let mut busy = Reply::empty(503);
                busy.headers
                    .push(("retry-after".into(), RETRY_AFTER_SECONDS.to_string()));
                return Some(busy);
            }
            Some(slot) => slot,
            None => None,
        };
        let Some(mut stream) = UnixStream::connect(&socket)
            .and_then(|s| s.set_nonblocking(true).map(|()| s))
            .ok()
        else {
            return Some(Reply::empty(503));
        };
        let context = self.sessions.context(request.cookie.as_deref());
        let deadline = Instant::now() + limits::MOUNT_RESPONSE;
        let forwarded = self.forward(&request, rest, extension, context.as_ref(), websocket);
        if send(&mut stream, forwarded.as_bytes(), deadline)
            .and_then(|()| send(&mut stream, &request.body, deadline))
            .is_err()
        {
            return Some(Reply::empty(502));
        }
        let Ok(ReplyHead {
            status,
            headers,
            rest: mut body,
        }) = read_head(&mut stream, deadline)
        else {
            return Some(Reply::empty(502));
        };
        if status == 101 {
            if !websocket {
                return Some(Reply::empty(502));
            }
            let head = http::head(101, &headers, None);
            if let (Some(slot), Ok(owned)) = (slot, client.try_clone())
                && write_all(client, head.as_bytes(), deadline)
                    .and_then(|()| write_all(client, &body, deadline))
                    .is_ok()
            {
                self.adopt(owned, stream, slot, extension.tunnel_idle);
            }
            return None;
        }
        if !(200..=599).contains(&status) {
            return Some(Reply::empty(502));
        }
        let length = match header(&headers, "content-length") {
            Some(value) if value.bytes().all(|b| b.is_ascii_digit()) && !value.is_empty() => {
                value.parse::<usize>().ok()
            }
            None if matches!(status, 204 | 304) => Some(0),
            _ => None,
        };
        let Some(length) = length.filter(|n| *n <= extension.reply_bytes && body.len() <= *n)
        else {
            return Some(Reply::empty(502));
        };
        // Stream the body so each worker holds one chunk, not a whole reply.
        // A reply that fails after its head is sent ends with a short body.
        let headers: Vec<_> = headers
            .into_iter()
            .filter(|(name, _)| name != "set-cookie")
            .collect();
        let head = http::head(status, &headers, Some(length));
        let _ = (|| -> io::Result<()> {
            write_all(client, head.as_bytes(), deadline)?;
            write_all(client, &body, deadline)?;
            let mut sent = body.len();
            while sent < length {
                body.clear();
                read_some(&mut stream, &mut body, deadline)?;
                body.truncate(length - sent);
                write_all(client, &body, deadline)?;
                sent += body.len();
            }
            client.shutdown(Shutdown::Write)
        })();
        None
    }
    fn forward(
        &self,
        request: &Request,
        rest: &str,
        extension: &Extension,
        context: Option<&DeviceContext>,
        websocket: bool,
    ) -> String {
        let mut text = format!(
            "{} {rest} HTTP/1.1\r\nhost: {}\r\n{MOUNT_HEADER}: /x/{}/\r\n",
            request.method, self.host, extension.name
        );
        if let Some(origin) = &request.origin {
            text.push_str(&format!("origin: {origin}\r\n"));
        }
        for (name, value) in &request.headers {
            if FORWARDED.contains(&name.as_str()) {
                text.push_str(&format!("{name}: {value}\r\n"));
            }
        }
        if let Some(context) = context {
            text.push_str(&format!("{CONTEXT_HEADER}: {}\r\n", context.header_value()));
        }
        text.push_str(if websocket {
            "connection: upgrade\r\nupgrade: websocket\r\n"
        } else {
            "connection: close\r\n"
        });
        text.push_str(&format!("content-length: {}\r\n\r\n", request.body.len()));
        text
    }
}
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(present, _)| present == name)
        .map(|(_, value)| value.as_str())
}
fn remaining(deadline: Instant) -> io::Result<std::time::Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::ErrorKind::TimedOut.into())
}
/// Blocking write to the browser client within the deadline.
fn write_all(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
/// Wait until `stream` is ready for `flags` or the deadline passes. Polling
/// replaces per-read socket timeouts, which macOS refuses with EINVAL once
/// the peer has closed even though buffered bytes remain readable.
fn ready(stream: &UnixStream, flags: PollFlags, deadline: Instant) -> io::Result<()> {
    loop {
        let timeout = remaining(deadline)?.as_millis().min(u16::MAX as u128) as u16;
        let mut events = [PollFd::new(stream.as_fd(), flags)];
        match poll(&mut events, timeout) {
            Ok(0) => return Err(io::ErrorKind::TimedOut.into()),
            Ok(_) => return Ok(()),
            Err(nix::errno::Errno::EINTR) => {}
            Err(e) => return Err(e.into()),
        }
    }
}
/// Nonblocking write of every byte to the extension within the deadline.
fn send(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                ready(stream, PollFlags::POLLOUT, deadline)?
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
/// Nonblocking read of at least one byte from the extension within the deadline.
fn read_some(stream: &mut UnixStream, bytes: &mut Vec<u8>, deadline: Instant) -> io::Result<()> {
    let mut chunk = [0; 16 * 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                bytes.extend_from_slice(&chunk[..n]);
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                ready(stream, PollFlags::POLLIN, deadline)?
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}
/// Bounded extension reply head.
struct ReplyHead {
    status: u16,
    headers: Vec<(String, String)>,
    /// Bytes after the head: the start of the body or of the tunnel.
    rest: Vec<u8>,
}
fn read_head(stream: &mut UnixStream, deadline: Instant) -> io::Result<ReplyHead> {
    let invalid = || io::Error::from(io::ErrorKind::InvalidData);
    let mut bytes = Vec::new();
    let end = loop {
        if let Some(p) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if bytes.len() >= limits::HEADER_BYTES {
            return Err(invalid());
        }
        read_some(stream, &mut bytes, deadline)?;
    };
    if end > limits::HEADER_BYTES {
        return Err(invalid());
    }
    let mut fields = [httparse::EMPTY_HEADER; limits::HEADER_FIELDS];
    let mut reply = httparse::Response::new(&mut fields);
    if reply.parse(&bytes[..end]).map_err(|_| invalid())? != httparse::Status::Complete(end)
        || reply.version != Some(1)
    {
        return Err(invalid());
    }
    let status = reply.code.ok_or_else(invalid)?;
    let mut headers = Vec::new();
    for field in reply.headers.iter() {
        let name = field.name.to_ascii_lowercase();
        let value = std::str::from_utf8(field.value).map_err(|_| invalid())?;
        if name == "transfer-encoding"
            || headers.iter().any(|(seen, _)| *seen == name) && !name.eq("set-cookie")
        {
            return Err(invalid());
        }
        headers.push((name, value.to_owned()));
    }
    let rest = bytes.split_off(end);
    Ok(ReplyHead {
        status,
        headers,
        rest,
    })
}
/// Copy bytes both ways, unchanged and unparsed, until either side closes.
/// Each direction holds at most one bounded chunk; reads stop while it is
/// unflushed, so a slow peer applies backpressure instead of growing memory.
/// Pending bytes that make no progress within the write bound end the tunnel,
/// and door shutdown closes the client socket, which ends it too.
fn splice(mut client: TcpStream, mut extension: UnixStream, idle: Duration) {
    let client = &mut client;
    const CHUNK: usize = 16 * 1024;
    let _ = (|| -> io::Result<()> {
        client.set_nonblocking(true)?;
        extension.set_nonblocking(true)?;
        // Pending bytes client→extension and extension→client.
        let (mut upstream, mut downstream) = (Vec::with_capacity(CHUNK), Vec::with_capacity(CHUNK));
        let mut closing = false;
        let mut progress = Instant::now();
        let mut activity = Instant::now();
        loop {
            if closing && upstream.is_empty() && downstream.is_empty() {
                return Ok(());
            }
            if upstream.is_empty() && downstream.is_empty() && activity.elapsed() >= idle {
                return Ok(());
            }
            if !(upstream.is_empty() && downstream.is_empty())
                && progress.elapsed() >= limits::SPLICE_WRITE
            {
                return Ok(());
            }
            let interest = |read: bool, write: bool| {
                let mut flags = PollFlags::empty();
                flags.set(PollFlags::POLLIN, read);
                flags.set(PollFlags::POLLOUT, write);
                flags
            };
            let mut events = [
                PollFd::new(
                    client.as_fd(),
                    interest(!closing && upstream.is_empty(), !downstream.is_empty()),
                ),
                PollFd::new(
                    extension.as_fd(),
                    interest(!closing && downstream.is_empty(), !upstream.is_empty()),
                ),
            ];
            match poll(&mut events, 100u16) {
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(e.into()),
            }
            let ready = |e: &PollFd<'_>| e.revents().unwrap_or(PollFlags::empty());
            let (client_events, extension_events) = (ready(&events[0]), ready(&events[1]));
            let hangup = PollFlags::POLLHUP | PollFlags::POLLERR;
            if client_events.intersects(PollFlags::POLLIN | hangup)
                && upstream.is_empty()
                && !closing
            {
                closing |= fill(client, &mut upstream, CHUNK)?;
                if !upstream.is_empty() {
                    activity = Instant::now();
                }
            }
            if extension_events.intersects(PollFlags::POLLIN | hangup)
                && downstream.is_empty()
                && !closing
            {
                closing |= fill(&mut extension, &mut downstream, CHUNK)?;
                if !downstream.is_empty() {
                    activity = Instant::now();
                }
            }
            if drain(client, &mut downstream)? | drain(&mut extension, &mut upstream)? {
                progress = Instant::now();
            }
        }
    })();
    let _ = client.shutdown(Shutdown::Both);
    let _ = extension.shutdown(Shutdown::Both);
}
/// Read into an empty buffer; `true` means the peer closed.
fn fill(stream: &mut impl Read, buffer: &mut Vec<u8>, chunk: usize) -> io::Result<bool> {
    buffer.resize(chunk, 0);
    match stream.read(buffer) {
        Ok(n) => {
            buffer.truncate(n);
            Ok(n == 0)
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            buffer.clear();
            Ok(false)
        }
        Err(e) => Err(e),
    }
}
/// Write as much pending data as the peer accepts; `true` if any was written.
fn drain(stream: &mut impl Write, buffer: &mut Vec<u8>) -> io::Result<bool> {
    let mut wrote = false;
    while !buffer.is_empty() {
        match stream.write(buffer) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                buffer.drain(..n);
                wrote = true;
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                break;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(wrote)
}
