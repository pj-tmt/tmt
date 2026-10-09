//! Externally driven WebSocket sync. The caller authenticates the principal and
//! upgrades the socket; this module never owns HTTP, cookies, keys or Yjs state.
pub(crate) mod wire;
use crate::{
    limits,
    store::{self, Accepted, Store, StreamScope},
};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tmt_colab_model::{crypto, object, payload, statement, values};
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
impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Code {}
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
            store::Fault::ResyncRequired => Self::ResyncRequired,
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
    Publish,
    Append(&'a object::Context),
}
pub use crate::store::owner::WrapRecipients;
/// Caller-verified bootstrap metadata. The head comes from owner_head through
/// the admission implementation. None means this epoch has no reset baseline.
pub struct CatchupContext {
    pub membership_head: statement::Head,
    pub owner_key: [u8; 32],
    pub recipients: WrapRecipients,
    /// Exact model baseline descriptor JSON, not reconstructed signing bytes.
    pub baseline: Option<Vec<u8>>,
}
pub trait Admission: Send {
    fn alive(&self, _: &str) -> Result<(), Code> {
        Ok(())
    }
    fn catchup_context(
        &self,
        principal: &str,
        scope: &SyncScope,
        store: &Store,
    ) -> Result<CatchupContext, Code>;

    fn authorize(
        &self,
        principal: &str,
        scope: &SyncScope,
        access: Access<'_>,
    ) -> Result<[u8; 32], Code>;

