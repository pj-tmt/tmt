//! Optional local observations. Implementations must not call a provider or schedule work.
use crate::store::Store;
use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MemberInvitePhase {
    Open,
    AwaitingOwner,
    Confirmed,
    Refused,
    Cancelled,
    Expired,
    Unconfirmed,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MemberInviteCleanup {
    NotRequested,
    Acknowledged,
    Unconfirmed,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum MemberInvitationView {
    Unavailable,
    NoCurrentOffer,
    Current {
        expires_at_ms: Option<u64>,
        phase: MemberInvitePhase,
        local_confirmation_known: bool,
        cleanup: MemberInviteCleanup,
    },
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum MemberRouteView {
    Unavailable,
    NotRecorded,
    Recorded {
        publication_pending: bool,
        removal_pending: bool,
    },
}
pub trait MemberObservationSource: Send + Sync {
    /// Captured before any live/Store guard; may observe the existing coordinator.
    fn invitation(&self) -> MemberInvitationView;
    /// Aggregate only existing route facts under this serving Store. No second opener.
    /// `member=None` is the admitted administrator view; otherwise restrict to that member.
    fn routes(&self, store: &Store, member: Option<&str>) -> MemberRouteView;
}
pub struct NoMemberObservations;
impl MemberObservationSource for NoMemberObservations {
    fn invitation(&self) -> MemberInvitationView {
        MemberInvitationView::Unavailable
    }
    fn routes(&self, _: &Store, _: Option<&str>) -> MemberRouteView {
        MemberRouteView::Unavailable
    }
}
