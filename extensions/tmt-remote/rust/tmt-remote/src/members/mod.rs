//! Member identity and policy over the existing grant/Store authority owner.
pub mod observations;
pub(crate) mod policy;
mod projection;
pub mod reads;
pub mod service;
mod store;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    User,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Member {
    pub remote_id: String,
    pub member_id: String,
    pub role: Role,
    pub display_name: String,
    pub sending_enabled: bool,
    pub revision: u64,
    pub created_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
}
/// LocalOwner is only for the existing owner-only local control path. Browser
/// callers must supply their signed, current designated grant; a role is not proof.
pub enum Owner<'a> {
    LocalOwner,
    DesignatedBrowser { client_id: &'a str, revision: u64 },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Association {
    pub operation_id: String,
    pub remote_id: String,
    pub member_id: String,
    pub member_revision: u64,
    pub client_id: String,
    pub grant_revision: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AssociationOutcome {
    pub member: Member,
    pub client_id: String,
    pub grant_revision: u64,
}
/// Callers release the Store lock before ending every affected live session.
#[derive(Debug)]
pub struct Change {
    pub member: Member,
    pub clients: Vec<String>,
}

pub(crate) use store::for_client as member_for_client_in;
pub(crate) use store::require_owner as authorize_owner_in;
pub(crate) use store::{
    confirm_member_grant_in, grants_for_client, insert_mapping, migrate_legacy, revoked,
    sending_for_client, sync_designation,
};
