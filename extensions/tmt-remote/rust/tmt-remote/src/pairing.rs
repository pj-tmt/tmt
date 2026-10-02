//! The pairing ceremony's offer state (`tmt-device-pair-v1`). Only serve's
//! control socket opens an offer; a device submits one enrollment candidate;
//! the owner's terminal confirmation creates the grant. The raw code lives
//! only in this process's memory and is erased on confirmation or close.
use crate::{
    canonical::{self, Enrollment},
    crypto,
    error::RemoteError,
    store::{DEFAULT_SCOPES, Grant, Store, uuid_v4},
};
use serde_json::{Map, Value, json};
use std::{
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Offer timing. Production uses [`Timing::CONTRACT`].
#[derive(Clone, Copy)]
pub struct Timing {
    /// Code lifetime: one use, at most ten minutes.
    pub lifetime: Duration,
    /// How long one `/pair` request waits for the owner before reporting pending.
    pub submit_wait: Duration,
}
impl Timing {
    pub const CONTRACT: Self = Self {
        lifetime: Duration::from_secs(600),
        submit_wait: Duration::from_secs(20),
    };
}
/// Failed code proofs that cancel an offer.
pub const MAX_FAILURES: u8 = 3;
const FIELDS: [&str; 12] = [
    "profile",
    "machineId",
    "windowId",
    "offerId",
    "serverChallenge",
    "clientNonce",
    "kind",
    "origin",
    "name",
    "publicKey",
    "mac",
    "signature",
];

/// What the control client shows for a new offer.
pub struct Offered {
    pub offer_id: String,
    pub server_challenge: [u8; 16],
    pub code: [u8; 16],
    pub expires_at_ms: u64,
}
/// Events delivered to the control client that opened the offer.
#[derive(Debug, PartialEq, Eq)]
pub enum PairingEvent {
    Candidate {
        kind: String,
        origin: String,
        name: String,
        words: [&'static str; 4],
    },
    Ended(End),
}
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    Paired { client_id: String },
    Refused,
    Expired,
    TooManyFailures,
    Replaced,
    Cancelled,
    Failed(String),
}
impl End {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Paired { .. } => "paired",
            Self::Refused => "refused",
            Self::Expired => "expired",
            Self::TooManyFailures => "too-many-failures",
            Self::Replaced => "replaced",
            Self::Cancelled => "cancelled",
            Self::Failed(_) => "failed",
        }
    }
}
/// Outcome of one enrollment submission. Refusals are generic on the wire.
#[derive(Debug, PartialEq, Eq)]
pub enum Submission {
    Receipt { receipt: Vec<u8>, proof: [u8; 32] },
    Pending,
    Refused,
}

#[derive(Clone, PartialEq, Eq)]
struct Candidate {
    enrollment: Vec<u8>,
    mac: [u8; 32],
    signature: [u8; 64],
    public_key: [u8; 32],
    kind: String,
    origin: String,
    name: String,
}
enum Phase {
    Open,
    Pinned(Candidate),
    /// Bounded lost-response recovery until the original deadline.
    Confirmed {
        candidate: Candidate,
        receipt: Vec<u8>,
        proof: [u8; 32],
    },
}
struct Offer {
    id: String,
    code: [u8; 16],
    challenge: [u8; 16],
    deadline: Instant,
    failures: u8,
    phase: Phase,
    events: mpsc::Sender<PairingEvent>,
}
impl Drop for Offer {
    fn drop(&mut self) {
        self.code.fill(0);
    }
}

