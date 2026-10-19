//! The pairing ceremony's offer state (`tmt-device-pair-v1`). Only serve's
//! control socket opens an offer; a device submits one enrollment candidate;
//! manual offers await terminal confirmation; native-open offers admit the first
//! proved browser with the read-only default. Raw codes remain in memory only.
use crate::{
    canonical::{self, Enrollment},
    crypto,
    error::RemoteError,
    store::{DEFAULT_SCOPES, Grant, SUPPORTED_SCOPES, Store, uuid_v4},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Policy supplied only by the local owner's confirmation, never enrollment.
/// Sending is an explicit opt-in; the default policy is read-only.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PairingPolicy {
    pub talk: bool,
    pub agents: Option<Vec<String>>,
    pub hold: bool,
}
impl PairingPolicy {
    pub fn validate(&self) -> Result<(), RemoteError> {
        if ((!self.talk) && (self.agents.is_some() || self.hold))
            || self.agents.as_ref().is_some_and(|ids| {
                ids.is_empty()
                    || ids.len() > 256
                    || ids.iter().any(|id| !canonical::is_core_id(id))
                    || ids.windows(2).any(|pair| pair[0] >= pair[1])
            })
        {
            return Err(RemoteError::new(
                "REMOTE_INPUT_INVALID",
                "--agents and --hold require --talk; agents must be distinct canonical UUIDs.",
            ));
        }
        Ok(())
    }
}
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
/// Native browser opens authorize at most one enrollment during this window.
pub const AUTO_LIFETIME: Duration = Duration::from_secs(60);
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
    Cancelled,
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
    /// Only the owner-only native opener can select automatic confirmation.
    automatic: Option<String>,
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
    /// This door's exact origin, the only origin a `browser` device may bind.
    door_origin: String,
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
        door_origin: String,
        store: Arc<Mutex<Store>>,
        timing: Timing,
    ) -> Self {
        Self {
            machine_id,
            window_id,
            machine_public,
            door_origin,
            store,
            timing,
            offer: Mutex::new(None),
            changed: Condvar::new(),
        }
    }
    /// Public current-offer data only; no code, device inventory or authority.
    pub fn descriptor(&self, address: &str) -> Option<Value> {
        let mut slot = self.lock();
        self.expire_locked(&mut slot);
        let offer = slot.as_ref()?;
        if matches!(offer.phase, Phase::Confirmed { .. }) {
            return None;
        }
        let mut descriptor = self.descriptor_values(&offer.id, &offer.challenge, address);
        if let Some(target) = &offer.automatic {
            descriptor["automatic"] = json!(true);
            descriptor["target"] = json!(target);
        }
        Some(descriptor)
    }
    /// Snapshot for the control client that opened this exact offer, even if replaced.
    pub fn offered_descriptor(&self, offered: &Offered, address: &str) -> Value {
        self.descriptor_values(&offered.offer_id, &offered.server_challenge, address)
    }
    fn descriptor_values(&self, id: &str, challenge: &[u8; 16], address: &str) -> Value {
        json!({
            "profile": "local-v1", "binding": "loopback-http",
            "machineId": self.machine_id, "windowId": self.window_id,
            "offerId": id, "address": address,
            "serverChallenge": crate::state::hex(challenge),
        })
    }
    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }
    pub fn window_id(&self) -> &str {
        &self.window_id
    }
    /// Hold the offer owner briefly while lifecycle admission checks the other owners.
    pub(crate) fn while_idle(
        &self,
        deadline: std::time::Instant,
        action: impl FnOnce() -> Result<bool, RemoteError>,
    ) -> Result<bool, RemoteError> {
        let Some(mut slot) = crate::control::idle_lock(&self.offer, deadline)? else {
            return Ok(false);
        };
        self.expire_locked(&mut slot);
        if slot
            .as_ref()
            .is_some_and(|offer| !matches!(offer.phase, Phase::Confirmed { .. }))
        {
            return Ok(false);
        }
        action()
    }
    /// Open the single offer, explicitly cancelling any previous one.
    pub fn open(&self) -> Result<(Offered, mpsc::Receiver<PairingEvent>), RemoteError> {
        self.open_mode(None)
    }
    pub(crate) fn open_automatic(&self, target: String) -> Result<Offered, RemoteError> {
        self.open_mode(Some(target)).map(|(offer, _events)| offer)
    }
    fn open_mode(
        &self,
        automatic: Option<String>,
    ) -> Result<(Offered, mpsc::Receiver<PairingEvent>), RemoteError> {
        let mut code = [0; 16];
        let mut challenge = [0; 16];
        for buffer in [&mut code[..], &mut challenge[..]] {
            getrandom::fill(buffer)
                .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain entropy."))?;
        }
        let id = uuid_v4()?;
        let (events, receiver) = mpsc::channel();
        let lifetime = if automatic.is_some() {
            AUTO_LIFETIME
        } else {
            self.timing.lifetime
        };
        let expires_at_ms = now_ms()? + lifetime.as_millis() as u64;
        let mut slot = self.lock();
        self.expire_locked(&mut slot);
        if automatic.is_some()
            && slot.as_ref().is_some_and(|offer| {
                offer.automatic.is_none() && !matches!(offer.phase, Phase::Confirmed { .. })
            })
        {
            return Err(RemoteError::new(
                "REMOTE_PAIRING_BUSY",
                "A manual pairing is still open. Finish or cancel it first.",
            ));
        }
        end(&mut slot, End::Replaced);
        *slot = Some(Offer {
            id: id.clone(),
            code,
            challenge,
            deadline: Instant::now() + lifetime,
            failures: 0,
            phase: Phase::Open,
            events,
            automatic,
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
        self.confirm_with(offer_id, PairingPolicy::default());
    }
    pub fn confirm_with(&self, offer_id: &str, policy: PairingPolicy) {
        let mut slot = self.lock();
        self.expire_locked(&mut slot);
        let Some(offer) = slot.as_mut().filter(|o| o.id == offer_id) else {
            return;
        };
        let Phase::Pinned(candidate) = &offer.phase else {
            return;
        };
        let candidate = candidate.clone();
        match policy
            .validate()
            .and_then(|()| self.issue(offer, &candidate, &policy))
        {
            Ok((grant, receipt, proof)) => {
                // Only the bounded recovery data stays; the code is erased.
                offer.code.fill(0);
                offer.phase = Phase::Confirmed {
                    candidate,
                    receipt,
                    proof,
                };
                let _ = offer.events.send(PairingEvent::Ended(End::Paired {
                    client_id: grant.client_id,
                }));
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
    pub(crate) fn cancel_automatic(&self, offer_id: &str) {
        let mut slot = self.lock();
        if slot
            .as_ref()
            .is_some_and(|offer| offer.id == offer_id && offer.automatic.is_some())
        {
            end(&mut slot, End::Cancelled);
            self.changed.notify_all();
        }
    }
    fn failed_code(&self, slot: &mut Option<Offer>) {
        let offer = slot.as_mut().expect("admitted offer");
        offer.failures += 1;
        if offer.failures >= MAX_FAILURES {
            end(slot, End::TooManyFailures);
            self.changed.notify_all();
        }
    }
    pub fn shutdown(&self) {
        let mut slot = self.lock();
        end(&mut slot, End::Cancelled);
        self.changed.notify_all();
    }
    /// Admit one enrollment candidate from `/pair`. `origin` is the request's
    /// HTTP Origin header, which must match the proposed browser/add-on origin.
    pub fn submit(&self, origin: Option<&str>, body: &[u8]) -> Submission {
        let (wire, cancel) = match serde_json::from_slice::<Value>(body) {
            Ok(Value::Object(mut wire)) if wire.get("cancel") == Some(&Value::Bool(true)) => {
                wire.remove("cancel");
                (serde_json::to_vec(&wire).unwrap_or_default(), true)
            }
            _ => (body.to_vec(), false),
        };
        let Some(wire) = parse(&wire) else {
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
        if cancel {
            if offer.automatic.is_none() || candidate.kind != "browser" {
                return Submission::Refused;
            }
            if crypto::verify_mac(&offer.code, &candidate.enrollment, &candidate.mac).is_err() {
                self.failed_code(&mut slot);
                return Submission::Refused;
            }
            if !canonical::cancellation(&candidate.enrollment, &candidate.mac).is_ok_and(|bytes| {
                crypto::verify_signature(&candidate.public_key, &bytes, &candidate.signature)
                    .is_ok()
            }) {
                return Submission::Refused;
            }
            let live = self
                .store
                .lock()
                .ok()
                .and_then(|store| store.grants().ok())
                .is_some_and(|grants| {
                    grants.iter().any(|grant| {
                        grant.public_key == candidate.public_key
                            && grant.kind == "browser"
                            && grant.origin == candidate.origin
                            && now_ms().is_ok_and(|now| grant.live_at(now))
                    })
                });
            if !live {
                return Submission::Refused;
            }
            end(&mut slot, End::Cancelled);
            self.changed.notify_all();
            return Submission::Cancelled;
        }
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
            self.failed_code(&mut slot);
            return Submission::Refused;
        }
        if !possession(&candidate) {
            return Submission::Refused;
        }
        if offer.automatic.is_some() {
            if candidate.kind != "browser" {
                return Submission::Refused;
            }
            let result = self.issue(offer, &candidate, &PairingPolicy::default());
            end(&mut slot, End::Cancelled);
            self.changed.notify_all();
            return match result {
                Ok((grant, receipt, proof)) => {
                    // Identifiers/origin are public; never format the offer or launch URL.
                    let _ = writeln!(
                        tmt_cli_style::stream::stderr(),
                        "Remote auto-paired device {}; issuedAtMs {}; origin {}",
                        grant.client_id,
                        grant.issued_at_ms,
                        grant.origin
                    );
                    Submission::Receipt { receipt, proof }
                }
                Err(_) => Submission::Refused,
            };
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
        // A browser or add-on must send the exact proposed Origin; a CLI sends
        // none. A browser enrolled here must be on this door's own origin, not
        // another loopback port that merely proposes itself.
        let origin_matches = match kind {
            "cli" => request_origin.is_none(),
            "browser" => origin == self.door_origin && request_origin == Some(origin),
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
    /// Insert the owner-selected grant and derive the exact receipt and its proof.
    fn issue(
        &self,
        offer: &Offer,
        candidate: &Candidate,
        policy: &PairingPolicy,
    ) -> Result<(Grant, Vec<u8>, [u8; 32]), RemoteError> {
        let grant = Grant {
            client_id: uuid_v4()?,
            public_key: candidate.public_key,
            kind: candidate.kind.clone(),
            origin: candidate.origin.clone(),
            name: candidate.name.clone(),
            agents: policy.agents.as_ref().map_or_else(
                || "all".into(),
                |ids| serde_json::to_string(ids).expect("UUID list"),
            ),
            scopes: if policy.talk {
                SUPPORTED_SCOPES.as_slice()
            } else {
                DEFAULT_SCOPES.as_slice()
            }
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect(),
            mode: if policy.hold { "hold" } else { "direct" }.into(),
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
            .insert_paired_grant(&grant, &offer.id, &Sha256::digest(&receipt))?;
        Ok((grant, receipt, proof))
    }
}
/// Exact receipt JSON bytes `{grant, machinePublicKey}`; the proof covers these bytes.
pub fn receipt(
    grant: &Grant,
    machine_id: &str,
    machine_public: &[u8; 32],
) -> Result<Vec<u8>, RemoteError> {
    let agents = match grant.authority()?.agents {
        crate::authority::PermittedAgents::All => json!("all"),
        crate::authority::PermittedAgents::Selected(ids) => json!(ids),
    };
    serde_json::to_vec(&json!({
        "grant": {
            "clientId": grant.client_id,
            "machineId": machine_id,
            "profile": "local-v1",
            "publicKey": canonical::base64url(&grant.public_key),
            "kind": grant.kind,
            "origin": grant.origin,
            "name": grant.name,
            "agents": agents,
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

#[cfg(test)]
mod restart_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn with_pairing(test: impl FnOnce(&Pairing, &crate::state::Layout)) {
        let root =
            Scratch(std::env::temp_dir().join(format!("tmt-restart-pair-{}", uuid_v4().unwrap())));
        let layout = crate::state::Layout::open(&root.0).unwrap();
        let serving = layout.serve_lock().unwrap();
        let mut store = Store::open(&serving).unwrap();
        let machine = store.machine().unwrap();
        let pairing = Pairing::new(
            machine.id,
            uuid_v4().unwrap(),
            [1; 32],
            "http://127.0.0.1:12345".into(),
            Arc::new(Mutex::new(store)),
            Timing::CONTRACT,
        );
        test(&pairing, &layout);
    }

    #[test]
    fn automatic_deadline_and_owner_cancel_erase_only_the_matching_auto_offer() {
        with_pairing(|pairing, _| {
            let first = pairing.open_automatic("/".into()).unwrap();
            let next = pairing.open_automatic("/settings".into()).unwrap();
            pairing.cancel_automatic(&first.offer_id);
            assert_eq!(
                pairing.offer.lock().unwrap().as_ref().unwrap().id,
                next.offer_id
            );
            pairing.offer.lock().unwrap().as_mut().unwrap().deadline = Instant::now();
            pairing.expire();
            assert!(pairing.offer.lock().unwrap().is_none());
            let (manual, _events) = pairing.open().unwrap();
            pairing.cancel_automatic(&manual.offer_id);
            assert!(pairing.offer.lock().unwrap().is_some());
            pairing.shutdown();
            assert!(pairing.offer.lock().unwrap().is_none());
        });
    }

    #[test]
    fn restart_open_and_unconfirmed_offers_still_defer_without_changes() {
        with_pairing(|pairing, _layout| {
            let (offered, _events) = pairing.open().unwrap();
            for pinned in [false, true] {
                if pinned {
                    pairing.offer.lock().unwrap().as_mut().unwrap().phase =
                        Phase::Pinned(Candidate {
                            enrollment: vec![1],
                            mac: [2; 32],
                            signature: [3; 64],
                            public_key: [4; 32],
                            kind: "browser".into(),
                            origin: pairing.door_origin.clone(),
                            name: "pending device".into(),
                        });
                }
                assert!(
                    !pairing
                        .while_idle(
                            Instant::now() + crate::limits::RESTART_IDLE_WAIT,
                            || panic!("busy offer must not stop the door")
                        )
                        .unwrap()
                );
                let slot = pairing.offer.lock().unwrap();
                let offer = slot.as_ref().unwrap();
                assert_eq!(offer.id, offered.offer_id);
                assert_eq!(offer.code, offered.code);
                assert!(if pinned {
                    matches!(offer.phase, Phase::Pinned(_))
                } else {
                    matches!(offer.phase, Phase::Open)
                });
            }
        });
    }
    #[test]
    fn restart_waits_for_store_in_offer_live_store_order_and_holds_the_fences() {
        with_pairing(|pairing, layout| {
            let sessions = crate::session::DoorSessions::new(
                pairing.machine_id.clone(),
                pairing.window_id.clone(),
                pairing.door_origin.clone(),
                "/r/test/".into(),
                crate::state::MachineKey::open(layout).unwrap(),
                pairing.store.clone(),
                crate::session::IDLE,
            );
            let (held, ready) = mpsc::channel();
            let (finished, result) = mpsc::channel();
            std::thread::scope(|scope| {
                scope.spawn(move || {
                    let _store = pairing.store.lock().unwrap();
                    held.send(()).unwrap();
                    let _ = result.recv_timeout(Duration::from_millis(50));
                });
                ready.recv_timeout(Duration::from_secs(5)).unwrap();
                let deadline = Instant::now() + crate::limits::RESTART_IDLE_WAIT;
                let admitted = pairing
                    .while_idle(deadline, || {
                        sessions.while_idle(deadline, || {
                            // Real owners stay held at the stop decision, not just at a snapshot.
                            assert!(pairing.offer.try_lock().is_err());
                            assert!(pairing.store.try_lock().is_err());
                            Ok(true)
                        })
                    })
                    .unwrap();
                finished.send(()).ok();
                assert!(admitted);
            });
        });
    }
    #[test]
    fn restart_empty_offer_contention_is_not_pairing_activity() {
        with_pairing(|pairing, _layout| {
            let stopped = AtomicBool::new(false);
            let (held, ready) = mpsc::channel();
            let (finished, result) = mpsc::channel();
            std::thread::scope(|scope| {
                scope.spawn(move || {
                    let _offer = pairing.offer.lock().unwrap();
                    held.send(()).unwrap();
                    // A completed admission must not mistake this empty slot for activity.
                    // Otherwise release the brief observation lock so admission can finish.
                    let _ = result.recv_timeout(Duration::from_millis(50));
                });
                ready.recv_timeout(Duration::from_secs(5)).unwrap();
                let admitted = pairing
                    .while_idle(Instant::now() + crate::limits::RESTART_IDLE_WAIT, || {
                        stopped.store(true, Ordering::Release);
                        Ok(true)
                    })
                    .unwrap();
                finished.send(()).ok();
                assert!(admitted, "an empty offer mutex is not a busy pairing offer");
                assert!(stopped.load(Ordering::Acquire));
            });
        });
    }
}
