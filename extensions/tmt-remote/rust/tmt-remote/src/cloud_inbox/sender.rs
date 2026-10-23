//! A live HPKE context is never persisted. Restart recovery asks the existing journal by ID.
use super::provider::Provider;
use crate::{
    canonical::{self, Envelope},
    cloud_hpke::{ResponseOpener, Sender as Peer, VerifiedDescriptor},
    error::RemoteError,
    objects::IoBudget,
    state::MachineKey,
    store::{Store, uuid_v4},
    wire::SignedMessage,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

pub struct Sender {
    provider: Provider,
    store: Arc<Mutex<Store>>,
    peer: String,
    key: MachineKey,
}
impl Sender {
    pub fn new(
        provider: Provider,
        store: Arc<Mutex<Store>>,
        peer: &str,
        key: MachineKey,
    ) -> Result<Self, RemoteError> {
        canonical::uuid(peer).map_err(|_| invalid())?;
        Ok(Self {
            provider,
            store,
            peer: peer.into(),
            key,
        })
    }
    /// The same schema owner is used by the public command and the fake Core caller.
    pub async fn prepared(
        &self,
        raw: &[u8],
        descriptor: &VerifiedDescriptor,
        resolved: &str,
        now: u64,
        io: &IoBudget<'_>,
    ) -> Result<crate::cloud_command::Answer, RemoteError> {
        use crate::cloud_command::{Answer, Request, State};
        let request = Request::read(raw)?;
        if resolved == "local" || !crate::remote_catalog::name_ok(resolved) {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_UNAVAILABLE",
                "The selected target does not use the cloud message driver.",
            ));
        }
        if request
            .remote
            .as_deref()
            .is_some_and(|name| name != resolved)
        {
            return Err(RemoteError::new(
                "REMOTE_SELECTOR_INVALID",
                "The selected remote does not match the prepared request.",
            ));
        }
        let payload = request.dispatch().to_string();
        let window = uuid_v4()?;
        let session = uuid_v4()?;
        let envelope = Envelope {
            kind: "request",
            id: &request.request_id,
            correlation_id: None,
            machine_id: self.provider.machine(),
            window_id: &window,
            client_id: &self.peer,
            session_id: &session,
            sequence: "1",
            timestamp_ms: now,
            origin: "cli",
            operation: "dispatch.create",
            payload: payload.as_bytes(),
        };
        let signature = canonical::base64url(
            &self
                .key
                .sign(&canonical::envelope(&envelope).map_err(|_| invalid())?),
        );
        let exact=json!({"version":1,"profile":"local-v1","kind":"request","id":request.request_id,"correlationId":null,"machineId":envelope.machine_id,"windowId":window,"clientId":self.peer,"sessionId":session,"sequence":"1","timestampMs":now,"origin":"cli","operation":"dispatch.create","payload":canonical::base64url(payload.as_bytes()),"signature":signature}).to_string();
        let answer = self
            .append(exact.as_bytes(), descriptor, request.created_at_ms, now, io)
            .await;
        let bytes = match answer {
            Ok(bytes) => bytes,
            Err(error) if error.code == "REMOTE_CLOUD_UNCONFIRMED" => {
                return Ok(Answer {
                    version: 1,
                    state: State::Unconfirmed,
                    remote: resolved.into(),
                    reason: Some(error.code),
                });
            }
            Err(error) => return Err(error),
        };
        let signed = SignedMessage::decode_any(&bytes, crate::limits::CLOUD_ENVELOPE_BYTES)
            .ok_or_else(invalid)?;
        let payload = signed.input;
        let (state, reason) = match payload["state"].as_str() {
            Some("accepted") => (State::Sent, None),
            Some("held") => (State::Held, None),
            Some("refused") => (State::Refused, payload["reason"].as_str()),
            Some("uncertain") => (State::Unconfirmed, Some("REMOTE_CLOUD_UNCONFIRMED")),
            _ if payload["error"]["code"].is_string() => {
                (State::Refused, payload["error"]["code"].as_str())
            }
            _ => (State::Unavailable, Some("REMOTE_CLOUD_UNAVAILABLE")),
        };
        if reason.is_some_and(|r| {
            r.is_empty()
                || r.len() > 64
                || !r
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        }) {
            return Err(invalid());
        }
        Ok(Answer {
            version: 1,
            state,
            remote: resolved.into(),
            reason: reason.map(str::to_owned),
        })
    }
    /// Dispatch intent is already frozen by Core. Same-ID recovery never posts new bytes there.
    pub async fn append(
        &self,
        exact: &[u8],
        descriptor: &VerifiedDescriptor,
        created_at_ms: u64,
        now: u64,
        io: &IoBudget<'_>,
    ) -> Result<Vec<u8>, RemoteError> {
        super::check_request_age(created_at_ms, now)?;
        super::check_envelope_size(exact.len())?;
        let request = SignedMessage::decode(exact, crate::limits::CLOUD_ENVELOPE_BYTES)
            .ok_or_else(invalid)?;
        let e = request.envelope();
        if e.client_id != self.peer
            || e.machine_id != self.provider.machine()
            || e.kind != "request"
            || e.operation != "dispatch.create"
        {
            return Err(invalid());
        }
        if crate::cloud_command::dispatch_created_at(&request)? != Some(created_at_ms) {
            return Err(conflict());
        }
        // The operation and exact payload are Core's frozen intent; refreshed transport headers
        // cannot replace it, and a changed intent under the same ID is refused by storage.
        let intent = canonical::framed(&[e.operation.as_bytes(), &request.payload])
            .map_err(|_| invalid())?;
        let old = self
            .store
            .lock()
            .map_err(crate::store::database)?
            .cloud_outgoing(&self.peer, e.id, &intent, created_at_ms)?;
        if let Some(old) = old {
            if now.saturating_sub(old.created_ms) >= crate::journal::RECOVERY {
                return Err(expired());
            }
            let mut observed = self
                .provider
                .get(&self.peer, "inbox", e.id, io)
                .await
                .map_err(|_| unconfirmed())?;
            if observed.is_none() && !old.attempted {
                observed = self
                    .provider
                    .create(
                        &self.peer,
                        "inbox",
                        e.id,
                        &old.carrier,
                        self.claim(e.id),
                        io,
                    )
                    .await?;
            }
            match observed {
                Some(doc) if doc.bytes() == old.carrier => {
                    return self.recover(e.id, descriptor, now, io).await;
                }
                Some(_) => return Err(conflict()),
                None => return Err(unconfirmed()),
            }
        }
        let (carrier, opener) = super::seal_native(
            exact,
            descriptor,
            now,
            Peer {
                client_id: &self.peer,
                authentication_key: &self.key.public(),
            },
            |b| self.key.sign(b),
        )?;
        let stored = self
            .store
            .lock()
            .map_err(crate::store::database)?
            .cloud_freeze_outgoing(&self.peer, e.id, &intent, &carrier, created_at_ms)?;
        if stored.carrier != carrier {
            return Err(unconfirmed());
        }
        self.exchange(e.id, &carrier, opener, io).await
    }
    fn claim(
        &self,
        id: &str,
    ) -> Box<dyn FnOnce() -> Result<bool, crate::sealed_sync::SyncError> + Send> {
        let store = Arc::clone(&self.store);
        let peer = self.peer.clone();
        let id = id.to_owned();
        Box::new(move || {
            store
                .lock()
                .map_err(|_| crate::sealed_sync::SyncError::Unavailable)?
                .cloud_claim_outgoing(&peer, &id)
                .map_err(|_| crate::sealed_sync::SyncError::Unavailable)
        })
    }
    async fn exchange(
        &self,
        id: &str,
        carrier: &[u8],
        opener: ResponseOpener,
        io: &IoBudget<'_>,
    ) -> Result<Vec<u8>, RemoteError> {
        match self
            .provider
            .create(&self.peer, "inbox", id, carrier, self.claim(id), io)
            .await?
        {
            Some(doc) if doc.bytes() == carrier => {}
            Some(_) => return Err(conflict()),
            None => return Err(unconfirmed()),
        }
        loop {
            if let Some(response) = self
                .provider
                .get(&self.peer, "responses", id, io)
                .await
                .map_err(|_| unconfirmed())?
            {
                return opener
                    .open(response.bytes(), crate::limits::CLOUD_ENVELOPE_BYTES)
                    .map_err(|e| RemoteError::new(e.code(), &e.to_string()));
            }
            self.provider.wait(io, crate::limits::CLOUD_POLL_MIN_MS)?;
        }
    }
    async fn recover(
        &self,
        original: &str,
        descriptor: &VerifiedDescriptor,
        now: u64,
        io: &IoBudget<'_>,
    ) -> Result<Vec<u8>, RemoteError> {
        // Fresh signed observation, never a second dispatch. A lost observation can itself be
        // replaced by another read-only observation of the original operation, not a reseal.
        let id = uuid_v4()?;
        let window = uuid_v4()?;
        let session = uuid_v4()?;
        let payload = json!({"operationId":original}).to_string();
        let envelope = Envelope {
            kind: "request",
            id: &id,
            correlation_id: None,
            machine_id: self.provider.machine(),
            window_id: &window,
            client_id: &self.peer,
            session_id: &session,
            sequence: "1",
            timestamp_ms: now,
            origin: "cli",
            operation: "operation.show",
            payload: payload.as_bytes(),
        };
        let signature = canonical::base64url(
            &self
                .key
                .sign(&canonical::envelope(&envelope).map_err(|_| invalid())?),
        );
        let exact=json!({"version":1,"profile":"local-v1","kind":"request","id":id,"correlationId":null,"machineId":envelope.machine_id,"windowId":window,"clientId":self.peer,"sessionId":session,"sequence":"1","timestampMs":now,"origin":"cli","operation":"operation.show","payload":canonical::base64url(payload.as_bytes()),"signature":signature}).to_string();
        let (carrier, opener) = super::seal_native(
            exact.as_bytes(),
            descriptor,
            now,
            Peer {
                client_id: &self.peer,
                authentication_key: &self.key.public(),
            },
            |b| self.key.sign(b),
        )?;
        // Observation bytes also freeze before their only create. Context secrets stay in memory.
        self.store
            .lock()
            .map_err(crate::store::database)?
            .cloud_freeze_outgoing(&self.peer, &id, payload.as_bytes(), &carrier, now)?;
        self.exchange(&id, &carrier, opener, io).await
    }
}
fn invalid() -> RemoteError {
    RemoteError::new("REMOTE_INPUT_INVALID", "Invalid cloud dispatch input.")
}
fn conflict() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_CONFLICT",
        "The operation ID belongs to different cloud ciphertext.",
    )
}
fn unconfirmed() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_UNCONFIRMED",
        "The cloud result could not be confirmed; observe the same operation ID before continuing.",
    )
}
fn expired() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_MESSAGE_EXPIRED",
        "The cloud message has expired; it will not be sent again.",
    )
}
