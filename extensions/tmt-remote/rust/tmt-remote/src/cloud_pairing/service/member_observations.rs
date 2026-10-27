//! Weak composition link: Cloud owns Messages/Operations, so observations cannot own Cloud.
use super::Service;
use crate::invitations::service::{ProviderCleanup, State};
use crate::members::observations::{
    MemberInvitationView, MemberInviteCleanup, MemberInvitePhase, MemberObservationSource,
    MemberRouteView,
};
use crate::{error::RemoteError, store::Store};
use std::sync::{Arc, OnceLock, Weak};

#[derive(Default)]
pub struct MemberObservationLink {
    service: OnceLock<Weak<Service>>,
}
impl MemberObservationLink {
    pub(super) fn current(&self) -> Option<Arc<Service>> {
        self.service.get().and_then(Weak::upgrade)
    }
    pub fn attach(&self, service: &Arc<Service>) -> Result<(), RemoteError> {
        self.service
            .set(Arc::downgrade(service))
            .map_err(|_| crate::store::database("member observations already attached"))
    }
}
impl MemberObservationSource for MemberObservationLink {
    fn invitation(&self) -> MemberInvitationView {
        self.service
            .get()
            .and_then(Weak::upgrade)
            .map_or(MemberInvitationView::Unavailable, |s| s.invitation())
    }
    fn routes(&self, store: &Store, member: Option<&str>) -> MemberRouteView {
        self.service
            .get()
            .and_then(Weak::upgrade)
            .map_or(MemberRouteView::Unavailable, |s| s.routes(store, member))
    }
}
impl MemberObservationSource for Service {
    fn invitation(&self) -> MemberInvitationView {
        match self.invitation_status() {
            Ok(None) => MemberInvitationView::NoCurrentOffer,
            Err(_) => MemberInvitationView::Unavailable,
            Ok(Some(status)) => MemberInvitationView::Current {
                expires_at_ms: status.expires_at_ms,
                phase: match status.state {
                    State::Open => MemberInvitePhase::Open,
                    State::CandidateAwaitingOwner => MemberInvitePhase::AwaitingOwner,
                    State::Confirmed => MemberInvitePhase::Confirmed,
                    State::Refused => MemberInvitePhase::Refused,
                    State::Cancelled => MemberInvitePhase::Cancelled,
                    State::Expired => MemberInvitePhase::Expired,
                    State::ProviderOutcomeUnconfirmed => MemberInvitePhase::Unconfirmed,
                },
                local_confirmation_known: status.local_confirmation_known,
                cleanup: match status.cleanup {
                    ProviderCleanup::NotRequested => MemberInviteCleanup::NotRequested,
                    ProviderCleanup::Acknowledged => MemberInviteCleanup::Acknowledged,
                    ProviderCleanup::Unconfirmed => MemberInviteCleanup::Unconfirmed,
                },
            },
        }
    }
    fn routes(&self, store: &Store, member: Option<&str>) -> MemberRouteView {
        store
            .member_route_view(member)
            .unwrap_or(MemberRouteView::Unavailable)
    }
}
