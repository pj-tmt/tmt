//! The stdio MCP server Claude starts for an enrolled session. It exposes no
//! tools: it declares the channel capability, writes one-way notifications for
//! frames received on its owner-only Unix socket, and marks its launch's
//! enrollment ready once Claude has completed the MCP handshake. It never
//! creates an enrollment: `tmt run --channel` wrote it before Claude started.
//! Its lifetime is Claude's:
//! stdin closing ends it. The calling thread is the only writer of the output:
//! the stdin reader and the socket acceptor send it events.

use super::{
    CAPABILITY, CONTENT_LIMIT, Frame, NOTIFICATION_METHOD, PROTOCOL_VERSION, Process,
    RECORD_VERSION, Reply, SERVER_NAME, ensure_private_directory, locked, parent_incarnation,
    read_record, socket_fits, socket_path, write_record,
};
use crate::runtime::channel::ServeRequest;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const ACCEPT_POLL: Duration = Duration::from_millis(20);
const MESSAGE_LIMIT: u64 = 1024 * 1024;
/// JSON escaping can expand each content byte up to six bytes.
const FRAME_LIMIT: u64 = CONTENT_LIMIT as u64 * 6 + 4096;
const PARENT_DEADLINE: Duration = Duration::from_secs(3);
/// Revisions Claude may request; anything else is answered with ours.
const KNOWN_PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Events from this channel are tmt requests from other agents. \
Each event carries its own instructions for replying with the tmt command; follow them.";

/// Removes the socket this server bound, on every exit path, but only while the
/// enrollment still carries this server's generation (under the directory
/// lock): a replacement enrollment's socket is not ours to remove, and neither
/// is anything when the lock cannot be taken. The enrollment itself stays: it
/// belongs to the launch, which withdraws it when it ends.
struct Bound<'a> {
    directory: &'a Path,
    binding_id: &'a str,
    generation: &'a str,
}

impl Drop for Bound<'_> {
    fn drop(&mut self) {
        let _ = locked(self.directory, || {
            if matches!(
                read_record(self.directory, self.binding_id),
                Ok(Some(record)) if record.generation == self.generation
            ) {
                let _ = fs::remove_file(socket_path(self.directory, self.binding_id));
            }
        });
    }
}

/// Everything the main thread reacts to.
enum ServerEvent {
    /// One line of Claude's MCP stream, or its end.
    Line(Vec<u8>),
    Closed,
    /// A validated frame, with where to report whether it was written.
    Frame(String, mpsc::Sender<bool>),
}

/// The acceptor's view: it decides a frame's admissibility, never writes it.
struct Ingress {
    generation: String,
    ready: Arc<AtomicBool>,
    events: mpsc::Sender<ServerEvent>,
    stop: Arc<AtomicBool>,
}

pub(super) fn serve(
    request: &ServeRequest<'_>,
    input: Box<dyn BufRead + Send>,
    output: &mut dyn Write,
) -> io::Result<()> {
    if uuid::Uuid::parse_str(request.binding_id).is_err()
        || uuid::Uuid::parse_str(request.generation).is_err()
        || !request.directory.is_absolute()
        || !socket_fits(request.directory, request.binding_id)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid channel server arguments",
        ));
    }
    ensure_private_directory(request.directory)?;
    let (listener, owner) = start(request)?;
    let _bound = Bound {
        directory: request.directory,
        binding_id: request.binding_id,
        generation: request.generation,
    };
    listener.set_nonblocking(true)?;
    let (events, inbox) = mpsc::channel();
    let ingress = Ingress {
        generation: request.generation.to_owned(),
        ready: Arc::new(AtomicBool::new(false)),
        events: events.clone(),
        stop: Arc::new(AtomicBool::new(false)),
    };
    let ready = Arc::clone(&ingress.ready);
    let stop = Arc::clone(&ingress.stop);
    // Both helpers are detached: a stdin read cannot be interrupted, and the
    // process ends with this function's caller. They only send events.
    std::thread::spawn(move || read_lines(input, &events));
    std::thread::spawn(move || accept_loop(&listener, &ingress));
    let result = converse(request, &owner, &inbox, output, &ready);
    stop.store(true, Ordering::SeqCst);
    result
}

fn read_lines(mut input: Box<dyn BufRead + Send>, events: &mpsc::Sender<ServerEvent>) {
    loop {
        let mut line = Vec::new();
        match Read::take(&mut input, MESSAGE_LIMIT + 1).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if events.send(ServerEvent::Line(line)).is_err() {
                    return;
                }
            }
        }
    }
    let _ = events.send(ServerEvent::Closed);
}

/// Binds the socket for this server's enrollment, all under the directory lock:
/// the record must still be this launch's pending enrollment (this generation,
/// not yet ready), a live second server is refused, and only a socket nobody
/// answers on is replaced. Returns the launch owner the enrollment names, which
/// the readiness publish must find unchanged.
fn start(request: &ServeRequest<'_>) -> io::Result<(UnixListener, Process)> {
    locked(request.directory, || {
        let owner = match read_record(request.directory, request.binding_id) {
            Ok(Some(record))
                if record.generation == request.generation
                    && record.binding_id == request.binding_id
                    && record.claude.is_none() =>
            {
                record.launch_owner
            }
            // Without this launch's enrollment there is nothing to serve.
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "no pending channel enrollment for this launch",
                ));
            }
        };
        let path: PathBuf = socket_path(request.directory, request.binding_id);
        if UnixStream::connect(&path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another channel server owns this binding",
            ));
        }
        // Nothing answered, so a leftover socket file is stale.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok((listener, owner))
    })?
}

