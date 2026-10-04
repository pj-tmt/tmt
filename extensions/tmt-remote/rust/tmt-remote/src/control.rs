//! Serve's owner-only control socket `<dataRoot>/remote/control.sock`.
//! Local commands such as `tmt remote pair` reach remote state only through
//! here, since serve is the state's only opener. One JSON object per line.
use crate::{
    canonical,
    devices::{Devices, device_json},
    error::RemoteError,
    pairing::{End, Pairing, PairingEvent},
    state::Serving,
};
use nix::poll::{PollFd, PollFlags, poll};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    os::{
        fd::{AsFd, AsRawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const SOCKET: &str = "control.sock";
/// Bound on one request line from a local client.
const LINE_BYTES: usize = 4096;
const REQUEST_WAIT: Duration = Duration::from_secs(5);

/// The door address and route prefix a pairing link names.
pub struct Door {
    pub origin: String,
    pub prefix: String,
}
pub struct Control {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}
impl Control {
    /// Bind under the held serve lock. Any existing socket file is a stale
    /// leftover of an earlier serve, because only the lock holder binds here.
    pub fn start(
        serving: &Serving,
        pairing: Arc<Pairing>,
        devices: Arc<Devices>,
        door: Door,
        approval: Option<Arc<crate::approval::Approval>>,
        serve_stop: Arc<AtomicBool>,
    ) -> Result<Self, RemoteError> {
        let path = serving.layout().directory.join(SOCKET);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_socket() && m.uid() == nix::unistd::getuid().as_raw() => {
                fs::remove_file(&path).map_err(io_error)?
            }
            Ok(_) => {
                return Err(RemoteError::new(
                    "REMOTE_STATE_UNSAFE",
                    "control.sock exists and is not this user's socket.",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
        let listener = UnixListener::bind(&path).map_err(|e| {
            RemoteError::new(
                "REMOTE_IO",
                &format!(
                    "Could not bind {} ({e}); a very deep data root can exceed the Unix socket path limit.",
                    path.display()
                ),
            )
        })?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let door = Arc::new(door);
        let accept = thread::Builder::new()
            .name("remote-control".into())
            .spawn(move || {
                accept_loop(
                    listener,
                    &flag,
                    &pairing,
                    &devices,
                    &door,
                    approval.as_deref(),
                    &serve_stop,
                )
            })
            .map_err(io_error)?;
        Ok(Self {
            path,
            stop,
            accept: Some(accept),
        })
    }
    /// Stop accepting, end every session and join their threads.
    pub fn stop(mut self) {
        self.shutdown();
    }
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn io_error(error: impl std::fmt::Display) -> RemoteError {
    RemoteError::new("REMOTE_IO", &format!("Control socket failed: {error}."))
}
fn accept_loop(
    listener: UnixListener,
    stop: &AtomicBool,
    pairing: &Arc<Pairing>,
    devices: &Arc<Devices>,
    door: &Arc<Door>,
    approval: Option<&crate::approval::Approval>,
    serve_stop: &AtomicBool,
) {
    thread::scope(|scope| {
        while !stop.load(Ordering::Acquire) {
            pairing.expire();
            match listener.accept() {
                Ok((stream, _)) => {
                    scope.spawn(|| {
                        session(stream, stop, pairing, devices, door, approval, serve_stop)
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50))
                }
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }
        // Scoped sessions are joined when the scope ends; ending the offer
        // first wakes any session waiting for an event.
        pairing.shutdown();
    });
}
/// Serve one owner-only control request.
fn session(
    mut stream: UnixStream,
    stop: &AtomicBool,
    pairing: &Pairing,
    devices: &Devices,
    door: &Door,
    approval: Option<&crate::approval::Approval>,
    serve_stop: &AtomicBool,
) {
    let _ = stream.set_nonblocking(true);
    let mut buffer = Vec::new();
    let Some(request) = read_line(
        &mut stream,
        &mut buffer,
        stop,
        Instant::now() + REQUEST_WAIT,
    ) else {
        return;
    };
    if request == json!({"op":"stop"}) {
        // Same flag as SIGTERM: foreground owns all cleanup and lease release.
        serve_stop.store(true, Ordering::Release);
        let _ = write_line(&mut stream, &json!({"stopping":true}));
        return;
    }
    if matches!(request["op"].as_str(), Some("approve" | "cancel")) {
        let result = (|| {
            let approval = approval.ok_or_else(crate::operations::invalid)?;
            let id = request["operationId"]
                .as_str()
                .ok_or_else(crate::operations::invalid)?;
            if request["op"] == "cancel" {
                return approval.cancel(id);
            }
            let (grant, preview) = approval.preview(id)?;
            write_line(&mut stream, &preview).map_err(io_error)?;
            let answer = read_line(
                &mut stream,
                &mut buffer,
                stop,
                Instant::now() + Duration::from_secs(600),
            );
            let confirmed = answer.is_some_and(|value| value == json!({"op":"confirm"}));
            let mut result = if confirmed {
                approval.confirm(id, &grant)?
            } else {
                approval.cancel(id)?
            };
            result["event"] = json!("ended");
            Ok(result)
        })();
        let result = result.unwrap_or_else(
            |error: RemoteError| json!({"error":{"code":error.code,"message":error.message}}),
        );
        let _ = write_line(&mut stream, &result);
        return;
    }
    let answer =
        match request.get("op").and_then(Value::as_str) {
            Some("status") if request == json!({"op":"status"}) => Some(Ok(json!({
                "running":true, "origin":door.origin, "path":door.prefix,
            }))),
            Some("pair") => None,
            Some("devices") => Some(devices.list().map(
                |grants| json!({"devices": grants.iter().map(device_json).collect::<Vec<_>>()}),
            )),
            Some("revoke") => Some(match request.get("clientId").and_then(Value::as_str) {
                Some(client_id) => devices
                    .revoke(client_id)
                    .map(|grant| json!({"device": device_json(&grant)})),
                None => Err(RemoteError::new(
                    "REMOTE_INPUT_INVALID",
                    "Revoke needs a clientId.",
                )),
            }),
            Some("rename") => Some(
                match (
                    request.get("clientId").and_then(Value::as_str),
                    request.get("name").and_then(Value::as_str),
                ) {
                    (Some(client_id), Some(name)) => devices
                        .rename(client_id, name)
                        .map(|grant| json!({"device": device_json(&grant)})),
                    _ => Err(RemoteError::new(
                        "REMOTE_INPUT_INVALID",
                        "Rename needs a clientId and name.",
                    )),
                },
            ),
            _ => Some(Err(RemoteError::new(
                "REMOTE_INPUT_INVALID",
                "Unknown control operation.",
            ))),
        };
    if let Some(answer) = answer {
        let line = answer
            .unwrap_or_else(|error| json!({"error":{"code":error.code,"message":error.message}}));
        let _ = write_line(&mut stream, &line);
        return;
    }
    let (offered, events) = match pairing.open() {
        Ok(opened) => opened,
        Err(error) => {
            let _ = write_line(
                &mut stream,
                &json!({"error":{"code":error.code,"message":error.message}}),
            );
            return;
        }
    };
    let offer_id = offered.offer_id.clone();
    let descriptor = json!({
        "profile": "local-v1",
        "binding": "loopback-http",
        "machineId": pairing.machine_id(),
        "windowId": pairing.window_id(),
        "offerId": offered.offer_id,
        "address": format!("{}{}", door.origin, door.prefix),
        "serverChallenge": hex(&offered.server_challenge),
    });
    let code = canonical::pairing_code_text(&offered.code);
    // The code travels only in the link's fragment, never in its path.
    let link = format!(
        "{}/pair/{}#{}",
        door.origin,
        canonical::base64url(descriptor.to_string().as_bytes()),
        code.replace('-', "")
    );
    let opened = json!({
        "event": "offer",
        "link": link,
        "code": code,
        "descriptor": descriptor,
        "expiresAtMs": offered.expires_at_ms,
    });
    if write_line(&mut stream, &opened).is_err() {
        pairing.cancel(&offer_id);
        return;
    }
    loop {
        if stop.load(Ordering::Acquire) {
            pairing.cancel(&offer_id);
        }
        match events.recv_timeout(Duration::from_millis(50)) {
            Ok(PairingEvent::Candidate {
                kind,
                origin,
                name,
                words,
            }) => {
                let line = json!({"event":"candidate","kind":kind,"origin":origin,"name":name,"words":words});
                if write_line(&mut stream, &line).is_err() {
                    pairing.cancel(&offer_id);
                }
            }
            Ok(PairingEvent::Ended(end)) => {
                let mut line = json!({"event":"ended","reason":end.reason()});
                if let End::Paired { client_id } = &end {
                    line["clientId"] = json!(client_id);
                }
                if let End::Failed(message) = &end {
                    line["message"] = json!(message);
                }
                let _ = write_line(&mut stream, &line);
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // The owner's answer, or a closed client, which cancels the offer.
        match poll_line(&mut stream, &mut buffer) {
            Ok(Some(answer)) => match answer.get("op").and_then(Value::as_str) {
                Some("confirm") => pairing.confirm(&offer_id),
                _ => pairing.refuse(&offer_id),
            },
            Ok(None) => {}
            Err(_) => pairing.cancel(&offer_id),
        }
    }
}
fn read_line(
    stream: &mut UnixStream,
    buffer: &mut Vec<u8>,
    stop: &AtomicBool,
    deadline: Instant,
) -> Option<Value> {
    while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
        match poll_line(stream, buffer) {
            Ok(Some(value)) => return Some(value),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    }
    None
}
/// Nonblocking: a complete line, nothing yet, or an error for EOF/oversize.
fn poll_line(stream: &mut UnixStream, buffer: &mut Vec<u8>) -> io::Result<Option<Value>> {
    let mut events = [PollFd::new(stream.as_fd(), PollFlags::POLLIN)];
    let ready = poll(&mut events, 0u16).map_err(io::Error::from)? > 0;
    if ready {
        let mut chunk = [0; 1024];
        match stream.read(&mut chunk) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
    if let Some(end) = buffer.iter().position(|b| *b == b'\n') {
        if end > LINE_BYTES {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let line: Vec<u8> = buffer.drain(..=end).collect();
        return crate::wire::strict_json(&line[..end])
            .map(Some)
            .ok_or_else(|| io::ErrorKind::InvalidData.into());
    }
    if buffer.len() > LINE_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(None)
}
fn write_line(stream: &mut UnixStream, value: &Value) -> io::Result<()> {
    let mut bytes = value.to_string().into_bytes();
    bytes.push(b'\n');
    let deadline = Instant::now() + REQUEST_WAIT;
    let mut rest = bytes.as_slice();
    while !rest.is_empty() {
        match stream.write(rest) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => rest = &rest[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
/// Client side, used by `tmt remote pair`: connect to a running serve.
pub fn connect(remote_directory: &Path) -> Result<UnixStream, RemoteError> {
    let path = socket_path(remote_directory)?;
    UnixStream::connect(&path).map_err(connect_error)
}
fn connect_error(error: io::Error) -> RemoteError {
    if matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    ) {
        not_running()
    } else {
        io_error(error)
    }
}
fn not_running() -> RemoteError {
    RemoteError::new(
        "REMOTE_NOT_RUNNING",
        "Remote is not running; start it with tmt remote serve.",
    )
}
fn socket_path(remote_directory: &Path) -> Result<PathBuf, RemoteError> {
    let path = remote_directory.join(SOCKET);
    let metadata = fs::symlink_metadata(&path).map_err(connect_error)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(RemoteError::new(
            "REMOTE_STATE_UNSAFE",
            "control.sock is not this user's owner-only socket.",
        ));
    }
    Ok(path)
}

/// Bounded read-only discovery. The live address comes from this run, never
/// from remembered state. Malformed or silent peers cannot become stopped status.
pub fn status(remote_directory: &Path) -> Result<Option<Value>, RemoteError> {
    let Some(answer) = request(remote_directory, &json!({"op":"status"}))? else {
        return Ok(None);
    };
    let origin = answer["origin"].as_str().unwrap_or("");
    let port = origin
        .strip_prefix("http://127.0.0.1:")
        .and_then(|port| port.parse::<u16>().ok())
        .filter(|port| *port != 0);
    let origin_valid = port.is_some_and(|port| origin == format!("http://127.0.0.1:{port}"));
    let path_valid = answer["path"]
        .as_str()
        .and_then(|path| path.strip_prefix("/r/"))
        .is_some_and(|prefix| {
            prefix.len() == 32
                && prefix
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        });
    if answer.as_object().is_none_or(|fields| fields.len() != 3)
        || answer["running"] != true
        || !origin_valid
        || !path_valid
    {
        return Err(io_error("invalid live status descriptor"));
    }
    Ok(Some(answer))
}
/// Request shutdown only through the admitted control socket. A response means
/// requested, not stopped; the caller confirms lifecycle lease release.
pub fn request_stop(remote_directory: &Path) -> Result<bool, RemoteError> {
    let Some(answer) = request(remote_directory, &json!({"op":"stop"}))? else {
        return Ok(false);
    };
    if answer != json!({"stopping":true}) {
        return Err(io_error(
            "invalid stop acknowledgment; shutdown may have been requested",
        ));
    }
    Ok(true)
}
fn request(remote_directory: &Path, value: &Value) -> Result<Option<Value>, RemoteError> {
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket};
    let path = match socket_path(remote_directory) {
        Ok(path) => path,
        Err(error) if error.code == "REMOTE_NOT_RUNNING" => return Ok(None),
        Err(error) => return Err(error),
    };
    let fd = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::empty(),
        None,
    )
    .map_err(io_error)?;
    nix::fcntl::fcntl(
        &fd,
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
    )
    .map_err(io_error)?;
    let mut stream = UnixStream::from(fd);
    stream.set_nonblocking(true).map_err(io_error)?;
    let address = UnixAddr::new(&path).map_err(io_error)?;
    match connect(stream.as_raw_fd(), &address) {
        Ok(()) => {}
        Err(nix::errno::Errno::EINPROGRESS) => {
            let mut events = [PollFd::new(stream.as_fd(), PollFlags::POLLOUT)];
            if poll(&mut events, REQUEST_WAIT.as_millis() as u16).map_err(io_error)? == 0 {
                return Err(io_error("control connection timed out"));
            }
            if let Some(error) = stream.take_error().map_err(io_error)? {
                return request_connect_error(error);
            }
        }
        Err(error) => return request_connect_error(error.into()),
    }
    write_line(&mut stream, value).map_err(io_error)?;
    let answer = read_line(
        &mut stream,
        &mut Vec::new(),
        &AtomicBool::new(false),
        Instant::now() + REQUEST_WAIT,
    )
    .ok_or_else(|| io_error("missing or invalid control reply; request may have been submitted"))?;
    if let Some(error) = answer.get("error") {
        if let (Some(code), Some(message)) = (error["code"].as_str(), error["message"].as_str())
            && code.starts_with("REMOTE_")
            && answer.as_object().is_some_and(|fields| fields.len() == 1)
            && error.as_object().is_some_and(|fields| fields.len() == 2)
        {
            return Err(RemoteError::new(code, message));
        }
        return Err(io_error("invalid control error document"));
    }
    Ok(Some(answer))
}

/// Absence is a transport observation, distinct from a connected peer's
/// error document (including a peer reporting REMOTE_NOT_RUNNING).
fn request_connect_error(error: io::Error) -> Result<Option<Value>, RemoteError> {
    if matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    ) {
        Ok(None)
    } else {
        Err(io_error(error))
    }
}
