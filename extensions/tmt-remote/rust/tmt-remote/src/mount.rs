//! `<prefix>/x/<extension>/` mounts, under the machine's unpredictable route
//! prefix so the door cookie scoped there is not sent to other loopback
//! listeners at a guessable path. Remote owns admission (Host, Origin, framing and
//! bounds) and forwards to an owner-only local socket in the extension's data
//! subtree, adding the authenticated device context only for owner sessions.
//! The extension owns its replies: content type, CSP and other headers.
use crate::{
    canonical,
    http::{self, Head, Reply, Request},
    limits,
};
use nix::poll::{PollFd, PollFlags, poll};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    os::{
        fd::{AsFd, AsRawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt},
            net::UnixStream,
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    /// Whether Remote may open the private object channel to this extension.
    pub objects: ObjectDeclaration,
}
/// The trusted, static decision whether an installed extension has an object channel.
/// Nothing at run time (request, environment, setting or command) can change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectDeclaration {
    /// No channel is opened and no object storage is prepared for the extension.
    Disabled,
    /// Remote may open the channel and serve this extension's object storage.
    Local,
}
/// Slice 1 mounts exactly colab; a general enabled-extension registry is later work.
/// Colab's object declaration stays disabled until its real adapter is ready.
pub static EXTENSIONS: [Extension; 1] = [Extension {
    name: "colab",
    body_bytes: 64 * 1024,
    reply_bytes: 16 * 1024 * 1024,
    tunnels: 16,
    tunnel_idle: Duration::from_secs(120),
    objects: ObjectDeclaration::Disabled,
}];
/// Seconds a client should wait before retrying a refused upgrade.
pub const RETRY_AFTER_SECONDS: u32 = 5;
/// Socket file name inside `<dataRoot>/<extension>/`.
pub const SOCKET: &str = "door.sock";
/// Set only by remote; never copied from a client request.
pub const CONTEXT_HEADER: &str = "tmt-device-context";
/// The mount the forwarded path was taken from, for example `/r/<prefix>/x/colab/`.
pub const MOUNT_HEADER: &str = "tmt-mount";
/// The origin of a websocket upgrade to an extension with an object channel: one lowercase
/// UUIDv4 set only by Remote, never copied from a client, and never on a non-upgrade request.
pub const ORIGIN_HEADER: &str = "tmt-origin";
/// Reserved for remote-to-extension callbacks, never browser mount traffic.
pub const DEVICE_EVENT_PATH: &str = "/.tmt/remote/device-events";
pub const DEVICE_EVENT_HEADER: &str = "tmt-device-event";
const DEVICE_EVENT_WAIT: Duration = Duration::from_secs(1);
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
    /// The grant's raw Ed25519 device key; forwarded only as canonical
    /// unpadded base64url, so an extension can verify `tmt-ext-cert-v1`.
    pub public_key: [u8; 32],
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
            "publicKey": canonical::base64url(&self.public_key),
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
/// Resolves a door session cookie to an owner device, rechecking its grant.
/// `session::DoorSessions` is the serving implementation.
pub trait Sessions: Send + Sync {
    /// `cookie` is the request's raw `Cookie` header.
    fn context(&self, cookie: Option<&str>) -> Option<Admitted>;
    fn context_for(&self, cookie: Option<&str>, session: Option<&str>) -> Option<Admitted> {
        if session.is_some() {
            None
        } else {
            self.context(cookie)
        }
    }
}
pub struct NoSessions;
impl Sessions for NoSessions {
    fn context(&self, _: Option<&str>) -> Option<Admitted> {
        None
    }
}
/// An owner request admitted through a live door session.
pub struct Admitted {
    pub context: DeviceContext,
    pub session: Arc<SessionState>,
}
/// The owner session an upgrade was admitted under, retained for as long as the tunnel
/// lives so the session can be checked again; it proves nothing by itself.
#[derive(Clone)]
pub struct OwnerBinding {
    pub session: Arc<SessionState>,
    pub device_id: String,
    pub grant_revision: u64,
}
/// Where the life of tunnels to object-declared extensions is reported. The object service
/// implements it; mounts name neither it nor the object protocol.
pub trait OriginSink: Send + Sync {
    /// A websocket upgrade to `extension` is about to be forwarded. `None` when the
    /// extension has no active object channel: that connection gets no object work.
    fn pending(
        &self,
        extension: &str,
        owner: Option<OwnerBinding>,
    ) -> Option<Arc<dyn OriginTicket>>;
}
/// One tunnel's origin. It is Pending until the tunnel is adopted, then established, and a
/// close ends it for good: establishing after a close does nothing. Dropping the last
/// handle closes it, and closing never blocks on a socket or on sessions.
pub trait OriginTicket: Send + Sync {
    /// The origin as the one lowercase UUIDv4 the extension is given.
    fn id(&self) -> String;
    fn establish(&self);
    fn close(&self);
}
/// Monotonic time source shared by session idle checks and activity touches.
pub type IdleClock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// What a session's tunnels share with it: ending the session (revocation or
/// authority loss) closes them, and their traffic counts as use.
pub struct SessionState {
    clock: IdleClock,
    ended: AtomicBool,
    used: Mutex<Instant>,
    transports: Mutex<usize>,
}
impl Default for SessionState {
    fn default() -> Self {
        Self::with_clock(Arc::new(Instant::now))
    }
}
impl SessionState {
    pub(crate) fn with_clock(clock: IdleClock) -> Self {
        Self {
            used: Mutex::new(clock()),
            transports: Mutex::default(),
            clock,
            ended: AtomicBool::new(false),
        }
    }
    pub fn has_transport(&self) -> bool {
        self.transports.lock().is_ok_and(|active| *active > 0)
    }
    fn attach(self: &Arc<Self>) -> Option<SessionTransport> {
        let mut transports = self.transports.lock().ok()?;
        if self.ended() {
            return None;
        }
        *transports += 1;
        self.touch();
        Some(SessionTransport(Arc::clone(self)))
    }
    pub fn end(&self) {
        self.ended.store(true, Ordering::Release);
    }
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }
    pub fn touch(&self) {
        if let Ok(mut used) = self.used.lock() {
            *used = (self.clock)();
        }
    }
    pub fn idle(&self) -> Duration {
        self.used.lock().map_or(Duration::MAX, |used| {
            (self.clock)().saturating_duration_since(*used)
        })
    }
}

