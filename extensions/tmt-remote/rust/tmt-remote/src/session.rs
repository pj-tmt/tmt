//! Door sessions (`session.open`). A paired device opens one session per run
//! with a signed control envelope; a `browser` device on this door's origin
//! also receives the door cookie, of which serve keeps only the SHA-256.
//! Sessions live in serve's memory: they end on stop, revocation, a newer
//! session for the same device or idle expiry, and reopening is a silent
//! signed `session.open` from the device key.
use crate::{
    canonical::{self, Envelope},
    crypto,
    mount::{Admitted, DeviceContext, SessionState, Sessions},
    pairing::now_ms,
    state::MachineKey,
    store::{Grant, Store, uuid_v4},
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Cookie name; the value is the base64url of a 256-bit random token.
pub const COOKIE: &str = "tmt_door";
/// Accepted distance between a control's timestamp and machine time.
pub const CLOCK_SKEW: Duration = Duration::from_secs(60);
/// A session unused for this long ends; the page reopens it silently.
pub const IDLE: Duration = Duration::from_secs(12 * 60 * 60);
/// Bound on remembered `session.open` nonces; beyond it opens refuse.
const NONCES: usize = 4096;
const FIELDS: [&str; 15] = [
    "version",
    "profile",
    "kind",
    "id",
    "correlationId",
    "machineId",
    "windowId",
    "clientId",
    "sessionId",
    "sequence",
    "timestampMs",
    "origin",
    "operation",
    "payload",
    "signature",
];

pub struct DoorSessions {
    machine_id: String,
    window_id: String,
    door_origin: String,
    machine_key: MachineKey,
    store: Arc<Mutex<Store>>,
    idle: Duration,
    live: Mutex<Live>,
}
#[derive(Default)]
struct Live {
    by_client: HashMap<String, Session>,
    /// Cookie token SHA-256 to client ID.
    by_token: HashMap<[u8; 32], String>,
    /// `(clientId, clientNonce)` to the time it may be forgotten.
    nonces: HashMap<(String, String), u64>,
}
struct Session {
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
    id: String,
    grant: Grant,
    nonce: String,
}
impl DoorSessions {
    pub fn new(
        machine_id: String,
        window_id: String,
        door_origin: String,
        machine_key: MachineKey,
        store: Arc<Mutex<Store>>,
        idle: Duration,
    ) -> Self {
        Self {
            machine_id,
            window_id,
            door_origin,
            machine_key,
            store,
            idle,
            live: Mutex::default(),
        }
    }
    /// Admit one `session.open` control from `/append`. Every refusal is
    /// `None`, so the route answers with the generic pre-auth 404.
    pub fn open(&self, request_origin: Option<&str>, body: &[u8]) -> Option<Opened> {
        let now = now_ms().ok()?;
        let control = self.control(request_origin, body, now)?;
        let token = (control.grant.kind == "browser")
            .then(random::<32>)
            .transpose()
            .ok()?;
        let session_id = uuid_v4().ok()?;
        {
            let mut live = self.live.lock().ok()?;
            let validity = 2 * CLOCK_SKEW.as_millis() as u64;
            live.nonces.retain(|_, forget| *forget > now);
            let key = (control.grant.client_id.clone(), control.nonce.clone());
            if live.nonces.contains_key(&key) || live.nonces.len() >= NONCES {
                return None;
            }
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
                    token: hash,
                    grant_revision: control.grant.revision,
                    state: Arc::default(),
                },
            );
        }
        let payload = json!({
            "sessionId": session_id,
            "serverTimeMs": now,
            "grantRevision": control.grant.revision,
            "expiresAtMs": control.grant.expires_at_ms,
        })
        .to_string()
        .into_bytes();
        let id = uuid_v4().ok()?;
        let envelope = Envelope {
            kind: "response",
            id: &id,
            correlation_id: Some(&control.id),
            machine_id: &self.machine_id,
            window_id: &self.window_id,
            client_id: &control.grant.client_id,
            session_id: &session_id,
            sequence: "1",
            timestamp_ms: now,
            origin: &control.grant.origin,
            operation: "session.open",
            payload: &payload,
        };
        let signature = self.machine_key.sign(&canonical::envelope(&envelope).ok()?);
        let response = json!({
            "version": 1,
            "profile": "local-v1",
            "kind": "response",
            "id": id,
            "correlationId": control.id,
            "machineId": self.machine_id,
            "windowId": self.window_id,
            "clientId": control.grant.client_id,
            "sessionId": session_id,
            "sequence": "1",
            "timestampMs": now,
            "origin": control.grant.origin,
            "operation": "session.open",
            "payload": canonical::base64url(&payload),
            "signature": canonical::base64url(&signature),
        });
        Some(Opened {
            response: response.to_string().into_bytes(),
            cookie: token.map(|t| {
                format!(
                    "{COOKIE}={}; Path=/x/; HttpOnly; SameSite=Strict",
                    canonical::base64url(&t)
                )
            }),
        })
    }
    /// End the device's session and close its tunnels.
    pub fn end_device(&self, client_id: &str) {
        if let Ok(mut live) = self.live.lock() {
            remove(&mut live, client_id);
        }
    }
    /// Strict envelope admission against the live grant. The signature is
    /// checked last over the exact canonical bytes; nothing is recorded here.
    fn control(&self, request_origin: Option<&str>, body: &[u8], now: u64) -> Option<Control> {
        let Value::Object(wire) = serde_json::from_slice(body).ok()? else {
            return None;
        };
        if wire.len() != FIELDS.len() || !FIELDS.iter().all(|f| wire.contains_key(*f)) {
            return None;
        }
        let text = |name: &str| wire.get(name)?.as_str();
        let timestamp = wire.get("timestampMs")?.as_u64()?;
        if wire.get("version")?.as_u64()? != 1
            || !wire.get("correlationId")?.is_null()
            || text("profile")? != "local-v1"
            || text("kind")? != "control"
            || text("operation")? != "session.open"
            || text("machineId")? != self.machine_id
            || text("windowId")? != self.window_id
            || text("sessionId")? != "new"
            || text("sequence")? != "0"
            || now.abs_diff(timestamp) > CLOCK_SKEW.as_millis() as u64
        {
            return None;
        }
        let grant = self.store.lock().ok()?.grant(text("clientId")?).ok()??;
        let origin = text("origin")?;
        if !live(&grant, now) || origin != grant.origin {
            return None;
        }
        // A browser or add-on sends its exact Origin; a CLI sends none. A
        // browser grant is bound to this door's own origin.
        let origin_matches = match grant.kind.as_str() {
            "cli" => request_origin.is_none(),
            "browser" => origin == self.door_origin && request_origin == Some(origin),
            _ => request_origin == Some(origin),
        };
        if !origin_matches {
            return None;
        }
        let payload = canonical::base64url_decode(text("payload")?).ok()?;
        let nonce = client_nonce(&payload)?;
        let signature = canonical::base64url_bytes(text("signature")?, 64).ok()?;
        let id = text("id")?;
        let signed = canonical::envelope(&Envelope {
            kind: "control",
            id,
            correlation_id: None,
            machine_id: &self.machine_id,
            window_id: &self.window_id,
            client_id: &grant.client_id,
            session_id: "new",
            sequence: "0",
            timestamp_ms: timestamp,
            origin,
            operation: "session.open",
            payload: &payload,
        })
        .ok()?;
        crypto::verify_signature(&grant.public_key, &signed, &signature).ok()?;
        Some(Control {
            id: id.to_owned(),
            grant,
            nonce,
        })
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
        let Some(grant) = grant.filter(|g| live(g, now) && g.revision == revision) else {
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
fn live(grant: &Grant, now: u64) -> bool {
    !grant.disabled && grant.expires_at_ms.is_none_or(|expiry| expiry > now)
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
fn client_nonce(payload: &[u8]) -> Option<String> {
    let Value::Object(object) = serde_json::from_slice::<Value>(payload).ok()? else {
        return None;
    };
    let object: Map<String, Value> = object;
    let nonce = object.get("clientNonce")?.as_str()?;
    (object.len() == 1
        && nonce.len() == 32
        && nonce
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
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
