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
    /// Exactly one of `originator` and `identity`, as Core takes them. A client sends only
    /// `originator:"anonymous"`; the journal may also hold the person's `identity` (#4255).
    originator: Option<String>,
    identity: Option<String>,
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
/// The message state a client may show. `canSendNow` is display only: the owner's browser sees
/// Core's answer, any other grant sees false, and a send-now from it is refused regardless.
fn message_state(detail: &Value, owner: bool) -> Option<Value> {
    let mut state = detail
        .get("messageState")
        .filter(|s| s.is_object())?
        .clone();
    if !owner {
        state["canSendNow"] = json!(false);
    }
    Some(state)
}
fn result_state(
    id: &str,
    result: Result<Value, RemoteError>,
    owner: bool,
) -> Result<Value, RemoteError> {
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
        Some("not_submitted") => {
            let mut pending = json!({"state":"pending","requestId":id});
            if let Some(state) = message_state(&detail, owner) {
                pending["messageState"] = state;
            }
            pending
        }
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
/// A client's `dispatch.create`: only the anonymous originator. A caller never selects an identity.
fn dispatch(bytes: &[u8]) -> Result<Dispatch, RemoteError> {
    let value = frozen_dispatch(bytes)?;
    if value.identity.is_some() {
        return Err(invalid());
    }
    Ok(value)
}
/// A journaled `dispatch.create`: the anonymous form a client sent, or the same send attributed to
/// the person at this machine. Never both and never neither.
fn frozen_dispatch(bytes: &[u8]) -> Result<Dispatch, RemoteError> {
    let value: Dispatch = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let attribution = match (&value.originator, &value.identity) {
        (Some(originator), None) => originator == "anonymous",
        (None, Some(identity)) => canonical::is_core_id(identity),
        _ => false,
    };
    if value.version != 1
        || value.operation != "dispatch.create"
        || !attribution
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
    let intent = frozen_dispatch(frozen)?.input;
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
    management: Option<Arc<crate::devices::Devices>>,
    firestore: Arc<dyn crate::readiness::FirestoreEvidenceSource>,
}
impl Operations {
    pub fn new(core: CoreClient, stop: Arc<AtomicBool>, input_limit: usize) -> Self {
        Self {
            core,
            stop,
            input_limit,
            retry_safe: AtomicBool::new(true),
            management: None,
            firestore: Arc::new(crate::readiness::NotConfigured),
        }
    }
    pub fn with_management(mut self, devices: Arc<crate::devices::Devices>) -> Self {
        self.management = Some(devices);
        self
    }
    /// Shares the same recorded-evidence owner as the optional control status view.
    pub fn with_firestore(
        mut self,
        evidence: Arc<dyn crate::readiness::FirestoreEvidenceSource>,
    ) -> Self {
        self.firestore = evidence;
        self
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
        if operation.starts_with("remote.") {
            return crate::management::append(
                permit,
                self.management.as_deref().ok_or_else(invalid)?,
                self.firestore.as_ref(),
            );
        }
        if operation == "capabilities" {
            if permit.message.input != json!({}) {
                return Err(invalid());
            }
            let mut supported = vec![
                "agents.list",
                "identities.status",
                "check",
                "dispatch.create",
                "dispatch.show",
                "operation.show",
                "requests.show",
                "result",
            ];
            if permit.owner_browser() && permit.grant.permits_scope("talk") {
                supported.push("requests.sendNow");
            }
            if self.management.is_some() && permit.owner_browser() {
                supported.extend([
                    "remote.settings.show",
                    "remote.settings.set",
                    "remote.devices.list",
                    "remote.devices.rename",
                    "remote.devices.revoke",
                    "remote.devices.talk",
                    "remote.management.operation",
                ]);
            }
            return permit.response(&json!({"version":1,"profile":"local-v1","binding":"loopback-http",
                "operations":supported,"limits":{"inputBytes":self.input_limit,"outputBytes":crate::core::OUTPUT_LIMIT,
                    "callsPerDevicePerMinute":crate::budgets::CALLS,"newSendsPerDevicePerMinute":crate::budgets::SENDS,
                    "outstandingHeldPerDevice":crate::budgets::HELDS,"approvalsPerRecipientPerMinute":crate::budgets::APPROVALS}}));
        }
        if crate::admission::RemoteOperation::parse(operation).is_some_and(|operation| {
            matches!(
                operation.class(),
                crate::admission::OperationClass::Read
                    | crate::admission::OperationClass::RecoveryObservation
            )
        }) {
            return self.read(permit);
        }
        if operation == "requests.sendNow" {
            return self.send_now(permit);
        }
        let (id, notice) = match operation {
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
                // A retry finds the send already frozen: it asks Core nothing.
                let known = permit
                    .with_store(|store| {
                        store.owned(&permit.grant, &value.input.operation_id, now_ms()?)
                    })
                    .is_ok();
                let (attributed, said) = if known || !permit.owner_browser() {
                    (frozen.clone(), None)
                } else {
                    self.attributed(&frozen)
                };
                let (frozen, said) = if attributed.len() > self.input_limit {
                    (frozen, None)
                } else {
                    (attributed, said)
                };
                let adopted = permit.adopt(Some(&frozen), &value.input.recipient_ids)?;
                // What was said is about this freeze: a retry that finds an earlier one stored says
                // nothing about it.
                let notice = said.filter(|_| adopted.frozen.as_deref() == Some(frozen.as_slice()));
                (adopted.id, notice)
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
        let mut payload = payload;
        if let (Some(notice), Some(fields)) = (notice, payload.as_object_mut()) {
            fields.insert("identity".into(), notice);
        }
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
            "dispatch.show" | "operation.show" => {
                let input: Lookup = if operation == "dispatch.show" {
                    read_input(permit)?
                } else {
                    serde_json::from_value(permit.message.input.clone()).map_err(|_| invalid())?
                };
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
        let payload = permit.with_store(|store| store.observe(&permit.grant, || {
            match operation {
                "agents.list" => {
                    let value = self.core.agents(&self.stop)?;
                    let observed_at_ms = now_ms()?;
                    let rows = value["identities"].as_array().ok_or_else(invalid)?;
                    let mut agents = Vec::new();
                    for row in rows {
                        let id = row["id"].as_str().ok_or_else(invalid)?;
                        if !permit.authority.permits(id) { continue; }
                        let mut projected = json!({"id":id,"name":row["name"].as_str().ok_or_else(invalid)?,"presence":row["presence"].as_str().ok_or_else(invalid)?});
                        if let Some(delivery) = row.get("delivery") { projected["delivery"] = delivery.clone(); }
                        if let Some(driver) = row.get("runningDriver").and_then(Value::as_str).filter(|driver| {
                            !driver.is_empty() && driver.len() <= crate::limits::RUNNING_DRIVER_BYTES
                                && driver.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                        }) { projected["runningDriver"] = json!(driver); }
                        if let Some(activity) = row.pointer("/session/activity").filter(|activity| {
                            matches!(activity["state"].as_str(), Some("working" | "idle" | "unknown" | "ended"))
                                && activity.get("sinceMs").is_some_and(|since| since.is_null()
                                    || since.as_u64().is_some_and(|value| value <= 9_007_199_254_740_991))
                        }) {
                            projected["activity"] = json!({"state":activity["state"],"sinceMs":activity["sinceMs"]});
                        }
                        projected["observedAtMs"] = json!(observed_at_ms);
                        agents.push(projected);
                    }
                    Ok(json!({"identities":agents}))
                },
                "identities.status" => self.api(&permit.message.payload),
                "requests.show" => self.api(&permit.message.payload).map(|mut detail| {
                    if let Some(state) = detail
                        .get_mut("messageState")
                        .filter(|s| s.is_object() && !permit.owner_browser())
                    {
                        state["canSendNow"] = json!(false);
                    }
                    detail
                }),
                "check" => {
                    let input = agent.as_ref().expect("validated check input");
                    self.core.check(&input.agent_id, input.lines, &self.stop)
                },
                "result" => {
                    let id = request.as_deref().expect("validated request input");
                    let api = json!({"version":1,"operation":"requests.show","input":{"requestId":id}});
                    result_state(id, self.api(api.to_string().as_bytes()), permit.owner_browser())
                },
                "operation.show" => {
                    let owned = owned.as_ref().expect("owned operation");
                    if matches!(owned.phase.as_str(), "held" | "accepted" | "cancelled" | "refused") {
                        return Ok(state(owned));
                    }
                    let frozen = owned.frozen.as_deref().ok_or_else(|| database("owned frozen intent absent"))?;
                    held_intent(&permit.grant, &owned.id, frozen)?;
                    // Missing/malformed core receipts stay uncertain; observation never sends.
                    Ok(self.resolve(frozen, false, &permit.grant).unwrap_or_else(|_| uncertain(&owned.id)))
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
        if operation == "operation.show"
            && payload["state"] == "accepted"
            && owned
                .as_ref()
                .is_some_and(|owned| matches!(owned.phase.as_str(), "dispatching" | "uncertain"))
        {
            let owned = owned.as_ref().expect("owned recovery");
            let metadata = match permit.sessions.operation_response(
                &permit.grant,
                &owned.id,
                &payload,
                Some(permit.message.envelope().session_id),
            ) {
                Ok(metadata) => metadata,
                Err(_) => return permit.response(&uncertain(&owned.id)),
            };
            if permit
                .with_store(|store| {
                    store.settle_recovery(
                        &permit.grant,
                        &owned.id,
                        &payload,
                        metadata.as_deref(),
                        now_ms()?,
                    )
                })
                .is_err()
            {
                return permit.response(&uncertain(&owned.id));
            }
            permit.sessions.changed();
        }
        permit.response(&payload)
    }
    /// The Core payload for a send from a paired owner device: attributed to the person at this
    /// machine through `user.identity`, so Digest does not hold the person's own messages (#4255).
    /// It runs once the send is admitted and its bytes are frozen with the result, so a held send
    /// is replayed as frozen and never asks Core again. A refusal of the name (`USER_NAME_*`) keeps
    /// the send anonymous and returns the typed notice for the surface; any other failure keeps it
    /// anonymous silently. A send is never blocked or retried by it.
    fn attributed(&self, frozen: &[u8]) -> (Vec<u8>, Option<Value>) {
        let ask = json!({"version":1,"operation":"user.identity","input":{}});
        let anonymous = |notice| (frozen.to_vec(), notice);
        let person = match self.api(ask.to_string().as_bytes()) {
            Ok(person) => person,
            Err(error) if error.code.starts_with("USER_NAME_") => {
                return anonymous(Some(
                    json!({"state":"anonymous","reason":error.code,"message":error.message}),
                ));
            }
            Err(_) => return anonymous(None),
        };
        let Some(identity) = person["identityId"]
            .as_str()
            .filter(|id| canonical::is_core_id(id))
        else {
            return anonymous(None);
        };
        let Ok(mut payload) = serde_json::from_slice::<Value>(frozen) else {
            return anonymous(None);
        };
        let Some(fields) = payload.as_object_mut() else {
            return anonymous(None);
        };
        fields.remove("originator");
        fields.insert("identity".into(), json!(identity));
        let notice = person["renamedFrom"]
            .as_str()
            .map(|from| json!({"state":"person","name":person["name"],"renamedFrom":from}));
        (payload.to_string().into_bytes(), notice)
    }
    /// `requests.sendNow`: the person, at their own paired browser, asks that a message they sent
    /// be typed now (#4378). Core decides whether it may be and does the typing; this only carries
    /// the ask. It is refused here, with Core never called, unless the grant is the owner's browser
    /// and this device sent the request. No journal row is kept: a repeat is answered from Core's
    /// own state (`already`), and the audit row is written with the call's fence.
    fn send_now(&self, permit: &MessagePermit) -> Result<Vec<u8>, RemoteError> {
        if !permit.owner_browser() {
            return Err(RemoteError::new(
                "REMOTE_SCOPE_DENIED",
                "Send now is for the owner's own browser.",
            ));
        }
        let input: ResultInput = read_input(permit)?;
        let reference = request_reference(&input.request_id)?;
        let digest = crate::journal::intent(&permit.message)?;
        let (state, reason) = permit.with_store(|store| {
            store.send_now(
                &permit.grant,
                permit.message.envelope().id,
                &reference,
                &digest,
                now_ms,
                || self.send_now_outcome(&input.request_id),
            )
        })?;
        // Read back after the fence has committed, so the lock is not held for it.
        let reply = self.send_now_reply(&input.request_id, state, reason);
        permit.response(&reply)
    }
    /// What the ask came to, from Core's own answers and never a guess: `sent` only when Core
    /// typed it; `refused` with Core's code when it typed nothing; `unconfirmed` when it may have
    /// typed it, which includes every failure on this side of the call (a timeout, an interruption,
    /// an unreadable answer); `unavailable` when the person could not be resolved or Core answered
    /// with some other error, so nothing was typed.
    fn send_now_outcome(&self, id: &str) -> (&'static str, Option<String>) {
        let who = json!({"version":1,"operation":"user.identity","input":{}});
        let person = match self.api(who.to_string().as_bytes()) {
            Ok(person) => person["identityId"]
                .as_str()
                .filter(|id| canonical::is_core_id(id))
                .map(str::to_owned)
                .ok_or_else(|| "REMOTE_CORE_UNAVAILABLE".to_owned()),
            Err(error) => Err(error.code),
        };
        let person = match person {
            Ok(person) => person,
            Err(code) => return ("unavailable", Some(code)),
        };
        let ask = json!({"version":1,"operation":"requests.sendNow","identity":person,"input":{"requestId":id}});
        match self.api(ask.to_string().as_bytes()) {
            Ok(done) => match done["status"].as_str() {
                Some("sent") => ("sent", None),
                Some("already") => ("already", None),
                _ => ("unconfirmed", None),
            },
            Err(error) => match error.code.as_str() {
                "DELIVERY_UNCERTAIN"
                | "REMOTE_CORE_UNAVAILABLE"
                | "REMOTE_CORE_UNCERTAIN"
                | "REMOTE_IO" => ("unconfirmed", Some(error.code)),
                code if code.starts_with("SEND_NOW_")
                    || matches!(
                        code,
                        "DELIVERY_AWAITING_APPROVAL" | "DELIVERY_CHANNEL_UNAVAILABLE"
                    ) =>
                {
                    ("refused", Some(error.code))
                }
                _ => ("unavailable", Some(error.code)),
            },
        }
    }
    /// The reply to the browser: the outcome plus Core's fresh `messageState`.
    fn send_now_reply(&self, id: &str, state: &str, reason: Option<String>) -> Value {
        let mut reply = json!({"state":state,"requestId":id});
        if let Some(reason) = reason.filter(|reason| {
            reason.len() <= 64 && reason.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        }) {
            reply["reason"] = json!(reason);
        }
        let show = json!({"version":1,"operation":"requests.show","input":{"requestId":id}});
        if let Some(state) = self
            .api(show.to_string().as_bytes())
            .ok()
            .and_then(|detail| message_state(&detail, true))
        {
            reply["messageState"] = state;
        }
        reply
    }
    fn resolve(&self, frozen: &[u8], retry: bool, grant: &Grant) -> Result<Value, RemoteError> {
        let intent = frozen_dispatch(frozen)?.input;
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettlementKind {
    Effect,
    RecoveryObservation,
}
impl Store {
    /// A bounded public observation is ordered against cross-process grant revocation.
    fn observe<T>(&mut self, grant: &Grant, action: impl FnOnce() -> T) -> Result<T, RemoteError> {
        let (tx, _) = self.authorized_at(grant, now_ms)?;
        let value = action();
        tx.commit().map_err(database)?;
        Ok(value)
    }
    /// A send-now is ordered against revocation like a dispatch effect: authority, this device's
    /// ownership of the request and the audit row commit with the bounded core call.
    pub(crate) fn send_now<T>(
        &mut self,
        grant: &Grant,
        envelope: &str,
        request: &str,
        digest: &[u8],
        clock: impl FnOnce() -> Result<u64, RemoteError>,
        action: impl FnOnce() -> T,
    ) -> Result<T, RemoteError> {
        let (tx, now) = self.authorized_at(grant, clock)?;
        let sent_by_device: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM operations o, json_each(o.references_json) r
                 WHERE o.client_id=?1 AND o.operation='dispatch.create' AND r.value=?2)",
                params![grant.client_id, request],
                |row| row.get(0),
            )
            .map_err(database)?;
        if !sent_by_device {
            return Err(RemoteError::new(
                "REMOTE_SCOPE_DENIED",
                "Request was not sent by this device.",
            ));
        }
        audit::append(
            &tx,
            audit::AuditMetadata {
                time: now,
                client: &grant.client_id,
                request: envelope,
                operation: "requests.sendNow",
                operation_id: None,
                resources: &[request.to_owned()],
                digest,
                revision: grant.revision,
                decision: "dispatching",
                code: "",
            },
        )?;
        let result = action();
        tx.commit().map_err(database)?;
        Ok(result)
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
        self.settlement(grant, id, payload, metadata, now, SettlementKind::Effect)
    }
    // Recovery updates existing ownership even if no notification entry fits.
    fn settle_recovery(
        &mut self,
        grant: &Grant,
        id: &str,
        payload: &Value,
        metadata: Option<&[u8]>,
        now: u64,
    ) -> Result<(), RemoteError> {
        self.settlement(
            grant,
            id,
            payload,
            metadata,
            now,
            SettlementKind::RecoveryObservation,
        )
    }
    fn settlement(
        &mut self,
        grant: &Grant,
        id: &str,
        payload: &Value,
        metadata: Option<&[u8]>,
        now: u64,
        kind: SettlementKind,
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
        if matches!(old.as_str(), "accepted" | "cancelled" | "refused")
            || (kind == SettlementKind::RecoveryObservation
                && (!matches!(old.as_str(), "dispatching" | "uncertain") || phase != "accepted"))
        {
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
            "UPDATE operations SET phase=?2,receipt=?3,references_json=?4,updated_ms=?5,frozen=CASE WHEN ?2 IN ('accepted','cancelled','refused') THEN NULL ELSE frozen END WHERE id=?1",
            params![id, phase, payload.to_string(),serde_json::to_string(&resources).map_err(database)?,now as i64],
        )
        .map_err(database)?;
        if let Some(metadata) = metadata {
            crate::journal::prune(&tx, &grant.client_id, now)?;
            crate::journal::make_room_for_entry(&tx, &grant.client_id)?;
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
        store::{SUPPORTED_SCOPES, uuid_v4},
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
            scopes: SUPPORTED_SCOPES.iter().map(|s| (*s).into()).collect(),
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
