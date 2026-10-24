//! Pushed finals use the common inbox, current grant fence and Core's final owner.
use crate::{
    admission::MessagePermit,
    canonical,
    cloud_inbox::store::ReplyOrigin,
    core::CloudFinalDelivery,
    error::RemoteError,
    pairing::now_ms,
    store::{Store, database},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Final {
    pub version: u64,
    pub operation_id: String,
    pub request_id: String,
    pub response: String,
}

/// SHA-256 of a domain label and canonical operation UUID; keep 122 digest bits in
/// UUIDv4 form so existing carrier IDs and Rules need no second ID grammar.
pub fn id(operation_id: &str) -> Result<String, RemoteError> {
    canonical::uuid(operation_id).map_err(|_| invalid())?;
    let input = canonical::framed(&[b"tmt-cloud-final-v1", operation_id.as_bytes()])
        .map_err(|_| invalid())?;
    let mut bytes: [u8; 16] = Sha256::digest(input)[..16]
        .try_into()
        .expect("digest prefix");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

impl Final {
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.version != 1 {
            return Err(crate::cloud_command::upgrade());
        }
        canonical::uuid(&self.operation_id).map_err(|_| invalid())?;
        self.request_id
            .strip_prefix("req_")
            .filter(|id| canonical::is_core_id(id))
            .ok_or_else(invalid)?;
        if self.response.len() > crate::limits::CLOUD_MESSAGE_BYTES {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_MESSAGE_TOO_LARGE",
                "The final exceeds the cloud transport limit.",
            ));
        }
        Ok(())
    }
}

pub(crate) fn deliver(
    permit: &MessagePermit,
    core: &dyn CloudFinalDelivery,
    stop: &AtomicBool,
) -> Result<Vec<u8>, RemoteError> {
    // Local browser grants never admit this new operation, even with results.read.
    if !permit.cloud() {
        return Err(invalid());
    }
    let final_value: Final =
        serde_json::from_slice(&permit.message.payload).map_err(|_| invalid())?;
    final_value.validate()?;
    if permit.message.envelope().id != id(&final_value.operation_id)? {
        return Err(invalid());
    }
    permit.with_store(|store| {
        let (tx, now) = store.authorized_at(&permit.grant, now_ms)?;
        let local = &permit.message.envelope().machine_id;
        let row: Option<(String, i64)> = tx.query_row(
            "SELECT binding,created_ms FROM cloud_transport WHERE peer_id=?1 AND kind='outbox' AND id=?2 AND binding IS NOT NULL AND attempted=1",
            params![local, final_value.operation_id], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional().map_err(database)?;
        let (binding, created_ms) = row.ok_or_else(invalid)?;
        let binding: ReplyOrigin = serde_json::from_str(&binding).map_err(database)?;
        if binding.machine != permit.grant.client_id
            || !permit.authority.permits(&binding.sender_id)
        {
            return Err(RemoteError::new("REMOTE_SCOPE_DENIED", "Final does not belong to this peer and permitted request."));
        }
        super::check_request_age(created_ms as u64, now)?;
        // The Core seam is idempotent on this exact request/route/body. If Remote
        // crashes after Core commits, replay observes the same immutable final.
        let reply = binding.reply.as_ref().ok_or_else(crate::cloud_command::upgrade)?;
        core.deliver_final(reply, &final_value.response, stop)?;
        let digest = crate::journal::intent(&permit.message)?;
        crate::audit::append(&tx, crate::audit::AuditMetadata {
            time: now, client: &permit.grant.client_id, request: permit.message.envelope().id,
            operation: "final", operation_id: Some(&final_value.operation_id),
            resources: &[binding.sender_id], digest: &digest,
            revision: permit.grant.revision, decision: "accepted", code: "",
        })?;
        tx.commit().map_err(database)
    })?;
    permit.response(&json!({"state":"accepted","operationId":final_value.operation_id}))
}

impl Store {
    pub(crate) fn cloud_accepted_dispatches(&self, peer: &str) -> Result<Vec<String>, RemoteError> {
        let mut statement = self.connection.prepare(
            "SELECT o.id FROM operations o JOIN cloud_transport t ON t.peer_id=o.client_id AND t.id=o.id AND t.kind='inbox' WHERE o.client_id=?1 AND o.operation='dispatch.create' AND o.phase='accepted' AND o.updated_ms>?3 ORDER BY o.updated_ms LIMIT ?2",
        ).map_err(database)?;
        statement
            .query_map(
                params![
                    peer,
                    crate::journal::OPERATIONS,
                    now_ms()?.saturating_sub(crate::journal::RECOVERY) as i64
                ],
                |row| row.get(0),
            )
            .map_err(database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database)
    }
    pub(crate) fn cloud_final_outgoing(
        &self,
        peer: &str,
        final_id: &str,
        payload: &[u8],
    ) -> Result<Option<super::store::Outgoing>, RemoteError> {
        let created: Option<i64> = self.connection.query_row(
            "SELECT created_ms FROM cloud_transport WHERE peer_id=?1 AND kind='outbox' AND id=?2",
            [peer, final_id], |r| r.get(0),
        ).optional().map_err(database)?;
        created
            .map(|created| {
                self.cloud_outgoing(peer, final_id, payload, created as u64)
                    .and_then(|row| row.ok_or_else(invalid))
            })
            .transpose()
    }
    pub(crate) fn cloud_claim_final(
        &mut self,
        grant: &crate::store::Grant,
        peer: &str,
        final_id: &str,
        operation: &str,
        request_id: &str,
        carrier: &[u8],
    ) -> Result<bool, RemoteError> {
        let (tx, now) = self.authorized_at(grant, now_ms)?;
        if mapped_request(&tx, grant, operation, now)? != request_id {
            return Err(invalid());
        }
        let claimed = tx.execute(
            "UPDATE cloud_transport SET attempted=1 WHERE peer_id=?1 AND kind='outbox' AND id=?2 AND attempted=0 AND carrier=?3",
            params![peer, final_id, carrier],
        ).map_err(database)? == 1;
        tx.commit().map_err(database)?;
        Ok(claimed)
    }
    /// No HTTP was invoked while attempted=0. Refresh only that preparation with
    /// a fresh HPKE context so a long budget/Auth delay cannot publish stale headers.
    /// The final intent is immutable, and a claimed carrier is never changed.
    pub(crate) fn cloud_prepare_final(
        &mut self,
        peer: &str,
        final_id: &str,
        payload: &[u8],
        carrier: &[u8],
        now: u64,
    ) -> Result<super::store::Outgoing, RemoteError> {
        super::store::validate(peer, final_id, carrier, now)?;
        let Some(old) = self.cloud_final_outgoing(peer, final_id, payload)? else {
            return self.cloud_freeze_outgoing(peer, final_id, payload, carrier, now);
        };
        if old.attempted {
            return Ok(old);
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database)?;
        let used: i64 = tx.query_row(
            "SELECT COALESCE(SUM(length(carrier)+COALESCE(length(response),0)),0) FROM cloud_transport WHERE peer_id=?1 AND kind<>'checkpoint'",
            [peer], |row| row.get(0),
        ).map_err(database)?;
        if used - old.carrier.len() as i64 + carrier.len() as i64 > crate::journal::FROZEN_BYTES {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_CAPACITY",
                "Cloud recovery storage is full.",
            ));
        }
        let changed = tx.execute("UPDATE cloud_transport SET carrier=?3,created_ms=?4 WHERE peer_id=?1 AND kind='outbox' AND id=?2 AND attempted=0 AND carrier=?5 AND created_ms=?6",
            params![peer, final_id, carrier, now as i64, old.carrier, old.created_ms as i64]).map_err(database)?;
        if changed != 1 {
            return Err(invalid());
        }
        tx.commit().map_err(database)?;
        Ok(super::store::Outgoing {
            attempted: false,
            carrier: carrier.to_vec(),
            created_ms: now,
        })
    }
}

