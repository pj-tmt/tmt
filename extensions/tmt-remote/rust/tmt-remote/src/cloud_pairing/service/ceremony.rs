//! Local terminal composition of the shared ceremony. Firestore evidence enters
//! only through its typed port, and authority enters only through the Store sink.
use super::*;
use crate::cloud_pairing::{confirmation::FirestoreConfirmation, invitation::Firestore, wire::v2};
use crate::invitations::{
    BackendBinding, Context,
    port::{OwnerActor, OwnerDecision},
    service::{Coordinator, State, Status},
};
pub(super) type Ceremony = Coordinator<Firestore, FirestoreConfirmation>;

pub(super) fn remaining(context: &Context) -> Result<Instant, RemoteError> {
    let now = crate::pairing::now_ms()?;
    if now >= context.expires_at_ms {
        return Err(wire::expired());
    }
    Ok(Instant::now() + Duration::from_millis(context.expires_at_ms - now))
}
impl Service {
    /// Read-only memory observation. Consumers capture it before taking Store;
    /// it performs no provider I/O and is not a durable provider-state ledger.
    pub fn invitation_status(&self) -> Result<Option<Status>, RemoteError> {
        let coordinator = self
            .invitation
            .lock()
            .map_err(crate::store::database)?
            .clone();
        coordinator
            .map(|coordinator| coordinator.status(crate::pairing::now_ms()?))
            .transpose()
    }
    pub(super) fn invitation_backend(
        &self,
        provider: Arc<Provider>,
        context: &Context,
        config: &Value,
        previous: &[u8],
        io: &IoBudget<'_>,
    ) -> Result<(Firestore, FirestoreConfirmation, VerifiedDescriptor), RemoteError> {
        let row = run(provider.machine_metadata(io), io)?.ok_or_else(wire::invalid)?;
        let descriptor = self.verify_descriptor(&provider, &context.owner_key, previous, &row)?;
        let fields = Self::machine_fields(&provider, &context.owner_key, &row)?;
        if fields["uid"]["stringValue"].as_str().map(str::as_bytes)
            != Some(context.binding.owner_principal.as_slice())
        {
            return Err(wire::invalid());
        }
        let sink = FirestoreConfirmation::new(
            self.store.clone(),
            context.owner_key,
            config.clone(),
            std::str::from_utf8(&context.binding.owner_principal)
                .map_err(|_| wire::invalid())?
                .into(),
        )?;
        let backend = Firestore::new(
            provider,
            context.clone(),
            sink.clone(),
            self.store.clone(),
            self.wake.clone(),
        )?;
        Ok((backend, sink, descriptor))
    }
    pub(super) fn invite_selected(
        &self,
        project: &str,
        config: Value,
        mut emit: impl FnMut(Value) -> Result<Option<(bool, bool)>, RemoteError>,
    ) -> Result<(), RemoteError> {
        // Hold only the handle reservation through preparation. Coordinator
        // calls and the terminal loop run after releasing this composition lock.
        let mut handle = self.invitation.lock().map_err(crate::store::database)?;
        if let Some(previous) = handle.as_ref() {
            let now = crate::pairing::now_ms()?;
            let status = previous.status(now)?;
            if status.expires_at_ms.is_none_or(|expiry| now < expiry)
                && !matches!(
                    status.state,
                    State::Confirmed | State::Refused | State::Cancelled | State::Expired
                )
            {
                return Err(RemoteError::new(
                    "REMOTE_PAIRING_BUSY",
                    "An invite is already open.",
                ));
            }
        }
        let provider = Arc::new(self.provider(&config, &self.machine, true)?);
        if provider.identity().project != project {
            return Err(wire::invalid());
        }
        let now = crate::pairing::now_ms()?;
        self.keys.maintain(now)?;
        let descriptor = self.keys.advertised(now)?;
        self.register(&provider, descriptor.bytes(), None, &self.io())?;
        self.store()?.remember_cloud_owner(project, &config)?;
        let _ = self.cleanup_invites(&provider, now, &self.io());
        let now = crate::pairing::now_ms()?;
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| unavailable())?;
        let context = Context {
            binding: BackendBinding {
                backend: "firestore".into(),
                reference: project.as_bytes().to_vec(),
                owner_principal: provider.uid().as_bytes().to_vec(),
            },
            owner: self.machine.clone(),
            owner_key: self.key.public(),
            offer: crate::store::uuid_v4()?,
            nonce,
            issued_at_ms: now,
            expires_at_ms: now + crate::pairing::Timing::CONTRACT.lifetime.as_millis() as u64,
        };
        let (backend, sink, _) =
            self.invitation_backend(provider.clone(), &context, &config, &[], &self.io())?;
        let coordinator = Arc::new(Coordinator::new(backend, sink));
        let deadline = remaining(&context)?;
        *handle = Some(coordinator.clone());
        drop(handle);
        let actor = OwnerActor::local_interactive();
        let io = IoBudget {
            deadline,
            cancelled: &self.stop,
        };
        let result = (|| {
            coordinator.create(&actor, context.clone(), now, &io)?;
            let delivery = coordinator.deliver(&actor, crate::pairing::now_ms()?)?;
            let owner_name = std::env::var("USER")
                .ok()
                .filter(|name| canonical::device_name(name))
                .unwrap_or("Owner".into());
            let invite = v2::Invite::from_delivery(
                &delivery,
                config.clone(),
                owner_name,
                crate::pairing::now_ms()?,
            )?;
            emit(json!({"event":"invite","code":invite.token()?}))?;
            // The presenter has its own clearing owner; the coordinator remains
            // the sole memory owner of the live offer and its code.
            drop(invite);
            drop(delivery);
            let mut selected = None;
            loop {
                emit(json!({"event":"waiting"}))?;
                let now = crate::pairing::now_ms()?;
                if now >= context.expires_at_ms {
                    return Err(self.invitation_incomplete(&coordinator, now)?);
                }
                let status = coordinator.observe(now, &io)?;
                if selected.is_none()
                    && let Some(snapshot) = coordinator.candidate(now)?
                {
                    let (matched,sending)=emit(json!({"event":"candidate","name":snapshot.candidate.name,
                        "words":canonical::fingerprint_words(&snapshot.candidate.public_key).map_err(|_| wire::invalid())?}))?.ok_or_else(wire::invalid)?;
                    selected = Some(snapshot.clone());
                    coordinator.confirm_owner_decision(
                        &actor,
                        &snapshot,
                        OwnerDecision {
                            matched,
                            enable_sending: sending,
                        },
                        &crate::pairing::PairingPolicy::default(),
                        crate::pairing::now_ms()?,
                        &io,
                    )?;
                    if !matched {
                        return Err(RemoteError::new(
                            "REMOTE_CLOUD_PAIR_REFUSED",
                            "The owner declined the connection.",
                        ));
                    }
                } else if status.state == State::Confirmed {
                    // The exact original snapshot is held by the coordinator;
                    // the confirmed connection supplies the stable public name.
                    let snapshot = selected.as_ref().ok_or_else(wire::invalid)?;
                    let grant = self
                        .store()?
                        .grant(&snapshot.candidate.machine)?
                        .ok_or_else(wire::invalid)?;
                    let unique = self
                        .store()?
                        .active_grants()?
                        .iter()
                        .filter(|g| g.name == grant.name)
                        .count()
                        == 1;
                    let reference = self
                        .store()?
                        .cloud_connection(&grant.client_id)?
                        .ok_or_else(wire::invalid)?
                        .name;
                    emit(
                        json!({"event":"connected","name":grant.name,"talk":grant.permits_scope("talk"),"clientId":grant.client_id,
                        "hintPerson":if unique {&grant.name}else{&grant.client_id},"remote":reference}),
                    )?;
                    self.request_reconcile();
                    return Ok(());
                }
                provider.wait(&io, crate::limits::CLOUD_POLL_MIN_MS)?;
            }
        })();
        let cancel = AtomicBool::new(false);
        let cleanup_io = IoBudget {
            deadline: Instant::now() + crate::limits::CLOUD_RECONCILE_CALL,
            cancelled: &cancel,
        };
        if coordinator
            .disconnect(&context.offer, crate::pairing::now_ms()?, &cleanup_io)
            .is_err()
        {
            let _ = coordinator.withdraw(&actor, &context.offer, &cleanup_io);
        }
        if coordinator
            .status(crate::pairing::now_ms()?)
            .is_ok_and(|status| status.offer.is_none())
        {
            let mut handle = self.invitation.lock().map_err(crate::store::database)?;
            if handle
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &coordinator))
            {
                *handle = None;
            }
        }
        result
    }
    fn invitation_incomplete(
        &self,
        coordinator: &Ceremony,
        now: u64,
    ) -> Result<RemoteError, RemoteError> {
        let status = coordinator.status(now)?;
        Ok(if status.local_confirmation_known {
            RemoteError::new(
                "REMOTE_CLOUD_UNCONFIRMED",
                "The member was confirmed locally, but the original receipt was not delivered before expiry. Remove this member and send a new invite.",
            )
        } else if status.state == State::ProviderOutcomeUnconfirmed {
            RemoteError::new(
                "REMOTE_CLOUD_UNCONFIRMED",
                "The original confirmation outcome could not be established before expiry. Check member status before sending a new invite.",
            )
        } else {
            wire::expired()
        })
    }
}
