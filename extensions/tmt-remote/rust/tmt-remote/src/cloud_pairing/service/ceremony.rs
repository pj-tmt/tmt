//! Local terminal composition of the shared ceremony. Firestore evidence enters
//! only through its typed port, and authority enters only through the Store sink.
use super::*;
use crate::cloud_pairing::{confirmation::FirestoreConfirmation, invitation::Firestore, wire::v2};
use crate::invitations::{
    BackendBinding, Context,
    port::{OwnerActor, OwnerDecision},
    service::{Coordinator, OwnerDecisionSource, State, Status},
};
use std::sync::OnceLock;
pub(super) type Ceremony = Coordinator<Firestore, FirestoreConfirmation>;

/// An unset slot is reserved preparation, not an idle or published offer.
#[derive(Default)]
pub(super) struct Slot {
    coordinator: OnceLock<Arc<Ceremony>>,
}
impl Slot {
    pub(super) fn current(&self) -> Option<Arc<Ceremony>> {
        self.coordinator.get().cloned()
    }
    pub(super) fn idle(&self) -> Result<bool, RemoteError> {
        let Some(coordinator) = self.current() else {
            return Ok(false);
        };
        let now = crate::pairing::now_ms()?;
        let status = coordinator.status(now)?;
        Ok(status.expires_at_ms.is_some_and(|expiry| now >= expiry)
            || matches!(
                status.state,
                State::Confirmed | State::Refused | State::Cancelled | State::Expired
            ))
    }
}