    /// Where a browser Save prepares from, outside every lock. `None`: this admission has no
    /// native writer, so a save is denied.
    fn save_source(&self) -> Option<crate::page::save::SourceOpener> {
        None
    }
    /// The public key that must have signed a root-local save; never request-selected.
    fn save_author(&self) -> crate::Result<[u8; 32]> {
        Err(crate::page::Fault::Denied.into())
    }
    /// Commit one frozen save publication under the writer, with the revision read while no
    /// other writer could run.
    fn commit_save(
        &mut self,
        _job: &crate::publication::SignedJob,
        _packet: &[u8],
        _chain: &[u8],
        _now: u64,
        _combine_until: Instant,
    ) -> crate::Result<(crate::page::PublicationCommitted, Option<String>)> {
        Err(crate::page::Fault::Denied.into())
    }
    /// What the root-local stream recorded for an operation a browser chose. Read-only.
    fn save_status(
        &self,
        _page: &str,
        _operation_id: &str,
    ) -> crate::Result<crate::page::save::SaveResult> {
        Err(crate::page::Fault::Denied.into())
    }
}
struct Peer {
    principal: String,
    scope: Option<SyncScope>,
    subscribed: bool,
    queue: VecDeque<Delivery>,
    hello: bool,
    catchup: Option<Catchup>,
    chains: std::collections::HashSet<String>,
    in_flight: usize,
    incoming: Option<Incoming>,
    buffered_slot: bool,
    terminal: Option<Code>,
}
struct Catchup {
    recipients: WrapRecipients,
    positions: HashMap<(String, String), store::NamespaceCursor>,
    head: statement::Head,
    owner: [u8; 32],
    revision: u64,
    wrap_offset: usize,
    wraps_done: bool,
    wrap_epoch: u64,
}
fn bootstrap_error(error: Box<dyn std::error::Error + Send + Sync>) -> Code {
    if let Some(fault) = error.downcast_ref::<store::Fault>() {
        return match fault {
            store::Fault::ResyncRequired => Code::ResyncRequired,
            store::Fault::Capacity => Code::Capacity,
            _ => Code::Invalid,
        };
    }
    if error.downcast_ref::<store::owner::OwnerFault>() == Some(&store::owner::OwnerFault::Capacity)
    {
        Code::Capacity
    } else {
        Code::Invalid
    }
}
impl Peer {
    fn new(principal: String) -> Self {
        Self {
            principal,
            scope: None,
            subscribed: false,
            queue: VecDeque::new(),
            hello: false,
            catchup: None,
            chains: Default::default(),
            in_flight: 0,
            incoming: None,
            buffered_slot: false,
            terminal: None,
        }
    }
    fn end(&mut self, code: Code) {
        self.queue.clear();
        self.catchup = None;
        self.incoming = None;
        self.terminal = Some(code);
    }
    fn push(&mut self, text: String) {
        self.enqueue(Delivery::Frame(text));
    }
    fn membership_transfer(&mut self, scope: &SyncScope, transfer: Option<([u8; 32], Vec<u8>)>) {
        if let Some((hash, bytes)) = transfer {
            self.enqueue(Delivery::Transfer {
                scope: scope.clone(),
                identity: ChunkIdentity::Statement {
                    statement_hash: values::encode_binary(&hash),
                },
                bytes: Arc::new(bytes),
                index: 0,
            });
        }
    }
    fn enqueue(&mut self, delivery: Delivery) {
        if self.terminal.is_some() {
            return;
        }
        if self.queue.len() + usize::from(self.buffered_slot) >= limits::SEND_QUEUE_FRAMES {
            self.end(Code::ResyncRequired);
        } else {
            self.queue.push_back(delivery);
        }
    }
}
/// A transfer reserves one queue slot and emits only one bounded frame per
/// poll. Arc shares frozen broadcast bytes; no eager list of chunk frames exists.
#[derive(Clone)]
enum Delivery {
    Frame(String),
    Object {
        text: String,
        refused: String,
        fence: Arc<crate::object_channel::admission::RequestCapture>,
    },
    Transfer {
        scope: SyncScope,
        identity: ChunkIdentity,
        bytes: Arc<Vec<u8>>,
        index: usize,
    },
}
#[derive(Clone, Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
enum ChunkIdentity {
    Object {
        object_id: String,
        envelope_hash: String,
    },
    Statement {
        statement_hash: String,
    },
}
impl Delivery {
    fn next(&mut self) -> Result<(String, bool), Code> {
        match self {
            Self::Frame(text) => Ok((text.clone(), true)),
            Self::Object {
                text,
                refused,
                fence,
            } => {
                // No backend or decoder work runs here. Recheck captured native
                // metadata under the same authority lock held through the write.
                Ok((
                    if Instant::now() < fence.deadline
                        && fence.current().is_ok()
                        && Instant::now() < fence.deadline
                    {
                        text
                    } else {
                        refused
                    }
                    .clone(),
                    true,
                ))
            }
            Self::Transfer {
                scope,
                identity,
                bytes,
                index,
            } => {
                let count = bytes.len().div_ceil(limits::CHUNK_BYTES);
                let start = *index * limits::CHUNK_BYTES;
                let end = (start + limits::CHUNK_BYTES).min(bytes.len());
                let mut fields = serde_json::to_value(identity).map_err(|_| Code::Invalid)?;
                fields["index"] = serde_json::json!(index);
                fields["count"] = serde_json::json!(count);
                fields["bytes"] = serde_json::json!(values::encode_binary(&bytes[start..end]));
                let text = wire::output(scope, "chunk", fields)?;
                *index += 1;
                Ok((text, *index == count))
            }
        }
    }
}
struct AppendRequest {
    stream: String,
    seq: String,
    hash: String,
    bytes: Vec<u8>,
    object_id: Option<String>,
}
/// One assembled save, handed out of the sync lock for its long native preparation.
pub(crate) struct SaveJob {
    open: crate::page::save::SourceOpener,
    scope: SyncScope,
    principal: String,
    save: crate::page::save::Save,
}
/// What an assembling upload becomes when its last chunk arrives.
enum Upload {
    Append {
        stream: String,
        seq: String,
    },
    Save {
        operation_id: String,
        base: [u8; 32],
    },
}
impl Upload {
    /// Bytes the whole object may hold, the chunks it may take and how long it may take.
    fn bounds(&self) -> (usize, usize, Duration) {
        match self {
            Self::Append { .. } => (
                limits::UPDATE_BYTES,
                limits::UPDATE_BYTES.div_ceil(limits::CHUNK_BYTES),
                limits::ACQUISITION,
            ),
            Self::Save { .. } => (
                crate::decoder::BASELINE_BYTES,
                limits::SAVE_CHUNKS,
                limits::SAVE_UPLOAD,
            ),
        }
    }
}
struct Incoming {
    upload: Upload,
    hash: String,
    object_id: String,
    count: Option<usize>,
    next: usize,
    bytes: Vec<u8>,
    started: Instant,
}
fn namespace(name: &str) -> Result<store::Namespace, Code> {
    match name {
        "content" => Ok(store::Namespace::Content),
        "own" => Ok(store::Namespace::Own),
        _ => Err(Code::Invalid),
    }
}
/// Returns an inline payload or a reference plus a lazy transfer. Validate exact
/// stored bytes before disclosure; this does not replace client log verification.
fn delivery(
    scope: &SyncScope,
    hash: [u8; 32],
    bytes: Vec<u8>,
) -> Result<(serde_json::Value, Option<Delivery>), Code> {
    if bytes.is_empty() || bytes.len() > limits::OBJECT_BYTES {
        return Err(Code::Capacity);
    }
    let decoded = object::Envelope::from_json(&bytes)?;
    let header = object::Header::decode(decoded.header())?;
    if decoded.hash()? != hash
        || header.context.space != scope.space
        || header.context.page != scope.page
        || header.context.epoch != scope.epoch
    {
        return Err(Code::Invalid);
    }
    // Leave room for catchup metadata and routing fields, even in the first page.
    if bytes.len() <= limits::CHUNK_BYTES {
        Ok((serde_json::json!(values::encode_binary(&bytes)), None))
    } else {
        Ok((
            serde_json::json!({"objectId":header.object_id}),
            Some(Delivery::Transfer {
                scope: scope.clone(),
                identity: ChunkIdentity::Object {
                    object_id: header.object_id,
                    envelope_hash: values::encode_binary(&hash),
                },
                bytes: Arc::new(bytes),
                index: 0,
            }),
        ))
    }
}
struct State<A> {
    store: Store,
    admission: A,
    peers: HashMap<u64, Peer>,
    next: u64,
    /// Saves between `save_job` and `finish_save`, by (page, operation ID) with a count: while
    /// one is preparing, a status request for it is `pending`, never `absent`, even after the
    /// connection that sent it is gone.
    saving: HashMap<(String, String), usize>,
}
/// Shared queue/admission/store owner. No worker threads; dropping a Connection
/// removes its subscriber. Mutate authority only through update_admission.
pub struct Server<A>(Arc<Mutex<State<A>>>);
impl<A> Clone for Server<A> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
/// One transport broadcast per verified entry, in sequence order, prepared before the commit so
/// that the transaction never waits on transport. The opaque server never opens the source.
fn broadcasts(
    job: &crate::publication::SignedJob,
    packet: &[u8],
    chain: &str,
    author: &[u8; 32],
) -> crate::Result<Vec<(SyncScope, String, Option<Delivery>)>> {
    let mut outputs = Vec::new();
    for entry in job.verify_packet(packet, author)? {
        let c = &entry.header.context;
        let scope = SyncScope {
            space: c.space.clone(),
            page: c.page.clone(),
            epoch: c.epoch.clone(),
        };
        let hash = entry.envelope.hash()?;
        let (envelope, transfer) = delivery(&scope, hash, entry.bytes.to_vec())?;
        let broadcast = wire::output(
            &scope,
            "broadcast",
            serde_json::json!({
                "streamId": c.author_device, "seq": c.stream_seq,
                "envelopeHash": values::encode_binary(&hash), "envelope": envelope,
                "chains": [{"deviceId": c.author_device, "chain": chain}]
            }),
        )?;
        outputs.push((scope, broadcast, transfer));
    }
    Ok(outputs)
}
impl Server<crate::registration::OwnerAdmission> {
    /// Prepare transport before committing. The opaque server never opens source.
    pub(crate) fn publish(
        &self,
        body: &[u8],
        now: u64,
        received: std::time::Instant,
    ) -> crate::Result<crate::page::Published> {
        use crate::{page::Fault, publication};
        // The client stops waiting at PUBLISH_REPLY, counted from when it finished sending; a
        // combine must be done, or give up, a response interval before that.
        let combine_until = received + limits::PUBLISH_COMBINE;
        let author = self
            .0
            .lock()
            .map_err(|_| Fault::Unavailable)?
            .admission
            .0
            .lock()
            .map_err(|_| Fault::Unavailable)?
            .author_key()?;
        if publication::names_other_write_version(body) {
            return Err(Fault::ServerMismatch.into());
        }
        let write =
            publication::LocalWrite::from_json(body, &author).map_err(|_| Fault::Invalid)?;
        let job = &write.signed_job;
        let packet = values::binary(
            &write.packet,
            publication::packet_limit(job.manifest.entries.len())?,
        )?;
        let chain = values::binary(&write.chain, publication::CHAIN_BYTES)?;
        let outputs = broadcasts(job, &packet, &write.chain, &author)?;
        let mut state = self.0.lock().map_err(|_| Fault::Unavailable)?;
        let (committed, revision) = state
            .admission
            .0
            .lock()
            .map_err(|_| Fault::Unavailable)?
            .publish(job, &packet, &chain, now, combine_until)?;
        if committed.accepted == Accepted::New
            && matches!(
                committed.record.outcome,
                publication::Outcome::Committed { .. }
            )
        {
            for (scope, broadcast, transfer) in outputs {
                state.fanout(&scope, broadcast, transfer);
            }
        }
        Ok(crate::page::Published {
            record: committed.record,
            revision,
        })
    }
}
impl<A: Admission> Server<A> {
    pub fn new(store: Store, admission: A) -> Self {
        Self(Arc::new(Mutex::new(State {
            store,
            admission,
            peers: HashMap::new(),
            next: 0,
            saving: HashMap::new(),
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
        state.peers.insert(id, Peer::new(principal));
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
            objects: None,
            ping_due: None,
        })
    }
}
/// The same bounded catchup codec without a transport or a second admission
/// identity. Its caller owns the actual outer peer, frozen deadline and fences.
pub(crate) struct DetachedCatchup<A: Admission> {
    state: State<A>,
}
impl<A: Admission> DetachedCatchup<A> {
    pub(crate) fn new(
        store: Store,
        admission: A,
        scope: SyncScope,
        principal: String,
        wrap_epoch: u64,
    ) -> Result<Self, Code> {
        let mut state = State {
            store,
            admission,
            peers: HashMap::new(),
            next: 1,
            saving: HashMap::new(),
        };
        // The consumer already admitted current membership and eligible opaque
        // historical keys on its live connection. Name that exact head without
        // replaying statements or re-importing addressed wraps into live state.
        let revision = state
            .admission
            .catchup_context(&principal, &scope, &state.store)?
            .membership_head
            .revision;
        let mut peer = Peer::new(principal.clone());
        peer.scope = Some(scope.clone());
        peer.hello = true;
        state.peers.insert(0, peer);
        state.start_catchup(
            0,
            &scope,
            &principal,
            &serde_json::from_str("[]").map_err(|_| Code::Invalid)?,
            revision,
        )?;
        let catchup = state
            .peers
            .get_mut(&0)
            .ok_or(Code::Denied)?
            .catchup
            .as_mut()
            .ok_or(Code::Invalid)?;
        catchup.wrap_epoch = wrap_epoch;
        catchup.wraps_done = true;
        Ok(Self { state })
    }
    pub(crate) fn next(&mut self) -> Result<Option<(String, bool)>, Code> {
        self.state.recheck();
        if let Some(code) = self.state.peers[&0].terminal {
            return Err(code);
        }
        if self.state.peers[&0].queue.is_empty() {
            self.state.catchup_page(0)?;
        }
        let peer = self.state.peers.get_mut(&0).ok_or(Code::Denied)?;
        let Some(mut delivery) = peer.queue.pop_front() else {
            return Ok(None);
        };
        let (text, done) = delivery.next()?;
        if !done {
            peer.queue.push_front(delivery);
        }
        Ok(Some((
            text,
            peer.queue.is_empty() && peer.catchup.is_none(),
        )))
    }
}
impl<A: Admission> State<A> {
    fn recheck(&mut self) {
        for peer in self.peers.values_mut() {
            let result = self
                .admission
                .alive(&peer.principal)
                .and_then(|()| match &peer.scope {
                    Some(scope) => self
                        .admission
                        .authorize(&peer.principal, scope, Access::Read)
                        .map(|_| ()),
                    None => Ok(()),
                });
            if let Err(code) = result {
                peer.end(code);
            }
        }
    }
    fn fanout(&mut self, scope: &SyncScope, text: String, transfer: Option<Delivery>) {
        self.recheck();
        for peer in self
            .peers
            .values_mut()
            .filter(|p| p.subscribed && p.scope.as_ref() == Some(scope))
        {
            peer.push(text.clone());
            if let Some(transfer) = &transfer {
                peer.enqueue(transfer.clone());
            }
        }
    }
    /// `Some` is a save whose long preparation the caller runs outside this lock, then hands back
    /// to `finish_save`.
    fn process(&mut self, id: u64, frame: Frame, now: Instant) -> Result<Option<SaveJob>, Code> {
        let scope = frame.scope()?;
        let peer = self.peers.get(&id).ok_or(Code::Denied)?;
        if peer.scope.as_ref().is_some_and(|old| old != &scope) {
            return Err(Code::Denied);
        }
        let principal = peer.principal.clone();
        self.admission.authorize(&principal, &scope, Access::Read)?;
        if matches!(
            &frame,
            Frame::Append { .. }
                | Frame::Chunk { .. }
                | Frame::Awareness { .. }
                | Frame::Save { .. }
                | Frame::SaveStatus { .. }
        ) {
            self.admission
                .authorize(&principal, &scope, Access::Publish)?;
        }
        self.peers.get_mut(&id).ok_or(Code::Denied)?.scope = Some(scope.clone());
        let subscribing = matches!(&frame, Frame::Subscribe { .. });
        match frame {
            Frame::Object { .. } => return Err(Code::Invalid),
            Frame::Hello {
                device,
                membership_revision,
                cursors,
                ..
            } => {
                if device != principal {
                    return Err(Code::Denied);
                }
                wire::cursors(&cursors)?;
                if self.peers[&id].hello {
                    return Err(Code::Invalid);
                }
                self.start_catchup(
                    id,
                    &scope,
                    &principal,
                    &cursors,
                    values::decimal(&membership_revision, true)?,
                )?;
                self.peers.get_mut(&id).ok_or(Code::Denied)?.hello = true;
            }
            Frame::Subscribe { cursors, .. } | Frame::Ack { cursors, .. } => {
                wire::cursors(&cursors)?;
                if subscribing && !cursors.as_slice().is_empty() {
                    self.start_catchup(id, &scope, &principal, &cursors, 0)?;
                } else {
                    for c in cursors.as_slice() {
                        self.resolve(&scope, c)?;
                    }
                    if !subscribing {
                        let peer = self.peers.get_mut(&id).ok_or(Code::Denied)?;
                        peer.in_flight = peer.in_flight.checked_sub(1).ok_or(Code::Invalid)?;
                    }
                    if subscribing {
                        let peer = self.peers.get_mut(&id).ok_or(Code::Denied)?;
                        if peer.catchup.is_some() {
                            return Err(Code::Invalid);
                        }
                        peer.subscribed = true;
                    }
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
                    None,
                );
            }
            Frame::Append {
                stream_id,
                seq,
                envelope_hash,
                envelope,
                ..
            } => {
                if self.peers[&id].incoming.is_some() {
                    return Err(Code::Invalid);
                }
                if stream_id != principal {
                    return Err(Code::Denied);
                }
                values::decimal(&seq, false)?;
                wire::hash(&envelope_hash)?;
                match envelope {
                    wire::Payload::Inline(envelope) => {
                        let bytes = values::binary(&envelope, limits::UPDATE_BYTES)?;
                        self.append(
                            id,
                            &scope,
                            &principal,
                            AppendRequest {
                                stream: stream_id,
                                seq,
                                hash: envelope_hash,
                                bytes,
                                object_id: None,
                            },
                        )?;
                    }
                    wire::Payload::Reference(reference) => {
                        values::object_id(&reference.object_id)?;
                        self.peers.get_mut(&id).ok_or(Code::Denied)?.incoming = Some(Incoming {
                            upload: Upload::Append {
                                stream: stream_id,
                                seq,
                            },
                            hash: envelope_hash,
                            object_id: reference.object_id,
                            count: None,
                            next: 0,
                            bytes: Vec::new(),
                            started: now,
                        });
                    }
                }
            }
            Frame::Chunk {
                object_id,
                envelope_hash,
                index,
                count,
                bytes,
                ..
            } => {
                values::object_id(&object_id)?;
                wire::hash(&envelope_hash)?;
                let mut incoming = self
                    .peers
                    .get_mut(&id)
                    .ok_or(Code::Denied)?
                    .incoming
                    .take()
                    .ok_or(Code::Invalid)?;
                let (cap, chunks, window) = incoming.upload.bounds();
                if now.saturating_duration_since(incoming.started) >= window
                    || object_id != incoming.object_id
                    || envelope_hash != incoming.hash
                    || index != incoming.next
                    || count == 0
                    || count > chunks
                    || index >= count
                    || incoming.count.is_some_and(|old| old != count)
                {
                    return Err(Code::Invalid);
                }
                let chunk = values::binary(&bytes, limits::CHUNK_BYTES)?;
                if chunk.is_empty() || (index + 1 < count && chunk.len() != limits::CHUNK_BYTES) {
                    return Err(Code::Invalid);
                }
                if incoming.bytes.len() + chunk.len() > cap {
                    return Err(Code::Capacity);
                }
                incoming.count = Some(count);
                incoming.next += 1;
                incoming.bytes.extend_from_slice(&chunk);
                if incoming.next == count {
                    match incoming.upload {
                        Upload::Append { stream, seq } => self.append(
                            id,
                            &scope,
                            &principal,
                            AppendRequest {
                                stream,
                                seq,
                                hash: incoming.hash,
                                bytes: incoming.bytes,
                                object_id: Some(object_id),
                            },
                        )?,
                        Upload::Save { operation_id, base } => {
                            return self
                                .save_job(
                                    &scope,
                                    principal,
                                    operation_id,
                                    base,
                                    &incoming.hash,
                                    incoming.bytes,
                                )
                                .map(Some);
                        }
                    }
                } else {
                    self.peers.get_mut(&id).ok_or(Code::Denied)?.incoming = Some(incoming);
                }
            }
            Frame::Save {
                operation_id,
                base_sha256,
                source_sha256,
                source,
                ..
            } => {
                if self.peers[&id].incoming.is_some() {
                    return Err(Code::Invalid);
                }
                values::generated_id(&operation_id)?;
                let base = wire::hash(&base_sha256)?;
                wire::hash(&source_sha256)?;
                match source {
                    wire::Payload::Inline(text) => {
                        let bytes = values::binary(&text, crate::decoder::BASELINE_BYTES)?;
                        return self
                            .save_job(&scope, principal, operation_id, base, &source_sha256, bytes)
                            .map(Some);
                    }
                    wire::Payload::Reference(reference) => {
                        values::object_id(&reference.object_id)?;
                        self.admission.save_source().ok_or(Code::Denied)?;
                        self.peers.get_mut(&id).ok_or(Code::Denied)?.incoming = Some(Incoming {
                            upload: Upload::Save { operation_id, base },
                            hash: source_sha256,
                            object_id: reference.object_id,
                            count: None,
                            next: 0,
                            bytes: Vec::new(),
                            started: now,
                        });
                    }
                }
            }
            Frame::SaveStatus { operation_id, .. } => {
                if self.peers[&id].incoming.is_some() {
                    return Err(Code::Invalid);
                }
                values::generated_id(&operation_id)?;
                let reply = if self
                    .saving
                    .contains_key(&(scope.page.clone(), operation_id.clone()))
                {
                    crate::page::save::SaveResult::pending(&operation_id)
                } else {
                    self.admission
                        .save_status(&scope.page, &operation_id)
                        .unwrap_or_else(|error| {
                            crate::page::save::SaveResult::rejected(&operation_id, error.as_ref())
                        })
                };
                self.reply_save(id, &scope, &reply)?;
            }
        }
        Ok(None)
    }
    /// An assembled source, once its bytes match the digest the client bound to it.
    fn save_job(
        &mut self,
        scope: &SyncScope,
        principal: String,
        operation_id: String,
        base: [u8; 32],
        source_sha256: &str,
        bytes: Vec<u8>,
    ) -> Result<SaveJob, Code> {
        if wire::hash(source_sha256)? != crypto::digest(&bytes) {
            return Err(Code::Invalid);
        }
        let source = String::from_utf8(bytes).map_err(|_| Code::Invalid)?;
        let open = self.admission.save_source().ok_or(Code::Denied)?;
        *self
            .saving
            .entry((scope.page.clone(), operation_id.clone()))
            .or_default() += 1;
        Ok(SaveJob {
            open,
            scope: scope.clone(),
            principal,
            save: crate::page::save::Save {
                page: scope.page.clone(),
                operation_id,
                base_sha256: base,
                source,
            },
        })
    }
    /// The save is no longer preparing; its outcome, if any, is in the store.
    fn unsave(&mut self, job: &SaveJob) {
        let key = (job.save.page.clone(), job.save.operation_id.clone());
        if let Some(count) = self.saving.get_mut(&key) {
            *count -= 1;
            if *count == 0 {
                self.saving.remove(&key);
            }
        }
    }
    fn reply_save(
        &mut self,
        id: u64,
        scope: &SyncScope,
        reply: &crate::page::save::SaveResult,
    ) -> Result<(), Code> {
        let text = wire::output(
            scope,
            "saveresult",
            serde_json::to_value(reply).map_err(|_| Code::Invalid)?,
        )?;
        if let Some(peer) = self.peers.get_mut(&id) {
            peer.push(text);
        }
        Ok(())
    }
    /// Commit a prepared save: authority is rechecked, the fences of `commit_publication`
    /// turn a page that moved during preparation into a stale-base refusal, and the originator
    /// gets one reply while every subscriber gets the same broadcasts as for any write.
    fn finish_save(
        &mut self,
        id: u64,
        job: SaveJob,
        prepared: crate::Result<crate::page::save::Prepared>,
        now_ms: u64,
    ) -> Result<(), Code> {
        use crate::page::save::{Prepared, SaveResult};
        // Under this lock hold the commit and the removal are one step for a status request.
        self.unsave(&job);
        let operation = job.save.operation_id.clone();
        let done = (|| -> crate::Result<(SaveResult, Vec<_>)> {
            let write = match prepared? {
                Prepared::Unchanged(receipt) => {
                    return Ok((
                        SaveResult::unchanged(&operation, receipt.revision),
                        Vec::new(),
                    ));
                }
                Prepared::Write(frozen) => frozen,
            };
            self.admission
                .authorize(&job.principal, &job.scope, Access::Publish)
                .map_err(|_| crate::page::Fault::Denied)?;
            let author = self.admission.save_author()?;
            let outputs = broadcasts(
                write.job(),
                write.packet(),
                &values::encode_binary(write.chain()),
                &author,
            )?;
            let (committed, revision) = self.admission.commit_save(
                write.job(),
                write.packet(),
                write.chain(),
                now_ms,
                Instant::now() + limits::PUBLISH_COMBINE,
            )?;
            let receipt = crate::page::publication_receipt(write.job(), &committed.record)?;
            let fan_out = committed.accepted == Accepted::New
                && matches!(
                    committed.record.outcome,
                    crate::publication::Outcome::Committed { .. }
                );
            Ok((
                SaveResult::committed(&operation, revision.unwrap_or(receipt.revision)),
                if fan_out { outputs } else { Vec::new() },
            ))
        })();
        let (reply, outputs) = match done {
            Ok(done) => done,
            Err(error) => (SaveResult::rejected(&operation, error.as_ref()), Vec::new()),
        };
        self.reply_save(id, &job.scope, &reply)?;
        for (scope, broadcast, transfer) in outputs {
            self.fanout(&scope, broadcast, transfer);
        }
        Ok(())
    }
    fn resolve(&self, scope: &SyncScope, cursor: &wire::SyncCursor) -> Result<(), Code> {
        self.store.resolve_cursor(
            StreamScope {
                page: &scope.page,
                epoch: values::decimal(&scope.epoch, false)?,
                stream: &cursor.stream_id,
            },
            namespace(&cursor.namespace)?,
            store::NamespaceCursor {
                seq: values::decimal(&cursor.seq, true)?,
                hash: wire::hash(&cursor.envelope_hash)?,
            },
        )?;
        Ok(())
    }
    fn start_catchup(
        &mut self,
        id: u64,
        scope: &SyncScope,
        principal: &str,
        cursors: &tmt_colab_model::bounded::List<wire::SyncCursor, 256>,
        revision: u64,
    ) -> Result<(), Code> {
        if self.peers[&id].catchup.is_some() {
            return Err(Code::Invalid);
        }
        wire::cursors(cursors)?;
        let mut positions = HashMap::new();
        for c in cursors.as_slice() {
            self.resolve(scope, c)?;
            positions.insert(
                (c.stream_id.clone(), c.namespace.clone()),
                store::NamespaceCursor {
                    seq: values::decimal(&c.seq, true)?,
                    hash: wire::hash(&c.envelope_hash)?,
                },
            );
        }
        let context = self
            .admission
            .catchup_context(principal, scope, &self.store)?;
        if context.membership_head.revision == 0 {
            return Err(Code::Invalid);
        }
        let mut transfer = None;
        let mut fields = serde_json::json!({
            "membershipHead":{"revision":context.membership_head.revision.to_string(),"statementHash":values::encode_binary(&context.membership_head.hash),"ownerKey":values::encode_binary(&context.owner_key),"statements":[],"more":true},
            "baseline":null,"streams":[],"more":true
        });
        if let Some(bytes) = context.baseline {
            if bytes.len() > limits::SYNC_CONTEXT_BYTES {
                return Err(Code::Capacity);
            }
            let descriptor = payload::decode_baseline(&bytes)?;
            if descriptor.page_id != scope.page
                || descriptor.epoch != scope.epoch
                || values::decimal(&descriptor.membership_revision, false)?
                    > context.membership_head.revision
            {
                return Err(Code::Invalid);
            }
            let saved = self
                .store
                .baseline(&scope.page, values::decimal(&scope.epoch, false)?)
                .map_err(bootstrap_error)?
                .ok_or(Code::ResyncRequired)?;
            if saved.descriptor != bytes {
                return Err(Code::ResyncRequired);
            }
            let hash = wire::hash(&descriptor.object_envelope_hash)?;
            let decoded = object::Envelope::from_json(&saved.envelope)?;
            let header = object::Header::decode(decoded.header())?;
            if header.context.kind != "html"
                || header.context.membership_revision != descriptor.membership_revision
            {
                return Err(Code::Invalid);
            }
            let (envelope, chunks) = delivery(scope, hash, saved.envelope)?;
            fields["baseline"] = serde_json::json!(values::encode_binary(&bytes));
            fields["baselineObject"] = serde_json::json!({"envelopeHash":descriptor.object_envelope_hash,"envelope":envelope});
            transfer = chunks;
        }
        // The first page also carries the descriptor and possibly an inline
        // baseline. Budget statements against those exact bytes, without truncation.
        let overhead = wire::output(scope, "catchup", fields.clone())?.len();
        let membership = self
            .store
            .owner_read(&scope.space, &context.owner_key, |tx| {
                tx.membership_page(
                    revision,
                    &context.membership_head,
                    limits::WS_FRAME_BYTES
                        .saturating_sub(overhead + 64)
                        .min(60 * 1024),
                    fields.get("baselineObject").is_none(),
                )
            })
            .map_err(bootstrap_error)?;
        fields["membershipHead"]["statements"] = serde_json::json!(membership.statements);
        fields["membershipHead"]["more"] = serde_json::json!(membership.more);
        let text = wire::output(scope, "catchup", fields)?;
        let peer = self.peers.get_mut(&id).ok_or(Code::Denied)?;
        peer.subscribed = false;
        peer.catchup = Some(Catchup {
            positions,
            recipients: context.recipients,
            head: context.membership_head,
            owner: context.owner_key,
            revision: membership.revision,
            wrap_offset: 0,
            wraps_done: false,
            wrap_epoch: values::decimal(&scope.epoch, false)?,
        });
        peer.push(text);
        if let Some(transfer) = transfer {
            peer.enqueue(transfer);
        }
        peer.membership_transfer(scope, membership.transfer);
        Ok(())
    }
    /// Scan all namespaces anew each turn. An append to an already visited
    /// namespace is included before the empty final page; no snapshot/live gap.
    fn catchup_page(&mut self, id: u64) -> Result<(), Code> {
        let peer = self.peers.get(&id).ok_or(Code::Denied)?;
        let Some(catchup) = &peer.catchup else {
            return Ok(());
        };
        if !peer.queue.is_empty()
            || peer.buffered_slot
            || (peer.hello && peer.in_flight == limits::SEND_QUEUE_FRAMES)
        {
            return Ok(());
        }
        let scope = peer.scope.clone().ok_or(Code::Denied)?;
        let epoch = values::decimal(&scope.epoch, false)?;
        // Live catchup binds wraps and content to one epoch. The detached
        // history owner supplies current entitlement wraps for an older epoch.
        let namespaces = if catchup.wrap_epoch > epoch {
            self.store.retained_namespaces(&scope.page, epoch)?
        } else {
            self.store.namespaces(&scope.page, epoch)?
        };
        if catchup.revision < catchup.head.revision {
            let membership = self
                .store
                .owner_read(&scope.space, &catchup.owner, |tx| {
                    tx.membership_page(catchup.revision, &catchup.head, 60 * 1024, true)
                })
                .map_err(bootstrap_error)?;
            let text = wire::output(
                &scope,
                "catchup",
                serde_json::json!({"membership":{"statements":membership.statements,"more":membership.more},"streams":[],"more":true}),
            )?;
            let peer = self.peers.get_mut(&id).ok_or(Code::Denied)?;
            peer.catchup.as_mut().ok_or(Code::Invalid)?.revision = membership.revision;
            peer.push(text);
            peer.membership_transfer(&scope, membership.transfer);
            return Ok(());
        }
        if !catchup.wraps_done {
            let (wraps, more) = self
                .store
                .owner_read(&scope.space, &catchup.owner, |tx| {
                    tx.wrap_page(
                        &scope.page,
                        catchup.wrap_epoch,
                        &peer.principal,
                        &catchup.head,
                        catchup.wrap_offset,
                        &catchup.recipients,
                    )
                })
                .map_err(bootstrap_error)?;
            let count = wraps.len();
            let complete = !more && namespaces.is_empty();
            let text = wire::output(
                &scope,
                "catchup",
                serde_json::json!({"wraps":wraps,"streams":[],"more":!complete}),
            )?;
            let peer = self.peers.get_mut(&id).ok_or(Code::Denied)?;
            let catchup = peer.catchup.as_mut().ok_or(Code::Invalid)?;
            catchup.wrap_offset += count;
            catchup.wraps_done = !more;
            if complete {
                peer.catchup = None;
                peer.subscribed = true;
            }
            peer.push(text);
            return Ok(());
        }
        let positions = &catchup.positions;
        let mut next = None;
        for (stream, ns) in namespaces {
            let name = if ns == store::Namespace::Content {
                "content"
            } else {
                "own"
            };
            let key = (stream.clone(), name.to_owned());
            let cursor = positions.get(&key).copied().unwrap_or_default();
            let stream_scope = StreamScope {
                page: &scope.page,
                epoch,
                stream: &stream,
            };
            let object = if catchup.wrap_epoch > epoch {
                self.store
                    .retained_namespace_next(stream_scope, ns, cursor)?
            } else {
                self.store.namespace_next(stream_scope, ns, cursor)?
            };
            if let Some(object) = object {
                // Every paired checkpoint precedes the shared tail. Namespace
                // tails interleave by stream sequence, preserving signed prevHash.
                let order = (!object.checkpoint, stream.as_str(), object.cursor.seq);
                if next.as_ref().is_none_or(
                    |(old_key, old): &((String, String), store::ReadObject)| {
                        order < (!old.checkpoint, old_key.0.as_str(), old.cursor.seq)
                    },
                ) {
                    next = Some((key, object));
                }
            }
        }
        if let Some(((stream, ns), object)) = next {
            let decoded = object::Envelope::from_json(&object.bytes)?;
            let header = object::Header::decode(decoded.header())?;
            if header.context.author_device != stream
                || header.context.namespace != ns
                || values::decimal(&header.context.stream_seq, false)? != object.cursor.seq
                || header.context.kind
                    != if object.checkpoint {
                        "checkpoint"
                    } else {
                        "update"
                    }
            {
                return Err(Code::Invalid);
            }
            let (envelope, transfer) = delivery(&scope, object.cursor.hash, object.bytes)?;
            let entry = serde_json::json!({"seq":object.cursor.seq.to_string(),"envelopeHash":values::encode_binary(&object.cursor.hash),"envelope":envelope});
            let chains = if peer.chains.contains(&stream) {
                Vec::new()
            } else {
                let chain = self
                    .store
                    .owner_read(&scope.space, &catchup.owner, |tx| tx.author_chain(&stream))
                    .map_err(bootstrap_error)?;
                vec![serde_json::json!({"deviceId":stream,"chain":values::encode_binary(&chain)})]
            };
            let text = wire::output(
                &scope,
                "catchup",
                serde_json::json!({"chains":chains,"streams":[{
                "streamId":stream,"namespace":ns,"checkpoint":if object.checkpoint { entry.clone() } else { serde_json::Value::Null },
                "tail":if object.checkpoint { Vec::<serde_json::Value>::new() } else { vec![entry] }
            }],"more":true}),
            )?;
            let peer = self.peers.get_mut(&id).ok_or(Code::Denied)?;
            peer.catchup
                .as_mut()
                .ok_or(Code::Invalid)?
                .positions
                .insert((stream.clone(), ns), object.cursor);
            peer.chains.insert(stream);
            peer.push(text);
            if let Some(transfer) = transfer {
                peer.enqueue(transfer);
            }
        } else {
            let text = wire::output(
                &scope,
                "catchup",
                serde_json::json!({"streams":[],"more":false}),
            )?;
            let peer = self.peers.get_mut(&id).ok_or(Code::Denied)?;
            peer.push(text);
            peer.catchup = None;
            peer.subscribed = true;
        }
        Ok(())
    }
    fn append(
        &mut self,
        id: u64,
        scope: &SyncScope,
        principal: &str,
        request: AppendRequest,
    ) -> Result<(), Code> {
        let AppendRequest {
            stream,
            seq,
            hash: envelope_hash,
            bytes,
            object_id,
        } = request;
        let sequence = values::decimal(&seq, false)?;
        let hash = wire::hash(&envelope_hash)?;
        let decoded = object::Envelope::from_json(&bytes)?;
        let header = object::Header::decode(decoded.header())?;
        let c = &header.context;
        if c.space != scope.space
            || c.page != scope.page
            || c.epoch != scope.epoch
            || c.author_device != principal
            || stream != principal
            || c.stream_seq != seq
            || c.kind != "update"
            || decoded.hash()? != hash
            || object_id.as_ref().is_some_and(|id| id != &header.object_id)
        {
            return Err(Code::Invalid);
        }
        let key = self
            .admission
            .authorize(principal, scope, Access::Append(c))?;
        crypto::verify_signature(&key, &decoded.signature_input()?, decoded.signature())?;
        let ns = namespace(&c.namespace)?;
        let previous = c.prev_hash;
        let receipt = wire::output(
            scope,
            "receipt",
            serde_json::json!({"streamId":stream,"seq":seq,"envelopeHash":envelope_hash}),
        )?;
        let (envelope, transfer) = delivery(scope, hash, bytes.clone())?;
        let broadcast = wire::output(
            scope,
            "broadcast",
            serde_json::json!({"streamId":stream,"seq":seq,"envelopeHash":envelope_hash,"envelope":envelope}),
        )?;
        let accepted = self.store.append(&store::Envelope {
            scope: StreamScope {
                page: &scope.page,
                epoch: values::decimal(&scope.epoch, false)?,
                stream: principal,
            },
            namespace: ns,
            seq: sequence,
            hash,
            previous,
            bytes: &bytes,
        })?;
        self.peers.get_mut(&id).ok_or(Code::Denied)?.push(receipt);
        if accepted == Accepted::New {
            self.fanout(scope, broadcast, transfer);
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
    objects: Option<crate::object_channel::PeerObjects>,
    /// When the next keepalive Ping is due; armed by the first poll.
    ping_due: Option<Instant>,
}
impl<S: Read + Write, A: Admission> Connection<S, A> {
    pub(crate) fn attach_objects(&mut self, objects: Option<crate::object_channel::PeerObjects>) {
        self.objects = objects;
    }
    fn object_request(
        &self,
        scope: &SyncScope,
        id: String,
        request: crate::object_channel::ObjectRequest,
        now: Instant,
    ) -> Result<(), Code> {
        {
            let mut state = self.server.0.lock().map_err(|_| Code::Denied)?;
            state.recheck();
            let peer = state.peers.get(&self.id).ok_or(Code::Denied)?;
            if !peer.hello || peer.scope.as_ref() != Some(scope) || peer.terminal.is_some() {
                return Err(Code::Denied);
            }
            state
                .admission
                .authorize(&peer.principal, scope, Access::Read)?;
        }
        if let Some(objects) = &self.objects {
            match objects.submit(
                scope.clone(),
                id.clone(),
                request,
                now + limits::OBJECT_REPLY,
            ) {
                Ok(()) => return Ok(()),
                Err(Code::Capacity | Code::Denied) => {}
                Err(code) => return Err(code),
            }
        }
        values::generated_id(&id)?;
        let text = wire::output(
            scope,
            "object-result",
            serde_json::json!({"requestId":id,"result":{"error":{"code":"unavailable"}}}),
        )?;
        let mut state = self.server.0.lock().map_err(|_| Code::Denied)?;
        state
            .peers
            .get_mut(&self.id)
            .ok_or(Code::Denied)?
            .push(text);
        Ok(())
    }
    /// A save's native preparation can take seconds: it runs here, holding no lock, so other
    /// connections keep appending. The commit takes the lock again and every fence rechecks, so
    /// an edit that landed meanwhile turns this save into a stale-base reply.
    fn run_save(&self, job: SaveJob) -> Result<(), Code> {
        let clock = || crate::registration::now_ms().map_err(|_| Code::Denied);
        let prepared = crate::page::save::prepare(&job.open, &job.save, &|| {
            clock().map_err(|_| crate::page::Fault::Unavailable.into())
        });
        let now = clock();
        let mut state = self.server.0.lock().map_err(|_| Code::Denied)?;
        match now {
            Ok(now) => state.finish_save(self.id, job, prepared, now),
            Err(code) => {
                state.unsave(&job);
                Err(code)
            }
        }
    }
    fn close(&mut self, code: Code) -> Progress {
        self.objects.take();
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
        self.poll_at(Instant::now())
    }
    /// The caller supplies a monotonic clock instant for deadlines and tests.
    pub fn poll_at(&mut self, now: Instant) -> Progress {
        if self.socket.is_none() {
            return Progress::Closed;
        }
        let terminal = match self.server.0.lock() {
            Ok(mut state) => {
                state.recheck();
                state.peers.get(&self.id).and_then(|p| {
                    p.terminal.or_else(|| {
                        p.incoming
                            .as_ref()
                            .filter(|i| {
                                now.saturating_duration_since(i.started) >= i.upload.bounds().2
                            })
                            .map(|_| Code::Invalid)
                    })
                })
            }
            Err(_) => Some(Code::Denied),
        };
        if let Some(code) = terminal {
            return self.close(code);
        }
        if self
            .blocked_since
            .is_some_and(|t| now.saturating_duration_since(t) >= limits::RESPONSE)
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
                    peer.buffered_slot = false;
                    Ok(true)
                }
                Err(e) if would_block(&e) => {
                    self.blocked_since.get_or_insert(now);
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
        // An idle tunnel is torn down after limits::TUNNEL_IDLE by the remote door and by the
        // serve loop, and a browser cannot send WebSocket pings. A Ping every KEEPALIVE moves
        // bytes both ways (the browser answers with a Pong), so a quiet tab stays connected.
        if now >= *self.ping_due.get_or_insert(now + limits::KEEPALIVE) {
            self.ping_due = Some(now + limits::KEEPALIVE);
            match self
                .socket
                .as_mut()
                .expect("checked socket")
                .send(Message::Ping(Vec::new().into()))
            {
                Ok(()) => advanced = true,
                // tungstenite keeps the exact buffered frame; the next flush completes it.
                Err(e) if would_block(&e) => {
                    self.blocked_since.get_or_insert(now);
                }
                Err(_) => return self.close(Code::ResyncRequired),
            }
        }
        match self.socket.as_mut().expect("checked socket").read() {
            Ok(Message::Text(text)) => {
                advanced = true;
                let Ok(frame) = serde_json::from_str::<Frame>(&text) else {
                    return self.close(Code::Invalid);
                };
                let Ok(scope) = frame.scope() else {
                    return self.close(Code::Invalid);
                };
                let result = match frame {
                    Frame::Object {
                        request_id,
                        request,
                        ..
                    } => self
                        .object_request(&scope, request_id, *request, now)
                        .map(|()| None),
                    frame => self
                        .server
                        .0
                        .lock()
                        .map_err(|_| Code::Denied)
                        .and_then(|mut state| state.process(self.id, frame, now)),
                };
                let result = match result {
                    Ok(Some(job)) => self.run_save(job),
                    Ok(None) => Ok(()),
                    Err(code) => Err(code),
                };
                if let Err(code) = result {
                    let message = wire::output(&scope, "error", serde_json::json!({"code":code}));
                    if let (Ok(text), Ok(mut state)) = (message, self.server.0.lock())
                        && let Some(peer) = state.peers.get_mut(&self.id)
                    {
                        peer.incoming = None;
                        peer.catchup = None;
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
        if let Some(objects) = &self.objects {
            while let Some(reply) = objects.take_reply() {
                if let Ok(mut state) = self.server.0.lock() {
                    state.recheck();
                    if let Some(peer) = state.peers.get_mut(&self.id)
                        && peer.terminal.is_none()
                        && peer.scope.as_ref() == Some(&reply.scope)
                    {
                        // A bounded projection failure still settles the
                        // correlation. Never silently discard a possible effect.
                        let fallback = serde_json::json!({"requestId":reply.id,"result":{"error":{"code":if reply.fence.as_ref().is_some_and(|fence| fence.mutation) {"unknown"} else {"unavailable"}}}});
                        let text = match wire::output(
                            &reply.scope,
                            "object-result",
                            serde_json::json!({"requestId":reply.id,"result":reply.value}),
                        )
                        .or_else(|_| wire::output(&reply.scope, "object-result", fallback))
                        {
                            Ok(text) => text,
                            Err(_) => {
                                peer.end(Code::Capacity);
                                continue;
                            }
                        };
                        if let Some(fence) = reply.fence {
                            if let Ok(refused) = wire::output(
                                &reply.scope,
                                "object-result",
                                serde_json::json!({"requestId":reply.id,"result":{"error":{"code":if fence.mutation { "unknown" } else { "unavailable" }}}}),
                            ) {
                                peer.enqueue(Delivery::Object {
                                    text,
                                    refused,
                                    fence,
                                });
                            }
                        } else {
                            peer.push(text);
                        }
                    }
                }
            }
        }
        // Hold the authority lock through the nonblocking write: revocation
        // cannot interleave between dequeue and disclosure to the socket.
        let outgoing = (|| -> Result<bool, Code> {
            let mut state = self.server.0.lock().map_err(|_| Code::Denied)?;
            state.recheck();
            if let Err(code) = state.catchup_page(self.id) {
                let peer = state.peers.get_mut(&self.id).ok_or(Code::Denied)?;
                peer.catchup = None;
                peer.subscribed = false;
                peer.push(wire::output(
                    peer.scope.as_ref().ok_or(Code::Denied)?,
                    "error",
                    serde_json::json!({"code":code}),
                )?);
            }
            let peer = state.peers.get_mut(&self.id).ok_or(Code::Denied)?;
            if let Some(code) = peer.terminal {
                return Err(code);
            }
            if peer.hello && peer.in_flight == limits::SEND_QUEUE_FRAMES {
                return Ok(false);
            }
            let Some(mut delivery) = peer.queue.pop_front() else {
                return Ok(false);
            };
            let (text, done) = delivery.next()?;
            if peer.hello {
                peer.in_flight += 1;
            }
            if !done {
                peer.queue.push_front(delivery);
            }
            // An unfinished transfer already reserves its buffered chunk through
            // the continuation entry; only a completed delivery needs another slot.
            peer.buffered_slot = done;
            match self
                .socket
                .as_mut()
                .expect("checked socket")
                .send(Message::Text(text.into()))
            {
                Ok(()) => peer.buffered_slot = false,
                Err(e) if would_block(&e) => {
                    self.blocked_since.get_or_insert(now);
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
        self.objects.take();
        if let Ok(mut state) = self.server.0.lock() {
            state.peers.remove(&self.id);
        }
    }
}
fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock)
}