/// Call under the Store authority transaction. The journal, rather than caller
/// request IDs or provider metadata, identifies the local Core final to export.
pub(crate) fn mapped_request(
    tx: &rusqlite::Transaction<'_>,
    grant: &crate::store::Grant,
    operation: &str,
    now: u64,
) -> Result<String, RemoteError> {
    if !grant.permits_scope("results.read") {
        return Err(RemoteError::new(
            "REMOTE_SCOPE_DENIED",
            "Results are not permitted.",
        ));
    }
    let row: Option<(String, String)> = tx.query_row(
        "SELECT o.receipt,o.references_json FROM operations o JOIN cloud_transport t ON t.peer_id=o.client_id AND t.id=o.id AND t.kind='inbox' WHERE o.id=?1 AND o.client_id=?2 AND o.operation='dispatch.create' AND o.phase='accepted' AND o.updated_ms>?3",
        params![operation, grant.client_id, now.saturating_sub(crate::journal::RECOVERY) as i64], |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional().map_err(database)?;
    let (receipt, resources) = row.ok_or_else(invalid)?;
    let receipt: serde_json::Value = serde_json::from_str(&receipt).map_err(database)?;
    let request = receipt["requestId"].as_str().ok_or_else(invalid)?;
    let request_bare = request
        .strip_prefix("req_")
        .filter(|id| canonical::is_core_id(id))
        .ok_or_else(invalid)?;
    let resources: Vec<String> = serde_json::from_str(&resources).map_err(database)?;
    let (mapped, agents) = resources.split_last().ok_or_else(invalid)?;
    if mapped != request_bare {
        return Err(invalid());
    }
    let authority = grant.authority()?;
    if agents.is_empty() || agents.iter().any(|id| !authority.permits(id)) {
        return Err(RemoteError::new(
            "REMOTE_SCOPE_DENIED",
            "The original recipient is no longer permitted.",
        ));
    }
    Ok(request.into())
}

fn invalid() -> RemoteError {
    RemoteError::new(
        "REMOTE_INPUT_INVALID",
        "Invalid remote final or request binding.",
    )
}

#[cfg(test)]
#[path = "finals_tests.rs"]
mod tests;
