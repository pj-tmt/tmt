//! Join and original recovery use the same proof and receipt owners. Version 1
//! stays in the legacy Service path for its original remaining lifetime only.
use super::ceremony::remaining;
use super::*;
use crate::cloud_pairing::{
    adoption::ReceiptAdoption, confirmation::FirestoreConfirmation, invitation::Firestore, wire::v2,
};
use crate::invitations::{
    self, Context, port::OwnerActor, service::Coordinator, submission::SubmissionAttempt,
};
impl Service {
    pub(super) fn recover_invitation(
        &self,
        provider: &Provider,
        config: &Value,
        doc: &super::super::EnrollmentDocument,
        now: u64,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        // Malformed or unconfirmed submissions are not background work. The
        // current original journal is the only source of receipt authority.
        let prepared = self.invitation_original(provider, config, doc, now);
        let prepared = match prepared {
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "REMOTE_CLOUD_PAIR_INVALID"
                        | "REMOTE_INVITATION_INVALID"
                        | "REMOTE_ID_CONFLICT"
                ) =>
            {
                None
            }
            other => other?,
        };
        if let Some((backend, sink, snapshot)) = prepared {
            let current = self
                .invitation
                .lock()
                .map_err(crate::store::database)?
                .clone()
                .and_then(|slot| slot.current());
            if let Some(current) = current.filter(|current| {
                current.status(now).is_ok_and(|status| {
                    status.offer.as_deref() == Some(snapshot.context.offer.as_str())
                })
            }) {
                // Share the live memory owner so its status learns the same
                // durable acknowledgment. No Store guard is held here.
                current.observe(now, io)?;
            } else {
                let coordinator = Coordinator::new(backend, sink);
                coordinator.recover(&OwnerActor::local_interactive(), &snapshot, now, io)?;
            }
        }
        Ok(())
    }

    fn invitation_original(
        &self,
        provider: &Provider,
        config: &Value,
        doc: &super::super::EnrollmentDocument,
        now: u64,
    ) -> Result<
        Option<(
            Firestore,
            FirestoreConfirmation,
            invitations::port::Snapshot,
        )>,
        RemoteError,
    > {
        let value = crate::wire::strict_json(&doc.body).ok_or_else(wire::invalid)?;
        let context: Context =
            serde_json::from_value(value["context"].clone()).map_err(|_| wire::invalid())?;
        v2::validate_context(&context)?;
        if context.owner != self.machine
            || context.owner_key != self.key.public()
            || context.offer != doc.id
            || context.binding.reference != provider.identity().project.as_bytes()
            || context.binding.owner_principal != provider.uid().as_bytes()
            || now >= context.expires_at_ms
            || doc.created_ms >= context.expires_at_ms
            || context.issued_at_ms.saturating_sub(doc.created_ms)
                > crate::session::CLOCK_SKEW.as_millis() as u64
        {
            return Err(wire::invalid());
        }
        let submitted = v2::read_submission(
            &doc.body,
            &context,
            &doc.uid,
            doc.token.ok_or_else(wire::invalid)?,
        )?;
        let sink = FirestoreConfirmation::new(
            self.store.clone(),
            context.owner_key,
            config.clone(),
            provider.uid().into(),
        )?;
        let Some(original) = sink.captured(&context)? else {
            return Ok(None);
        };
        let digest = invitations::possession_digest(
            &context,
            &submitted.candidate,
            &submitted.mac,
            &submitted.signature,
        )?;
        if original.snapshot.candidate != submitted.candidate || original.snapshot.digest != digest
        {
            return Err(wire::invalid());
        }
        // Registration for this serving owner was already authenticated; do not
        // add a machine GET to every enrollment recovery observation.
        let backend = Firestore::new(
            Arc::new(self.provider(config, &self.machine, false)?),
            context,
            sink.clone(),
            self.store.clone(),
            self.wake.clone(),
        )?;
        Ok(Some((backend, sink, original.snapshot)))
    }

    pub fn join(
        &self,
        token: &str,
        emit: impl FnMut(Value) -> Result<Option<bool>, RemoteError>,
    ) -> Result<(), RemoteError> {
        if token.len() > crate::limits::CLOUD_INVITE_BYTES * 4 / 3 + 4 {
            return Err(wire::invalid());
        }
        let raw = canonical::base64url_decode(token).map_err(|_| wire::invalid())?;
        if raw.len() > crate::limits::CLOUD_INVITE_BYTES {
            return Err(wire::invalid());
        }
        let value = crate::wire::strict_json(&raw).ok_or_else(wire::invalid)?;
        match value["version"].as_u64() {
            Some(1) => self.join_legacy(token, emit),
            Some(2) => self.join_current(v2::Invite::read(token, crate::pairing::now_ms()?)?, emit),
            Some(_) => Err(wire::upgrade()),
            None => Err(wire::invalid()),
        }
    }
    fn join_current(
        &self,
        invite: v2::Invite,
        mut emit: impl FnMut(Value) -> Result<Option<bool>, RemoteError>,
    ) -> Result<(), RemoteError> {
        let context = &invite.context;
        let provider = Arc::new(self.provider(&invite.public_config, &context.owner, true)?);
        let previous = self.store()?.cloud_connection(&context.owner)?;
        let descriptor_previous = previous
            .as_ref()
            .filter(|old| old.owner_key == context.owner_key)
            .map_or(&[][..], |old| old.descriptor.as_slice());
        let (backend, _, descriptor) = self.invitation_backend(
            provider.clone(),
            context,
            &invite.public_config,
            descriptor_previous,
            &self.io(),
        )?;
        let io = IoBudget {
            deadline: remaining(context)?,
            cancelled: &self.stop,
        };
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| unavailable())?;
        let candidate = invitations::Candidate {
            machine: self.machine.clone(),
            public_key: self.key.public(),
            principal: provider.uid().as_bytes().to_vec(),
            name: std::env::var("USER")
                .ok()
                .filter(|name| canonical::device_name(name))
                .unwrap_or("This machine".into()),
            nonce,
        };
        let connection = Connection {
            version: 1,
            name: self.connection_name(&context.owner, &invite.machine_name)?,
            owner: context.owner.clone(),
            owner_key: context.owner_key,
            config: invite.public_config.clone(),
            peer: self.machine.clone(),
            uid: provider.uid().into(),
            request: Vec::new(),
            confirmed: false,
            descriptor: Vec::new(),
            reroute_token: None,
            pending_submission: None,
        };
        let original = previous.as_ref().and_then(|old| {
            old.pending_submission
                .as_ref()
                .filter(|pending| pending.context == *context)
                .map(|pending| pending.request.clone())
                .or_else(|| {
                    (old.confirmed
                        && crate::wire::strict_json(&old.request).is_some_and(|body| {
                            body["version"] == 2 && body["context"]["offer"] == context.offer
                        }))
                    .then(|| old.request.clone())
                })
        });
        let make_resumed = |raw: &[u8]| {
            invite.with_code(|code| {
                let signed = v2::read_submission(
                    raw,
                    context,
                    provider.uid(),
                    invitations::service::gate(context, code)?,
                )?;
                SubmissionAttempt::resume_original(
                    backend.clone(),
                    context.clone(),
                    signed,
                    code,
                    crate::pairing::now_ms()?,
                )
            })
        };
        let mut attempt = if let Some(original) = original {
            make_resumed(&original)?
        } else {
            invite.with_code(|code| {
                SubmissionAttempt::new(
                    backend.clone(),
                    context.clone(),
                    candidate,
                    code,
                    &self.key,
                    crate::pairing::now_ms()?,
                )
            })?
        };
        let submitted = attempt.signed_submission().clone();
        if !self
            .store()?
            .claim_cloud_submission(&connection, context, &submitted, &io)?
        {
            attempt = make_resumed(&v2::submission(context, &submitted)?)?;
        }
        emit(
            json!({"event":"joining","name":invite.machine_name,"words":canonical::fingerprint_words(&self.key.public()).map_err(|_| wire::invalid())?}),
        )?;
        attempt.submit(&io)?;
        loop {
            let adopted = attempt.observe_receipt_with(&io, |bytes, key, budget| {
                let needs_choice = {
                    let store = self.store()?;
                    store
                        .grant(&context.owner)?
                        .is_none_or(|grant| grant.disabled)
                };
                let sending = if needs_choice {
                    emit(json!({"event":"reverse-permission","name":invite.machine_name}))?
                        .unwrap_or(false)
                } else {
                    false
                };
                let mut verified_connection = connection.clone();
                verified_connection.descriptor = descriptor.bytes().to_vec();
                self.store()?.adopt_cloud_receipt(
                    ReceiptAdoption {
                        context,
                        submission: &submitted,
                        receipt: bytes,
                        key,
                        connection: &verified_connection,
                        owner_name: &invite.machine_name,
                        enable_sending: sending,
                    },
                    budget,
                )
            })?;
            if let Some(confirmed) = adopted {
                let local = self.provider(&confirmed.config, &self.machine, false)?;
                let now = crate::pairing::now_ms()?;
                self.keys.maintain(now)?;
                let descriptor = self.keys.advertised(now)?;
                self.register(&local, descriptor.bytes(), Some(&context.owner), &io)?;
                self.flush(&local, &confirmed.config, &io)?;
                let receipt = attempt.observe_receipt(&io)?.ok_or_else(wire::invalid)?;
                let receipt = crate::wire::strict_json(&receipt).ok_or_else(wire::invalid)?;
                let talk = receipt["grant"]["scopes"]
                    .as_array()
                    .is_some_and(|scopes| scopes.iter().any(|scope| scope == "talk"));
                emit(
                    json!({"event":"connected","name":invite.machine_name,"talk":talk,"remote":confirmed.name}),
                )?;
                self.request_reconcile();
                return Ok(());
            }
            provider.wait(&io, crate::limits::CLOUD_POLL_MIN_MS)?;
        }
    }
}
