//! A second presenter over the existing coordinator, not a provider poller.
use super::{Service, member_observations::MemberObservationLink};
use crate::{
    admission::MessagePermit,
    canonical,
    error::RemoteError,
    invitations::{
        port::{OwnerActor, OwnerDecision},
        service::{ProviderCleanup, State},
    },
    management::invitation_requests::{self, BrowserInvitationInput, OwnerInvitationPort},
};
use serde_json::{Value, json};

impl OwnerInvitationPort for MemberObservationLink {
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
        let result = (|| {
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
            let snapshot = observed
                .candidate
                .ok_or_else(invitation_requests::unavailable)?;
            if snapshot.context.offer != input.offer_id
                || canonical::base64url(&snapshot.digest) != input.snapshot
            {
                return Err(super::wire::invalid());
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
        // An effect error cannot establish a refusal. The existing original
        // journal is the only outcome, including commit/rollback uncertainty.
        let _ = result;
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
