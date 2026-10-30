//! Signed owner-browser invitation actions over the existing CLI coordinator.
use super::{Service, member_observations::MemberObservationLink};
use crate::{
    admission::MessagePermit,
    canonical,
    error::RemoteError,
    invitations::{
        port::{OwnerActor, OwnerDecision, OwnerDecisionFailure},
        service::{ProviderCleanup, State},
    },
    management::invitation_requests::{
        self, BrowserInvitationInput, CancelInvitationInput, CreateInvitationInput,
        IssuedInvitationPage, OwnerInvitationPort,
    },
};
use serde_json::{Value, json};

impl OwnerInvitationPort for MemberObservationLink {
    fn create(
        &self,
        permit: &MessagePermit,
        input: &CreateInvitationInput,
    ) -> Result<Value, RemoteError> {
        self.current()
            .ok_or_else(invitation_requests::unavailable)?
            .create(permit, input)
    }
    fn list(
        &self,
        permit: &MessagePermit,
        input: &IssuedInvitationPage,
    ) -> Result<Value, RemoteError> {
        self.current()
            .ok_or_else(invitation_requests::unavailable)?
            .list(permit, input)
    }
    fn cancel(
        &self,
        permit: &MessagePermit,
        input: &CancelInvitationInput,
    ) -> Result<Value, RemoteError> {
        self.current()
            .ok_or_else(invitation_requests::unavailable)?
            .cancel(permit, input)
    }

