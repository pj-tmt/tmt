//! Fixed Remote-only management, with live authority and immutable outcome ownership.
//! No historical-key reader, agent dispatch or held-work path exists here.
use crate::{
    admission::MessagePermit,
    audit, canonical,
    error::RemoteError,
    journal,
    pairing::now_ms,
    settings,
    store::{Grant, Store, database, grant_in, rename_in, revoke_in},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, RemoteError>;
const READ_ONLY: &str = "REMOTE_MANAGEMENT_READ_ONLY";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingInput {
    operation_id: String,
    setting: String,
    value: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeviceInput {
    operation_id: String,
    client_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenameInput {
    operation_id: String,
    client_id: String,
    name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Lookup {
    operation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageInput {
    cursor: Option<String>,
    limit: usize,
}

enum Mutation {
    Setting { key: &'static str, value: Value },
    Rename { client: String, name: String },
    Revoke { client: String },
}
fn invalid() -> RemoteError {
    RemoteError::new("REMOTE_INPUT_INVALID", "Invalid Remote management input.")
}
fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|_| invalid())
}
/// Decimal wire admission never rounds through JavaScript's number representation.
fn cap(value: &Value) -> Result<Option<usize>> {
    if value.is_null() {
        return Ok(None);
    }
    let text = value.as_str().ok_or_else(invalid)?;
    if text.is_empty() || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    text.parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .map(Some)
        .ok_or_else(invalid)
}
fn safe_time(now: u64) -> Result<i64> {
    if now > 9_007_199_254_740_991 {
        return Err(invalid());
    }
    i64::try_from(now).map_err(database)
}
fn settings_json(loaded: &settings::RemoteSettings) -> Value {
    let defaults = settings::RemoteSettings::default();
    let effective = if loaded.malformed { &defaults } else { loaded };
    json!({"open":effective.open(),"source":effective.source(),
        "sessionsPerDevice":effective.sessions_per_device().map(|n|n.to_string()),
        "sessionsPerDeviceSource":effective.sessions_source(),
        "warning":if loaded.malformed { Some(settings::UNREADABLE) } else { None }})
}
fn summary(grant: &Grant) -> Value {
    json!({"clientId":grant.client_id,"name":grant.name,"kind":grant.kind,
        "issuedAtMs":grant.issued_at_ms,"expiresAtMs":grant.expires_at_ms,
        "revision":grant.revision,"revoked":grant.disabled})
}
fn unknown(id: &str) -> Value {
    json!({"operationId":id,"state":"unknown","reason":"effect_outcome_unconfirmed"})
}
fn refused(id: &str, code: &str) -> Value {
    json!({"operationId":id,"state":"refused","reason":code})
}
fn writable(connection: &Connection, grant: &Grant) -> Result<bool> {
    connection.query_row("SELECT EXISTS(SELECT 1 FROM settings_designation d JOIN machine m ON m.singleton=d.singleton JOIN grants g ON g.client_id=d.client_id WHERE d.machine_id=m.machine_id AND d.client_id=?1 AND d.public_key=?2 AND g.public_key=?2)", params![grant.client_id,grant.public_key.as_slice()], |row|row.get(0)).map_err(database)
}
fn require_writable(connection: &Connection, grant: &Grant) -> Result<()> {
    if writable(connection, grant)? {
        Ok(())
    } else {
        Err(RemoteError::new(
            READ_ONLY,
            "Read-only in this browser. Use the local CLI.",
        ))
    }
}
fn receipt(connection: &Connection, client: &str, id: &str, now: u64) -> Result<Value> {
    let row: Option<(String, String, i64)> = connection
        .query_row(
            "SELECT client_id,outcome,deadline_ms FROM management_receipts WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(database)?;
    match row {
        Some((owner, outcome, deadline))
            if owner == client && i64::try_from(now).is_ok_and(|now| now < deadline) =>
        {
            serde_json::from_str(&outcome).map_err(database)
        }
        _ => Err(RemoteError::new(
            "REMOTE_MANAGEMENT_UNAVAILABLE",
            "Original management outcome is unavailable.",
        )),
    }
}
fn settle(tx: &Transaction<'_>, id: &str, outcome: &Value) -> Result<()> {
    tx.execute(
        "UPDATE management_receipts SET outcome=?2 WHERE id=?1",
        params![id, outcome.to_string()],
    )
    .map_err(database)?;
    Ok(())
}
fn audit_change(
    tx: &Transaction<'_>,
    grant: &Grant,
    id: &str,
    operation: &str,
    digest: &[u8],
    now: u64,
    outcome: (&str, &str),
) -> Result<()> {
    audit::append(
        tx,
        audit::AuditMetadata {
            time: now,
            client: &grant.client_id,
            request: id,
            operation,
            operation_id: Some(id),
            resources: &[],
            digest,
            revision: grant.revision,
            decision: outcome.0,
            code: outcome.1,
        },
    )
}

// Owner-local observation points: production uses a no-op; interruption tests
// pause the same writer/transaction path without replacing its algorithm.
enum EffectStage {
    BeforeTouch,
    Truncated,
    Written,
    Synced,
    BeforeCommit,
    Committed,
}

impl Store {
    /// Local owner authority only; this method is not a signed operation.
    pub fn designate(&mut self, client: &str, origin: &str, now: u64) -> Result<Grant> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database)?;
        let grant = grant_in(&tx, client)?.ok_or_else(invalid)?;
        if grant.kind != "browser" || grant.origin != origin || !grant.live_at(now) {
            return Err(invalid());
        }
        tx.execute("INSERT INTO settings_designation SELECT 1,machine_id,?1,?2 FROM machine WHERE singleton=1 ON CONFLICT(singleton) DO UPDATE SET machine_id=excluded.machine_id,client_id=excluded.client_id,public_key=excluded.public_key",params![client,grant.public_key.as_slice()]).map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(grant)
    }
    pub fn undesignate(&mut self) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database)?;
        tx.execute("DELETE FROM settings_designation", [])
            .map_err(database)?;
        tx.commit().map_err(database)
    }
    /// SQL keyset pagination happens before any grants are materialized. The cursor
    /// carries caller/last UUID only and is not an authorization capability.
    fn management_page(
        &self,
        caller: &str,
        input: PageInput,
    ) -> Result<(Vec<Grant>, Option<String>)> {
        if !(1..=crate::limits::MANAGEMENT_PAGE).contains(&input.limit) {
            return Err(invalid());
        }
        let after = match input.cursor {
            None => String::new(),
            Some(cursor) => {
                if cursor.len() != 98 {
                    return Err(invalid());
                }
                let bytes = canonical::base64url_decode(&cursor).map_err(|_| invalid())?;
                let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
                let (owner, after) = text.split_once(':').ok_or_else(invalid)?;
                if owner != caller || !canonical::is_core_id(after) {
                    return Err(invalid());
                }
                after.to_owned()
            }
        };
        let mut query = self
            .connection
            .prepare(&format!(
                "SELECT {} FROM grants WHERE client_id>?1 ORDER BY client_id LIMIT ?2",
                crate::store::GRANT_COLUMNS
            ))
            .map_err(database)?;
        let mut rows = query
            .query_map(
                params![after, (input.limit + 1) as i64],
                crate::store::grant_row,
            )
            .map_err(database)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(database)?;
        let next = if rows.len() > input.limit {
            rows.pop();
            Some(canonical::base64url(
                format!(
                    "{}:{}",
                    caller,
                    rows.last().expect("nonempty page").client_id
                )
                .as_bytes(),
            ))
        } else {
            None
        };
        Ok((rows, next))
    }
    fn management_adopt(
        &mut self,
        grant: &Grant,
        id: &str,
        operation: &str,
        digest: &[u8],
        now: u64,
    ) -> Result<Option<Value>> {
        let tx = self.authorized(grant, now)?;
        require_writable(&tx, grant)?;
        if tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM operations WHERE id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database)?
        {
            return Err(RemoteError::new(
                "REMOTE_INTENT_CONFLICT",
                "Original operation ID belongs to another intent.",
            ));
        }
        let existing: Option<(String, String, Vec<u8>)> = tx
            .query_row(
                "SELECT client_id,operation,digest FROM management_receipts WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(database)?;
        if let Some((client, op, old)) = existing {
            if client != grant.client_id || op != operation || old != digest {
                return Err(RemoteError::new(
                    "REMOTE_INTENT_CONFLICT",
                    "Original management intent conflicts.",
                ));
            }
            return receipt(&tx, &grant.client_id, id, now).map(Some);
        }
        // Never evict an unknown ID to make it eligible for an effect again.
        // These small permanent identity tombstones count toward the fixed bounds;
        // expiry makes lookup unavailable, without resetting adoption identity.
        let (client,total):(i64,i64)=tx.query_row("SELECT (SELECT COUNT(*) FROM management_receipts WHERE client_id=?1),COUNT(*) FROM management_receipts",[&grant.client_id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(database)?;
        if client >= journal::OPERATIONS || total >= 4 * journal::OPERATIONS {
            return Err(RemoteError::new(
                "REMOTE_MANAGEMENT_CAPACITY",
                "Retained management identity limit reached. Use the local CLI.",
            ));
        }
        tx.execute(
            "INSERT INTO management_receipts VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                grant.client_id,
                operation,
                digest,
                i64::try_from(grant.revision).map_err(database)?,
                safe_time(now)?,
                safe_time(now.checked_add(journal::RECOVERY).ok_or_else(invalid)?)?,
                unknown(id).to_string()
            ],
        )
        .map_err(database)?;
        audit_change(&tx, grant, id, operation, digest, now, ("adopted", ""))?;
        tx.commit().map_err(database)?;
        Ok(None)
    }
    fn management_effect(
        &mut self,
        grant: &Grant,
        id: &str,
        operation: &str,
        digest: &[u8],
        mutation: Mutation,
    ) -> Result<(Value, Option<String>)> {
        self.management_effect_observed(grant, id, operation, digest, mutation, |_| {})
    }
    // Observers run under the existing effect locks; never reacquire live/Store/settings.
    fn management_effect_observed(
        &mut self,
        grant: &Grant,
        id: &str,
        operation: &str,
        digest: &[u8],
        mutation: Mutation,
        mut observe: impl FnMut(EffectStage),
    ) -> Result<(Value, Option<String>)> {
        let root = self.data_root.clone();
        let (tx, now) = self.authorized_at(grant, now_ms)?;
        require_writable(&tx, grant)?;
        receipt(&tx, &grant.client_id, id, now)?; // Immutable deadline also fences delayed effects.
        let (outcome, changed) = match mutation {
            Mutation::Setting { key, value } => {
                let mut touched = false;
                let saved = settings::set_observed(&root, key, value, |stage| {
                    if stage == settings::WriteStage::BeforeTouch {
                        touched = true;
                    }
                    observe(match stage {
                        settings::WriteStage::BeforeTouch => EffectStage::BeforeTouch,
                        settings::WriteStage::Truncated => EffectStage::Truncated,
                        settings::WriteStage::Written => EffectStage::Written,
                        settings::WriteStage::Synced => EffectStage::Synced,
                    });
                    Ok(())
                });
                match saved {
                    Ok(loaded) => (
                        json!({"operationId":id,"state":"committed","result":{"settings":settings_json(&loaded)},"sessionEnded":false}),
                        None,
                    ),
                    Err(_) if !touched => (refused(id, "REMOTE_SETTINGS_UNAVAILABLE"), None),
                    Err(_) => (unknown(id), None),
                }
            }
            Mutation::Rename { client, name } => {
                let before = grant_in(&tx, &client)?;
                match rename_in(&tx, &client, &name) {
                    Ok(Some(target)) => {
                        let changed = before.is_some_and(|g| g.revision != target.revision);
                        (
                            json!({"operationId":id,"state":"committed","result":{"device":summary(&target)},"sessionEnded":changed && client==grant.client_id}),
                            changed.then_some(client),
                        )
                    }
                    Ok(None) => (refused(id, "REMOTE_DEVICE_NOT_FOUND"), None),
                    Err(error)
                        if matches!(
                            error.code.as_str(),
                            "REMOTE_INPUT_INVALID" | "REMOTE_DEVICE_REVOKED"
                        ) =>
                    {
                        (refused(id, &error.code), None)
                    }
                    Err(error) => return Err(error),
                }
            }
            Mutation::Revoke { client } => match revoke_in(&tx, &client) {
                Ok(Some(target)) => (
                    json!({"operationId":id,"state":"committed","result":{"device":summary(&target)},"sessionEnded":client==grant.client_id}),
                    Some(client),
                ),
                Ok(None) => (refused(id, "REMOTE_DEVICE_NOT_FOUND"), None),
                Err(error) => return Err(error),
            },
        };
        settle(&tx, id, &outcome)?;
        let decision = match outcome["state"].as_str() {
            Some("committed") => "committed",
            Some("refused") => "refused",
            _ => "uncertain",
        };
        audit_change(
            &tx,
            grant,
            id,
            operation,
            digest,
            now,
            (decision, outcome["reason"].as_str().unwrap_or("")),
        )?;
        observe(EffectStage::BeforeCommit);
        tx.commit().map_err(database)?;
        observe(EffectStage::Committed);
        Ok((outcome, changed))
    }
}

/// Called only after ordinary signed Session admission. Current browser kind/origin
/// is checked even when the caller has all agent scopes.
pub(crate) fn append(permit: &MessagePermit, devices: &crate::devices::Devices) -> Result<Vec<u8>> {
    if permit.message.payload.len() > crate::limits::MANAGEMENT_INPUT_BYTES {
        return Err(invalid());
    }
    if permit.grant.kind != "browser" || permit.grant.origin != permit.sessions.door_origin() {
        return Err(invalid());
    }
    let envelope = permit.message.envelope();
    let operation = envelope.operation;
    match operation {
        "remote.settings.show" => {
            if permit.message.input != json!({}) {
                return Err(invalid());
            }
            let value=permit.with_store(|store| {
                let root=store.data_root.clone();
                let tx=store.authorized(&permit.grant,now_ms()?)?;
                let allowed=writable(&tx,&permit.grant)?;
                let loaded=settings::read_or_default(&root);
                Ok(json!({"settings":settings_json(&loaded),"capabilities":{"settingsWrite":allowed,"devicesWrite":allowed},"readOnlyReason":if allowed {None}else{Some("local_cli_required")}}))
            })?;
            return permit.response(&value);
        }
        "remote.devices.list" => {
            if !permit
                .message
                .input
                .as_object()
                .is_some_and(|value| value.len() == 2)
            {
                return Err(invalid());
            }
            let input: PageInput = decode(&permit.message.input)?;
            let (rows, next) =
                permit.with_store(|store| store.management_page(&permit.grant.client_id, input))?;
            let metadata = permit.sessions.management_activity(&rows, now_ms()?)?;
            let values = rows
                .iter()
                .zip(metadata)
                .map(|(grant, (count, activity))| {
                    let mut value = summary(grant);
                    value["thisBrowser"] = json!(grant.client_id == permit.grant.client_id);
                    value["liveSessionCount"] = json!(count);
                    value["lastActivityAtMs"] = json!(activity);
                    value
                })
                .collect::<Vec<_>>();
            permit.revalidate()?;
            return permit.response(&json!({"devices":values,"nextCursor":next}));
        }
        "remote.management.operation" => {
            let input: Lookup = decode(&permit.message.input)?;
            if !canonical::is_core_id(&input.operation_id) {
                return Err(invalid());
            }
            let value = permit.with_store(|store| {
                receipt(
                    &store.connection,
                    &permit.grant.client_id,
                    &input.operation_id,
                    now_ms()?,
                )
            })?;
            return permit.response(&value);
        }
        _ => {}
    }
    let (id, mutation) = match operation {
        "remote.settings.set" => {
            let input: SettingInput = decode(&permit.message.input)?;
            let (key, value) = match input.setting.as_str() {
                "open" if input.value.is_boolean() => ("open", input.value),
                "sessions-per-device" => ("sessionsPerDevice", json!(cap(&input.value)?)),
                _ => return Err(invalid()),
            };
            (input.operation_id, Mutation::Setting { key, value })
        }
        "remote.devices.rename" => {
            let input: RenameInput = decode(&permit.message.input)?;
            if !canonical::is_core_id(&input.client_id) || !canonical::device_name(&input.name) {
                return Err(invalid());
            }
            (
                input.operation_id,
                Mutation::Rename {
                    client: input.client_id,
                    name: input.name,
                },
            )
        }
        "remote.devices.revoke" => {
            let input: DeviceInput = decode(&permit.message.input)?;
            if !canonical::is_core_id(&input.client_id) {
                return Err(invalid());
            }
            (
                input.operation_id,
                Mutation::Revoke {
                    client: input.client_id,
                },
            )
        }
        _ => return Err(invalid()),
    };
    if id != envelope.id || !canonical::is_core_id(&id) {
        return Err(invalid());
    }
    let digest = journal::intent(&permit.message)?;
    let previous = permit.with_store(|store| {
        store.management_adopt(&permit.grant, &id, operation, &digest, now_ms()?)
    })?;
    let outcome = if let Some(outcome) = previous {
        outcome
    } else {
        match permit.with_store(|store| {
            store.management_effect(&permit.grant, &id, operation, &digest, mutation)
        }) {
            Ok((outcome, changed)) => {
                if let Some(client) = changed {
                    devices.committed_change(&client);
                }
                outcome
            }
            Err(_) => unknown(&id),
        }
    };
    permit.response(&outcome)
}

#[cfg(test)]
mod tests;
