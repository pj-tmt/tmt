//! Owner-only held-operation confirmation. No remote approval route exists.
use crate::{
    error::RemoteError,
    operations::{Operations, invalid},
    pairing::now_ms,
    session::DoorSessions,
    store::{Grant, Store, database},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub struct Approval {
    store: Arc<Mutex<Store>>,
    sessions: Arc<DoorSessions>,
    operations: Arc<Operations>,
}
impl Approval {
    pub fn new(
        store: Arc<Mutex<Store>>,
        sessions: Arc<DoorSessions>,
        operations: Arc<Operations>,
    ) -> Self {
        Self {
            store,
            sessions,
            operations,
        }
    }
    pub(crate) fn preview(&self, id: &str) -> Result<(Grant, Value), RemoteError> {
        if !crate::canonical::is_core_id(id) {
            return Err(invalid());
        }
        let mut store = self.store.lock().map_err(database)?;
        let grant = store.operation_grant(id)?;
        let owned = store.owned(&grant, id, now_ms()?)?;
        if owned.phase != "held" {
            return Err(invalid());
        }
        let frozen = crate::operations::held_intent(
            &grant,
            id,
            owned.frozen.as_deref().ok_or_else(invalid)?,
        )?;
        Ok((
            grant.clone(),
            json!({"event":"held","operationId":id,"clientId":grant.client_id,"deviceName":grant.name,
            "recipientId":frozen.recipient_ids[0],"message":frozen.message}),
        ))
    }
    pub(crate) fn confirm(&self, id: &str, grant: &Grant) -> Result<Value, RemoteError> {
        let payload = self.sessions.with_operation_store(grant, id, |store| {
            store.claim_held(grant, id, now_ms()?)?;
            Ok(self
                .operations
                .release(store, grant, id)
                .unwrap_or_else(|_| json!({"state":"uncertain","operationId":id})))
        })?;
        // Confirmation may already have reached core. Publication failure is an
        // uncertain observation; the durable frozen intent remains recoverable.
        let published = (|| {
            let metadata = self
                .sessions
                .operation_response(grant, id, &payload, None)?;
            self.store.lock().map_err(database)?.settle(
                grant,
                id,
                &payload,
                metadata.as_deref(),
                now_ms()?,
            )
        })();
        if published.is_err() {
            return Ok(json!({"state":"uncertain","operationId":id}));
        }
        self.sessions.changed();
        Ok(payload)
    }
    pub(crate) fn cancel(&self, id: &str) -> Result<Value, RemoteError> {
        let payload = {
            let mut store = self.store.lock().map_err(database)?;
            let grant = store.operation_grant(id)?;
            store.cancel_held(&grant, id, now_ms()?)?
        };
        self.sessions.changed();
        Ok(payload)
    }
    pub fn cancel_pending(&self) -> Result<(), RemoteError> {
        let ids = self.store.lock().map_err(database)?.pending_holds()?;
        for id in ids {
            self.cancel(&id)?;
        }
        Ok(())
    }
}
impl Store {
    fn pending_holds(&self) -> Result<Vec<String>, RemoteError> {
        let mut query = self
            .connection
            .prepare("SELECT id FROM operations WHERE phase='held'")
            .map_err(database)?;
        query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database)
    }
    fn operation_grant(&self, id: &str) -> Result<Grant, RemoteError> {
        let client: String = self
            .connection
            .query_row(
                "SELECT client_id FROM operations WHERE id=?1 AND operation='dispatch.create'",
                [id],
                |r| r.get(0),
            )
            .map_err(database)?;
        self.grant(&client)?.ok_or_else(invalid)
    }
    fn claim_held(&mut self, grant: &Grant, id: &str, now: u64) -> Result<(), RemoteError> {
        let tx = self.authorized(grant, now)?;
        let frozen: Vec<u8> = tx
            .query_row(
                "SELECT frozen FROM operations WHERE id=?1 AND client_id=?2 AND phase='held'",
                params![id, grant.client_id],
                |r| r.get(0),
            )
            .map_err(database)?;
        let intent = crate::operations::held_intent(grant, id, &frozen)?;
        crate::budgets::charge(
            &tx,
            &intent.recipient_ids[0],
            crate::budgets::Budget::Approval,
            now,
        )?;
        let changed=tx.execute("UPDATE operations SET phase='dispatching' WHERE id=?1 AND client_id=?2 AND phase='held'",params![id,grant.client_id]).map_err(database)?;
        if changed != 1 {
            return Err(invalid());
        }
        tx.commit().map_err(database)
    }
    pub(crate) fn cancel_held(
        &mut self,
        grant: &Grant,
        id: &str,
        now: u64,
    ) -> Result<Value, RemoteError> {
        // Cancellation is the local owner's authority, even after grant expiry/revocation.
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database)?;
        let phase: Option<String> = tx
            .query_row(
                "SELECT phase FROM operations WHERE id=?1 AND client_id=?2",
                params![id, grant.client_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(database)?;
        let payload = json!({"state":"cancelled","operationId":id});
        if phase.as_deref() == Some("cancelled") {
            return Ok(payload);
        }
        if phase.as_deref() != Some("held") {
            return Err(invalid());
        }
        let (digest, references): (Vec<u8>, String) = tx
            .query_row(
                "SELECT digest,references_json FROM operations WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(database)?;
        let resources: Vec<String> = serde_json::from_str(&references).map_err(database)?;
        crate::audit::append(
            &tx,
            crate::audit::AuditMetadata {
                time: now,
                client: &grant.client_id,
                request: id,
                operation: "dispatch.create",
                operation_id: Some(id),
                resources: &resources,
                digest: &digest,
                revision: grant.revision,
                decision: "cancelled",
                code: "",
            },
        )?;
        tx.execute(
            "UPDATE operations SET phase='cancelled',receipt=?2,frozen=NULL WHERE id=?1",
            params![id, payload.to_string()],
        )
        .map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(payload)
    }
}