    fn pending(&self, permit: &MessagePermit) -> Result<Value, RemoteError> {
        self.current()
            .ok_or_else(invitation_requests::unavailable)?
            .pending(permit)
    }
    fn decide(
        &self,
        permit: &MessagePermit,
        input: &BrowserInvitationInput,
    ) -> Result<Value, RemoteError> {
        self.current()
            .ok_or_else(invitation_requests::unavailable)?
            .decide(permit, input)
    }
}
fn actor(permit: &MessagePermit) -> Result<OwnerActor, RemoteError> {
    OwnerActor::owner_browser(
        permit.grant.client_id.clone(),
        permit.grant.public_key,
        permit.grant.revision,
    )
    .with_admission(permit)
}
fn phase(state: State) -> &'static str {
    match state {
        State::Open => "open",
        State::CandidateAwaitingOwner => "awaiting-owner",
        State::Confirmed => "confirmed",
        State::Refused => "refused",
        State::Cancelled => "cancelled",
        State::Expired => "expired",
        State::ProviderOutcomeUnconfirmed => "unconfirmed",
    }
}
impl OwnerInvitationPort for Service {
    fn create(
        &self,
        permit: &MessagePermit,
        input: &CreateInvitationInput,
    ) -> Result<Value, RemoteError> {
        invitation_requests::authorize(permit)?;
        let intent = crate::journal::intent(&permit.message)?;
        if let Some(previous) = permit.with_store(|store| {
            store.adopt_invitation_action(
                &permit.grant,
                &input.operation_id,
                "remote.invites.create",
                &intent,
            )
        })? {
            return Ok(previous);
        }
        let created = (|| {
            let selected = crate::remote_selector::client(&self.layout, None, None)?;
            if !matches!(selected.kind, crate::remote_selector::ClientKind::Owned) {
                return Err(invitation_requests::unavailable());
            }
            invitation_requests::authorize(permit)?;
            let (reservation, _, context, coordinator) =
                self.prepare_invitation(&selected.project, &selected.public_config)?;
            let actor = OwnerActor::owner_browser_issuance(
                &permit.grant,
                input.operation_id.clone(),
                intent,
            )?
            .with_admission(permit)?;
            let created =
                coordinator.create(&actor, context, crate::pairing::now_ms()?, &self.io());
            reservation.activate(coordinator.clone())?;
            self.request_reconcile();
            created?;
            invitation_requests::authorize(permit)?;
            let event = self.invitation_event(
                &coordinator,
                &actor,
                &selected.public_config,
                &selected.project,
            )?;
            Ok::<_, RemoteError>(
                json!({"code":event["code"],"link":event.get("link").cloned().unwrap_or(Value::Null)}),
            )
        })();
        // The sink committed metadata with the adopted original before publishing.
        // Any failure or lost response therefore recovers status, never the code.
        let outcome = match &created {
            Err(error) => {
                let reason = match error.code.as_str() {
                    "REMOTE_PAIRING_BUSY" | "REMOTE_INVITATION_BUSY" => "REMOTE_INVITATION_BUSY",
                    "REMOTE_INVITATION_CAPACITY" => "REMOTE_INVITATION_CAPACITY",
                    "REMOTE_INVITATION_STOPPED" => "REMOTE_INVITATION_STOPPED",
                    "REMOTE_MANAGEMENT_READ_ONLY" => "REMOTE_MANAGEMENT_READ_ONLY",
                    "REMOTE_INTENT_CONFLICT" => "REMOTE_INTENT_CONFLICT",
                    _ => "REMOTE_INVITATION_UNAVAILABLE",
                };
                permit.with_store(|store| {
                    store.settle_invitation_action(
                        &permit.grant,
                        &input.operation_id,
                        "remote.invites.create",
                        &intent,
                        json!({"operationId":input.operation_id,"state":"refused","reason":reason}),
                    )
                })?
            }
            Ok(_) => permit.with_store(|store| {
                crate::management::receipt(
                    &store.connection,
                    &permit.grant.client_id,
                    &input.operation_id,
                    crate::pairing::now_ms()?,
                )
            })?,
        };
        invitation_requests::authorize(permit)?;
        let mut outcome = outcome;
        if let Ok(secret) = created {
            outcome["secret"] = secret;
        }
        Ok(outcome)
    }
    fn list(
        &self,
        permit: &MessagePermit,
        input: &IssuedInvitationPage,
    ) -> Result<Value, RemoteError> {
        invitation_requests::authorize(permit)?;
        self.invitation_status()?;
        let page = permit.with_store(|store| {
            store.invitation_history(input.cursor.as_deref(), crate::pairing::now_ms()?)
        })?;
        invitation_requests::authorize(permit)?;
        serde_json::to_value(page).map_err(crate::store::database)
    }
    fn cancel(
        &self,
        permit: &MessagePermit,
        input: &CancelInvitationInput,
    ) -> Result<Value, RemoteError> {
        invitation_requests::authorize(permit)?;
        let intent = crate::journal::intent(&permit.message)?;
        if let Some(previous) = permit.with_store(|store| {
            store.adopt_invitation_action(
                &permit.grant,
                &input.operation_id,
                "remote.invites.cancel",
                &intent,
            )
        })? {
            return Ok(previous);
        }
        let current = self
            .invitation
            .lock()
            .map_err(crate::store::database)?
            .as_ref()
            .and_then(|slot| slot.current());
        let mut cancelled = false;
        if let Some(current) = current {
            let status = current.status(crate::pairing::now_ms()?)?;
            if status.offer.as_deref() == Some(input.offer_id.as_str())
                && !status.local_confirmation_known
                && matches!(
                    status.state,
                    State::Open | State::CandidateAwaitingOwner | State::ProviderOutcomeUnconfirmed
                )
            {
                cancelled = match current.cancel(
                    &actor(permit)?,
                    &input.offer_id,
                    crate::pairing::now_ms()?,
                    &self.io(),
                ) {
                    Ok(_) => true,
                    Err(error) if error.code == "REMOTE_INVITATION_INVALID" => false,
                    Err(error) => return Err(error),
                };
            }
        }
        invitation_requests::authorize(permit)?;
        let (state, code) = if cancelled {
            ("cancelled", "REMOTE_INVITATION_CANCELLED")
        } else {
            ("not-active", "REMOTE_INVITATION_NOT_ACTIVE")
        };
        let outcome = json!({"operationId":input.operation_id,"state":"committed","sessionEnded":false,
            "result":{"invitation":{"offerId":input.offer_id,"state":state,"code":code}}});
        permit.with_store(|store| {
            store.settle_invitation_action(
                &permit.grant,
                &input.operation_id,
                "remote.invites.cancel",
                &intent,
                outcome,
            )
        })
    }