pub struct Pairing {
    machine_id: String,
    window_id: String,
    machine_public: [u8; 32],
    store: Arc<Mutex<Store>>,
    timing: Timing,
    offer: Mutex<Option<Offer>>,
    changed: Condvar,
}
impl Pairing {
    pub fn new(
        machine_id: String,
        window_id: String,
        machine_public: [u8; 32],
        store: Arc<Mutex<Store>>,
        timing: Timing,
    ) -> Self {
        Self {
            machine_id,
            window_id,
            machine_public,
            store,
            timing,
            offer: Mutex::new(None),
            changed: Condvar::new(),
        }
    }
    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }
    pub fn window_id(&self) -> &str {
        &self.window_id
    }
    /// Open the single offer, explicitly cancelling any previous one.
    pub fn open(&self) -> Result<(Offered, mpsc::Receiver<PairingEvent>), RemoteError> {
        let mut code = [0; 16];
        let mut challenge = [0; 16];
        for buffer in [&mut code[..], &mut challenge[..]] {
            getrandom::fill(buffer)
                .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain entropy."))?;
        }
        let id = uuid_v4()?;
        let (events, receiver) = mpsc::channel();
        let expires_at_ms = now_ms()? + self.timing.lifetime.as_millis() as u64;
        let mut slot = self.lock();
        end(&mut slot, End::Replaced);
        *slot = Some(Offer {
            id: id.clone(),
            code,
            challenge,
            deadline: Instant::now() + self.timing.lifetime,
            failures: 0,
            phase: Phase::Open,
            events,
        });
        self.changed.notify_all();
        let offered = Offered {
            offer_id: id,
            server_challenge: challenge,
            code,
            expires_at_ms,
        };
        code.fill(0);
        Ok((offered, receiver))
    }
    /// End the offer at its deadline; call periodically and on every access.
    pub fn expire(&self) {
        let mut slot = self.lock();
        self.expire_locked(&mut slot);
    }
    /// Owner confirmation: create the grant and the receipt atomically.
    pub fn confirm(&self, offer_id: &str) {
        let mut slot = self.lock();
        self.expire_locked(&mut slot);
        let Some(offer) = slot.as_mut().filter(|o| o.id == offer_id) else {
            return;
        };
        let Phase::Pinned(candidate) = &offer.phase else {
            return;
        };
        let candidate = candidate.clone();
        match self.issue(offer, &candidate) {
            Ok((client_id, receipt, proof)) => {
                // Only the bounded recovery data stays; the code is erased.
                offer.code.fill(0);
                offer.phase = Phase::Confirmed {
                    candidate,
                    receipt,
                    proof,
                };
                let _ = offer
                    .events
                    .send(PairingEvent::Ended(End::Paired { client_id }));
                self.changed.notify_all();
            }
            Err(error) => end(&mut slot, End::Failed(error.message)),
        }
        self.changed.notify_all();
    }
    pub fn refuse(&self, offer_id: &str) {
        self.close(offer_id, End::Refused);
    }
    /// The control client went away or serve is stopping.
    pub fn cancel(&self, offer_id: &str) {
        self.close(offer_id, End::Cancelled);
    }
    pub fn shutdown(&self) {
        let mut slot = self.lock();
        end(&mut slot, End::Cancelled);
        self.changed.notify_all();
    }
    /// Admit one enrollment candidate from `/pair`. `origin` is the request's
    /// HTTP Origin header, which must match the proposed browser/add-on origin.
    pub fn submit(&self, origin: Option<&str>, body: &[u8]) -> Submission {
        let Some(wire) = parse(body) else {
            return Submission::Refused;
        };
        let mut slot = self.lock();
        self.expire_locked(&mut slot);
        let Some(offer) = slot.as_mut() else {
            return Submission::Refused;
        };
        let Some(candidate) = self.candidate(offer, &wire, origin) else {
            return Submission::Refused;
        };
        match &offer.phase {
            Phase::Confirmed {
                candidate: confirmed,
                receipt,
                proof,
            } => {
                // Exact retry recovers the receipt; nothing else is accepted.
                return if *confirmed == candidate && possession(&candidate) {
                    Submission::Receipt {
                        receipt: receipt.clone(),
                        proof: *proof,
                    }
                } else {
                    Submission::Refused
                };
            }
            Phase::Pinned(pinned) if *pinned != candidate => return Submission::Refused,
            _ => {}
        }
        if crypto::verify_mac(&offer.code, &candidate.enrollment, &candidate.mac).is_err() {
            offer.failures += 1;
            if offer.failures >= MAX_FAILURES {
                end(&mut slot, End::TooManyFailures);
                self.changed.notify_all();
            }
            return Submission::Refused;
        }
        if !possession(&candidate) {
            return Submission::Refused;
        }
        if matches!(offer.phase, Phase::Open) {
            let _ = offer.events.send(PairingEvent::Candidate {
                kind: candidate.kind.clone(),
                origin: candidate.origin.clone(),
                name: candidate.name.clone(),
                words: canonical::fingerprint_words(&candidate.public_key).unwrap_or(["?"; 4]),
            });
            offer.phase = Phase::Pinned(candidate.clone());
        }
        // Wait for the owner, bounded by this request's budget and the offer.
        let offer_id = offer.id.clone();
        let until = Instant::now() + self.timing.submit_wait;
        loop {
            let deadline = match slot.as_ref() {
                Some(offer) if offer.id == offer_id => {
                    if let Phase::Confirmed { receipt, proof, .. } = &offer.phase {
                        return Submission::Receipt {
                            receipt: receipt.clone(),
                            proof: *proof,
                        };
                    }
                    offer.deadline.min(until)
                }
                _ => return Submission::Refused,
            };
            let Some(wait) = deadline.checked_duration_since(Instant::now()) else {
                return Submission::Pending;
            };
            slot = match self.changed.wait_timeout(slot, wait) {
                Ok((guard, _)) => guard,
                Err(_) => return Submission::Refused,
            };
            self.expire_locked(&mut slot);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Offer>> {
        self.offer
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
    fn expire_locked(&self, slot: &mut Option<Offer>) {
        if slot.as_ref().is_some_and(|o| Instant::now() >= o.deadline) {
            end(slot, End::Expired);
            self.changed.notify_all();
        }
    }
    fn close(&self, offer_id: &str, reason: End) {
        let mut slot = self.lock();
        if slot.as_ref().is_some_and(|o| o.id == offer_id) {
            end(&mut slot, reason);
            self.changed.notify_all();
        }
    }
    fn candidate(
        &self,
        offer: &Offer,
        wire: &Map<String, Value>,
        request_origin: Option<&str>,
    ) -> Option<Candidate> {
        let text = |name: &str| wire.get(name)?.as_str();
        let hex16 = |name: &str| -> Option<[u8; 16]> {
            let value = text(name)?;
            if value.len() != 32
                || !value
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                return None;
            }
            let mut bytes = [0; 16];
            for (i, pair) in value.as_bytes().chunks(2).enumerate() {
                bytes[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
            }
            Some(bytes)
        };
        let binary =
            |name: &str, length: usize| canonical::base64url_bytes(text(name)?, length).ok();
        if text("profile")? != "local-v1"
            || text("machineId")? != self.machine_id
            || text("windowId")? != self.window_id
            || text("offerId")? != offer.id
            || hex16("serverChallenge")? != offer.challenge
        {
            return None;
        }
        let client_nonce = hex16("clientNonce")?;
        let kind = text("kind")?;
        let origin = text("origin")?;
        // A browser or add-on must send the exact proposed Origin; a CLI sends none.
        let origin_matches = match kind {
            "cli" => request_origin.is_none(),
            _ => request_origin == Some(origin),
        };
        if !origin_matches {
            return None;
        }
        let public_key: [u8; 32] = binary("publicKey", 32)?.try_into().ok()?;
        let mac: [u8; 32] = binary("mac", 32)?.try_into().ok()?;
        let signature: [u8; 64] = binary("signature", 64)?.try_into().ok()?;
        let name = text("name")?;
        let enrollment = canonical::enrollment(&Enrollment {
            machine_id: &self.machine_id,
            window_id: &self.window_id,
            offer_id: &offer.id,
            server_challenge: &offer.challenge,
            client_nonce: &client_nonce,
            kind,
            origin,
            name,
            public_key: &public_key,
        })
        .ok()?;
        Some(Candidate {
            enrollment,
            mac,
            signature,
            public_key,
            kind: kind.into(),
            origin: origin.into(),
            name: name.into(),
        })
    }
    /// Insert the default grant and derive the exact receipt and its proof.
    fn issue(
        &self,
        offer: &Offer,
        candidate: &Candidate,
    ) -> Result<(String, Vec<u8>, [u8; 32]), RemoteError> {
        let grant = Grant {
            client_id: uuid_v4()?,
            public_key: candidate.public_key,
            kind: candidate.kind.clone(),
            origin: candidate.origin.clone(),
            name: candidate.name.clone(),
            agents: "all".into(),
            scopes: DEFAULT_SCOPES.map(str::to_owned).to_vec(),
            mode: "direct".into(),
            issued_at_ms: now_ms()?,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        let receipt = receipt(&grant, &self.machine_id, &self.machine_public)?;
        let key = crypto::response_key(&offer.code, &candidate.enrollment)
            .map_err(|_| RemoteError::new("REMOTE_INPUT_INVALID", "Invalid enrollment."))?;
        let proof = crypto::server_proof(&key, &receipt)
            .map_err(|_| RemoteError::new("REMOTE_INPUT_INVALID", "Invalid receipt."))?;
        self.store
            .lock()
            .map_err(|_| {
                RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote state is unavailable.")
            })?
            .insert_grant(&grant)?;
        Ok((grant.client_id, receipt, proof))
    }
}
/// Exact receipt JSON bytes `{grant, machinePublicKey}`; the proof covers these bytes.
pub fn receipt(
    grant: &Grant,
    machine_id: &str,
    machine_public: &[u8; 32],
) -> Result<Vec<u8>, RemoteError> {
    serde_json::to_vec(&json!({
        "grant": {
            "clientId": grant.client_id,
            "machineId": machine_id,
            "profile": "local-v1",
            "publicKey": canonical::base64url(&grant.public_key),
            "kind": grant.kind,
            "origin": grant.origin,
            "name": grant.name,
            "agents": grant.agents,
            "scopes": grant.scopes,
            "mode": grant.mode,
            "issuedAtMs": grant.issued_at_ms,
            "expiresAtMs": grant.expires_at_ms,
            "revision": grant.revision,
            "disabled": grant.disabled,
        },
        "machinePublicKey": canonical::base64url(machine_public),
    }))
    .map_err(|_| RemoteError::new("REMOTE_IO", "Could not encode the receipt."))
}
/// Strict enrollment JSON: one object with exactly the contract's string fields.
fn parse(body: &[u8]) -> Option<Map<String, Value>> {
    let Value::Object(wire) = serde_json::from_slice(body).ok()? else {
        return None;
    };
    (wire.len() == FIELDS.len()
        && FIELDS
            .iter()
            .all(|field| wire.get(*field).is_some_and(Value::is_string)))
    .then_some(wire)
}
fn possession(candidate: &Candidate) -> bool {
    crypto::public_key(&candidate.public_key).is_ok()
        && canonical::possession(&candidate.enrollment, &candidate.mac).is_ok_and(|bytes| {
            crypto::verify_signature(&candidate.public_key, &bytes, &candidate.signature).is_ok()
        })
}
fn end(slot: &mut Option<Offer>, reason: End) {
    if let Some(offer) = slot.take() {
        // A confirmed offer already reported its outcome.
        if !matches!(offer.phase, Phase::Confirmed { .. }) {
            let _ = offer.events.send(PairingEvent::Ended(reason));
        }
    }
}
pub fn now_ms() -> Result<u64, RemoteError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .ok_or_else(|| RemoteError::new("REMOTE_CLOCK", "System clock is before 1970."))
}
