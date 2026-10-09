//! Colab's owner-only socket `<dataRoot>/colab/door.sock`, reached only
//! through the remote door's mount. Remote owns Host, Origin, cookies and the
//! browser-facing framing; colab trusts `tmt-device-context` because only
//! this user can connect to the socket, and answers its own pages and
//! WebSocket upgrades. Relocated from colab's former loopback door: the
//! bounded single-request reader and the drained reply are unchanged.
use crate::{
    Result,
    assets::{self, App},
    control::{self, Control, StopReply},
    keyring::{Layout, StateFault},
    limits, management,
    object_channel::ChannelOwner,
    registration::{self, OwnerAdmission, Registration},
    serve_release::ServeRelease,
    store::Store,
    sync::{Progress, Server},
};
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
        Arc, Mutex,
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
const EVENTS: &str = "/.tmt/remote/device-events";
const PROTOCOL: &str = "colab-sync-v1";
const POLICY: &str = "default-src 'none'; base-uri 'none'; frame-ancestors 'none'";

/// Tunnel bounds; colab caps its own tunnels even though the door does too.
#[derive(Clone, Copy)]
pub struct Tunnels {
    pub cap: usize,
    /// A tunnel that receives no inbound bytes for this long closes, until
    /// colab-sync-v1 heartbeats exist.
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
    browser: Browser,
    tunnels: Tunnels,
    registration: Option<Arc<Mutex<Registration>>>,
    sync: Option<Server<OwnerAdmission>>,
    objects: Arc<ChannelOwner>,
}
#[derive(Clone)]
struct Browser {
    space_id: String,
    app: Option<Arc<App>>,
    /// How a root-local stop request is answered and acted on.
    control: Control,
    /// Whether this serve is older than the installed release; absent for a bare socket.
    release: Option<Arc<ServeRelease>>,
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
        // The bind-then-chmod window is safe only because nobody else can
        // enter this directory, so recheck it right before binding.
        let directory = fs::symlink_metadata(&layout.directory)?;
        if !directory.is_dir()
            || directory.uid() != nix::unistd::getuid().as_raw()
            || directory.mode() & 0o077 != 0
        {
            return Err(StateFault::UnsafeDirectory.into());
        }
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
            browser: Browser {
                space_id: space_id.to_owned(),
                app: None,
                control: Control::new(),
                release: None,
            },
            tunnels,
            registration: None,
            sync: None,
            objects: Arc::new(ChannelOwner::new(Arc::new(|| None))),
        })
    }
    /// Record how the Remote door is held (one of `control::DOORS`) for a stop request to report.
    pub fn with_door(mut self, door: &'static str) -> Self {
        self.browser.control.door = door;
        self
    }
    pub fn with_release(mut self, release: Arc<ServeRelease>) -> Self {
        self.browser.release = Some(release);
        self
    }
    /// The executable refreshes live public Remote discovery for each handshake;
    /// this socket owns strict parsing, generation replacement and teardown.
    pub fn with_object_discovery(
        mut self,
        discover: impl Fn() -> Option<(String, String)> + Send + Sync + 'static,
    ) -> Self {
        self.objects = Arc::new(ChannelOwner::new(Arc::new(discover)));
        self
    }
    pub fn with_app(mut self, app: Option<App>) -> Self {
        self.browser.app = app.map(Arc::new);
        self
    }
    pub fn with_registration(
        mut self,
        layout: &Layout,
        registration: Arc<Mutex<Registration>>,
    ) -> Result<Self> {
        self.sync = Some(Server::new(
            Store::open(layout)?,
            OwnerAdmission(Arc::clone(&registration)),
        ));
        self.registration = Some(registration);
        Ok(self)
    }
    pub fn run(self, stop: &AtomicBool) -> Result<()> {
        let mut workers: Vec<Worker> = Vec::new();
        let live = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(Mutex::new(Vec::new()));
        let browser = Arc::new(self.browser.clone());
        let stopping = Arc::clone(&self.browser.control.stopping);
        let result = (|| -> Result<()> {
            // SIGTERM/SIGINT or a root-local stop request ends serving the same way.
            let halted = || stop.load(Ordering::Acquire) || stopping.load(Ordering::Acquire);
            while !halted() {
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
                    if halted() {
                        break;
                    }
                    socket.set_nonblocking(false)?;
                    let busy = workers.len().saturating_sub(live.load(Ordering::Acquire));
                    if busy >= limits::SOCKETS {
                        let _ = response(&mut socket, 429, b"CAPACITY", false);
                        continue;
                    }
                    let retained = socket.try_clone()?;
                    let (live, browser, tunnels) =
                        (Arc::clone(&live), Arc::clone(&browser), self.tunnels);
                    let registration = self.registration.clone();
                    let sync = self.sync.clone();
                    let active = Arc::clone(&active);
                    let objects = Arc::clone(&self.objects);
                    let handle =
                        thread::Builder::new()
                            .name("colab-socket".into())
                            .spawn(move || {
                                serve(
                                    socket,
                                    &browser,
                                    &live,
                                    tunnels,
                                    MountedServices {
                                        registration: registration.as_ref(),
                                        sync: sync.as_ref(),
                                        active: &active,
                                        objects: &objects,
                                    },
                                )
                            })?;
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
        // End object correlations before joining peers that may be waiting for
        // a backend result. Their retained clients cannot prolong shutdown.
        self.objects.close();
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
    /// When the whole request had been read; a publish measures its combine window from here.
    received: std::time::Instant,
    path: String,
    method: String,
    /// The owner device's name, when remote forwarded an owner context.
    owner: Option<String>,
    context: Option<String>,
    origin: Option<String>,
    body: Vec<u8>,
    event: Option<String>,
    prefetched: Vec<u8>,
    head: Vec<u8>,
    upgrade: bool,
    key: Option<String>,
    version: Option<String>,
    protocols: Vec<String>,
}
struct MountedServices<'a> {
    registration: Option<&'a Arc<Mutex<Registration>>>,
    sync: Option<&'a Server<OwnerAdmission>>,
    active: &'a ActiveTunnels,
    objects: &'a ChannelOwner,
}
fn serve(
    mut socket: UnixStream,
    browser: &Browser,
    live: &AtomicUsize,
    tunnels: Tunnels,
    services: MountedServices<'_>,
) {
    let MountedServices {
        registration,
        sync,
        active,
        objects,
    } = services;
    let request = match acquire(&mut socket) {
        Ok(request) => request,
        Err(status) => {
            let _ = response(&mut socket, status, b"INVALID", false);
            return;
        }
    };
    if request.path == "/.tmt/remote/object-channel-v1" {
        // The neutral owner validates the exact raw head, including all fields
        // and any pipelined bytes, before writing its only handshake response.
        let _ = objects.accept(
            socket,
            &request.head,
            &request.prefetched,
            request.received + limits::ACQUISITION,
        );
        return;
    }
    if request.path == EVENTS {
        let result =
            if request.method != "POST" || request.upgrade || request.event.as_deref() != Some("1")
            {
                Err(registration::Code::Invalid)
            } else {
                apply_event(&request.body, sync, active)
            };
        let (status, text) = match result {
            Ok(()) => (200, "OK"),
            Err(code) => (code.status(), code.text()),
        };
        let _ = response(&mut socket, status, text.as_bytes(), false);
        return;
    }
    if request.path == crate::page::ipc::PATH {
        let result = (|| -> Result<Vec<u8>> {
            if local_denied(&request) {
                return Err(crate::page::Fault::Denied.into());
            }
            if request.method != "POST" || request.upgrade {
                return Err(crate::page::Fault::Invalid.into());
            }
            let published = sync.ok_or(crate::page::Fault::Unavailable)?.publish(
                &request.body,
                registration::now_ms()?,
                request.received,
            )?;
            crate::page::ipc::reply_json(&published)
        })();
        match result {
            Ok(bytes) => {
                let _ = response_as(&mut socket, 200, &bytes, "application/json");
            }
            Err(error) => {
                let failure = crate::page::ipc::WriteError::from_error(error.as_ref());
                if let Ok(bytes) = serde_json::to_vec(&failure) {
                    let _ = response_as(&mut socket, failure.status(), &bytes, "application/json");
                }
            }
        }
        return;
    }
    if request.path == crate::attachments::ipc::PATH {
        // Native root-local admitted read through this serve's established object channel. The
        // authority is this owned private socket; it never activates storage, opens a backend
        // or answers a browser, and the plaintext reaches only this local caller.
        let result = (|| -> Result<Vec<u8>> {
            if local_denied(&request) {
                return Err(crate::page::Fault::Denied.into());
            }
            if request.method != "POST" || request.upgrade {
                return Err(crate::page::Fault::Invalid.into());
            }
            let (page, selector) = crate::attachments::ipc::parse(&request.body)?;
            let source = registration
                .and_then(|service| service.lock().ok()?.save_source())
                .ok_or(crate::page::Fault::Unavailable)?;
            objects.read_root_local(
                &page,
                &selector,
                source,
                request.received + limits::OBJECT_REPLY,
            )
        })();
        match result {
            Ok(bytes) => {
                let _ = write_response(
                    &mut socket,
                    200,
                    &bytes,
                    "application/octet-stream",
                    POLICY,
                    "",
                    limits::ATTACHMENT_RESPONSE,
                );
            }
            Err(error) => {
                let failure = crate::page::ipc::WriteError::from_error(error.as_ref());
                if let Ok(bytes) = serde_json::to_vec(&failure) {
                    let _ = response_as(&mut socket, failure.status(), &bytes, "application/json");
                }
            }
        }
        return;
    }
    if request.path == control::STOP_PATH {
        if local_denied(&request) {
            let _ = response(&mut socket, 403, b"DENIED", false);
        } else if request.method != "POST" || request.upgrade {
            let _ = response(&mut socket, 400, b"INVALID", false);
        } else if let Ok(bytes) = serde_json::to_vec(&StopReply::new(&browser.control)) {
            // Answer first: the accept loop closes retained sockets once it sees the flag.
            let _ = response_as(&mut socket, 200, &bytes, "application/json");
            browser.control.stopping.store(true, Ordering::Release);
        }
        return;
    }
    if request.path == management::PATH || request.path == management::LOCAL_PATH {
        let result = if request.path == management::LOCAL_PATH && local_denied(&request) {
            Err(management::Code::Denied)
        } else if request.method != "POST" || request.upgrade {
            Err(management::Code::Invalid)
        } else {
            apply_management(&request, &browser.space_id, sync)
        };
        match result {
            Ok(bytes) => {
                let _ = response_as(&mut socket, 200, &bytes, "application/json");
            }
            Err(code) => {
                let _ = response(
                    &mut socket,
                    management::status(code),
                    code.text().as_bytes(),
                    false,
                );
            }
        }
        return;
    }
    if request.path == "/.tmt" || request.path.starts_with("/.tmt/") {
        let _ = response(&mut socket, 404, b"NOT FOUND", false);
        return;
    }
    if matches!(
        request.path.as_str(),
        crate::readers::CHALLENGE_PATH | crate::readers::SESSION_PATH
    ) {
        let mut result = Err(registration::Code::Unavailable);
        if request.method != "POST" || request.upgrade {
            result = Err(registration::Code::Invalid);
        } else if let Some(server) = sync {
            let _ = server.update_admission(|admission| {
                result = admission
                    .0
                    .lock()
                    .map_err(|_| registration::Code::Unavailable)
                    .and_then(|mut service| service.reader_request(&request.path, &request.body));
            });
        }
        let (status, body) = match result {
            Ok(bytes) => (200, bytes),
            Err(code) => (
                code.status(),
                serde_json::to_vec(&serde_json::json!({"code":code.text()})).expect("error JSON"),
            ),
        };
        let _ = response_as(&mut socket, status, &body, "application/json");
        return;
    }
    if request.method == "POST" && request.path == registration::PATH && !request.upgrade {
        let result = registration
            .ok_or(registration::Code::Unavailable)
            .and_then(|service| {
                let mut service = service
                    .lock()
                    .map_err(|_| registration::Code::Unavailable)?;
                let now = registration::now_ms()?;
                service.register(request.context.as_deref(), &request.body, now)
            });
        match result {
            Ok(bytes) => {
                let _ = response_as(&mut socket, 200, &bytes, "application/json");
            }
            Err(code) => {
                let _ = response(&mut socket, code.status(), code.text().as_bytes(), false);
            }
        }
        return;
    }
    if request.method == "GET"
        && matches!(request.path.as_str(), "/api/session" | "/api/pages")
        && !request.upgrade
    {
        let result = if request.path == "/api/session" {
            Registration::session(request.context.as_deref())
        } else {
            registration
                .ok_or(registration::Code::Unavailable)
                .and_then(|s| {
                    s.lock()
                        .map_err(|_| registration::Code::Unavailable)?
                        .pages(request.context.as_deref())
                })
        };
        let (status, body) = match result {
            Ok(bytes) => (200, bytes),
            Err(code) => (
                code.status(),
                serde_json::to_vec(&serde_json::json!({"code":code.text()})).expect("error JSON"),
            ),
        };
        let _ = response_as(&mut socket, status, &body, "application/json");
        return;
    }
    if request.method == "GET" && request.path == "/api/serve-release" && !request.upgrade {
        // Admission is the same device context as `/api/session`; the body is read-only.
        let (status, body) = match Registration::session(request.context.as_deref()) {
            Ok(_) => {
                let stale = browser.release.as_ref().and_then(|release| release.stale());
                let running = browser.release.as_ref().map(|release| release.version());
                (
                    200,
                    serde_json::to_vec(&match stale {
                        Some(stale) => serde_json::json!({"running": stale.running, "installed": stale.installed}),
                        None => serde_json::json!({"running": running}),
                    })
                    .expect("release JSON"),
                )
            }
            Err(code) => (
                code.status(),
                serde_json::to_vec(&serde_json::json!({"code":code.text()})).expect("error JSON"),
            ),
        };
        let _ = response_as(&mut socket, status, &body, "application/json");
        return;
    }
    if request.upgrade {
        let accepted = request.method == "GET"
            && request.body.is_empty()
            && request.path == "/sync"
            && request.version.as_deref() == Some("13")
            && request.key.as_deref().is_some_and(websocket_key)
            && request.protocols.iter().any(|p| p == PROTOCOL);
        let Some(key) = request.key.filter(|_| accepted) else {
            let _ = response(&mut socket, 400, b"INVALID", false);
            return;
        };
        let token = match crate::readers::upgrade_token(&request.protocols) {
            Ok(token) => token,
            Err(code) => {
                let _ = response(&mut socket, code.status(), code.text().as_bytes(), false);
                return;
            }
        };
        let reader = token.is_some();
        if request.owner.is_none() && token.is_none() {
            let _ = response(&mut socket, 403, b"DENIED", false);
            return;
        }
        let Some(server) = sync else {
            let _ = response(&mut socket, 403, b"DENIED", false);
            return;
        };
        // Publish the retained tunnel under the sync lock, serializing admission
        // with durable revocation even before the peer has sent hello.
        let mut admitted = Err(registration::Code::Unavailable);
        let retained = match socket.try_clone() {
            Ok(s) => Arc::new(s),
            Err(_) => return,
        };
        if server
            .update_admission(|admission| {
                admitted = (|| {
                    let mut service = admission
                        .0
                        .lock()
                        .map_err(|_| registration::Code::Unavailable)?;
                    if let Some(token) = token {
                        let (principal, device) = service.reader_upgrade(&token)?;
                        active
                            .lock()
                            .map_err(|_| registration::Code::Unavailable)?
                            .push((device, Arc::clone(&retained)));
                        return Ok(principal);
                    }
                    service.active_device(request.context.as_deref(), registration::now_ms()?)?;
                    let context: Value = serde_json::from_str(
                        request
                            .context
                            .as_deref()
                            .ok_or(registration::Code::Denied)?,
                    )
                    .map_err(|_| registration::Code::Denied)?;
                    let device = context["deviceId"]
                        .as_str()
                        .ok_or(registration::Code::Denied)?
                        .to_owned();
                    active
                        .lock()
                        .map_err(|_| registration::Code::Unavailable)?
                        .push((device.clone(), Arc::clone(&retained)));
                    Ok(device)
                })();
            })
            .is_err()
        {
            return;
        }
        let device = match admitted {
            Ok(device) => device,
            Err(code) => {
                let _ = response(&mut socket, code.status(), code.text().as_bytes(), false);
                return;
            }
        };
        let _guard = TunnelGuard {
            active,
            socket: retained,
        };
        let reserved = live
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < tunnels.cap).then_some(n + 1)
            })
            .is_ok();
        if !reserved {
            let _ = server.update_admission(|a| {
                if let Ok(mut s) = a.0.lock() {
                    s.release_reader(&device);
                }
            });
            let _ = response(&mut socket, 503, b"CAPACITY", false);
            return;
        }
        let mut object_peer = None;
        let _ = server.update_admission(|admission| {
            object_peer = objects.peer(
                request.origin.as_deref(),
                device.clone(),
                request.context.clone(),
                reader,
                admission.clone(),
            );
        });
        drive(
            socket,
            &key,
            tunnels.idle,
            server,
            device.clone(),
            request.prefetched,
            object_peer,
        );
        let _ = server.update_admission(|a| {
            if let Ok(mut s) = a.0.lock() {
                s.release_reader(&device);
            }
        });
        live.fetch_sub(1, Ordering::AcqRel);
        return;
    }
    if let Some(prefix) = request.path.strip_prefix("/p/") {
        if request.method != "GET" || !crate::short_links::valid_prefix(prefix) {
            let _ = response(&mut socket, 404, b"NOT FOUND", false);
            return;
        }
        // Recovery must keep the requested alias without revealing private inventory.
        let result = if request.owner.is_none() {
            Ok(format!("../#path=%2Fshort%2F{prefix}"))
        } else {
            registration
                .ok_or(registration::Code::Unavailable)
                .and_then(|service| {
                    service
                        .lock()
                        .map_err(|_| registration::Code::Unavailable)?
                        .page_alias(request.context.as_deref(), prefix)
                })
        };
        match result {
            Ok(location) => {
                let _ = response_with_headers(
                    &mut socket,
                    302,
                    b"",
                    "text/plain",
                    POLICY,
                    &format!("Location: {location}\r\n"),
                );
            }
            Err(code) => {
                let _ = response(&mut socket, code.status(), code.text().as_bytes(), false);
            }
        }
        return;
    }
    if request.method == "GET"
        && let Some(file) = assets::anonymous_file(&request.path)
    {
        if file == crate::chrome::STYLESHEET_PATH {
            let _ = response_with_policy(
                &mut socket,
                200,
                crate::chrome::stylesheet().as_bytes(),
                "text/css; charset=utf-8",
                assets::POLICY,
            );
        } else if let Some((kind, bytes)) = browser.app.as_ref().and_then(|app| app.find(file)) {
            let policy = if file == "/renderer.html" {
                assets::RENDERER_POLICY
            } else {
                assets::POLICY
            };
            let _ = response_with_policy(&mut socket, 200, bytes, kind, policy);
        } else {
            let _ = response(&mut socket, 404, b"NOT FOUND", false);
        }
        return;
    }
    if request.method == "GET"
        && request.owner.is_some()
        && let Some((kind, bytes)) = browser.app.as_ref().and_then(|app| app.find(&request.path))
    {
        let policy = if request.path == "/renderer.html" {
            assets::RENDERER_POLICY
        } else {
            assets::POLICY
        };
        let _ = response_with_policy(&mut socket, 200, bytes, kind, policy);
        return;
    }
    if request.path.starts_with("/assets/")
        || request.path == "/index.html"
        || request.path == "/renderer.html"
        || request.path == "/reader.html"
        || request.path == "/THIRD-PARTY-NOTICES.txt"
    {
        let (status, bytes): (_, &[u8]) = if request.owner.is_none() {
            (403, b"DENIED")
        } else {
            (404, b"NOT FOUND")
        };
        let _ = response(&mut socket, status, bytes, false);
        return;
    }
    if request.method != "GET" || request.path != "/" {
        let _ = response(&mut socket, 404, b"NOT FOUND", false);
        return;
    }
    let (eyebrow, heading, detail) = match &request.owner {
        Some(name) => (
            "Signed-in space",
            "Colab is running",
            format!(
                "<p>Colab space {} is running. You are signed in as {}. {}.</p>",
                escape(&browser.space_id),
                escape(name),
                assets::BUILD_HINT
            ),
        ),
        None => (
            "Private space",
            "Pair this browser first",
            "<p>This colab space is private. Pair this browser with</p><div class=\"tmt-ui-command\"><code class=\"tmt-ui-command-text\">tmt remote pair</code></div><p>or open a share link.</p>".into(),
        ),
    };
    let recovery = request.owner.is_none()
        && browser
            .app
            .as_ref()
            .is_some_and(|app| app.find("/assets/recovery.js").is_some());
    let stylesheet = "<link rel=\"stylesheet\" href=\"./assets/chrome.css\">";
    let screen_title = if request.owner.is_some() {
        "App unavailable"
    } else {
        "Pair browser"
    };
    let recovery_status = if recovery {
        "<p id=\"colab-recovery-status\" class=\"guidance-status\" role=\"status\">Opening your paired browser…</p>"
    } else {
        ""
    };
    let hidden = if recovery { " hidden" } else { "" };
    let script = if recovery {
        "<script type=\"module\" src=\"./assets/recovery.js\"></script>"
    } else {
        ""
    };
    // Lucide Circle/Diamond geometry (lucide-react 1.52.0, ISC); text carries the state.
    let mark = if request.owner.is_some() {
        "<svg class=\"guidance-mark live lucide\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"currentColor\" stroke=\"currentColor\"><circle cx=\"12\" cy=\"12\" r=\"10\"/></svg>"
    } else {
        "<svg class=\"guidance-mark lucide\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\"><path d=\"M2.7 10.3a2.41 2.41 0 0 0 0 3.41l7.59 7.59a2.41 2.41 0 0 0 3.41 0l7.59-7.59a2.41 2.41 0 0 0 0-3.41l-7.59-7.59a2.41 2.41 0 0 0-3.41 0Z\"/></svg>"
    };
    let (state, state_label) = if request.owner.is_some() {
        ("working", "running")
    } else {
        ("waiting", "waiting")
    };
    // Aperture paths and viewBox copied from site/src/home/assets/v9-0.svg.
    let brand_mark = concat!(
        r#"<svg class="tmt-ui-mark" viewBox="0 0 200 200" aria-hidden="true" fill="currentColor">"#,
        r#"<g transform="rotate(0 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z"/></g>"#,
        r#"<g transform="rotate(60 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z"/></g>"#,
        r#"<g transform="rotate(120 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z"/></g>"#,
        r#"<g transform="rotate(180 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z"/></g>"#,
        r#"<g transform="rotate(240 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z"/></g>"#,
        r#"<g transform="rotate(300 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z"/></g>"#,
        "</svg>",
    );
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Colab</title>{stylesheet}</head><body class=\"guidance\"><header class=\"tmt-ui-header\"><span class=\"tmt-ui-brand\">{brand_mark}<span class=\"tmt-ui-wordmark\">Colab</span></span><h1 class=\"tmt-ui-title\">{screen_title}</h1><div class=\"tmt-ui-actions\"></div></header><main class=\"guidance-main\"><section class=\"guidance-card tmt-ui-notice\" data-tone=\"{state}\"><div class=\"tmt-ui-notice-eyebrow\">{eyebrow}</div><div class=\"tmt-ui-notice-mark\"><span aria-hidden=\"true\">{mark}</span><span>{state_label}</span></div><h2 class=\"tmt-ui-notice-heading\">{heading}</h2><div class=\"tmt-ui-notice-body\">{recovery_status}<div id=\"colab-guidance\" class=\"guidance-detail\"{hidden}>{detail}</div></div></section></main>{script}</body></html>"
    );
    let _ = response_with_policy(
        &mut socket,
        200,
        page.as_bytes(),
        "text/html; charset=utf-8",
        assets::POLICY,
    );
}
type ActiveTunnels = Arc<Mutex<Vec<(String, Arc<UnixStream>)>>>;
struct TunnelGuard<'a> {
    active: &'a ActiveTunnels,
    socket: Arc<UnixStream>,
}
impl Drop for TunnelGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.retain(|(_, socket)| !Arc::ptr_eq(socket, &self.socket));
        }
    }
}
#[derive(serde::Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum DeviceEvent {
    #[serde(rename = "device.revoked")]
    Revoked {
        #[serde(rename = "deviceId")]
        device: String,
        #[serde(rename = "grantRevision")]
        revision: u64,
    },
    #[serde(rename = "device.renamed")]
    Renamed {
        #[serde(rename = "deviceId")]
        device: String,
        #[serde(rename = "grantRevision")]
        revision: u64,
        name: String,
    },
}
fn apply_event(
    body: &[u8],
    server: Option<&Server<OwnerAdmission>>,
    active: &ActiveTunnels,
) -> std::result::Result<(), registration::Code> {
    use registration::Code;
    let event: DeviceEvent = serde_json::from_slice(body).map_err(|_| Code::Invalid)?;
    let (device, revision) = match &event {
        DeviceEvent::Revoked { device, revision }
        | DeviceEvent::Renamed {
            device, revision, ..
        } => (device, *revision),
    };
    tmt_colab_model::values::generated_id(device)?;
    if revision == 0 || revision > 9_007_199_254_740_991 {
        return Err(Code::Invalid);
    }
    if let DeviceEvent::Renamed { name, .. } = &event {
        if name.is_empty()
            || name.len() > 64
            || name.trim().is_empty()
            || name.chars().any(char::is_control)
        {
            return Err(Code::Invalid);
        }
        // Colab owns no remote presentation state or rename effects.
        return Ok(());
    }
    let mut result = Err(Code::Unavailable);
    server
        .ok_or(Code::Unavailable)?
        .update_admission(|admission| {
            result = (|| {
                // Lock the handles before committing so a poisoned registry cannot
                // acknowledge a tombstone whose tunnels were left open.
                let mut handles = active.lock().map_err(|_| Code::Unavailable)?;
                let changed = admission
                    .0
                    .lock()
                    .map_err(|_| Code::Unavailable)?
                    .revoke(device, revision)
                    .map_err(|_| Code::Unavailable)?;
                if changed {
                    handles.retain(|(id, socket)| {
                        if id != device {
                            return true;
                        }
                        let _ = socket.shutdown(std::net::Shutdown::Both);
                        false
                    });
                }
                Ok(())
            })();
        })
        .map_err(|_| Code::Unavailable)?;
    result
}
/// Sync first, then Registration, matching upgrade/event and per-turn admission.
/// The callback commits owner effects before pending subscriptions are rechecked.
/// Reserved local routes derive authority from the socket, never forwarded headers.
fn local_denied(request: &Request) -> bool {
    request.context.is_some() || request.event.is_some()
}
fn apply_management(
    request: &Request,
    space: &str,
    server: Option<&Server<OwnerAdmission>>,
) -> std::result::Result<Vec<u8>, management::Code> {
    use management::Code;
    let mut result = Err(Code::Unavailable);
    server
        .ok_or(Code::Unavailable)?
        .update_admission(|admission| {
            result = (|| {
                let mut service = admission.0.lock().map_err(|_| Code::Unavailable)?;
                let now = registration::now_ms().map_err(|_| Code::Unavailable)?;
                if request.path == management::LOCAL_PATH {
                    // Authority is this owned private Unix socket, not context headers.
                    management::local(&mut service, space, &request.body, now)
                } else {
                    management::browser(
                        &mut service,
                        space,
                        request.context.as_deref(),
                        &request.body,
                        now,
                    )
                }
            })();
        })
        .map_err(|_| Code::Unavailable)?;
    result
}
/// Count inbound bytes, including partial frames, for the existing idle bound.
struct Transport {
    socket: UnixStream,
    prefetched: std::io::Cursor<Vec<u8>>,
    received: Arc<Mutex<Instant>>,
}
impl Read for Transport {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let n = if self.prefetched.position() < self.prefetched.get_ref().len() as u64 {
            self.prefetched.read(bytes)?
        } else {
            self.socket.read(bytes)?
        };
        if n > 0 {
            *self
                .received
                .lock()
                .map_err(|_| std::io::ErrorKind::Other)? = Instant::now();
        }
        Ok(n)
    }
}
impl Write for Transport {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.socket.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
    }
}
fn drive(
    mut socket: UnixStream,
    key: &str,
    idle: Duration,
    server: &Server<OwnerAdmission>,
    device: String,
    prefetched: Vec<u8>,
    objects: Option<crate::object_channel::PeerObjects>,
) {
    let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
    let head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: {PROTOCOL}\r\n\r\n"
    );
    if socket.set_write_timeout(Some(limits::RESPONSE)).is_err()
        || socket.write_all(head.as_bytes()).is_err()
        || socket.set_nonblocking(true).is_err()
    {
        return;
    }
    let readiness = match socket.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let received = Arc::new(Mutex::new(Instant::now()));
    let transport = Transport {
        socket,
        prefetched: std::io::Cursor::new(prefetched),
        received: Arc::clone(&received),
    };
    let Ok(mut connection) = server.connect(transport, device) else {
        return;
    };
    connection.attach_objects(objects);
    loop {
        if received.lock().map_or(true, |last| last.elapsed() >= idle) {
            break;
        }
        match connection.poll() {
            Progress::Closed => break,
            Progress::Advanced => continue,
            Progress::Pending => {
                // Bounded timer turns also service cross-worker delivery and
                // acquisition/write deadlines when no input arrives.
                let mut events = [PollFd::new(readiness.as_fd(), PollFlags::POLLIN)];
                if let Err(e) = poll(&mut events, 20u16)
                    && e != nix::errno::Errno::EINTR
                {
                    break;
                }
            }
        }
    }
    let _ = readiness.shutdown(std::net::Shutdown::Both);
}
/// RFC 6455: the key is the base64 of exactly 16 bytes, so 22 symbols (the
/// last one carrying no unused bits) and `==`.
fn websocket_key(key: &str) -> bool {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = key.as_bytes();
    bytes.len() == 24
        && bytes.ends_with(b"==")
        && bytes[..22].iter().all(|b| ALPHABET.contains(b))
        && ALPHABET
            .iter()
            .position(|b| *b == bytes[21])
            .is_some_and(|value| value & 0b1111 == 0)
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn response(socket: &mut UnixStream, status: u16, body: &[u8], html: bool) -> std::io::Result<()> {
    let kind = if html {
        "text/html; charset=utf-8"
    } else {
        "text/plain; charset=utf-8"
    };
    response_as(socket, status, body, kind)
}
fn response_as(
    socket: &mut UnixStream,
    status: u16,
    body: &[u8],
    kind: &str,
) -> std::io::Result<()> {
    response_with_policy(socket, status, body, kind, POLICY)
}
fn response_with_policy(
    socket: &mut UnixStream,
    status: u16,
    body: &[u8],
    kind: &str,
    policy: &str,
) -> std::io::Result<()> {
    response_with_headers(socket, status, body, kind, policy, "")
}
fn response_with_headers(
    socket: &mut UnixStream,
    status: u16,
    body: &[u8],
    kind: &str,
    policy: &str,
    headers: &str,
) -> std::io::Result<()> {
    write_response(
        socket,
        status,
        body,
        kind,
        policy,
        headers,
        limits::RESPONSE,
    )
}
/// One response written, and its peer's close awaited, within `wait` altogether.
fn write_response(
    socket: &mut UnixStream,
    status: u16,
    body: &[u8],
    kind: &str,
    policy: &str,
    headers: &str,
    wait: std::time::Duration,
) -> std::io::Result<()> {
    let deadline = Instant::now() + wait;
    let bytes = format!(
        "HTTP/1.1 {status} Response\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nContent-Security-Policy: {policy}\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\n{headers}\r\n",
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
        received: std::time::Instant::now(),
        path: parsed.path.ok_or(400u16)?.to_owned(),
        method: parsed.method.ok_or(400u16)?.to_owned(),
        owner: None,
        context: None,
        origin: None,
        body: Vec::new(),
        event: None,
        prefetched: Vec::new(),
        head: bytes[..end].to_vec(),
        upgrade: false,
        key: None,
        version: None,
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
            "sec-websocket-version" => request.version = Some(value.to_owned()),
            "sec-websocket-protocol" => {
                request.protocols = value.split(',').map(|p| p.trim().to_owned()).collect()
            }
            "tmt-device-event" => request.event = Some(value.to_owned()),
            CONTEXT_HEADER => request.context = Some(value.to_owned()),
            "tmt-origin" => request.origin = Some(value.to_owned()),
            "content-length" => {
                if value.is_empty()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                    || (value.len() > 1 && value.starts_with('0'))
                {
                    return Err(400);
                }
                size = value.parse::<usize>().map_err(|_| 413u16)?;
                if size > limits::http_body_bytes(&request.path) {
                    return Err(413);
                }
            }
            _ => {}
        }
    }
    if !request.path.starts_with('/') || request.path.contains(['?', '#', '%']) {
        return Err(400);
    }
    if request
        .path
        .split('/')
        .any(|part| matches!(part, "." | ".."))
        || request.path.contains('\\')
        || request.path.starts_with("//")
    {
        return Err(400);
    }
    // Drop header borrows before acquiring the bounded body; no body is interpreted.
    while bytes.len() < end + size {
        read(socket, &mut bytes, &mut chunk, deadline)?;
    }
    if bytes.len() != end + size && !request.upgrade {
        return Err(400);
    }
    request.body = bytes[end..end + size].to_vec();
    request.prefetched = bytes[end + size..].to_vec();
    request.received = std::time::Instant::now();
    if !matches!(
        request.path.as_str(),
        registration::PATH
            | EVENTS
            | "/api/session"
            | "/api/serve-release"
            | "/api/pages"
            | management::PATH
            | management::LOCAL_PATH
            | crate::page::ipc::PATH
            | crate::attachments::ipc::PATH
            | control::STOP_PATH
            | crate::readers::CHALLENGE_PATH
            | crate::readers::SESSION_PATH
    ) {
        request.owner = request.context.as_deref().map(owner_name).transpose()?;
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