    fn pending(&self, permit: &MessagePermit) -> Result<Value, RemoteError> {
        invitation_requests::authorize(permit)?;
        let current = self
            .invitation
            .lock()
            .map_err(crate::store::database)?
            .as_ref()
            .and_then(|slot| slot.current());
        let Some(current) = current else {
            return Ok(json!({"state":"none"}));
        };
        let observed = current.owner_observation(&actor(permit)?, crate::pairing::now_ms()?)?;
        let Some(observed) = observed else {
            return Ok(json!({"state":"none"}));
        };
        let candidate = observed
            .candidate
            .map(|snapshot| {
                Ok::<_, RemoteError>(json!({
                    "clientId":snapshot.candidate.machine,"name":snapshot.candidate.name,
                    "words":canonical::fingerprint_words(&snapshot.candidate.public_key)
                        .map_err(|_| super::wire::invalid())?,
                    "snapshot":canonical::base64url(&snapshot.digest),
                }))
            })
            .transpose()?;
        invitation_requests::authorize(permit)?;
        Ok(json!({"state":"current","offerId":observed.status.offer,
            "expiresAtMs":observed.status.expires_at_ms,"phase":phase(observed.status.state),
            "localConfirmationKnown":observed.status.local_confirmation_known,
            "cleanup":match observed.status.cleanup { ProviderCleanup::NotRequested => "not-requested",
                ProviderCleanup::Acknowledged => "acknowledged", ProviderCleanup::Unconfirmed => "unconfirmed" },
            "candidate":candidate}))
    }
    fn decide(
        &self,
        permit: &MessagePermit,
        input: &BrowserInvitationInput,
    ) -> Result<Value, RemoteError> {
        invitation_requests::authorize(permit)?;
        let intent = crate::journal::intent(&permit.message)?;
        // An adopted ID is lookup-only even if the current offer has ended or changed.
        if let Some(previous) = permit.with_store(|store| {
            store.adopt_invitation_confirmation(&permit.grant, &input.operation_id, &intent)
        })? {
            return Ok(previous);
        }
        let result: Result<(), OwnerDecisionFailure> = (|| {
            let current = self
                .invitation
                .lock()
                .map_err(crate::store::database)?
                .as_ref()
                .and_then(|slot| slot.current())
                .ok_or_else(invitation_requests::unavailable)?;
            let observed = current
                .owner_observation(&actor(permit)?, crate::pairing::now_ms()?)?
                .ok_or_else(invitation_requests::unavailable)?;
            let snapshot = observed.candidate.ok_or_else(|| {
                if observed.decided_by.is_some() {
                    crate::invitations::service::already_decided()
                } else {
                    invitation_requests::unavailable()
                }
            })?;
            if snapshot.context.offer != input.offer_id
                || canonical::base64url(&snapshot.digest) != input.snapshot
            {
                return Err(crate::invitations::invalid().into());
            }
            let decision = OwnerDecision {
                matched: input.matched,
                enable_sending: input.sending,
            };
            let actor = OwnerActor::owner_browser_decision(
                &permit.grant,
                input.operation_id.clone(),
                intent,
                &snapshot,
                decision,
            )?
            .with_admission(permit)?;
            current.confirm_owner_decision(
                &actor,
                &snapshot,
                decision,
                &crate::pairing::PairingPolicy::default(),
                crate::pairing::now_ms()?,
                &self.io(),
            )?;
            Ok(())
        })();
        match result {
            Err(OwnerDecisionFailure::Refused(error)) => {
                // This operation made no effect. Preserve its adopted actor,
                // intent/deadline and any already recorded outcome.
                self.store()?.refuse_invitation_confirmation(
                    &permit.grant,
                    &input.operation_id,
                    &intent,
                    &error.code,
                )?;
            }
            Ok(())
            | Err(OwnerDecisionFailure::CommitUnknown(_) | OwnerDecisionFailure::Recorded(_)) => {}
        }
        // Recorded decisions and uncertain commits recover only their original.
        permit.with_store(|store| {
            crate::management::receipt(
                &store.connection,
                &permit.grant.client_id,
                &input.operation_id,
                crate::pairing::now_ms()?,
            )
        })
    }
}
