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
    Append(&'a object::Context),
}
/// Caller-verified bootstrap metadata. The head comes from owner_head through
/// the admission implementation. None means this epoch has no reset baseline.
pub struct CatchupContext {
    pub membership_head: statement::Head,
    pub owner_key: [u8; 32],
    /// Exact model baseline descriptor JSON, not reconstructed signing bytes.
    pub baseline: Option<Vec<u8>>,
}
pub trait Admission: Send {
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
    positions: HashMap<(String, String), store::NamespaceCursor>,
    head: statement::Head,
    owner: [u8; 32],
    revision: u64,
    wrap_offset: usize,
    wraps_done: bool,
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
struct Incoming {
    stream: String,
    seq: String,
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
                hello: false,
                catchup: None,
                chains: Default::default(),
                in_flight: 0,
                incoming: None,
                buffered_slot: false,
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
    fn process(&mut self, id: u64, frame: Frame, now: Instant) -> Result<(), Code> {
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
                            stream: stream_id,
                            seq,
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
                if now.saturating_duration_since(incoming.started) >= limits::ACQUISITION
                    || object_id != incoming.object_id
                    || envelope_hash != incoming.hash
                    || index != incoming.next
                    || count == 0
                    || count > limits::UPDATE_BYTES.div_ceil(limits::CHUNK_BYTES)
                    || index >= count
                    || incoming.count.is_some_and(|old| old != count)
                {
                    return Err(Code::Invalid);
                }
                let chunk = values::binary(&bytes, limits::CHUNK_BYTES)?;
                if chunk.is_empty() || (index + 1 < count && chunk.len() != limits::CHUNK_BYTES) {
                    return Err(Code::Invalid);
                }
                if incoming.bytes.len() + chunk.len() > limits::UPDATE_BYTES {
                    return Err(Code::Capacity);
                }
                incoming.count = Some(count);
                incoming.next += 1;
                incoming.bytes.extend_from_slice(&chunk);
                if incoming.next == count {
                    self.append(
                        id,
                        &scope,
                        &principal,
                        AppendRequest {
                            stream: incoming.stream,
                            seq: incoming.seq,
                            hash: incoming.hash,
                            bytes: incoming.bytes,
                            object_id: Some(object_id),
                        },
                    )?;
                } else {
                    self.peers.get_mut(&id).ok_or(Code::Denied)?.incoming = Some(incoming);
                }
            }
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
            head: context.membership_head,
            owner: context.owner_key,
            revision: membership.revision,
            wrap_offset: 0,
            wraps_done: false,
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
                        epoch,
                        &peer.principal,
                        &catchup.head,
                        catchup.wrap_offset,
                    )
                })
                .map_err(bootstrap_error)?;
            let count = wraps.len();
            let complete = !more && self.store.namespaces(&scope.page, epoch)?.is_empty();
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
        for (stream, ns) in self.store.namespaces(&scope.page, epoch)? {
            let name = if ns == store::Namespace::Content {
                "content"
            } else {
                "own"
            };
            let key = (stream.clone(), name.to_owned());
            let cursor = positions.get(&key).copied().unwrap_or_default();
            if let Some(object) = self.store.namespace_next(
                StreamScope {
                    page: &scope.page,
                    epoch,
                    stream: &stream,
                },
                ns,
                cursor,
            )? {
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
                                now.saturating_duration_since(i.started) >= limits::ACQUISITION
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
                    .and_then(|mut state| state.process(self.id, frame, now));
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
        if let Ok(mut state) = self.server.0.lock() {
            state.peers.remove(&self.id);
        }
    }
}
fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock)
}