fn accept_loop(listener: &UnixListener, ingress: &Ingress) {
    while !ingress.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => handle_connection(stream, ingress),
            Err(_) => std::thread::sleep(ACCEPT_POLL),
        }
    }
}

fn handle_connection(mut stream: UnixStream, ingress: &Ingress) {
    // The listener is nonblocking; accepted streams must block within bounds.
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(CONNECTION_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(CONNECTION_TIMEOUT)).is_err()
    {
        return;
    }
    let mut line = Vec::new();
    let read = io::BufReader::new((&stream).take(FRAME_LIMIT + 1)).read_until(b'\n', &mut line);
    // No frame means nothing to answer; the sender classifies the silence.
    if read.is_err() || line.is_empty() {
        return;
    }
    let reply = match decide(&line, ingress) {
        Ok(content) => {
            let (done, written) = mpsc::channel();
            if ingress
                .events
                .send(ServerEvent::Frame(content, done))
                .is_err()
            {
                return;
            }
            // A failed or unfinished write may already have reached Claude:
            // answer nothing, and the sender reports it as uncertain.
            match written.recv_timeout(CONNECTION_TIMEOUT) {
                Ok(true) => Reply {
                    written: true,
                    refused: None,
                },
                _ => return,
            }
        }
        Err(reason) => Reply {
            written: false,
            refused: Some(reason.into()),
        },
    };
    if let Ok(mut bytes) = serde_json::to_vec(&reply) {
        bytes.push(b'\n');
        let _ = stream.write_all(&bytes);
    }
}

/// The frame's content if it may be handed to Claude, else the refusal reason.
fn decide(line: &[u8], ingress: &Ingress) -> Result<String, &'static str> {
    if line.len() as u64 > FRAME_LIMIT || !line.ends_with(b"\n") {
        return Err("frame_invalid");
    }
    let frame: Frame = serde_json::from_slice(line).map_err(|_| "frame_invalid")?;
    if frame.version != RECORD_VERSION {
        return Err("version");
    }
    if frame.generation != ingress.generation {
        return Err("generation");
    }
    if !ingress.ready.load(Ordering::SeqCst) {
        return Err("not_ready");
    }
    if frame.content.is_empty() || frame.content.len() > CONTENT_LIMIT {
        return Err("content");
    }
    Ok(frame.content)
}

fn write_message(output: &mut dyn Write, message: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(message)?;
    bytes.push(b'\n');
    output.write_all(&bytes)?;
    output.flush()
}

/// Claude's side of the MCP stdio session: initialize, ping, then silence,
/// plus the notifications this server was asked to write.
fn converse(
    request: &ServeRequest<'_>,
    owner: &Process,
    inbox: &mpsc::Receiver<ServerEvent>,
    output: &mut dyn Write,
    ready: &AtomicBool,
) -> io::Result<()> {
    while let Ok(event) = inbox.recv() {
        let line = match event {
            ServerEvent::Closed => return Ok(()),
            ServerEvent::Frame(content, done) => {
                let written = write_message(
                    output,
                    &json!({
                        "jsonrpc": "2.0",
                        "method": NOTIFICATION_METHOD,
                        "params": {"content": content},
                    }),
                );
                let _ = done.send(written.is_ok());
                written?;
                continue;
            }
            ServerEvent::Line(line) => line,
        };
        if line.len() as u64 > MESSAGE_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP message exceeds its bound",
            ));
        }
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id").cloned();
        match (method, id) {
            (Some("initialize"), Some(id)) => {
                write_message(output, &initialize_result(id, &message))?;
            }
            (Some("notifications/initialized"), None) => publish(request, owner, ready),
            (Some("ping"), Some(id)) => {
                write_message(output, &json!({"jsonrpc": "2.0", "id": id, "result": {}}))?;
            }
            (Some(_), Some(id)) => write_message(
                output,
                &json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": "Method not found"}}),
            )?,
            _ => {}
        }
    }
    Ok(())
}

fn initialize_result(id: Value, message: &Value) -> Value {
    let requested = message
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str);
    let version = requested
        .filter(|version| KNOWN_PROTOCOLS.contains(version))
        .unwrap_or(PROTOCOL_VERSION);
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": version,
            "capabilities": {"experimental": {CAPABILITY: {}}},
            "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        },
    })
}

/// Ready only when the handshake is complete and the Claude process (this
/// server's parent) is observable: the enrollment then names that process, and
/// only then can `send` route through the channel. The read-modify-write runs
/// under the directory lock and only while the record is still this server's
/// enrollment, generation and launch owner, so it can never overwrite a newer
/// one.
pub(super) fn publish(request: &ServeRequest<'_>, owner: &Process, ready: &AtomicBool) {
    let Some(claude) = parent_incarnation(Instant::now() + PARENT_DEADLINE) else {
        return;
    };
    let published = locked(request.directory, || {
        let Ok(Some(mut record)) = read_record(request.directory, request.binding_id) else {
            return false;
        };
        if record.generation != request.generation
            || record.binding_id != request.binding_id
            || record.launch_owner != *owner
        {
            return false;
        }
        record.claude = Some(Process::of(&claude));
        write_record(request.directory, &record).is_ok()
    });
    if published.unwrap_or(false) {
        ready.store(true, Ordering::SeqCst);
    }
}
