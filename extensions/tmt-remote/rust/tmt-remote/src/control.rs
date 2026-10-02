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
        fd::AsFd,
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
            .spawn(move || accept_loop(listener, &flag, &pairing, &devices, &door))
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
) {
    thread::scope(|scope| {
        while !stop.load(Ordering::Acquire) {
            pairing.expire();
            match listener.accept() {
                Ok((stream, _)) => {
                    scope.spawn(|| session(stream, stop, pairing, devices, door));
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
/// Serve one local client: `pair`, `devices` or `revoke`.
fn session(
    mut stream: UnixStream,
    stop: &AtomicBool,
    pairing: &Pairing,
    devices: &Devices,
    door: &Door,
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
    let answer =
        match request.get("op").and_then(Value::as_str) {
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
        let line: Vec<u8> = buffer.drain(..=end).collect();
        return serde_json::from_slice(&line[..end])
            .map(Some)
            .map_err(|_| io::ErrorKind::InvalidData.into());
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
    let path = remote_directory.join(SOCKET);
    let not_running = || {
        RemoteError::new(
            "REMOTE_NOT_RUNNING",
            "Remote is not running; start it with tmt remote serve.",
        )
    };
    let metadata = fs::symlink_metadata(&path).map_err(|_| not_running())?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != nix::unistd::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(RemoteError::new(
            "REMOTE_STATE_UNSAFE",
            "control.sock is not this user's owner-only socket.",
        ));
    }
    UnixStream::connect(&path).map_err(|_| not_running())
}
