//! Door sessions (`session.open`). A paired device opens one session per run
//! with a signed control envelope; a `browser` device on this door's origin
//! also receives the door cookie, of which serve keeps only the SHA-256.
//! Sessions live in serve's memory: they end on stop, revocation, a newer
//! session for the same device or idle expiry, and reopening is a silent
//! signed `session.open` from the device key.
use crate::{
    admission::{self, BindingAction, MessagePermit, MessageRefusal},
    canonical::{self, Envelope},
    crypto,
    error::RemoteError,
    mount::{Admitted, DeviceContext, IdleClock, SessionState, Sessions},
    pairing::now_ms,
    state::MachineKey,
    store::{Grant, Store, uuid_v4},
    wire::SignedMessage,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Cookie name; the value is the base64url of a 256-bit random token.
pub const COOKIE: &str = "tmt_door";
/// Accepted distance between a control's timestamp and machine time.
pub const CLOCK_SKEW: Duration = Duration::from_secs(60);
/// A session unused for this long ends; the page reopens it silently.
pub const IDLE: Duration = Duration::from_secs(12 * 60 * 60);
/// Bound on remembered `session.open` nonces; beyond it opens refuse.
const NONCES: usize = 4096;

pub struct DoorSessions {
    machine_id: String,
    window_id: String,
    door_origin: String,
    /// `<prefix>/x/`: the cookie reaches mounted pages only.
    cookie_path: String,
    machine_key: MachineKey,
    store: Arc<Mutex<Store>>,
    idle: Duration,
    clock: IdleClock,
    live: Mutex<Live>,
    ready: Condvar,
}
#[derive(Default)]
struct Live {
    generation: u64,
    stopped: bool,
    by_client: HashMap<String, Session>,
    /// Cookie token SHA-256 to client ID.
    by_token: HashMap<[u8; 32], String>,
    /// `(clientId, clientNonce)` to the time it may be forgotten.
    nonces: HashMap<(String, String), u64>,
}
struct Session {
    id: String,
    busy: Arc<AtomicBool>,
    token: Option<[u8; 32]>,
    grant_revision: u64,
    state: Arc<SessionState>,
}
/// A machine-signed `session.open` response, and the `Set-Cookie` value for a
/// browser device on this door.
pub struct Opened {
    pub response: Vec<u8>,
    pub cookie: Option<String>,
}
/// The admitted parts of a `session.open` control.
struct Control {
    message: SignedMessage,
    grant: Grant,
    nonce: String,
}
impl DoorSessions {
    pub fn new(
        machine_id: String,
        window_id: String,
        door_origin: String,
        cookie_path: String,
        machine_key: MachineKey,
        store: Arc<Mutex<Store>>,
        idle: Duration,
    ) -> Self {
        Self {
            machine_id,
            window_id,
            door_origin,
            cookie_path,
            machine_key,
            store,
            idle,
            clock: Arc::new(Instant::now),
            live: Mutex::default(),
            ready: Condvar::new(),
        }
    }
    /// Integration-test seam for advancing session idle time deterministically.
    /// Set before opening sessions. The clock must be monotonic; wall-clock
    /// admission checks are unchanged.
    pub fn with_clock(mut self, clock: IdleClock) -> Self {
        self.clock = clock;
        self
    }
    /// Admit one `session.open` control from `/append`. Every refusal is
    /// `None`, so the route answers with the generic pre-auth 404.
    pub fn open(&self, request_origin: Option<&str>, body: &[u8]) -> Option<Opened> {
        let now = now_ms().ok()?;
        let control = self.control(request_origin, body, now)?;
        self.store
            .lock()
            .ok()?
            .charge_call(&control.grant.client_id, now)
            .ok()?;
        let token = (control.grant.kind == "browser")
            .then(random::<32>)
            .transpose()
            .ok()?;
        let session_id = uuid_v4().ok()?;
        let payload = json!({"sessionId":session_id,"serverTimeMs":now,"grantRevision":control.grant.revision,"expiresAtMs":control.grant.expires_at_ms});
        let response = self
            .signed_response(control.message.envelope(), &session_id, 1, &payload, now)
            .ok()?;
        {
            let mut live = self.live.lock().ok()?;
            if live.stopped {
                return None;
            }
            let validity = 2 * CLOCK_SKEW.as_millis() as u64;
            live.nonces.retain(|_, forget| *forget > now);
            let key = (control.grant.client_id.clone(), control.nonce.clone());
            if live.nonces.contains_key(&key) || live.nonces.len() >= NONCES {
                return None;
            }
            self.store
                .lock()
                .ok()?
                .start_session(&control.grant, &session_id, &self.window_id, now)
                .ok()?;
            live.nonces.insert(key, now + validity);
            // One session per device: a newer one ends the previous session
            // and its tunnels without widening scope.
            remove(&mut live, &control.grant.client_id);
            let hash = token.map(|t| <[u8; 32]>::from(Sha256::digest(t)));
            if let Some(hash) = hash {
                live.by_token.insert(hash, control.grant.client_id.clone());
            }
            live.by_client.insert(
                control.grant.client_id.clone(),
                Session {
                    id: session_id.clone(),
                    busy: Arc::new(AtomicBool::new(false)),
                    token: hash,
                    grant_revision: control.grant.revision,
                    state: Arc::new(SessionState::with_clock(Arc::clone(&self.clock))),
                },
            );
        }
        self.changed();
        Some(Opened {
            response,
            cookie: token.map(|token| {
                format!(
                    "{COOKIE}={}; Path={}; HttpOnly; SameSite=Strict",
                    canonical::base64url(&token),
                    self.cookie_path
                )
            }),
        })
    }

    /// End the device's session and close its tunnels.
    pub fn end_device(&self, client_id: &str) {
        if let Ok(mut live) = self.live.lock() {
            remove(&mut live, client_id);
            live.generation = live.generation.wrapping_add(1);
            self.ready.notify_all();
        }
    }
    /// Strict envelope admission against the live grant. The signature is
    /// checked last over the exact canonical bytes; nothing is recorded here.
    fn control(&self, request_origin: Option<&str>, body: &[u8], now: u64) -> Option<Control> {
        let message = SignedMessage::decode(body, crate::limits::PAIR_BODY_BYTES)?;
        let envelope = message.envelope();
        if envelope.kind != "control" || envelope.operation != "session.open" {
            return None;
        }
        let grant = self.authenticate(request_origin, &message, now)?;
        let nonce = client_nonce(&message.input)?;
        Some(Control {
            message,
            grant,
            nonce,
        })
    }
}
impl DoorSessions {
    fn authenticate(
        &self,
        request_origin: Option<&str>,
        message: &SignedMessage,
        now: u64,
    ) -> Option<Grant> {
        let envelope = message.envelope();
        if envelope.machine_id != self.machine_id
            || envelope.window_id != self.window_id
            || now.abs_diff(envelope.timestamp_ms) > CLOCK_SKEW.as_millis() as u64
        {
            return None;
        }
        let grant = self.store.lock().ok()?.grant(envelope.client_id).ok()??;
        if !grant.live_at(now) || envelope.origin != grant.origin {
            return None;
        }
        let origin_matches = match grant.kind.as_str() {
            "cli" => request_origin.is_none() && envelope.origin == "cli",
            "browser" => {
                envelope.origin == self.door_origin && request_origin == Some(envelope.origin)
            }
            "addon" => request_origin == Some(envelope.origin),
            _ => false,
        };
        if !origin_matches {
            return None;
        }
        crypto::verify_signature(
            &grant.public_key,
            &canonical::envelope(&envelope).ok()?,
            &message.signature,
        )
        .ok()?;
        Some(grant)
    }
    /// Admit one signed message for the journal/application owner; bindings
    /// never obtain authority to invoke core themselves.
    pub fn admit(
        self: &Arc<Self>,
        action: BindingAction,
        request_origin: Option<&str>,
        bytes: &[u8],
        payload_limit: usize,
    ) -> Result<MessagePermit, MessageRefusal> {
        let message =
            SignedMessage::decode(bytes, payload_limit).ok_or(MessageRefusal::Unauthenticated)?;
        let now = now_ms().map_err(|_| MessageRefusal::Unauthenticated)?;
        let grant = self
            .authenticate(request_origin, &message, now)
            .ok_or(MessageRefusal::Unauthenticated)?;
        let (busy, state) = {
            let mut live = self
                .live
                .lock()
                .map_err(|_| MessageRefusal::Unauthenticated)?;
            let session = live
                .by_client
                .get(&grant.client_id)
                .ok_or(MessageRefusal::Unauthenticated)?;
            if live.stopped
                || session.id != message.envelope().session_id
                || session.grant_revision != grant.revision
            {
                return Err(MessageRefusal::Unauthenticated);
            }
            if session.state.idle() >= self.idle {
                remove(&mut live, &grant.client_id);
                return Err(MessageRefusal::Unauthenticated);
            }
            (Arc::clone(&session.busy), Arc::clone(&session.state))
        };
        let charge = {
            self.store
                .lock()
                .map_err(|_| MessageRefusal::Unauthenticated)?
                .charge_call(&grant.client_id, now)
        };
        charge.map_err(|error| {
            admission::refusal(
                self,
                &message,
                &error.code,
                "Authenticated call budget exhausted.",
            )
        })?;
        let deny = |code, text| admission::refusal(self, &message, code, text);
        if !action.matches(&message) {
            return Err(deny(
                "REMOTE_INPUT_INVALID",
                "Route and signed operation disagree.",
            ));
        }
        let Some(scope) = admission::scope(message.envelope().operation) else {
            return Err(deny(
                "REMOTE_INPUT_INVALID",
                "Operation is not remotely callable.",
            ));
        };
        if scope.is_some_and(|scope| !grant.permits_scope(scope)) {
            return Err(deny(
                "REMOTE_SCOPE_DENIED",
                "Device scope does not admit this operation.",
            ));
        }
        if busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(deny("REMOTE_REPLAY", "A message is already in flight."));
        }
        let consume = self
            .store
            .lock()
            .map_err(|_| {
                RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote state is unavailable.")
            })
            .and_then(|mut store| {
                let envelope = message.envelope();
                store.consume_sequence(
                    envelope.client_id,
                    envelope.session_id,
                    envelope.window_id,
                    envelope.sequence.parse().expect("canonical sequence"),
                    now,
                )
            });
        if let Err(error) = consume {
            busy.store(false, Ordering::Release);
            return Err(deny(&error.code, "Message sequence could not be admitted."));
        }
        let authority = grant.authority().map_err(|_| {
            busy.store(false, Ordering::Release);
            MessageRefusal::Unauthenticated
        })?;
        state.touch();
        Ok(MessagePermit {
            message,
            grant,
            authority,
            sessions: Arc::clone(self),
            busy,
        })
    }
    pub(crate) fn revalidate(
        &self,
        expected: &Grant,
        message: &SignedMessage,
        now: u64,
    ) -> Result<(), RemoteError> {
        self.with_store(expected, message, now, |_| Ok(()))
            .map(|_| ())
    }
    /// Hold the live-session and Store owners through the bounded durable fence.
    pub(crate) fn with_store<T>(
        &self,
        expected: &Grant,
        message: &SignedMessage,
        now: u64,
        action: impl FnOnce(&mut Store) -> Result<T, RemoteError>,
    ) -> Result<(T, u64), RemoteError> {
        let live = self.live.lock().map_err(|_| {
            RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote session unavailable.")
        })?;
        if live.stopped
            || !live
                .by_client
                .get(&expected.client_id)
                .is_some_and(|session| {
                    session.id == message.envelope().session_id && session.state.idle() < self.idle
                })
        {
            return Err(RemoteError::new("REMOTE_CLOSED", "Device session ended."));
        }
        let mut store = self.store.lock().map_err(|_| {
            RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote state unavailable.")
        })?;
        if !store
            .grant(&expected.client_id)?
            .is_some_and(|grant| grant.revision == expected.revision && grant.live_at(now))
        {
            return Err(RemoteError::new("REMOTE_CLOSED", "Device authority ended."));
        }
        Ok((action(&mut store)?, live.generation))
    }
    pub(crate) fn changed(&self) {
        if let Ok(mut live) = self.live.lock() {
            live.generation = live.generation.wrapping_add(1);
            self.ready.notify_all();
        }
    }
    pub(crate) fn wait(&self, generation: u64, deadline: Instant) -> Result<(), RemoteError> {
        let live = self.live.lock().map_err(|_| {
            RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote session unavailable.")
        })?;
        let (live, _) = self
            .ready
            .wait_timeout_while(
                live,
                deadline.saturating_duration_since(Instant::now()),
                |live| live.generation == generation && !live.stopped,
            )
            .map_err(|_| {
                RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote session unavailable.")
            })?;
        if live.stopped {
            return Err(RemoteError::new("REMOTE_CLOSED", "Door stopped."));
        }
        Ok(())
    }
    pub fn shutdown(&self) {
        if let Ok(mut live) = self.live.lock() {
            live.stopped = true;
            for (_, session) in live.by_client.drain() {
                session.state.end();
            }
            live.by_token.clear();
            self.ready.notify_all();
        }
    }
    /// Publish operation metadata under its original ID to the currently live session.
    pub(crate) fn operation_response(
        &self,
        grant: &Grant,
        id: &str,
        payload: &Value,
    ) -> Result<Option<Vec<u8>>, RemoteError> {
        let live = self.live.lock().map_err(|_| {
            RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote session unavailable.")
        })?;
        let Some(session) = live
            .by_client
            .get(&grant.client_id)
            .filter(|session| !live.stopped && session.state.idle() < self.idle)
        else {
            return Ok(None);
        };
        let now = now_ms()?;
        let sequence = self
            .store
            .lock()
            .map_err(|_| RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote state unavailable."))?
            .response_sequence(&grant.client_id, &session.id, &self.window_id)?;
        let input = Envelope {
            kind: "request",
            id,
            correlation_id: None,
            machine_id: &self.machine_id,
            window_id: &self.window_id,
            client_id: &grant.client_id,
            session_id: &session.id,
            sequence: "1",
            timestamp_ms: now,
            origin: &grant.origin,
            operation: "dispatch.create",
            payload: b"{}",
        };
        self.signed_response(input, &session.id, sequence, payload, now)
            .map(Some)
    }
    pub(crate) fn response(
        &self,
        request: &SignedMessage,
        payload: &Value,
    ) -> Result<Vec<u8>, RemoteError> {
        let input = request.envelope();
        let now = now_ms()?;
        let sequence = self
            .store
            .lock()
            .map_err(|_| {
                RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote state is unavailable.")
            })?
            .response_sequence(input.client_id, input.session_id, &self.window_id)?;
        {
            let session_id = input.session_id;
            self.signed_response(input, session_id, sequence, payload, now)
        }
    }
    fn signed_response(
        &self,
        input: Envelope<'_>,
        session_id: &str,
        sequence: u64,
        payload: &Value,
        now: u64,
    ) -> Result<Vec<u8>, RemoteError> {
        let id = uuid_v4()?;
        let sequence = sequence.to_string();
        let payload = serde_json::to_vec(payload).expect("JSON value");
        let envelope = Envelope {
            kind: "response",
            id: &id,
            correlation_id: Some(input.id),
            machine_id: &self.machine_id,
            window_id: &self.window_id,
            client_id: input.client_id,
            session_id,
            sequence: &sequence,
            timestamp_ms: now,
            origin: input.origin,
            operation: input.operation,
            payload: &payload,
        };
        let bytes = canonical::envelope(&envelope)
            .map_err(|_| RemoteError::new("REMOTE_INPUT_INVALID", "Invalid response envelope."))?;
        Ok(json!({"version":1,"profile":"local-v1","kind":"response","id":id,"correlationId":input.id,"machineId":self.machine_id,"windowId":self.window_id,"clientId":input.client_id,"sessionId":session_id,"sequence":sequence,"timestampMs":now,"origin":input.origin,"operation":input.operation,"payload":canonical::base64url(&payload),"signature":canonical::base64url(&self.machine_key.sign(&bytes))}).to_string().into_bytes())
    }
}
impl Sessions for DoorSessions {
    /// Every request and upgrade rechecks the grant: revoked, expired or
    /// re-revisioned grants and idle sessions resolve to no context.
    fn context(&self, cookie: Option<&str>) -> Option<Admitted> {
        let token = cookie_token(cookie?)?;
        let hash = <[u8; 32]>::from(Sha256::digest(token));
        let (client_id, revision, state) = {
            let mut live = self.live.lock().ok()?;
            let client_id = live.by_token.get(&hash)?.clone();
            let session = live.by_client.get(&client_id)?;
            if session.state.idle() >= self.idle {
                remove(&mut live, &client_id);
                return None;
            }
            (
                client_id,
                session.grant_revision,
                Arc::clone(&session.state),
            )
        };
        let grant = self.store.lock().ok()?.grant(&client_id).ok().flatten();
        let now = now_ms().ok()?;
        let Some(grant) = grant.filter(|grant| grant.live_at(now) && grant.revision == revision)
        else {
            self.end_device(&client_id);
            return None;
        };
        state.touch();
        Some(Admitted {
            context: DeviceContext {
                device_id: grant.client_id,
                kind: grant.kind,
                origin: grant.origin,
                name: grant.name,
                public_key: grant.public_key,
                grant_revision: grant.revision,
            },
            session: state,
        })
    }
}
fn remove(live: &mut Live, client_id: &str) {
    if let Some(session) = live.by_client.remove(client_id) {
        session.state.end();
        if let Some(hash) = session.token {
            live.by_token.remove(&hash);
        }
    }
}
/// Exactly `{"clientNonce":"<32 lowercase hex>"}`.
fn client_nonce(payload: &Value) -> Option<String> {
    let object = payload.as_object()?;
    let nonce = object.get("clientNonce")?.as_str()?;
    (object.len() == 1
        && nonce.len() == 32
        && nonce
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
    .then(|| nonce.to_owned())
}

/// The single `tmt_door` value in a `Cookie` header; duplicates resolve to none.
fn cookie_token(header: &str) -> Option<Vec<u8>> {
    let mut values = header
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(name, _)| *name == COOKIE);
    let (_, value) = values.next()?;
    if values.next().is_some() {
        return None;
    }
    canonical::base64url_bytes(value, 32).ok()
}
fn random<const N: usize>() -> Result<[u8; N], ()> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| ())?;
    Ok(bytes)
}