struct SessionTransport(Arc<SessionState>);
impl Drop for SessionTransport {
    fn drop(&mut self) {
        if let Ok(mut transports) = self.0.transports.lock() {
            *transports -= 1;
            if *transports == 0 {
                // Start the reattach grace from last close, even after a quiet tunnel.
                self.0.touch();
            }
        }
    }
}

/// A connected object channel socket and the values its handshake must offer.
pub struct ObjectEndpoint {
    pub stream: UnixStream,
    pub host: String,
    pub mount: String,
}

pub struct Mounts {
    root: PathBuf,
    /// `<prefix>/x/`, the start of every mount path.
    base: String,
    origin: String,
    host: String,
    sessions: Arc<dyn Sessions>,
    extensions: &'static [Extension],
    /// Live tunnels per extension, indexed like `extensions`.
    active: Vec<Arc<AtomicUsize>>,
    tunnels: Mutex<Tunnels>,
    /// Receives the origins of upgrades to object-declared extensions, when there is one.
    origins: Option<Arc<dyn OriginSink>>,
}
#[derive(Default)]
struct Tunnels {
    closed: bool,
    running: Vec<(TcpStream, JoinHandle<()>)>,
}
/// One reserved tunnel slot, released when its splice ends.
struct TunnelSlot(Arc<AtomicUsize>);
impl Drop for TunnelSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl Mounts {
    /// `root` is the absolute core data root; `origin` the door's exact
    /// origin; `prefix` the machine's `/r/<16 lowercase base32>` route prefix.
    pub fn new(root: PathBuf, origin: &str, prefix: &str, sessions: Arc<dyn Sessions>) -> Self {
        Self::with_extensions(root, origin, prefix, sessions, &EXTENSIONS)
    }
    /// As [`Mounts::new`] with an explicit allowlist and bounds.
    pub fn with_extensions(
        root: PathBuf,
        origin: &str,
        prefix: &str,
        sessions: Arc<dyn Sessions>,
        extensions: &'static [Extension],
    ) -> Self {
        Self {
            root,
            base: format!("{prefix}/x/"),
            host: origin.trim_start_matches("http://").to_owned(),
            origin: origin.to_owned(),
            sessions,
            extensions,
            active: extensions.iter().map(|_| Arc::default()).collect(),
            tunnels: Mutex::default(),
            origins: None,
        }
    }
    /// Report the origins of upgrades to object-declared extensions to `origins`.
    pub fn with_origins(mut self, origins: Arc<dyn OriginSink>) -> Self {
        self.origins = Some(origins);
        self
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
    fn reserve(&self, index: usize) -> Option<TunnelSlot> {
        let active = &self.active[index];
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.extensions[index].tunnels).then_some(n + 1)
            })
            .ok()?;
        Some(TunnelSlot(Arc::clone(active)))
    }
    /// Run a splice on its own thread so it leaves the door's edge sockets. `true` once the
    /// tunnel is retained and its thread runs; when the splice ends the origin is closed.
    fn adopt(
        &self,
        client: TcpStream,
        extension: UnixStream,
        slot: TunnelSlot,
        idle: Duration,
        session: Option<SessionTransport>,
        origin: Option<Arc<dyn OriginTicket>>,
    ) -> bool {
        let Ok(mut tunnels) = self.tunnels.lock() else {
            return false;
        };
        tunnels.running.retain(|(_, thread)| !thread.is_finished());
        let Ok(retained) = client.try_clone() else {
            return false;
        };
        if tunnels.closed {
            let _ = client.shutdown(Shutdown::Both);
            return false;
        }
        let spawned = thread::Builder::new()
            .name("remote-tunnel".into())
            .spawn(move || {
                let _slot = slot;
                splice(
                    client,
                    extension,
                    idle,
                    session.as_ref().map(|t| t.0.as_ref()),
                );
                if let Some(origin) = origin {
                    origin.close();
                }
            });
        match spawned {
            Ok(thread) => {
                tunnels.running.push((retained, thread));
                true
            }
            Err(_) => false,
        }
    }
    /// Whether `path` is in the mount space; everything there is the
    /// mounts' to admit or refuse.
    pub fn serves(&self, path: &str) -> bool {
        path.starts_with(&self.base)
    }
    /// The cookie path covering every mount.
    pub fn base(&self) -> &str {
        &self.base
    }
    /// The allowlisted extension whose mount contains `path`, and that mount,
    /// from the door's own mapping; the SDK uses it to scope a page's
    /// extension certificates.
    pub fn extension_of(&self, path: &str) -> Option<(&'static str, String)> {
        self.extension(path)
            .map(|(_, extension, _)| (extension.name, self.mount(extension)))
    }
    fn mount(&self, extension: &Extension) -> String {
        format!("{}{}/", self.base, extension.name)
    }
    /// `<prefix>/x/<name>/<rest>` for an allowlisted name; `rest` keeps its
    /// leading slash.
    fn extension<'a>(&self, path: &'a str) -> Option<(usize, &'static Extension, &'a str)> {
        let path = path.split('?').next()?;
        let below = path.strip_prefix(&self.base)?;
        let (name, _) = below.split_once('/')?;
        let grammar = canonical::extension_name(name);
        let extensions: &'static [Extension] = self.extensions;
        let index = extensions.iter().position(|e| grammar && e.name == name)?;
        Some((index, &extensions[index], &below[name.len()..]))
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
    /// Connect to the owner-only socket of an extension that declares an object channel,
    /// with the exact `Host` and mount values the door sets on every request it sends
    /// that extension. Nothing is written: the caller owns the handshake.
    pub fn open_object_channel(&self, name: &str, deadline: Instant) -> io::Result<ObjectEndpoint> {
        let extension = self
            .extensions
            .iter()
            .find(|extension| {
                extension.name == name && extension.objects == ObjectDeclaration::Local
            })
            .ok_or(io::ErrorKind::NotFound)?;
        let path = self.socket(extension).ok_or(io::ErrorKind::NotFound)?;
        Ok(ObjectEndpoint {
            stream: connect_event(&path, deadline)?,
            host: self.host.clone(),
            mount: self.mount(extension),
        })
    }
    pub fn admit(&self, head: &Head<'_>) -> Result<usize, Reply> {
        if head.path.contains('?') && !head.upgrade {
            return Err(Reply::empty(404));
        }
        let Some((_, extension, rest)) = self.extension(head.path) else {
            return Err(Reply::empty(404));
        };
        if reserved(rest) {
            return Err(Reply::empty(404));
        }
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
        if reserved(rest) {
            return Some(Reply::empty(404));
        }
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
        let requested = request.path.split_once("?tmt-session=").map(|(_, id)| id);
        let admitted = self
            .sessions
            .context_for(request.cookie.as_deref(), requested);
        if requested.is_some() && admitted.is_none() {
            return Some(Reply::empty(404));
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
        let deadline = Instant::now() + limits::MOUNT_RESPONSE;
        let context = admitted.as_ref().map(|a| &a.context);
        // An origin exists only for a websocket upgrade to an extension that declares
        // objects and has an active channel; anything else carries no origin header.
        let origin = match (&self.origins, websocket) {
            (Some(sink), true) if extension.objects == ObjectDeclaration::Local => sink.pending(
                extension.name,
                admitted.as_ref().map(|a| OwnerBinding {
                    session: Arc::clone(&a.session),
                    device_id: a.context.device_id.clone(),
                    grant_revision: a.context.grant_revision,
                }),
            ),
            _ => None,
        };
        let origin_id = origin.as_ref().map(|ticket| ticket.id());
        let forwarded = self.forward(
            &request,
            rest,
            extension,
            context,
            websocket,
            origin_id.as_deref(),
        );
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
            // Count only an established upgrade; failed extension handshakes leave it unattached.
            let session_transport = match admitted.as_ref().map(|a| a.session.attach()) {
                Some(Some(transport)) => Some(transport),
                Some(None) => return Some(Reply::empty(404)),
                None => None,
            };
            let head = http::head(101, &headers, None);
            if let (Some(slot), Ok(owned)) = (slot, client.try_clone())
                && write_all(client, head.as_bytes(), deadline)
                    .and_then(|()| write_all(client, &body, deadline))
                    .is_ok()
            {
                let adopted = self.adopt(
                    owned,
                    stream,
                    slot,
                    extension.tunnel_idle,
                    session_transport,
                    origin.clone(),
                );
                // Established only once the session is attached, the browser has its
                // 101 and the tunnel runs; a close that already happened wins.
                if let (true, Some(origin)) = (adopted, &origin) {
                    origin.establish();
                }
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
            .filter(|(name, value)| name != "set-cookie" && referrer_kept(name, value))
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
    /// Remote's only event channel uses the same admitted owner-only socket and
    /// HTTP framing as mounts. Browser headers cannot acquire this marker.
    /// A 2xx head acknowledges; errors and malformed replies are retryable.
    pub fn device_event(&self, event: &Value) -> bool {
        let body = event.to_string();
        let mut acknowledged = true;
        for extension in self.extensions {
            let result = (|| -> io::Result<()> {
                let path = self.socket(extension).ok_or(io::ErrorKind::NotFound)?;
                if body.len() > extension.body_bytes {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let deadline = Instant::now() + DEVICE_EVENT_WAIT;
                let mut stream = connect_event(&path, deadline)?;
                let head = format!(
                    "POST {DEVICE_EVENT_PATH} HTTP/1.1\r\nHost: {}\r\n{DEVICE_EVENT_HEADER}: 1\r\n{MOUNT_HEADER}: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    self.host,
                    self.mount(extension),
                    body.len(),
                );
                send(&mut stream, head.as_bytes(), deadline)?;
                send(&mut stream, body.as_bytes(), deadline)?;
                let reply = read_head(&mut stream, deadline)?;
                if !(200..300).contains(&reply.status) {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Ok(())
            })();
            acknowledged &= result.is_ok();
        }
        acknowledged
    }
    fn forward(
        &self,
        request: &Request,
        rest: &str,
        extension: &Extension,
        context: Option<&DeviceContext>,
        websocket: bool,
        origin: Option<&str>,
    ) -> String {
        let mut text = format!(
            "{} {rest} HTTP/1.1\r\nhost: {}\r\n{MOUNT_HEADER}: {}\r\n",
            request.method,
            self.host,
            self.mount(extension)
        );
        if let Some(origin) = &request.origin {
            text.push_str(&format!("origin: {origin}\r\n"));
        }
        for (name, value) in &request.headers {
            if FORWARDED.contains(&name.as_str()) {
                text.push_str(&format!("{name}: {value}\r\n"));
            }
        }
        if let Some(origin) = origin {
            text.push_str(&format!("{ORIGIN_HEADER}: {origin}\r\n"));
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
fn reserved(path: &str) -> bool {
    let decoded = path.replace("%2e", ".").replace("%2E", ".");
    let mut segments = decoded.split('/').filter(|s| !s.is_empty() && *s != ".");
    let first = segments.next();
    first == Some(".tmt") || first == Some("..") || segments.any(|s| s == "..")
}

/// Nonblocking connect also bounds a full extension accept backlog. Unix EAGAIN
/// means no connection was queued; retry it through the worker's backoff.
fn connect_event(path: &Path, deadline: Instant) -> io::Result<UnixStream> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::empty(),
        None,
    )?;
    nix::fcntl::fcntl(
        &fd,
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
    )?;
    let address = UnixAddr::new(path)?;
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(true)?;
    let pending = connect(stream.as_raw_fd(), &address);
    match pending {
        Ok(()) => {}
        Err(nix::errno::Errno::EINPROGRESS) => {
            ready(&stream, PollFlags::POLLOUT, deadline)?;
            if let Some(error) = stream.take_error()? {
                return Err(error);
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(stream)
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
/// Page URLs contain the machine prefix, so a mounted reply may only narrow
/// the door's `no-referrer` default to `same-origin`; any other policy is
/// dropped and the default applies.
fn referrer_kept(name: &str, value: &str) -> bool {
    name != "referrer-policy" || matches!(value.trim(), "no-referrer" | "same-origin")
}
/// `session` is the owner session the upgrade was admitted under, if any.
fn splice(
    mut client: TcpStream,
    mut extension: UnixStream,
    idle: Duration,
    session: Option<&SessionState>,
) {
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
            if session.is_some_and(SessionState::ended) {
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
                    session.inspect(|s| s.touch());
                }
            }
            if extension_events.intersects(PollFlags::POLLIN | hangup)
                && downstream.is_empty()
                && !closing
            {
                closing |= fill(&mut extension, &mut downstream, CHUNK)?;
                if !downstream.is_empty() {
                    activity = Instant::now();
                    session.inspect(|s| s.touch());
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
