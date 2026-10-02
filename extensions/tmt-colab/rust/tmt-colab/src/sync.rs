//! Externally driven WebSocket sync. The caller authenticates the principal and
//! upgrades the socket; this module never owns HTTP, cookies, keys or Yjs state.
mod wire;
use crate::{
    limits,
    store::{self, Accepted, Store, StreamScope},
};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::Instant,
};
use tmt_colab_model::{crypto, object, values};
use tungstenite::{
    Message, WebSocket,
    protocol::{CloseFrame, Role, WebSocketConfig, frame::coding::CloseCode},
};
use wire::Frame;
pub use wire::SyncScope;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Code {
    Denied,
    Expired,
    StaleEpoch,
    Invalid,
    Gap,
    Capacity,
    Conflict,
    ResyncRequired,
}
impl From<tmt_colab_model::Invalid> for Code {
    fn from(_: tmt_colab_model::Invalid) -> Self {
        Self::Invalid
    }
}
impl From<store::Fault> for Code {
    fn from(f: store::Fault) -> Self {
        match f {
            store::Fault::StaleEpoch => Self::StaleEpoch,
            store::Fault::Gap => Self::Gap,
            store::Fault::Conflict => Self::Conflict,
            store::Fault::Capacity => Self::Capacity,
            _ => Self::Invalid,
        }
    }
}
/// The trusted caller resolves live device/page/epoch authority from its verified
/// owner log and certificate chain. It must reject expired/revoked principals,
/// stale epochs and unauthorized namespaces/revisions, including on Read.
/// Append receives the exact model-decoded header, never unsigned routing claims.
pub enum Access<'a> {
    Read,
    Append(&'a object::Context),
}
pub trait Admission: Send {
    fn authorize(
        &self,
        principal: &str,
        scope: &SyncScope,
        access: Access<'_>,
    ) -> Result<[u8; 32], Code>;
}
struct Peer {
    principal: String,
    scope: Option<SyncScope>,
    subscribed: bool,
    queue: VecDeque<String>,
    in_flight: bool,
    terminal: Option<Code>,
}
impl Peer {
    fn end(&mut self, code: Code) {
        self.queue.clear();
        self.terminal = Some(code);
    }
    fn push(&mut self, text: String) {
        if self.terminal.is_some() {
            return;
        }
        if self.queue.len() + usize::from(self.in_flight) >= limits::SEND_QUEUE_FRAMES {
            self.end(Code::ResyncRequired);
        } else {
            self.queue.push_back(text);
        }
    }
}
struct State<A> {
    store: Store,
    admission: A,
    peers: HashMap<u64, Peer>,
    next: u64,
}
/// Shared queue/admission/store owner. No worker threads; dropping a Connection
/// removes its subscriber. Mutate authority only through update_admission.
pub struct Server<A>(Arc<Mutex<State<A>>>);
impl<A> Clone for Server<A> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
impl<A: Admission> Server<A> {
    pub fn new(store: Store, admission: A) -> Self {
        Self(Arc::new(Mutex::new(State {
            store,
            admission,
            peers: HashMap::new(),
            next: 0,
        })))
    }
    /// Serializes live-authority changes with append and clears revoked peers'
    /// pending data before any subsequent poll. Persistent owner transitions
    /// remain the caller's responsibility; this is not a membership mutation API.
    pub fn update_admission(&self, change: impl FnOnce(&mut A)) -> Result<(), Code> {
        let mut state = self.0.lock().map_err(|_| Code::Denied)?;
        change(&mut state.admission);
        state.recheck();
        Ok(())
    }
    /// stream must already be upgraded, nonblocking and exclusively owned.
    /// The caller preserves any bytes read past HTTP upgrade via a Read wrapper.
    /// No handshake or principal is accepted from untrusted WebSocket frames.
    pub fn connect<S: Read + Write>(
        &self,
        stream: S,
        principal: String,
    ) -> Result<Connection<S, A>, Code> {
        values::generated_id(&principal)?;
        let mut state = self.0.lock().map_err(|_| Code::Denied)?;
        if state.peers.len() >= limits::SOCKETS {
            return Err(Code::Capacity);
        }
        let id = state.next;
        state.next = id.checked_add(1).ok_or(Code::Capacity)?;
        state.peers.insert(
            id,
            Peer {
                principal,
                scope: None,
                subscribed: false,
                queue: VecDeque::new(),
                in_flight: false,
                terminal: None,
            },
        );
        let config = WebSocketConfig::default()
            .read_buffer_size(4096)
            .write_buffer_size(0)
            .max_write_buffer_size(limits::WS_FRAME_BYTES + 256)
            .max_message_size(Some(limits::WS_FRAME_BYTES))
            .max_frame_size(Some(limits::WS_FRAME_BYTES));
        Ok(Connection {
            socket: Some(WebSocket::from_raw_socket(
                stream,
                Role::Server,
                Some(config),
            )),
            server: self.clone(),
            id,
            blocked_since: None,
        })
    }
}
impl<A: Admission> State<A> {
    fn recheck(&mut self) {
        for peer in self.peers.values_mut() {
            if let Some(scope) = &peer.scope
                && let Err(code) = self
                    .admission
                    .authorize(&peer.principal, scope, Access::Read)
            {
                peer.end(code);
            }
        }
    }
    fn fanout(&mut self, scope: &SyncScope, text: String) {
        self.recheck();
        for peer in self
            .peers
            .values_mut()
            .filter(|p| p.subscribed && p.scope.as_ref() == Some(scope))
        {
            peer.push(text.clone());
        }
    }
    fn process(&mut self, id: u64, frame: Frame) -> Result<(), Code> {
        let scope = frame.scope()?;
        let peer = self.peers.get(&id).ok_or(Code::Denied)?;
        if peer.scope.as_ref().is_some_and(|old| old != &scope) {
            return Err(Code::Denied);
        }
        let principal = peer.principal.clone();
        self.admission.authorize(&principal, &scope, Access::Read)?;
        self.peers.get_mut(&id).ok_or(Code::Denied)?.scope = Some(scope.clone());
        let subscribing = matches!(&frame, Frame::Subscribe { .. });
        match frame {
            Frame::Hello {
                device, cursors, ..
            } => {
                if device != principal {
                    return Err(Code::Denied);
                }
                wire::cursors(&cursors)?;
                // Catchup and membership/baseline paging are the explicitly separate #1166 slice.
                return Err(Code::ResyncRequired);
            }
            Frame::Subscribe { cursors, .. } | Frame::Ack { cursors, .. } => {
                wire::cursors(&cursors)?;
                if !cursors.as_slice().is_empty() {
                    return Err(Code::ResyncRequired);
                }
                if subscribing {
                    self.peers.get_mut(&id).ok_or(Code::Denied)?.subscribed = true;
                }
            }
            Frame::Awareness { device, data, .. } => {
                if device != principal {
                    return Err(Code::Denied);
                }
                values::binary(&data, 4096)?;
                self.fanout(
                    &scope,
                    wire::output(
                        &scope,
                        "awareness",
                        serde_json::json!({"device":device,"data":data}),
                    )?,
                );
            }
            Frame::Append {
                stream_id,
                seq,
                envelope_hash,
                envelope,
                ..
            } => {
                if stream_id != principal {
                    return Err(Code::Denied);
                }
                let sequence = values::decimal(&seq, false)?;
                let hash = wire::hash(&envelope_hash)?;
                let bytes = values::binary(&envelope, limits::WS_FRAME_BYTES)?;
                let decoded = object::Envelope::from_json(&bytes)?;
                let header = object::Header::decode(decoded.header())?;
                let c = &header.context;
                if c.space != scope.space
                    || c.page != scope.page
                    || c.epoch != scope.epoch
                    || c.author_device != principal
                    || c.stream_seq != seq
                    || c.kind != "update"
                    || decoded.hash()? != hash
                {
                    return Err(Code::Invalid);
                }
                let key = self
                    .admission
                    .authorize(&principal, &scope, Access::Append(c))?;
                crypto::verify_signature(&key, &decoded.signature_input()?, decoded.signature())?;
                // Prove every broadcast fits before durable acceptance.
                let fields = serde_json::json!({"streamId":stream_id,"seq":seq,"envelopeHash":envelope_hash,"envelope":envelope});
                let broadcast = wire::output(&scope, "broadcast", fields)?;
                let receipt = wire::output(
                    &scope,
                    "receipt",
                    serde_json::json!({"streamId":stream_id,"seq":seq,"envelopeHash":envelope_hash}),
                )?;
                let accepted = self.store.append(&store::Envelope {
                    scope: StreamScope {
                        page: &scope.page,
                        epoch: values::decimal(&scope.epoch, false)?,
                        stream: &principal,
                    },
                    namespace: if c.namespace == "content" {
                        store::Namespace::Content
                    } else {
                        store::Namespace::Own
                    },
                    seq: sequence,
                    hash,
                    previous: c.prev_hash,
                    bytes: &bytes,
                })?;
                self.peers.get_mut(&id).ok_or(Code::Denied)?.push(receipt);
                if accepted == Accepted::New {
                    self.fanout(&scope, broadcast);
                }
            }
        }
        Ok(())
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Advanced,
    Closed,
}
/// Call poll on readiness (and at least once per second for stalled writes),
/// then discard on Closed. Socket drop is immediate, including failed closes.
pub struct Connection<S: Read + Write, A> {
    socket: Option<WebSocket<S>>,
    server: Server<A>,
    id: u64,
    blocked_since: Option<Instant>,
}
impl<S: Read + Write, A: Admission> Connection<S, A> {
    fn close(&mut self, code: Code) -> Progress {
        if let Some(mut socket) = self.socket.take()
            && self.blocked_since.is_none()
        {
            let reason = serde_json::to_value(code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "DENIED".into());
            let _ = socket.close(Some(CloseFrame {
                code: CloseCode::Policy,
                reason: reason.into(),
            }));
        }
        if let Ok(mut state) = self.server.0.lock() {
            state.peers.remove(&self.id);
        }
        Progress::Closed
    }
    /// One inbound application message and one outbound queued frame per call.
    /// A blocked write retains tungstenite's exact buffered bytes; never resend.
    pub fn poll(&mut self) -> Progress {
        if self.socket.is_none() {
            return Progress::Closed;
        }
        let terminal = match self.server.0.lock() {
            Ok(mut state) => {
                state.recheck();
                state.peers.get(&self.id).and_then(|p| p.terminal)
            }
            Err(_) => Some(Code::Denied),
        };
        if let Some(code) = terminal {
            return self.close(code);
        }
        if self
            .blocked_since
            .is_some_and(|t| t.elapsed() >= limits::RESPONSE)
        {
            return self.close(Code::ResyncRequired);
        }
        let flushed = (|| -> Result<bool, Code> {
            let mut state = self.server.0.lock().map_err(|_| Code::Denied)?;
            state.recheck();
            let peer = state.peers.get_mut(&self.id).ok_or(Code::Denied)?;
            if let Some(code) = peer.terminal {
                return Err(code);
            }
            match self.socket.as_mut().expect("checked socket").flush() {
                Ok(()) => {
                    self.blocked_since = None;
                    peer.in_flight = false;
                    Ok(true)
                }
                Err(e) if would_block(&e) => {
                    self.blocked_since.get_or_insert_with(Instant::now);
                    Ok(false)
                }
                Err(_) => Err(Code::ResyncRequired),
            }
        })();
        match flushed {
            Ok(true) => {}
            Ok(false) => return Progress::Pending,
            Err(code) => return self.close(code),
        }
        let mut advanced = false;
        match self.socket.as_mut().expect("checked socket").read() {
            Ok(Message::Text(text)) => {
                advanced = true;
                let Ok(frame) = serde_json::from_str::<Frame>(&text) else {
                    return self.close(Code::Invalid);
                };
                let Ok(scope) = frame.scope() else {
                    return self.close(Code::Invalid);
                };
                let result = self
                    .server
                    .0
                    .lock()
                    .map_err(|_| Code::Denied)
                    .and_then(|mut state| state.process(self.id, frame));
                if let Err(code) = result {
                    let message = wire::output(&scope, "error", serde_json::json!({"code":code}));
                    if let (Ok(text), Ok(mut state)) = (message, self.server.0.lock())
                        && let Some(peer) = state.peers.get_mut(&self.id)
                    {
                        peer.push(text);
                    }
                }
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => advanced = true,
            Ok(Message::Close(_)) => return self.close(Code::Denied),
            Ok(_) => return self.close(Code::Invalid),
            Err(e) if would_block(&e) => {}
            Err(_) => return self.close(Code::Invalid),
        }
        // Hold the authority lock through the nonblocking write: revocation
        // cannot interleave between dequeue and disclosure to the socket.
        let outgoing = (|| -> Result<bool, Code> {
            let mut state = self.server.0.lock().map_err(|_| Code::Denied)?;
            state.recheck();
            let peer = state.peers.get_mut(&self.id).ok_or(Code::Denied)?;
            if let Some(code) = peer.terminal {
                return Err(code);
            }
            let Some(text) = peer.queue.pop_front() else {
                return Ok(false);
            };
            peer.in_flight = true;
            match self
                .socket
                .as_mut()
                .expect("checked socket")
                .send(Message::Text(text.into()))
            {
                Ok(()) => peer.in_flight = false,
                Err(e) if would_block(&e) => {
                    self.blocked_since.get_or_insert_with(Instant::now);
                }
                Err(_) => return Err(Code::ResyncRequired),
            }
            Ok(true)
        })();
        match outgoing {
            Ok(sent) => advanced |= sent,
            Err(code) => return self.close(code),
        }
        if advanced {
            Progress::Advanced
        } else {
            Progress::Pending
        }
    }
}
impl<S: Read + Write, A> Drop for Connection<S, A> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.server.0.lock() {
            state.peers.remove(&self.id);
        }
    }
}
fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock)
}