struct Reservation<'a> {
    service: &'a Service,
    slot: Arc<Slot>,
}
impl<'a> Reservation<'a> {
    fn reserve(service: &'a Service) -> Result<Self, RemoteError> {
        let previous = service
            .invitation
            .lock()
            .map_err(crate::store::database)?
            .clone();
        if let Some(previous) = &previous
            && !previous.idle()?
        {
            return Err(busy());
        }
        let slot = Arc::new(Slot::default());
        let mut handle = service.invitation.lock().map_err(crate::store::database)?;
        let unchanged = match (&previous, handle.as_ref()) {
            (None, None) => true,
            (Some(previous), Some(current)) => Arc::ptr_eq(previous, current),
            _ => false,
        };
        if !unchanged {
            return Err(busy());
        }
        *handle = Some(slot.clone());
        Ok(Self { service, slot })
    }
    fn activate(&self, coordinator: Arc<Ceremony>) -> Result<(), RemoteError> {
        self.slot
            .coordinator
            .set(coordinator)
            .map_err(|_| unavailable())
    }
    fn release(&self) -> Result<(), RemoteError> {
        let mut handle = self
            .service
            .invitation
            .lock()
            .map_err(crate::store::database)?;
        if handle
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &self.slot))
        {
            *handle = None;
        }
        Ok(())
    }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.slot.current().is_none() {
            let _ = self.release();
        }
    }
}
fn busy() -> RemoteError {
    RemoteError::new("REMOTE_PAIRING_BUSY", "An invite is already open.")
}

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
            .clone()
            .and_then(|slot| slot.current());
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
        // The reservation owns preparation without retaining the composition
        // mutex through provider I/O. A failed preparation releases only itself.
        let reservation = Reservation::reserve(self)?;
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
        let actor = OwnerActor::local_interactive();
        let io = IoBudget {
            deadline,
            cancelled: &self.stop,
        };
        let mut browser_completed = false;
        let result = (|| {
            let created = coordinator.create(&actor, context.clone(), now, &io);
            // Keep the slot reserved until create has established its state.
            // Even a lost publication answer must remain observable afterward.
            reservation.activate(coordinator.clone())?;
            created?;
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
            let token = invite.token()?;
            let mut event = json!({"event":"invite","code":token});
            let link = crate::deploy_record::read_project(&self.layout, project)
                .ok()
                .flatten()
                .and_then(|record| crate::invite_page::link(&record, &config, &token));
            if let Some(link) = link {
                event["link"] = json!(link);
                // The code remains available even at the private control bound.
                if serde_json::to_vec(&event)
                    .map_err(crate::store::database)?
                    .len()
                    + 1
                    > crate::limits::CLOUD_PAIR_CONTROL_BYTES
                {
                    event.as_object_mut().expect("invite event").remove("link");
                }
            }
            emit(event)?;
            // The presenter has its own clearing owner; the coordinator remains
            // the sole memory owner of the live offer and its code.
            drop(invite);
            drop(delivery);
            let mut selected = None;
            let mut terminal_decided = false;
            loop {
                let answer = emit(json!({"event":"waiting"}))?;
                let now = crate::pairing::now_ms()?;
                if self.browser_decided(&coordinator, &actor, now, &mut emit)? {
                    browser_completed = true;
                    return Ok(());
                }
                if now >= context.expires_at_ms {
                    return Err(self.invitation_incomplete(&coordinator, now)?);
                }
                let status = coordinator.observe(now, &io)?;
                if self.browser_decided(
                    &coordinator,
                    &actor,
                    crate::pairing::now_ms()?,
                    &mut emit,
                )? {
                    browser_completed = true;
                    return Ok(());
                }
                if selected.is_none()
                    && let Some(snapshot) = coordinator.candidate(now)?
                {
                    let answer = emit(json!({"event":"candidate","name":snapshot.candidate.name,
                        "words":canonical::fingerprint_words(&snapshot.candidate.public_key).map_err(|_| wire::invalid())?}))?;
                    selected = Some(snapshot);
                    if let Some(answer) = answer {
                        self.terminal_decision(
                            &coordinator,
                            &actor,
                            selected.as_ref().unwrap(),
                            answer,
                            &io,
                        )?;
                        terminal_decided = true;
                    }
                } else if !terminal_decided && let Some(answer) = answer {
                    if self.browser_decided(&coordinator, &actor, now, &mut emit)? {
                        browser_completed = true;
                        return Ok(());
                    }
                    self.terminal_decision(
                        &coordinator,
                        &actor,
                        selected.as_ref().ok_or_else(wire::invalid)?,
                        answer,
                        &io,
                    )?;
                    terminal_decided = true;
                } else if status.state == State::Confirmed {
                    let snapshot = selected.as_ref().ok_or_else(wire::invalid)?;
                    let grant = self
                        .store()?
                        .grant(&snapshot.candidate.machine)?
                        .ok_or_else(wire::invalid)?;
                    let member = self
                        .store()?
                        .member_for_client(&grant.client_id)?
                        .ok_or_else(wire::invalid)?;
                    let reference = self
                        .store()?
                        .cloud_connection(&grant.client_id)?
                        .ok_or_else(wire::invalid)?
                        .name;
                    emit(
                        json!({"event":"connected","name":grant.name,"talk":grant.permits_scope("talk"),
                        "clientId":grant.client_id,"memberId":member.member_id,"remote":reference}),
                    )?;
                    self.request_reconcile();
                    return Ok(());
                }
                if self.browser_decided(
                    &coordinator,
                    &actor,
                    crate::pairing::now_ms()?,
                    &mut emit,
                )? {
                    browser_completed = true;
                    return Ok(());
                }
                if let Err(error) = provider.wait(&io, crate::limits::CLOUD_POLL_MIN_MS) {
                    let now = crate::pairing::now_ms()?;
                    return Err(
                        if Instant::now() >= io.deadline || now >= context.expires_at_ms {
                            self.invitation_incomplete(&coordinator, now)?
                        } else {
                            error
                        },
                    );
                }
            }
        })();
        // A browser decision ends this presenter, not the shared ceremony.
        // The existing worker still owns route/receipt publication and expiry.
        if browser_completed {
            return result;
        }
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
            reservation.release()?;
        }
        result
    }
    fn terminal_decision(
        &self,
        coordinator: &Ceremony,
        actor: &OwnerActor,
        snapshot: &crate::invitations::port::Snapshot,
        answer: (bool, bool),
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        let now = crate::pairing::now_ms()?;
        let result = coordinator.confirm_owner_decision(
            actor,
            snapshot,
            OwnerDecision {
                matched: answer.0,
                enable_sending: answer.1,
            },
            &crate::pairing::PairingPolicy::default(),
            now,
            io,
        );
        if coordinator
            .owner_observation(actor, now)?
            .is_some_and(|o| o.decided_by == Some(OwnerDecisionSource::Browser))
        {
            return Ok(());
        }
        result?;
        if !answer.0 {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_PAIR_REFUSED",
                "The owner declined the connection.",
            ));
        }
        Ok(())
    }
    fn browser_decided(
        &self,
        coordinator: &Ceremony,
        actor: &OwnerActor,
        now: u64,
        emit: &mut impl FnMut(Value) -> Result<Option<(bool, bool)>, RemoteError>,
    ) -> Result<bool, RemoteError> {
        let Some(observed) = coordinator.owner_observation(actor, now)? else {
            return Ok(false);
        };
        if observed.decided_by != Some(OwnerDecisionSource::Browser) {
            return Ok(false);
        }
        let event = if let Some(committed) = observed.committed {
            let receipt: Value =
                serde_json::from_slice(&committed.receipt).map_err(crate::store::database)?;
            let scopes = receipt["grant"]["scopes"]
                .as_array()
                .ok_or_else(wire::invalid)?;
            json!({"event":"owner-decision","outcome":"confirmed","talk":scopes.iter().any(|scope| scope == "talk"),
                "memberId":committed.member,"receiptDelivered":observed.status.state == State::Confirmed})
        } else {
            json!({"event":"owner-decision","outcome":if observed.status.state == State::Refused { "refused" } else { "unconfirmed" }})
        };
        emit(event)?;
        self.request_reconcile();
        Ok(true)
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
