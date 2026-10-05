//! Direct/held dispatch and same-ID observation through the public core API.
use crate::{
    admission::MessagePermit,
    audit, canonical,
    core::CoreClient,
    error::RemoteError,
    journal::{Owned, RECOVERY},
    pairing::now_ms,
    store::{Grant, Store, database},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dispatch {
    version: u64,
    operation: String,
    originator: String,
    input: Intent,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Intent {
    operation_id: String,
    pub(crate) recipient_ids: Vec<String>,
    pub(crate) message: String,
    kind: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Lookup {
    operation_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadEnvelope<T> {
    version: u64,
    operation: String,
    input: T,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StatusInput {
    identity_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CheckInput {
    agent_id: String,
    lines: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResultInput {
    request_id: String,
}
fn read_input<T: serde::de::DeserializeOwned>(permit: &MessagePermit) -> Result<T, RemoteError> {
    let value: ReadEnvelope<T> =
        serde_json::from_slice(&permit.message.payload).map_err(|_| invalid())?;
    if value.version != 1 || value.operation != permit.message.envelope().operation {
        return Err(invalid());
    }
    Ok(value.input)
}
fn request_reference(id: &str) -> Result<String, RemoteError> {
    id.strip_prefix("req_")
        .filter(|id| canonical::is_core_id(id))
        .map(str::to_owned)
        .ok_or_else(invalid)
}
fn result_state(id: &str, result: Result<Value, RemoteError>) -> Result<Value, RemoteError> {
    let detail = match result {
        Ok(value) => value,
        Err(error) if error.code == "REQUEST_NOT_FOUND" => {
            return Ok(json!({"state":"unavailable","requestId":id,"reason":"REQUEST_NOT_FOUND"}));
        }
        Err(error) => return Err(error),
    };
    if detail["requestId"] != id {
        return Err(invalid());
    }
    Ok(match detail["final"]["status"].as_str() {
        Some("not_submitted") => json!({"state":"pending","requestId":id}),
        Some("retained") => {
            json!({"state":"replied","requestId":id,"message":detail["final"]["response"].as_str().ok_or_else(invalid)?})
        }
        Some("not_required" | "expired" | "unavailable") => {
            json!({"state":"unavailable","requestId":id})
        }
        _ => return Err(invalid()),
    })
}
pub(crate) fn invalid() -> RemoteError {
    RemoteError::new("REMOTE_INPUT_INVALID", "Invalid remote operation input.")
}
fn dispatch(bytes: &[u8]) -> Result<Dispatch, RemoteError> {
    let value: Dispatch = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if value.version != 1
        || value.operation != "dispatch.create"
        || value.originator != "anonymous"
        || value.input.kind != "request"
        || value.input.recipient_ids.len() != 1
        || !canonical::is_core_id(&value.input.operation_id)
        || !canonical::is_core_id(&value.input.recipient_ids[0])
    {
        return Err(invalid());
    }
    Ok(value)
}
fn permits(grant: &Grant, intent: &Intent) -> Result<(), RemoteError> {
    if !grant.permits_scope("talk") || !grant.authority()?.permits(&intent.recipient_ids[0]) {
        return Err(RemoteError::new(
            "REMOTE_SCOPE_DENIED",
            "Recipient is not permitted.",
        ));
    }
    Ok(())
}
pub(crate) fn held_intent(grant: &Grant, id: &str, frozen: &[u8]) -> Result<Intent, RemoteError> {
    let intent = dispatch(frozen)?.input;
    if intent.operation_id != id {
        return Err(invalid());
    }
    permits(grant, &intent)?;
    Ok(intent)
}
fn uncertain(id: &str) -> Value {
    json!({"state":"uncertain","operationId":id})
}
fn accepted(id: &str, recipient: &str, receipt: &Value) -> Result<Value, RemoteError> {
    let items = receipt["items"]
        .as_array()
        .filter(|items| items.len() == 1)
        .ok_or_else(invalid)?;
    let request = items[0]["requestId"]
        .as_str()
        .filter(|id| id.starts_with("req_"))
        .ok_or_else(invalid)?;
    if receipt["operationId"] != id
        || items[0]["recipientId"] != recipient
        || !matches!(
            items[0]["acceptance"].as_str(),
            Some("queued" | "recipientUnavailable")
        )
        || !canonical::is_core_id(&request[4..])
    {
        return Err(invalid());
    }
    // Wake is advisory. A receipt remains accepted even if the one-shot wake is uncertain.
    Ok(json!({"state":"accepted","operationId":id,"requestId":request}))
}

pub struct Operations {
    core: CoreClient,
    stop: Arc<AtomicBool>,
    input_limit: usize,
    /// Unconfirmed cleanup forbids another write until restart acquires the child-held lease.
    retry_safe: AtomicBool,
}
impl Operations {
    pub fn new(core: CoreClient, stop: Arc<AtomicBool>, input_limit: usize) -> Self {
        Self {
            core,
            stop,
            input_limit,
            retry_safe: AtomicBool::new(true),
        }
    }
    fn api(&self, input: &[u8]) -> Result<Value, RemoteError> {
        let result = self.core.api(input, &self.stop);
        if result
            .as_ref()
            .is_err_and(|e| e.code == "REMOTE_CORE_UNCERTAIN")
        {
            self.retry_safe.store(false, Ordering::Release);
        }
        result
    }
    pub(crate) fn append(&self, permit: &MessagePermit) -> Result<Vec<u8>, RemoteError> {
        let operation = permit.message.envelope().operation;
        if operation == "capabilities" {
            if permit.message.input != json!({}) {
                return Err(invalid());
            }
            return permit.response(&json!({"version":1,"profile":"local-v1","binding":"loopback-http",
                "operations":["agents.list","identities.status","check","dispatch.create","dispatch.show","operation.show","requests.show","result"],"limits":{"inputBytes":self.input_limit,"outputBytes":crate::core::OUTPUT_LIMIT,
                    "callsPerDevicePerMinute":crate::budgets::CALLS,"newSendsPerDevicePerMinute":crate::budgets::SENDS,
                    "outstandingHeldPerDevice":crate::budgets::HELDS,"approvalsPerRecipientPerMinute":crate::budgets::APPROVALS}}));
        }
        if matches!(
            operation,
            "agents.list"
                | "identities.status"
                | "check"
                | "requests.show"
                | "result"
                | "dispatch.show"
        ) {
            return self.read(permit);
        }
        let id = match operation {
            "dispatch.create" => {
                let mut value = dispatch(&permit.message.payload)?;
                if value.input.operation_id != permit.message.envelope().id {
                    return Err(invalid());
                }
                permits(&permit.grant, &value.input)?;
                value.input.message =
                    format!("[remote: {}]\n{}", permit.grant.name, value.input.message);
                let frozen=json!({"version":1,"operation":"dispatch.create","originator":"anonymous",
                    "input":{"operationId":value.input.operation_id,"recipientIds":value.input.recipient_ids,
                    "message":value.input.message,"kind":"request"}}).to_string().into_bytes();
                if frozen.len() > self.input_limit {
                    return Err(invalid());
                }
                permit.adopt(Some(&frozen), &value.input.recipient_ids)?.id
            }
            "operation.show" => {
                let value: Lookup =
                    serde_json::from_value(permit.message.input.clone()).map_err(|_| invalid())?;
                if !canonical::is_core_id(&value.operation_id) {
                    return Err(invalid());
                }
                permit.adopt(None, &[])?;
                value.operation_id
            }
            _ => return Err(invalid()),
        };
        if !canonical::is_core_id(&id) {
            return Err(invalid());
        }
        let now = now_ms()?;
        let owned = permit.with_store(|store| store.owned(&permit.grant, &id, now))?;
        if owned.operation != "dispatch.create" {
            return Err(invalid());
        }
        let payload = if matches!(
            owned.phase.as_str(),
            "held" | "cancelled" | "refused" | "accepted"
        ) {
            state(&owned)
        } else {
            // Holding the live session, Store and IMMEDIATE transaction orders revoke
            // against the actual bounded core call, not just earlier admission.
            let mut attempted = false;
            let result = permit.with_store(|store| {
                store.effect(&permit.grant, &id, now_ms, |frozen| {
                    attempted = true;
                    self.resolve(frozen, operation == "dispatch.create", &permit.grant)
                })
            });
            match result {
                Ok(Ok(payload)) => payload,
                Ok(Err(error)) if error.code == "REMOTE_CLOSED" => return Err(error),
                Err(error) if !attempted => return Err(error),
                _ => uncertain(&id),
            }
        };
        let metadata = match permit.sessions.operation_response(
            &permit.grant,
            &id,
            &payload,
            Some(permit.message.envelope().session_id),
        ) {
            Ok(metadata) => metadata,
            Err(_) => return permit.response(&uncertain(&id)),
        };
        // A post-effect storage/signing failure retains the adopted frozen intent for recovery.
        if permit
            .with_store(|store| {
                store.settle(&permit.grant, &id, &payload, metadata.as_deref(), now_ms()?)
            })
            .is_err()
        {
            return permit.response(&uncertain(&id));
        }
        permit.sessions.changed();
        permit.response(&payload)
    }
    fn read(&self, permit: &MessagePermit) -> Result<Vec<u8>, RemoteError> {
        let operation = permit.message.envelope().operation;
        let mut resources = Vec::new();
        let mut agent = None;
        let mut request = None;
        let mut owned = None;
        match operation {
            "agents.list" if permit.message.input == json!({}) => {}
            "identities.status" => {
                let input: StatusInput = read_input(permit)?;
                if input.identity_ids.len() > 256 {
                    return Err(invalid());
                }
                resources = input.identity_ids;
            }
            "check" => {
                let input: CheckInput =
                    serde_json::from_slice(&permit.message.payload).map_err(|_| invalid())?;
                if input.lines.is_some_and(|lines| lines > 2_147_483_647) {
                    return Err(invalid());
                }
                resources.push(input.agent_id.clone());
                agent = Some(input);
            }
            "requests.show" | "result" => {
                let input: ResultInput = if operation == "requests.show" {
                    read_input(permit)?
                } else {
                    serde_json::from_slice(&permit.message.payload).map_err(|_| invalid())?
                };
                resources.push(request_reference(&input.request_id)?);
                request = Some(input.request_id);
            }
            "dispatch.show" => {
                let input: Lookup = read_input(permit)?;
                if !canonical::is_core_id(&input.operation_id) {
                    return Err(invalid());
                }
                let value = permit.with_store(|store| {
                    store.owned(&permit.grant, &input.operation_id, now_ms()?)
                })?;
                if value.operation != "dispatch.create" {
                    return Err(invalid());
                }
                owned = Some(value);
            }
            _ => return Err(invalid()),
        }
        if matches!(operation, "identities.status" | "check") {
            if resources.iter().any(|id| !canonical::is_core_id(id)) {
                return Err(invalid());
            }
            if resources.iter().any(|id| !permit.authority.permits(id)) {
                return Err(RemoteError::new(
                    "REMOTE_SCOPE_DENIED",
                    "Agent is not permitted.",
                ));
            }
        }
        permit.adopt(None, &resources)?;
        let payload = permit.with_store(|store| store.observe(&permit.grant, || {
            match operation {
                "agents.list" => {
                    let value = self.core.agents(&self.stop)?;
                    let rows = value["identities"].as_array().ok_or_else(invalid)?;
                    let mut agents = Vec::new();
                    for row in rows {
                        let id = row["id"].as_str().ok_or_else(invalid)?;
                        if !permit.authority.permits(id) { continue; }
                        let mut projected = json!({"id":id,"name":row["name"].as_str().ok_or_else(invalid)?,"presence":row["presence"].as_str().ok_or_else(invalid)?});
                        if let Some(delivery) = row.get("delivery") { projected["delivery"] = delivery.clone(); }
                        agents.push(projected);
                    }
                    Ok(json!({"identities":agents}))
                },
                "identities.status" | "requests.show" => self.api(&permit.message.payload),
                "check" => {
                    let input = agent.as_ref().expect("validated check input");
                    self.core.check(&input.agent_id, input.lines, &self.stop)
                },
                "result" => {
                    let id = request.as_deref().expect("validated request input");
                    let api = json!({"version":1,"operation":"requests.show","input":{"requestId":id}});
                    result_state(id, self.api(api.to_string().as_bytes()))
                },
                "dispatch.show" => {
                    let owned = owned.as_ref().expect("owned operation");
                    if matches!(owned.phase.as_str(), "held" | "cancelled" | "refused") { return Ok(state(owned)); }
                    match self.api(&permit.message.payload) {
                        Ok(receipt) if receipt["operationId"] == owned.id => Ok(receipt),
                        Ok(_) => Err(invalid()),
                        Err(_) => Ok(uncertain(&owned.id)),
                    }
                },
                _ => unreachable!(),
            }
        }))??;
        permit.response(&payload)
    }
    fn resolve(&self, frozen: &[u8], retry: bool, grant: &Grant) -> Result<Value, RemoteError> {
        let intent = dispatch(frozen)?.input;
        let id = &intent.operation_id;
        let show = json!({"version":1,"operation":"dispatch.show","input":{"operationId":id}});
        match self.api(show.to_string().as_bytes()) {
            Ok(receipt) => accepted(id, &intent.recipient_ids[0], &receipt),
            Err(error)
                if error.code == "DISPATCH_NOT_FOUND"
                    && retry
                    && self.retry_safe.load(Ordering::Acquire) =>
            {
                if !grant.live_at(now_ms()?) {
                    return Err(RemoteError::new("REMOTE_CLOSED", "Device authority ended."));
                }
                match self.api(frozen) {
                    Ok(receipt) => accepted(id, &intent.recipient_ids[0], &receipt),
                    Err(_) => Ok(uncertain(id)),
                }
            }
            Err(_) => Ok(uncertain(id)),
        }
    }
    pub(crate) fn release(
        &self,
        store: &mut Store,
        grant: &Grant,
        id: &str,
    ) -> Result<Value, RemoteError> {
        store.effect(grant, id, now_ms, |frozen| {
            self.resolve(frozen, true, grant)
        })?
    }
}
fn state(owned: &Owned) -> Value {
    if matches!(owned.phase.as_str(), "accepted" | "cancelled" | "refused") {
        owned.receipt.clone()
    } else {
        json!({"state":owned.phase,"operationId":owned.id})
    }
}
impl Store {
    /// A bounded public observation is ordered against cross-process grant revocation.
    fn observe<T>(&mut self, grant: &Grant, action: impl FnOnce() -> T) -> Result<T, RemoteError> {
        let (tx, _) = self.authorized_at(grant, now_ms)?;
        let value = action();
        tx.commit().map_err(database)?;
        Ok(value)
    }
    /// The transaction owns authority, resource audit and the actual core invocation fence.
    pub(crate) fn effect<T>(
        &mut self,
        grant: &Grant,
        id: &str,
        clock: impl FnOnce() -> Result<u64, RemoteError>,
        action: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, RemoteError> {
        let (tx, now) = self.authorized_at(grant, clock)?;
        let (frozen,time,digest):(Vec<u8>,i64,Vec<u8>)=tx.query_row(
            "SELECT frozen,updated_ms,digest FROM operations WHERE id=?1 AND client_id=?2 AND operation='dispatch.create'",
            params![id,grant.client_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(database)?;
        if now.saturating_sub(time as u64) >= RECOVERY {
            return Err(database("recovery horizon"));
        }
        let intent = held_intent(grant, id, &frozen)?;
        audit::append(
            &tx,
            audit::AuditMetadata {
                time: now,
                client: &grant.client_id,
                request: id,
                operation: "dispatch.create",
                operation_id: Some(id),
                resources: &intent.recipient_ids,
                digest: &digest,
                revision: grant.revision,
                decision: "dispatching",
                code: "",
            },
        )?;
        tx.execute(
            "UPDATE operations SET references_json=?2 WHERE id=?1",
            params![
                id,
                serde_json::to_string(&intent.recipient_ids).map_err(database)?
            ],
        )
        .map_err(database)?;
        let result = action(&frozen);
        tx.commit().map_err(database)?;
        Ok(result)
    }
    pub(crate) fn settle(
        &mut self,
        grant: &Grant,
        id: &str,
        payload: &Value,
        metadata: Option<&[u8]>,
        now: u64,
    ) -> Result<(), RemoteError> {
        let phase = payload["state"]
            .as_str()
            .filter(|s| {
                matches!(
                    *s,
                    "held" | "accepted" | "uncertain" | "refused" | "cancelled"
                )
            })
            .ok_or_else(invalid)?;
        let tx = self.authorized(grant, now)?;
        let (old, digest, refs): (String, Vec<u8>, String) = tx
            .query_row(
                "SELECT phase,digest,references_json FROM operations WHERE id=?1 AND client_id=?2",
                params![id, grant.client_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(database)?
            .ok_or_else(|| database("owned state absent"))?;
        if old == phase {
            return Ok(());
        }
        if matches!(old.as_str(), "accepted" | "cancelled" | "refused") {
            return Err(invalid());
        }
        let mut resources: Vec<String> = serde_json::from_str(&refs).map_err(database)?;
        if let Some(request) = payload["requestId"].as_str() {
            resources.push(request.strip_prefix("req_").ok_or_else(invalid)?.into());
        }
        audit::append(
            &tx,
            audit::AuditMetadata {
                time: now,
                client: &grant.client_id,
                request: id,
                operation: "dispatch.create",
                operation_id: Some(id),
                resources: &resources,
                digest: &digest,
                revision: grant.revision,
                decision: phase,
                code: "",
            },
        )?;
        tx.execute(
            "UPDATE operations SET phase=?2,receipt=?3,references_json=?4,frozen=CASE WHEN ?2 IN ('accepted','cancelled','refused') THEN NULL ELSE frozen END WHERE id=?1",
            params![id, phase, payload.to_string(),serde_json::to_string(&resources).map_err(database)?],
        )
        .map_err(database)?;
        if let Some(metadata) = metadata {
            crate::journal::prune(&tx, &grant.client_id, now)?;
            let entries: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE client_id=?1",
                    [&grant.client_id],
                    |r| r.get(0),
                )
                .map_err(database)?;
            if entries >= crate::journal::ENTRIES {
                return Err(database("journal capacity"));
            }

            if metadata.len() > crate::limits::METADATA_BYTES {
                return Err(invalid());
            }
            tx.execute(
                "UPDATE streams SET tip=tip+1,last_ms=MAX(last_ms,?2) WHERE client_id=?1",
                params![grant.client_id, now as i64],
            )
            .map_err(database)?;
            tx.execute("INSERT INTO entries SELECT client_id,tip,?2,last_ms FROM streams WHERE client_id=?1",params![grant.client_id,metadata]).map_err(database)?;
        }
        tx.commit().map_err(database)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        state::Layout,
        store::{DEFAULT_SCOPES, uuid_v4},
        wire::SignedMessage,
    };
    #[test]
    fn expiry_boundary_refuses_at_the_effect_transaction_after_valid_adoption() {
        struct Root(std::path::PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = Root(format!("/tmp/t1055-expiry-{}", uuid_v4().unwrap()).into());
        let layout = Layout::open(&root.0).unwrap();
        let serving = layout.serve_lock().unwrap();
        let mut store = Store::open(&serving).unwrap();
        let machine = store.machine().unwrap();
        let recipient = uuid_v4().unwrap();
        let id = uuid_v4().unwrap();
        let grant = Grant {
            client_id: uuid_v4().unwrap(),
            public_key: ed25519_dalek::SigningKey::from_bytes(&[1; 32])
                .verifying_key()
                .to_bytes(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Expiry control".into(),
            agents: json!([recipient]).to_string(),
            scopes: DEFAULT_SCOPES.iter().map(|s| (*s).into()).collect(),
            mode: "direct".into(),
            issued_at_ms: 999,
            expires_at_ms: Some(1000),
            revision: 1,
            disabled: false,
        };
        store.insert_grant(&grant).unwrap();
        let frozen=json!({"version":1,"operation":"dispatch.create","originator":"anonymous","input":{"operationId":id,"recipientIds":[recipient],"message":"frozen","kind":"request"}}).to_string().into_bytes();
        // Store-only scenario; device admission has separate real-signature coverage.
        let wire = json!({"version":1,"profile":"local-v1","kind":"request","id":id,"correlationId":null,"machineId":machine.id,"windowId":uuid_v4().unwrap(),"clientId":grant.client_id,"sessionId":uuid_v4().unwrap(),"sequence":"1","timestampMs":999,"origin":"cli","operation":"dispatch.create","payload":canonical::base64url(&frozen),"signature":canonical::base64url(&[0;64])});
        let message = SignedMessage::decode(wire.to_string().as_bytes(), 65536).unwrap();
        store
            .adopt(
                &grant,
                &message,
                Some(&frozen),
                std::slice::from_ref(&recipient),
                b"{}",
                999,
            )
            .unwrap();
        let mut effects = 0;
        store
            .effect(&grant, &id, || Ok(999), |_| effects += 1)
            .unwrap();
        assert_eq!(effects, 1, "valid authority must pass the identical fence");
        assert_eq!(
            store
                .effect(&grant, &id, || Ok(1000), |_| effects += 1)
                .unwrap_err()
                .code,
            "REMOTE_CLOSED"
        );
        assert_eq!(
            effects, 1,
            "expiry must prevent invoking the effect closure"
        );
    }
}
