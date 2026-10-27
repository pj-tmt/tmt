//! Version-2 Firestore additions to the existing invitation confirmation
//! transaction. No provider I/O, policy writer or receipt ledger lives here.
use super::{Connection, RouteChange, store, wire};
use crate::{
    error::RemoteError,
    invitations::{
        Context,
        port::*,
        store::{ConfirmationExtras, StoreConfirmation},
    },
    limits,
    store::{Store, database},
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct FirestoreConfirmation {
    common: StoreConfirmation,
    config: Value,
    uid: String,
}

/// Captured inside the original management receipt. The desired-route owner
/// remains the sole publisher and ledger; this is its immutable ACK capability.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutePublication {
    pub offer: String,
    pub snapshot: [u8; 32],
    pub member: String,
    pub rows: Vec<RouteChange>,
}
impl RoutePublication {
    pub fn read(bytes: &[u8], context: &Context, snapshot: &[u8; 32]) -> Result<Self, RemoteError> {
        if bytes.len() > limits::CLOUD_ENROLLMENT_BYTES {
            return Err(wire::invalid());
        }
        let value = crate::wire::strict_json(bytes).ok_or_else(wire::invalid)?;
        let capture: Self = serde_json::from_value(value).map_err(|_| wire::invalid())?;
        if capture.offer != context.offer
            || capture.snapshot != *snapshot
            || crate::canonical::uuid(&capture.member).is_err()
            || capture.rows.is_empty()
            || capture.rows.len() > limits::CLOUD_ROUTE_BATCH
            || capture.rows.iter().any(|row| {
                row.project.as_bytes() != context.binding.reference
                    || row.member.as_deref() != Some(&capture.member)
                    || row.reroute_hash.is_none()
                    || !super::uid_ok(&row.uid)
                    || row.generation == 0
                    || row.generation >= 1 << 53
            })
            || capture.rows.iter().any(|row| {
                row.peer != capture.rows[0].peer || row.reroute_hash != capture.rows[0].reroute_hash
            })
            || capture.rows.iter().filter(|row| !row.revoked).count() != 1
            || crate::canonical::uuid(&capture.rows[0].peer).is_err()
        {
            return Err(wire::invalid());
        }
        Ok(capture)
    }
}
impl FirestoreConfirmation {
    pub fn new(
        store: Arc<Mutex<Store>>,
        owner_key: [u8; 32],
        config: Value,
        uid: String,
    ) -> Result<Self, RemoteError> {
        crate::firebase_auth::Project::from_public_config(&config).map_err(|_| wire::invalid())?;
        if !super::uid_ok(&uid) {
            return Err(wire::invalid());
        }
        Ok(Self {
            common: StoreConfirmation::new(store, owner_key)?,
            config,
            uid,
        })
    }
    pub(crate) fn captured(&self, context: &Context) -> Result<Option<Committed>, RemoteError> {
        self.common.captured(context)
    }
}
impl ConfirmationSink for FirestoreConfirmation {
    fn authorize(&self, context: &Context, actor: &OwnerActor) -> Result<(), RemoteError> {
        wire::v2::validate_context(context)?;
        if context.binding.owner_principal != self.uid.as_bytes()
            || context.binding.reference
                != self.config["projectId"]
                    .as_str()
                    .ok_or_else(wire::invalid)?
                    .as_bytes()
        {
            return Err(wire::invalid());
        }
        self.common.authorize(context, actor)
    }
    fn decline(
        &self,
        actor: &OwnerActor,
        snapshot: &Snapshot,
        io: &crate::objects::IoBudget<'_>,
    ) -> Result<(), ConfirmationFailure> {
        self.authorize(&snapshot.context, actor)
            .map_err(ConfirmationFailure::Refused)?;
        self.common.decline(actor, snapshot, io)
    }
    fn confirm(&self, input: Confirmation<'_>) -> Result<Committed, ConfirmationFailure> {
        self.authorize(&input.snapshot.context, input.actor)
            .map_err(ConfirmationFailure::Refused)?;
        self.common.confirm_with(input, |tx, input, member| {
            let snapshot = input.snapshot;
            let context = &snapshot.context;
            let candidate = &snapshot.candidate;
            let project = std::str::from_utf8(&context.binding.reference).map_err(|_| wire::invalid())?;
            let uid = std::str::from_utf8(&candidate.principal).map_err(|_| wire::invalid())?;
            if !super::uid_ok(uid) { return Err(wire::invalid()); }
            let (mut forward, reverse) = wire::v2::reroute_tokens(context, candidate, input.receipt_key)?;
            let hash: [u8; 32] = Sha256::digest(forward).into();
            let token = crate::canonical::base64url(&forward);
            forward.fill(0);
            let reciprocal = Connection {
                pending_submission: None,
                version: 1,
                name: if crate::remote_catalog::name_ok(&candidate.name) { candidate.name.clone() } else { candidate.machine.clone() },
                owner: candidate.machine.clone(), owner_key: candidate.public_key,
                config: self.config.clone(), peer: context.owner.clone(), uid: self.uid.clone(),
                request: Vec::new(), confirmed: true, descriptor: Vec::new(), reroute_token: Some(reverse),
            };
            store::adopt_cloud_routes_in(tx, store::PeerRoutes {
                project, owner: &context.owner, owner_key: &context.owner_key,
                peer: &candidate.machine, peer_key: &candidate.public_key, uid,
                reroute_hash: &hash, reciprocal: Some(reciprocal),
            }, store::RouteAdoption::NewMember)?;
            tx.execute("INSERT INTO cloud_owner_projects VALUES(?1,?2) ON CONFLICT(project) DO UPDATE SET config=excluded.config", params![project,self.config.to_string()]).map_err(database)?;
            let capture = RoutePublication {
                offer: context.offer.clone(), snapshot: snapshot.digest, member: member.member_id.clone(),
                rows: store::cloud_route_changes_in(tx)?.into_iter().filter(|row| row.project == project && row.peer == candidate.machine).collect(),
            };
            let routes = serde_json::to_vec(&capture).map_err(database)?;
            RoutePublication::read(&routes, context, &snapshot.digest)?;
            Ok(ConfirmationExtras {
                receipt_fields: serde_json::from_value(json!({"rerouteToken":token})).map_err(database)?,
                routes: Some(routes),
            })
        })
    }
    fn original(&self, snapshot: &Snapshot) -> Result<Option<Committed>, RemoteError> {
        self.common.original(snapshot)
    }
    fn claim(&self, committed: &Committed, part: Part) -> Result<Claim, RemoteError> {
        self.common.claim(committed, part)
    }
    fn acknowledge(&self, committed: &Committed, part: Part) -> Result<bool, RemoteError> {
        self.common.acknowledge(committed, part)
    }
}
