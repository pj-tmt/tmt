//! Local-owner composition. Control owns terminal interaction; this service owns
//! pairing policy and provider reconciliation, never Core preparation or state.
use super::{
    Candidate, Connection, RouteChange,
    wire::{self, Invite, Submission},
};
use crate::{
    canonical,
    cloud_hpke::{Descriptor, Pin, VerifiedDescriptor},
    cloud_inbox::{InboxDriver, Provider, Receiver, Sender},
    cloud_keys::CloudKeys,
    error::RemoteError,
    firestore_budget::{Model, NATIVE_DEFAULT_MODEL},
    message::MessageService,
    native_sync::{complete, projects::Projects},
    objects::IoBudget,
    session::DoorSessions,
    state::{Layout, MachineKey},
    store::Store,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

fn unavailable() -> RemoteError {
    RemoteError::new("REMOTE_CLOUD_UNAVAILABLE", "Cloud pairing is unavailable.")
}
fn run<T>(
    future: impl Future<Output = Result<T, RemoteError>>,
    io: &IoBudget<'_>,
) -> Result<T, RemoteError> {
    complete(future, io).map_err(|_| unavailable())?
}
pub struct Service {
    layout: Layout,
    store: Arc<Mutex<Store>>,
    key: MachineKey,
    machine: String,
    keys: Arc<CloudKeys>,
    sessions: Arc<DoorSessions>,
    messages: Arc<MessageService>,
    pub projects: Arc<Projects>,
    stop: Arc<AtomicBool>,
    wake: Arc<(Mutex<bool>, Condvar)>,
    inviting: AtomicBool,
    last_error: Mutex<Option<String>>,
    published: Mutex<BTreeMap<String, Vec<u8>>>,
    cleanup: Mutex<BTreeMap<String, Instant>>,
}
pub struct Owner {
    pub layout: Layout,
    pub store: Arc<Mutex<Store>>,
    pub key: MachineKey,
    pub machine: String,
    pub keys: Arc<CloudKeys>,
    pub sessions: Arc<DoorSessions>,
    pub messages: Arc<MessageService>,
    pub projects: Arc<Projects>,
    pub stop: Arc<AtomicBool>,
}
impl Service {
    pub fn new(owner: Owner) -> Self {
        let Owner {
            layout,
            store,
            key,
            machine,
            keys,
            sessions,
            messages,
            projects,
            stop,
        } = owner;
        Self {
            layout,
            store,
            key,
            machine,
            keys,
            sessions,
            messages,
            projects,
            stop,
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            inviting: AtomicBool::new(false),
            last_error: Mutex::new(None),
            published: Mutex::new(BTreeMap::new()),
            cleanup: Mutex::new(BTreeMap::new()),
        }
    }
    fn store(&self) -> Result<std::sync::MutexGuard<'_, Store>, RemoteError> {
        self.store.lock().map_err(crate::store::database)
    }
    fn provider(&self, config: &Value, machine: &str, pair: bool) -> Result<Provider, RemoteError> {
        let project = self.projects.load(config, pair)?;
        Provider::new(
            project.auth.clone(),
            project.runtime.clone(),
            Model::new(NATIVE_DEFAULT_MODEL.0, NATIVE_DEFAULT_MODEL.1)
                .map_err(|_| unavailable())?,
            machine,
        )
    }
    fn io(&self) -> IoBudget<'_> {
        IoBudget {
            deadline: Instant::now() + crate::limits::CLOUD_RECONCILE_CALL,
            cancelled: &self.stop,
        }
    }
    pub fn status(&self) -> Result<Value, RemoteError> {
        Ok(
            json!({"cleanupPending":!self.store()?.cloud_route_changes()?.is_empty(),"error":self.last_error.lock().map_err(crate::store::database)?.clone()}),
        )
    }
    pub fn request_reconcile(&self) {
        if let Ok(mut pending) = self.wake.0.lock() {
            *pending = true;
            self.wake.1.notify_all();
        }
    }
    pub fn idle(&self) -> bool {
        !self.inviting.load(Ordering::Acquire)
    }
    fn register(
        &self,
        provider: &Provider,
        descriptor: &[u8],
        inviting_owner: Option<&str>,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        let project = provider.identity().project.clone();
        let previous = self
            .published
            .lock()
            .map_err(crate::store::database)?
            .get(&project)
            .cloned();
        if previous.as_deref() == Some(descriptor) {
            return Ok(());
        }
        let existing = run(provider.machine_metadata(io), io)?;
        let inviting_owner = if let Some(row) = &existing {
            Self::machine_fields(provider, &self.key.public(), row)?
                .get("invitingOwner")
                .and_then(|value| value["stringValue"].as_str())
        } else {
            inviting_owner
        };
        run(
            provider.register_machine(&self.key.public(), descriptor, inviting_owner, io),
            io,
        )?;
        self.published
            .lock()
            .map_err(crate::store::database)?
            .insert(project, descriptor.to_vec());
        Ok(())
    }
    fn descriptor(
        &self,
        provider: &Provider,
        pin: &[u8; 32],
        previous: &[u8],
        io: &IoBudget<'_>,
    ) -> Result<VerifiedDescriptor, RemoteError> {
        let row = run(provider.machine_metadata(io), io)?.ok_or_else(wire::invalid)?;
        self.verify_descriptor(provider, pin, previous, &row)
    }
    fn machine_fields<'a>(
        provider: &Provider,
        pin: &[u8; 32],
        row: &'a Value,
    ) -> Result<&'a serde_json::Map<String, Value>, RemoteError> {
        let fields = row["fields"]
            .as_object()
            .filter(|f| {
                (f.len() == 3
                    || (f.len() == 4
                        && f.get("invitingOwner").is_some_and(|value| {
                            value["stringValue"].as_str().is_some_and(|id| {
                                canonical::is_core_id(id) && id != provider.machine()
                            })
                        })))
                    && f.get("uid").is_some_and(|value| {
                        value["stringValue"].as_str().is_some_and(super::uid_ok)
                    })
            })
            .ok_or_else(wire::invalid)?;
        let key = fields.get("authenticationKey").ok_or_else(wire::invalid)?["bytesValue"]
            .as_str()
            .and_then(|v| STANDARD.decode(v).ok())
            .ok_or_else(wire::invalid)?;
        if key != pin {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_MACHINE_CONFLICT",
                "The machine registration does not match the pinned identity; it was not changed.",
            ));
        }
        Ok(fields)
    }
    fn verify_descriptor(
        &self,
        provider: &Provider,
        pin: &[u8; 32],
        previous: &[u8],
        row: &Value,
    ) -> Result<VerifiedDescriptor, RemoteError> {
        let fields = Self::machine_fields(provider, pin, row)?;
        let descriptor = fields.get("descriptor").ok_or_else(wire::invalid)?["bytesValue"]
            .as_str()
            .and_then(|v| STANDARD.decode(v).ok())
            .filter(|v| v.len() <= crate::limits::CLOUD_DESCRIPTOR_BYTES)
            .ok_or_else(wire::invalid)?;
        let pin_value = Pin {
            machine_id: provider.machine(),
            authentication_key: pin,
        };
        let old = if previous.is_empty() {
            None
        } else {
            let start = crate::wire::strict_json(previous)
                .and_then(|v| v["notBeforeMs"].as_u64())
                .ok_or_else(wire::invalid)?;
            Some(
                Descriptor::verify(previous, pin_value, start, None)
                    .map_err(|e| RemoteError::new(e.code(), &e.to_string()))?,
            )
        };
        Descriptor::verify(
            &descriptor,
            pin_value,
            crate::pairing::now_ms()?,
            old.as_ref(),
        )
        .map_err(|e| RemoteError::new(e.code(), &e.to_string()))
    }
    fn flush(
        &self,
        provider: &Provider,
        config: &Value,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        let mut groups: BTreeMap<String, Vec<RouteChange>> = BTreeMap::new();
        for row in self
            .store()?
            .cloud_route_changes()?
            .into_iter()
            .filter(|r| r.project == provider.identity().project)
        {
            groups.entry(row.peer.clone()).or_default().push(row);
        }
        for rows in groups.values() {
            // Only a pending route adopted from a verified newer reroute can
            // move an invited child's uid. Preserve original-owner provenance.
            let child =
                if let Some(live) = rows.iter().find(|row| !row.revoked && row.generation > 1) {
                    let peer = self.provider(config, &live.peer, false)?;
                    if let Some(metadata) = run(peer.machine_metadata(io), io)? {
                        let grant = self.store()?.grant(&live.peer)?.ok_or_else(wire::invalid)?;
                        let fields = Self::machine_fields(&peer, &grant.public_key, &metadata)?;
                        (fields
                            .get("invitingOwner")
                            .is_some_and(|value| value["stringValue"] == self.machine)
                            && fields["uid"]["stringValue"] != live.uid)
                            .then_some(live.peer.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                };
            run(provider.publish_routes(rows, child, io), io)?;
            let store = self.store()?;
            for row in rows {
                store.cloud_route_published(row)?;
            }
        }
        Ok(())
    }
    pub fn flush_routes(&self) -> Result<(), RemoteError> {
        let configs = self.store()?.cloud_owner_configs()?;
        for config in configs {
            let provider = self.provider(&config, &self.machine, false)?;
            self.flush(&provider, &config, &self.io())?;
        }
        Ok(())
    }
    /// Each invocation holds its own offer secret. A guard erases it on every
    /// return, and no remote submission can manufacture the local confirmation.
    pub fn invite(
        &self,
        remote: Option<&str>,
        emit: impl FnMut(Value) -> Result<Option<(bool, bool)>, RemoteError>,
    ) -> Result<(), RemoteError> {
        let selected = crate::remote_selector::client(&self.layout, remote, None)?;
        if !matches!(selected.kind, crate::remote_selector::ClientKind::Owned) {
            return Err(unavailable());
        }
        self.invite_selected(&selected.project, selected.public_config, emit)
    }
    fn invite_selected(
        &self,
        project: &str,
        config: Value,
        mut emit: impl FnMut(Value) -> Result<Option<(bool, bool)>, RemoteError>,
    ) -> Result<(), RemoteError> {
        if self.inviting.swap(true, Ordering::AcqRel) {
            return Err(RemoteError::new(
                "REMOTE_PAIRING_BUSY",
                "An invite is already open.",
            ));
        }
        struct End<'a>(&'a AtomicBool);
        impl Drop for End<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _end = End(&self.inviting);
        let provider = self.provider(&config, &self.machine, true)?;
        let now = crate::pairing::now_ms()?;
        self.keys.maintain(now)?;
        let descriptor = self.keys.advertised(now)?;
        self.register(&provider, descriptor.bytes(), None, &self.io())?;
        self.store()?.remember_cloud_owner(project, &config)?;
        // An explicit owner action may collect a gate left by its first interrupted invite.
        // Without an enrolled peer, background reconciliation intentionally does no I/O.
        let _ = self.cleanup_invites(&provider, now, &self.io());
        let now = crate::pairing::now_ms()?;
        let owner_name = std::env::var("USER")
            .ok()
            .filter(|v| canonical::device_name(v))
            .unwrap_or("Owner".into());
        let invite = Invite::new(
            project,
            &self.machine,
            &self.key.public(),
            config,
            &owner_name,
            now,
        )?;
        run(provider.publish_invite(&invite, &self.io()), &self.io())?;
        let result = (|| {
            emit(json!({"event":"invite","code":invite.token()?}))?;
            let until = Instant::now() + crate::pairing::Timing::CONTRACT.lifetime;
            let io = IoBudget {
                deadline: until,
                cancelled: &self.stop,
            };
            loop {
                emit(json!({"event":"waiting"}))?;
                if crate::pairing::now_ms()? >= invite.challenge.expires_at_ms {
                    return Err(wire::expired());
                }
                if let Some(doc) = run(provider.enrollment_get(&invite.challenge.offer, &io), &io)?
                {
                    if doc.kind != "join" {
                        return Err(wire::invalid());
                    }
                    let submission = Submission::read(&doc.body, &invite.challenge.offer)?;
                    let verified =
                        invite.verify(&submission, &doc.uid, crate::pairing::now_ms()?)?;
                    let (matched,talk)=emit(json!({"event":"candidate","name":submission.candidate.name,"words":verified.words().map_err(|_|wire::invalid())?}))?.ok_or_else(wire::invalid)?;
                    if !matched {
                        return Err(RemoteError::new(
                            "REMOTE_CLOUD_PAIR_REFUSED",
                            "The owner declined the connection.",
                        ));
                    }
                    let candidate = verified.candidate();
                    let reciprocal = Connection {
                        version: 1,
                        name: self.connection_name(&candidate.peer, &candidate.name)?,
                        owner: candidate.peer.clone(),
                        owner_key: candidate.public_key,
                        config: invite.public_config.clone(),
                        peer: self.machine.clone(),
                        uid: provider.uid().into(),
                        request: Vec::new(),
                        confirmed: true,
                        descriptor: Vec::new(),
                        reroute_token: None,
                    };
                    let confirmed_at = crate::pairing::now_ms()?;
                    let mut store = self.store()?;
                    let mut response_key = invite.response_key(&submission.candidate)?;
                    let receipt = store.confirm_cloud_peer(
                        &verified,
                        &response_key,
                        confirmed_at,
                        talk,
                        Some(reciprocal),
                    );
                    response_key.fill(0);
                    drop(store);
                    let receipt = receipt?;
                    self.flush(&provider, &invite.public_config, &io)?;
                    let body=json!({"receipt":canonical::base64url(&receipt.bytes),"proof":canonical::base64url(&receipt.proof)}).to_string();
                    run(
                        provider.enrollment_create(
                            &format!("{}-response", invite.challenge.offer),
                            &doc.uid,
                            "receipt",
                            body.as_bytes(),
                            None,
                            &io,
                        ),
                        &io,
                    )?;
                    let unique = self
                        .store()?
                        .active_grants()?
                        .iter()
                        .filter(|g| g.name == submission.candidate.name)
                        .count()
                        == 1;
                    let hint = if unique {
                        &submission.candidate.name
                    } else {
                        &submission.candidate.peer
                    };
                    let reverse_reference = self
                        .store()?
                        .cloud_connection(&submission.candidate.peer)?
                        .ok_or_else(wire::invalid)?
                        .name;
                    emit(
                        json!({"event":"connected","name":submission.candidate.name,"talk":talk,"clientId":submission.candidate.peer,"hintPerson":hint,
                            "remote":reverse_reference}),
                    )?;
                    self.request_reconcile();
                    return Ok(());
                }
                provider.wait(&io, crate::limits::CLOUD_POLL_MIN_MS)?;
            }
        })();
        let cleanup_cancel = AtomicBool::new(false);
        let cleanup_io = IoBudget {
            deadline: Instant::now() + crate::limits::CLOUD_RECONCILE_CALL,
            cancelled: &cleanup_cancel,
        };
        let _ = run(
            provider.delete_invite(&invite.challenge.offer, &cleanup_io),
            &cleanup_io,
        );
        result
    }
    pub fn join(
        &self,
        token: &str,
        mut emit: impl FnMut(Value) -> Result<Option<bool>, RemoteError>,
    ) -> Result<(), RemoteError> {
        let invite = Invite::read(token, crate::pairing::now_ms()?)?;
        let provider = self.provider(&invite.public_config, &invite.challenge.owner, true)?;
        self.descriptor(&provider, &invite.challenge.owner_key, &[], &self.io())?;
        let mut name = if crate::remote_catalog::name_ok(&invite.machine_name) {
            invite.machine_name.clone()
        } else {
            invite.challenge.owner.clone()
        };
        let by_owner = self.store()?.cloud_connection(&invite.challenge.owner)?;
        let by_name = self.store()?.cloud_connection(&name)?;
        if by_name
            .as_ref()
            .is_some_and(|r| r.owner != invite.challenge.owner)
        {
            name = invite.challenge.owner.clone();
        }
        let old = by_owner.or_else(|| by_name.filter(|r| r.owner == invite.challenge.owner));
        if let Some(old) = &old {
            name = old.name.clone();
        }
        let submission = if let Some(old) = &old {
            if old.owner != invite.challenge.owner || old.owner_key != invite.challenge.owner_key {
                return Err(wire::invalid());
            }
            Submission::read(&old.request, &invite.challenge.offer)?
        } else {
            let mut nonce = [0; 16];
            getrandom::fill(&mut nonce).map_err(|_| unavailable())?;
            let person = std::env::var("USER")
                .ok()
                .filter(|v| canonical::device_name(v))
                .unwrap_or("This machine".into());
            invite.submit(
                Candidate {
                    peer: self.machine.clone(),
                    public_key: self.key.public(),
                    uid: provider.uid().into(),
                    name: person,
                    nonce,
                },
                &self.key,
            )?
        };
        let raw = submission.encode(&invite.challenge.offer)?;
        let mut connection = Connection {
            version: 1,
            name: name.clone(),
            owner: invite.challenge.owner.clone(),
            owner_key: invite.challenge.owner_key,
            config: invite.public_config.clone(),
            peer: self.machine.clone(),
            uid: provider.uid().into(),
            request: raw.clone(),
            confirmed: false,
            reroute_token: old.as_ref().and_then(|r| r.reroute_token),
            descriptor: old
                .as_ref()
                .map(|r| r.descriptor.clone())
                .unwrap_or_default(),
        };
        if old.is_none() {
            self.store()?.save_cloud_connection(&connection)?;
        }
        emit(
            json!({"event":"joining","name":invite.machine_name,"words":canonical::fingerprint_words(&self.key.public()).map_err(|_|wire::invalid())?}),
        )?;
        let until = Instant::now()
            + Duration::from_millis(
                invite
                    .challenge
                    .expires_at_ms
                    .saturating_sub(crate::pairing::now_ms()?),
            );
        let io = IoBudget {
            deadline: until,
            cancelled: &self.stop,
        };
        let mut invite_token = invite.gate_token()?;
        let submitted = run(
            provider.enrollment_create(
                &invite.challenge.offer,
                provider.uid(),
                "join",
                &raw,
                Some(super::EnrollmentProof::Invite(&invite_token)),
                &io,
            ),
            &io,
        );
        invite_token.fill(0);
        submitted?;
        loop {
            if let Some(doc) = run(
                provider.enrollment_get(&format!("{}-response", invite.challenge.offer), &io),
                &io,
            )? {
                if doc.uid != submission.candidate.uid || doc.kind != "receipt" {
                    return Err(wire::invalid());
                }
                let response = crate::wire::strict_json(&doc.body).ok_or_else(wire::invalid)?;
                let receipt = canonical::base64url_decode(
                    response["receipt"].as_str().ok_or_else(wire::invalid)?,
                )
                .map_err(|_| wire::invalid())?;
                let proof = canonical::base64url_bytes(
                    response["proof"].as_str().ok_or_else(wire::invalid)?,
                    32,
                )
                .map_err(|_| wire::invalid())?;
                let mut key = invite.response_key(&submission.candidate)?;
                let accepted: Result<Value, RemoteError> = (|| {
                    crate::crypto::verify_server_proof(&key, &receipt, &proof)
                        .map_err(|_| wire::invalid())?;
                    let receipt = crate::wire::strict_json(&receipt).ok_or_else(wire::invalid)?;
                    if receipt["grant"]["clientId"] != self.machine
                        || receipt["grant"]["machineId"] != connection.owner
                        || receipt["machinePublicKey"]
                            != canonical::base64url(&connection.owner_key)
                        || receipt["grant"]["publicKey"] != canonical::base64url(&self.key.public())
                    {
                        return Err(wire::invalid());
                    }
                    connection.reroute_token = Some(
                        canonical::base64url_bytes(
                            receipt["rerouteToken"]
                                .as_str()
                                .ok_or_else(wire::missing_reroute_token)?,
                            32,
                        )
                        .map_err(|_| wire::invalid())?
                        .try_into()
                        .map_err(|_| wire::invalid())?,
                    );
                    connection.confirmed = true;
                    let reciprocal_missing = {
                        let store = self.store()?;
                        store.grant(&connection.owner)?.is_none()
                    };
                    if reciprocal_missing {
                        let reverse_talk =
                            emit(json!({"event":"reverse-permission","name":invite.machine_name}))?
                                .unwrap_or(false);
                        let row =
                            run(provider.machine_metadata(&io), &io)?.ok_or_else(wire::invalid)?;
                        let fields = Self::machine_fields(&provider, &connection.owner_key, &row)?;
                        let owner_uid = fields["uid"]["stringValue"]
                            .as_str()
                            .ok_or_else(wire::invalid)?;
                        let grant = crate::store::Grant {
                            client_id: connection.owner.clone(),
                            public_key: connection.owner_key,
                            kind: "cli".into(),
                            origin: "cli".into(),
                            name: invite.machine_name.clone(),
                            agents: "all".into(),
                            scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
                            mode: "direct".into(),
                            issued_at_ms: crate::pairing::now_ms()?,
                            expires_at_ms: None,
                            revision: 1,
                            disabled: false,
                        };
                        let mut reverse_token =
                            wire::reverse_reroute_token(&key, &invite.challenge, &self.machine)?;
                        let confirmation = (|| {
                            self.store()?.confirm_reciprocal(
                                &connection,
                                owner_uid,
                                &grant,
                                &self.key.public(),
                                reverse_talk,
                                &reverse_token,
                            )
                        })();
                        reverse_token.fill(0);
                        confirmation?;
                    } else {
                        self.store()?.save_cloud_connection(&connection)?;
                    }
                    Ok(receipt)
                })();
                key.fill(0);
                let receipt = accepted?;
                let local = self.provider(&connection.config, &self.machine, false)?;
                let now = crate::pairing::now_ms()?;
                self.keys.maintain(now)?;
                let descriptor = self.keys.advertised(now)?;
                self.register(&local, descriptor.bytes(), Some(&connection.owner), &io)?;
                self.flush(&local, &connection.config, &io)?;
                let talk = receipt["grant"]["scopes"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|v| v == "talk"));
                emit(
                    json!({"event":"connected","name":invite.machine_name,"talk":talk,"remote":name}),
                )?;
                self.request_reconcile();
                return Ok(());
            }
            provider.wait(&io, crate::limits::CLOUD_POLL_MIN_MS)?;
        }
    }
    pub fn send(&self, raw: &[u8], explicit: Option<&str>) -> Result<Value, RemoteError> {
        let request = crate::cloud_command::Request::read(raw)?;
        let selected = explicit
            .or(request.remote.as_deref())
            .ok_or_else(unavailable)?;
        let mut record = self
            .store()?
            .cloud_connection(selected)?
            .filter(|r| r.confirmed)
            .ok_or_else(unavailable)?;
        let provider = self.provider(&record.config, &record.owner, false)?;
        let io = self.io();
        let descriptor = self.descriptor(&provider, &record.owner_key, &record.descriptor, &io)?;
        record.descriptor = descriptor.bytes().to_vec();
        self.store()?.save_cloud_connection(&record)?;
        self.reroute(&mut record, &provider, &io)?;
        let sender = Sender::new(provider, self.store.clone(), &record.peer, self.key.clone())?;
        serde_json::to_value(run(
            sender.prepared(raw, &descriptor, selected, crate::pairing::now_ms()?, &io),
            &io,
        )?)
        .map_err(|_| unavailable())
    }
    fn reroute(
        &self,
        record: &mut Connection,
        provider: &Provider,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        if record.uid != provider.uid() {
            let generation = self.store()?.next_cloud_reroute_generation(&record.owner)?;
            let id = crate::store::uuid_v4()?;
            let reroute = super::Reroute {
                machine_id: record.owner.clone(),
                owner_key: record.owner_key,
                peer: record.peer.clone(),
                uid: provider.uid().into(),
                generation,
                timestamp_ms: crate::pairing::now_ms()?,
            };
            let body = wire::route_request(&reroute, &self.key)?;
            run(
                provider.enrollment_create(
                    &id,
                    provider.uid(),
                    "reroute",
                    &body,
                    Some(super::EnrollmentProof::Reroute {
                        peer: &record.peer,
                        token: record
                            .reroute_token
                            .as_ref()
                            .ok_or_else(wire::missing_reroute_token)?,
                    }),
                    io,
                ),
                io,
            )?;
            loop {
                if let Some(doc) = run(provider.enrollment_get(&format!("{id}-response"), io), io)?
                {
                    if doc.kind != "route-response" || doc.uid != provider.uid() {
                        return Err(wire::invalid());
                    }
                    let signature = canonical::base64url_bytes(
                        std::str::from_utf8(&doc.body).map_err(|_| wire::invalid())?,
                        64,
                    )
                    .map_err(|_| wire::invalid())?;
                    crate::crypto::verify_signature(
                        &record.owner_key,
                        &canonical::framed(&[b"tmt-cloud-route-response-v1", id.as_bytes(), &body])
                            .map_err(|_| wire::invalid())?,
                        &signature,
                    )
                    .map_err(|_| wire::invalid())?;
                    break;
                }
                provider.wait(io, crate::limits::CLOUD_POLL_MIN_MS)?;
            }
            record.uid = provider.uid().into();
            self.store()?.save_cloud_connection(record)?;
        }
        Ok(())
    }
    pub fn reconcile(&self) -> Result<(), RemoteError> {
        let mut final_refusal = None;
        let configs = self.store()?.cloud_owner_configs()?;
        for config in configs {
            let provider = self.provider(&config, &self.machine, false)?;
            let io = self.io();
            let connections = self.store()?.cloud_connections()?;
            let inviting_owner = connections
                .iter()
                .find(|record| {
                    record.confirmed
                        && !record.request.is_empty()
                        && record.config["projectId"] == config["projectId"]
                })
                .map(|record| record.owner.clone());
            for mut record in connections.into_iter().filter(|record| {
                record.confirmed
                    && !record.request.is_empty()
                    && record.config["projectId"] == config["projectId"]
                    && record.uid != provider.uid()
            }) {
                let inviter = self.provider(&record.config, &record.owner, false)?;
                self.reroute(&mut record, &inviter, &io)?;
            }
            let now = crate::pairing::now_ms()?;
            self.keys.maintain(now)?;
            let descriptor = self.keys.advertised(now)?;
            self.register(
                &provider,
                descriptor.bytes(),
                inviting_owner.as_deref(),
                &io,
            )?;
            self.flush(&provider, &config, &io)?;
            self.cleanup_enrollment(&provider, now, &io)?;
            let checkpoint = self
                .store()?
                .cloud_checkpoint(&self.machine)?
                .query_from_ms();
            let mut cursor = None;
            let mut newest = checkpoint;
            for _ in 0..crate::journal::OPERATIONS {
                let Some(doc) = run(
                    provider.enrollment_next(checkpoint, cursor.as_ref(), &io),
                    &io,
                )?
                else {
                    break;
                };
                match doc.kind.as_str() {
                    "join" => {
                        if let Ok(submission) = Submission::read(&doc.body, &doc.id) {
                            let recovered = {
                                self.store()?.recover_cloud_join(
                                    &doc.id,
                                    &submission,
                                    &doc.uid,
                                    now,
                                )
                            };
                            let recovered = match recovered {
                                Ok(receipt) => Some(receipt),
                                Err(error) if error.code == "REMOTE_CLOUD_PAIR_INVALID" => None,
                                Err(error) => return Err(error),
                            };
                            if let Some(receipt) = recovered {
                                let body=json!({"receipt":canonical::base64url(&receipt.bytes),"proof":canonical::base64url(&receipt.proof)}).to_string();
                                run(
                                    provider.enrollment_create(
                                        &format!("{}-response", doc.id),
                                        &doc.uid,
                                        "receipt",
                                        body.as_bytes(),
                                        None,
                                        &io,
                                    ),
                                    &io,
                                )?;
                            }
                        }
                    }
                    "reroute" => self.process_reroute(&provider, &config, &doc, now, &io)?,
                    _ => {}
                }
                newest = newest.max(doc.created_ms);
                cursor = Some(doc.cursor);
            }
            self.store()?
                .cloud_complete(std::slice::from_ref(&self.machine), newest)?;
            let driver = InboxDriver::new(
                Receiver::new(
                    self.sessions.clone(),
                    self.keys.clone(),
                    self.messages.clone(),
                ),
                self.sessions.clone(),
                provider,
            );
            run(driver.reconcile(now, &io), &io)?;
            self.discover_reciprocal_peers(&config, now, &io)?;
            match self.push_finals(&config, &io) {
                Err(error) if crate::cloud_inbox::permanent(&error.code) => {
                    final_refusal.get_or_insert(error);
                }
                Err(error) => return Err(error),
                Ok(()) => {}
            }
        }
        self.projects.checkpoint()?;
        final_refusal.map_or(Ok(()), Err)
    }
    fn connection_name(&self, peer: &str, display: &str) -> Result<String, RemoteError> {
        if let Some(record) = self.store()?.cloud_connection(peer)? {
            return Ok(record.name);
        }
        let name = if crate::remote_catalog::name_ok(display) {
            display
        } else {
            peer
        };
        if self.store()?.cloud_connection(name)?.is_some() {
            Ok(peer.into())
        } else {
            Ok(name.into())
        }
    }
    fn discover_reciprocal_peers(
        &self,
        config: &Value,
        now: u64,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        let project = config["projectId"].as_str().ok_or_else(wire::invalid)?;
        let grants = self.store()?.cloud_project_peers(project)?;
        for grant in grants {
            if !grant.live_at(now)
                || self.store()?.cloud_connection(&grant.client_id)?.is_some()
                || !self
                    .store()?
                    .cloud_peer_discovery_ready(project, &grant.client_id, now)?
            {
                continue;
            }
            let provider = self.provider(config, &grant.client_id, false)?;
            let Some(row) = run(provider.machine_metadata(io), io)? else {
                self.store()?
                    .cloud_defer_peer_discovery(project, &grant.client_id, now)?;
                continue;
            };
            let descriptor = self.verify_descriptor(&provider, &grant.public_key, &[], &row)?;
            let mut name = if crate::remote_catalog::name_ok(&grant.name) {
                grant.name.clone()
            } else {
                grant.client_id.clone()
            };
            if self.store()?.cloud_connection(&name)?.is_some() {
                name = grant.client_id.clone();
            }
            self.store()?.save_cloud_connection(&Connection {
                version: 1,
                name,
                owner: grant.client_id.clone(),
                owner_key: grant.public_key,
                config: config.clone(),
                peer: self.machine.clone(),
                uid: provider.uid().into(),
                request: Vec::new(),
                confirmed: true,
                descriptor: descriptor.bytes().to_vec(),
                reroute_token: None,
            })?;
        }
        Ok(())
    }
    /// Only local Core reads discover completion. A ready final then uses the
    /// reverse inbox and the same Auth/runtime/budget/claim owner as a dispatch.
    fn push_finals(&self, config: &Value, io: &IoBudget<'_>) -> Result<(), RemoteError> {
        let project = config["projectId"].as_str().ok_or_else(wire::invalid)?;
        let mut refusal = None;
        let grants = self.store()?.cloud_project_peers(project)?;
        for grant in grants {
            if !grant.live_at(crate::pairing::now_ms()?) || !grant.permits_scope("results.read") {
                continue;
            }
            let operations = self.store()?.cloud_accepted_dispatches(&grant.client_id)?;
            for operation in operations {
                io.check().map_err(|_| unavailable())?;
                let outcome: Result<(), RemoteError> = (|| {
                    let Some(final_value) =
                        self.messages
                            .cloud_final_for(&grant, &self.machine, &operation)?
                    else {
                        return Ok(());
                    };
                    let provider = self.provider(config, &grant.client_id, false)?;
                    let previous = self.store()?.cloud_connection(&grant.client_id)?;
                    let descriptor = self.descriptor(
                        &provider,
                        &grant.public_key,
                        previous
                            .as_ref()
                            .map_or(&[], |record| record.descriptor.as_slice()),
                        io,
                    )?;
                    let mut record = previous.unwrap_or(Connection {
                        version: 1,
                        name: grant.client_id.clone(),
                        owner: grant.client_id.clone(),
                        owner_key: grant.public_key,
                        config: config.clone(),
                        peer: self.machine.clone(),
                        uid: provider.uid().into(),
                        request: Vec::new(),
                        confirmed: true,
                        descriptor: Vec::new(),
                        reroute_token: None,
                    });
                    record.descriptor = descriptor.bytes().to_vec();
                    self.store()?.save_cloud_connection(&record)?;
                    self.reroute(&mut record, &provider, io)?;
                    let sender = Sender::new(
                        provider,
                        self.store.clone(),
                        &self.machine,
                        self.key.clone(),
                    )?;
                    run(
                        sender.push_final(
                            &final_value,
                            &grant,
                            &descriptor,
                            crate::pairing::now_ms()?,
                            io,
                        ),
                        io,
                    )?;
                    Ok(())
                })();
                match outcome {
                    Err(error) if crate::cloud_inbox::permanent(&error.code) => {
                        refusal.get_or_insert(error);
                    }
                    Err(error) => return Err(error),
                    Ok(()) => {}
                }
            }
        }
        refusal.map_or(Ok(()), Err)
    }
    fn cleanup_enrollment(
        &self,
        provider: &Provider,
        now: u64,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        let project = &provider.identity().project;
        let due = self
            .cleanup
            .lock()
            .map_err(crate::store::database)?
            .get(project)
            .is_none_or(|last| last.elapsed() >= crate::limits::CLOUD_ENROLLMENT_CLEANUP);
        if !due {
            return Ok(());
        }
        let cutoff = now.saturating_sub(crate::limits::CLOUD_ENROLLMENT_RETENTION_MS);
        for _ in 0..crate::journal::OPERATIONS {
            let Some(doc) = run(provider.enrollment_next(0, None, io), io)? else {
                break;
            };
            if doc.created_ms >= cutoff {
                break;
            }
            run(provider.enrollment_delete(&doc.id, io), io)?;
        }
        // Older deployed Rules may deny this list; gate collection cannot stall reconciliation.
        let _ = self.cleanup_invites(provider, now, io);
        self.store()?.prune_cloud_receipts(now)?;
        self.cleanup
            .lock()
            .map_err(crate::store::database)?
            .insert(project.clone(), Instant::now());
        Ok(())
    }
    fn cleanup_invites(
        &self,
        provider: &Provider,
        now: u64,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        for _ in 0..crate::journal::OPERATIONS {
            let Some(offer) = run(provider.expired_invite(now, io), io)? else {
                break;
            };
            run(provider.delete_invite(&offer, io), io)?;
        }
        Ok(())
    }
    fn process_reroute(
        &self,
        provider: &Provider,
        config: &Value,
        doc: &super::EnrollmentDocument,
        now: u64,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        let value = crate::wire::strict_json(&doc.body).unwrap_or(Value::Null);
        let peer = value["peer"].as_str().unwrap_or("");
        let grant = self.store()?.grant(peer)?;
        let Some(grant) = grant else {
            return Ok(());
        };
        let Ok(proof) = wire::read_route(&doc.body, &doc.uid, &grant.public_key, doc.created_ms)
        else {
            return Ok(());
        };
        let outcome = self
            .store()?
            .apply_cloud_reroute(&provider.identity().project, &proof, now);
        match outcome {
            Ok(()) => {}
            Err(error) if error.code == "REMOTE_CLOUD_PAIR_INVALID" => return Ok(()),
            Err(error) => return Err(error),
        }
        self.flush(provider, config, io)?;
        let bytes =
            canonical::framed(&[b"tmt-cloud-route-response-v1", doc.id.as_bytes(), &doc.body])
                .map_err(|_| wire::invalid())?;
        let signature = canonical::base64url(&self.key.sign(&bytes));
        run(
            provider.enrollment_create(
                &format!("{}-response", doc.id),
                &doc.uid,
                "route-response",
                signature.as_bytes(),
                None,
                io,
            ),
            io,
        )
    }
    pub fn worker(self: &Arc<Self>) -> Result<Worker, RemoteError> {
        let service = self.clone();
        let wake = self.wake.clone();
        let stop = self.stop.clone();
        let flag = stop.clone();
        let join = thread::Builder::new()
            .name("remote-native-pairing".into())
            .spawn(move || {
                while !flag.load(Ordering::Acquire) && !service.stop.load(Ordering::Acquire) {
                    let outcome = service.reconcile();
                    if let Ok(mut status) = service.last_error.lock() {
                        *status = outcome.err().map(|e| e.code);
                    }
                    let guard = wake.0.lock().unwrap_or_else(|e| e.into_inner());
                    if flag.load(Ordering::Acquire) {
                        break;
                    }
                    let (mut pending, _) = wake
                        .1
                        .wait_timeout_while(
                            guard,
                            Duration::from_millis(crate::limits::CLOUD_POLL_IDLE_MS),
                            |pending| !*pending && !flag.load(Ordering::Acquire),
                        )
                        .unwrap_or_else(|e| e.into_inner());
                    *pending = false;
                }
            })?;
        Ok(Worker {
            stop,
            wake: self.wake.clone(),
            join: Some(join),
        })
    }
}
pub struct Worker {
    stop: Arc<AtomicBool>,
    wake: Arc<(Mutex<bool>, Condvar)>,
    join: Option<JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let guard = self.wake.0.lock().unwrap_or_else(|e| e.into_inner());
        self.stop.store(true, Ordering::Release);
        self.wake.1.notify_all();
        drop(guard);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
