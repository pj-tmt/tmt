//! Enrollment metadata uses the carrier driver's ordinary-user HTTP/Auth/budget
//! owner. No Admin credential, operation admission or second request runner.
use super::wire::invalid;
use crate::cloud_inbox::stamp;
use crate::{
    canonical,
    cloud_inbox::Provider,
    error::RemoteError,
    firebase_auth::{Operation, Response},
    firestore_budget::{READ_LOOKUPS, WRITE_LOOKUPS},
    limits,
    objects::IoBudget,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

/// Rules capability only. Native possession and generation checks remain separate.
pub enum EnrollmentProof<'a> {
    Invite(&'a [u8; 32]),
    Reroute { peer: &'a str, token: &'a [u8; 32] },
}

pub struct EnrollmentDocument {
    pub id: String,
    pub uid: String,
    pub kind: String,
    pub body: Vec<u8>,
    pub created_ms: u64,
    pub cursor: Value,
    pub token: Option<[u8; 32]>,
}

pub struct InviteGate {
    pub token_hash: [u8; 32],
    pub expires_at_ms: u64,
}
fn id_ok(id: &str) -> bool {
    canonical::uuid(id.strip_suffix("-response").unwrap_or(id)).is_ok()
}
fn status(response: &Response) -> RemoteError {
    RemoteError::new(
        if response.status == 403 {
            "REMOTE_CLOUD_ACCESS_DENIED"
        } else {
            "REMOTE_CLOUD_PROVIDER_UNAVAILABLE"
        },
        &format!("Cloud enrollment returned HTTP {}.", response.status),
    )
}
fn read(value: &Value, root: &str, machine: &str) -> Result<EnrollmentDocument, RemoteError> {
    let name = value["name"].as_str().ok_or_else(invalid)?;
    let id = name
        .strip_prefix(&format!("{root}/m/{machine}/enroll/"))
        .filter(|id| id_ok(id))
        .ok_or_else(invalid)?;
    let fields = value["fields"]
        .as_object()
        .filter(|f| (4..=6).contains(&f.len()))
        .ok_or_else(invalid)?;
    let text = |field: &str| {
        fields
            .get(field)
            .and_then(Value::as_object)
            .filter(|v| v.len() == 1)
            .and_then(|v| v.get("stringValue"))
            .and_then(Value::as_str)
            .ok_or_else(invalid)
    };
    let uid = text("uid")?;
    let kind = text("kind")?;
    if !super::uid_ok(uid)
        || !["join", "receipt", "reroute", "route-response", "refused"].contains(&kind)
    {
        return Err(invalid());
    }
    let token = if fields.len() != 4 {
        let token = fields
            .get("token")
            .and_then(Value::as_object)
            .filter(|v| v.len() == 1)
            .and_then(|v| v.get("bytesValue"))
            .and_then(Value::as_str)
            .and_then(|v| STANDARD.decode(v).ok())
            .filter(|v| v.len() == 32)
            .ok_or_else(invalid)?;
        let expected = match kind {
            "join" => 5,
            "reroute" => {
                canonical::uuid(text("peer")?).map_err(|_| invalid())?;
                6
            }
            _ => return Err(invalid()),
        };
        if fields.len() != expected || token.len() != 32 {
            return Err(invalid());
        }
        Some(token.try_into().map_err(|_| invalid())?)
    } else {
        None
    };
    let encoded = fields
        .get("body")
        .and_then(Value::as_object)
        .filter(|v| v.len() == 1)
        .and_then(|v| v.get("bytesValue"))
        .and_then(Value::as_str)
        .filter(|v| v.len() <= limits::CLOUD_ENROLLMENT_BYTES.div_ceil(3) * 4)
        .ok_or_else(invalid)?;
    let body = STANDARD.decode(encoded).map_err(|_| invalid())?;
    if body.is_empty()
        || body.len() > limits::CLOUD_ENROLLMENT_BYTES
        || STANDARD.encode(&body) != encoded
    {
        return Err(invalid());
    }
    let created_ms =
        crate::cloud_inbox::document::timestamp(value["createTime"].as_str().ok_or_else(invalid)?)?;
    let indexed = value["fields"]["createdAt"]["timestampValue"]
        .as_str()
        .ok_or_else(invalid)?;
    let indexed_ms = crate::cloud_inbox::document::timestamp(indexed)?;
    if created_ms.abs_diff(indexed_ms) > crate::session::CLOCK_SKEW.as_millis() as u64 {
        return Err(invalid());
    }
    Ok(EnrollmentDocument {
        id: id.into(),
        uid: uid.into(),
        kind: kind.into(),
        body,
        created_ms,
        cursor: json!({"values":[{"timestampValue":indexed},{"referenceValue":name}],"before":false}),
        token,
    })
}
impl Provider {
    pub fn uid(&self) -> &str {
        &self.identity().uid
    }
    pub async fn publish_invite_gate(
        &self,
        offer: &str,
        hash: &[u8; 32],
        expires_at_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        canonical::uuid(offer).map_err(|_| invalid())?;
        if expires_at_ms == 0 || expires_at_ms >= 1 << 53 {
            return Err(invalid());
        }
        let name = format!("{}/m/{}/invites/{}", self.root(), self.machine(), offer);
        let value = json!({"writes":[{"update":{"name":name,"fields":{
            "tokenHash":{"bytesValue":STANDARD.encode(hash)},
            "expiresAt":{"timestampValue":stamp(expires_at_ms)}
        }},"currentDocument":{"exists":false}}]});
        let response = self
            .call(Operation::Commit, ":commit", value, WRITE_LOOKUPS, None, io)
            .await?
            .ok_or_else(invalid)?;
        if response.status == 200 {
            Ok(())
        } else {
            Err(status(&response))
        }
    }
    pub async fn invite_gate(
        &self,
        offer: &str,
        io: &IoBudget<'_>,
    ) -> Result<Option<InviteGate>, RemoteError> {
        canonical::uuid(offer).map_err(|_| invalid())?;
        let suffix = format!("/m/{}/invites/{offer}", self.machine());
        let response = self
            .call(
                Operation::Read,
                &suffix,
                Value::Null,
                1 + READ_LOOKUPS,
                None,
                io,
            )
            .await?
            .ok_or_else(invalid)?;
        if response.status == 404 {
            return Ok(None);
        }
        if response.status != 200 {
            return Err(status(&response));
        }
        let value = crate::wire::strict_json(&response.body).ok_or_else(invalid)?;
        if value["name"] != format!("{}{suffix}", self.root()) {
            return Err(invalid());
        }
        let fields = value["fields"]
            .as_object()
            .filter(|f| f.len() == 2)
            .ok_or_else(invalid)?;
        let hash = fields
            .get("tokenHash")
            .and_then(Value::as_object)
            .filter(|v| v.len() == 1)
            .and_then(|v| v.get("bytesValue"))
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let bytes = STANDARD.decode(hash).map_err(|_| invalid())?;
        if STANDARD.encode(&bytes) != hash {
            return Err(invalid());
        }
        let expires = fields
            .get("expiresAt")
            .and_then(Value::as_object)
            .filter(|v| v.len() == 1)
            .and_then(|v| v.get("timestampValue"))
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let expires_at_ms = crate::cloud_inbox::document::timestamp(expires)?;
        if expires_at_ms == 0 || expires_at_ms >= 1 << 53 {
            return Err(invalid());
        }
        Ok(Some(InviteGate {
            token_hash: bytes.try_into().map_err(|_| invalid())?,
            expires_at_ms,
        }))
    }
    pub(super) async fn delete_invite(
        &self,
        offer: &str,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        canonical::uuid(offer).map_err(|_| invalid())?;
        let name = format!("{}/m/{}/invites/{offer}", self.root(), self.machine());
        let response = self
            .call(
                Operation::Commit,
                ":commit",
                json!({"writes":[{"delete":name}]}),
                WRITE_LOOKUPS,
                None,
                io,
            )
            .await?
            .ok_or_else(invalid)?;
        if response.status == 200 {
            Ok(())
        } else {
            Err(status(&response))
        }
    }
    /// Maintenance reads one expired gate at a time under the same ordinary-user budget.
    pub(super) async fn expired_invite(
        &self,
        now: u64,
        io: &IoBudget<'_>,
    ) -> Result<Option<String>, RemoteError> {
        let query = json!({"structuredQuery":{
            "from":[{"collectionId":"invites","allDescendants":false}],
            "where":{"fieldFilter":{"field":{"fieldPath":"expiresAt"},"op":"LESS_THAN_OR_EQUAL","value":{"timestampValue":stamp(now)}}},
            "orderBy":[{"field":{"fieldPath":"expiresAt"},"direction":"ASCENDING"}],
            "limit":1
        }});
        let response = self
            .call(
                Operation::Query,
                &format!("/m/{}:runQuery", self.machine()),
                query,
                1 + READ_LOOKUPS,
                None,
                io,
            )
            .await?
            .ok_or_else(invalid)?;
        if response.status != 200 {
            return Err(status(&response));
        }
        let rows = crate::wire::strict_json(&response.body)
            .and_then(|v| v.as_array().cloned())
            .filter(|v| v.len() <= 2)
            .ok_or_else(invalid)?;
        let mut documents = rows.iter().filter_map(|row| row.get("document"));
        let Some(document) = documents.next() else {
            return Ok(None);
        };
        if documents.next().is_some() {
            return Err(invalid());
        }
        let prefix = format!("{}/m/{}/invites/", self.root(), self.machine());
        let id = document["name"]
            .as_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .ok_or_else(invalid)?;
        canonical::uuid(id).map_err(|_| invalid())?;
        let fields = document["fields"]
            .as_object()
            .filter(|v| v.len() == 2)
            .ok_or_else(invalid)?;
        let expiry = fields
            .get("expiresAt")
            .and_then(Value::as_object)
            .filter(|v| v.len() == 1)
            .and_then(|v| v.get("timestampValue"))
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let expiry = crate::cloud_inbox::document::timestamp(expiry)?;
        let hash = fields
            .get("tokenHash")
            .and_then(Value::as_object)
            .filter(|v| v.len() == 1)
            .and_then(|v| v.get("bytesValue"))
            .and_then(Value::as_str)
            .filter(|v| v.len() == 44)
            .ok_or_else(invalid)?;
        let decoded = STANDARD.decode(hash).map_err(|_| invalid())?;
        if expiry > now || decoded.len() != 32 || STANDARD.encode(&decoded) != hash {
            return Err(invalid());
        }
        Ok(Some(id.to_owned()))
    }
    pub async fn enrollment_get(
        &self,
        id: &str,
        io: &IoBudget<'_>,
    ) -> Result<Option<EnrollmentDocument>, RemoteError> {
        if !id_ok(id) {
            return Err(invalid());
        }
        let response = self
            .call(
                Operation::Read,
                &format!("/m/{}/enroll/{id}", self.machine()),
                Value::Null,
                1 + READ_LOOKUPS,
                None,
                io,
            )
            .await?
            .ok_or_else(invalid)?;
        match response.status {
            404 => Ok(None),
            200 => Ok(Some(read(
                &crate::wire::strict_json(&response.body).ok_or_else(invalid)?,
                self.root(),
                self.machine(),
            )?)),
            _ => Err(status(&response)),
        }
    }
    pub async fn enrollment_create(
        &self,
        id: &str,
        uid: &str,
        kind: &str,
        body: &[u8],
        proof: Option<EnrollmentProof<'_>>,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        if !id_ok(id)
            || !super::uid_ok(uid)
            || body.is_empty()
            || body.len() > limits::CLOUD_ENROLLMENT_BYTES
            || !["join", "receipt", "reroute", "route-response", "refused"].contains(&kind)
        {
            return Err(invalid());
        }
        if let Some(old) = self.enrollment_get(id, io).await? {
            return if old.uid == uid && old.kind == kind && old.body == body {
                Ok(())
            } else {
                Err(RemoteError::new(
                    "REMOTE_CLOUD_PAIR_USED",
                    "The invite has already been used. Ask the owner for a new code.",
                ))
            };
        }
        let name = format!("{}/m/{}/enroll/{id}", self.root(), self.machine());
        let mut fields = json!({"uid":{"stringValue":uid},"kind":{"stringValue":kind},"body":{"bytesValue":STANDARD.encode(body)}});
        match proof {
            Some(EnrollmentProof::Invite(token)) if kind == "join" => {
                fields["token"] = json!({"bytesValue":STANDARD.encode(token)})
            }
            Some(EnrollmentProof::Reroute { peer, token }) if kind == "reroute" => {
                canonical::uuid(peer).map_err(|_| invalid())?;
                fields["peer"] = json!({"stringValue":peer});
                fields["token"] = json!({"bytesValue":STANDARD.encode(token)});
            }
            None if ["receipt", "route-response", "refused"].contains(&kind) => {}
            _ => return Err(invalid()),
        }
        let value = json!({"writes":[{"update":{"name":name,"fields":fields},"currentDocument":{"exists":false},"updateTransforms":[{"fieldPath":"createdAt","setToServerValue":"REQUEST_TIME"}]}]});
        let response = self
            .call(Operation::Commit, ":commit", value, WRITE_LOOKUPS, None, io)
            .await?
            .ok_or_else(|| {
                RemoteError::new(
                    "REMOTE_CLOUD_UNCONFIRMED",
                    "Cloud enrollment could not be confirmed. Observe the original invite.",
                )
            })?;
        if response.status == 200 {
            Ok(())
        } else if response.status == 409 {
            let old = self.enrollment_get(id, io).await?.ok_or_else(invalid)?;
            if old.uid == uid && old.kind == kind && old.body == body {
                Ok(())
            } else {
                Err(invalid())
            }
        } else {
            Err(status(&response))
        }
    }
    pub async fn enrollment_delete(&self, id: &str, io: &IoBudget<'_>) -> Result<(), RemoteError> {
        if !id_ok(id) {
            return Err(invalid());
        }
        let response = self.call(Operation::Commit, ":commit", json!({"writes":[{"delete":format!("{}/m/{}/enroll/{id}", self.root(), self.machine())}]}), WRITE_LOOKUPS, None, io).await?.ok_or_else(invalid)?;
        if response.status == 200 {
            Ok(())
        } else {
            Err(status(&response))
        }
    }
    pub async fn enrollment_next(
        &self,
        from: u64,
        after: Option<&Value>,
        io: &IoBudget<'_>,
    ) -> Result<Option<EnrollmentDocument>, RemoteError> {
        let mut value = json!({"structuredQuery":{"from":[{"collectionId":"enroll","allDescendants":false}],"where":{"fieldFilter":{"field":{"fieldPath":"createdAt"},"op":"GREATER_THAN_OR_EQUAL","value":{"timestampValue":stamp(from)}}},"orderBy":[{"field":{"fieldPath":"createdAt"},"direction":"ASCENDING"},{"field":{"fieldPath":"__name__"},"direction":"ASCENDING"}],"limit":1}});
        if let Some(after) = after {
            value["structuredQuery"]["startAt"] = after.clone();
        }
        let response = self
            .call(
                Operation::Query,
                &format!("/m/{}:runQuery", self.machine()),
                value,
                1 + READ_LOOKUPS,
                None,
                io,
            )
            .await?
            .ok_or_else(invalid)?;
        if response.status != 200 {
            return Err(status(&response));
        }
        let rows = crate::wire::strict_json(&response.body)
            .and_then(|v| v.as_array().cloned())
            .filter(|v| v.len() <= 2)
            .ok_or_else(invalid)?;
        rows.into_iter()
            .filter_map(|v| v.get("document").cloned())
            .map(|v| read(&v, self.root(), self.machine()))
            .next()
            .transpose()
    }
    pub async fn machine_metadata(&self, io: &IoBudget<'_>) -> Result<Option<Value>, RemoteError> {
        let response = self
            .call(
                Operation::Read,
                &format!("/m/{}", self.machine()),
                Value::Null,
                1 + READ_LOOKUPS,
                None,
                io,
            )
            .await?
            .ok_or_else(invalid)?;
        match response.status {
            404 => Ok(None),
            200 => Ok(Some(
                crate::wire::strict_json(&response.body).ok_or_else(invalid)?,
            )),
            _ => Err(status(&response)),
        }
    }
    pub async fn register_machine(
        &self,
        key: &[u8; 32],
        descriptor: &[u8],
        inviting_owner: Option<&str>,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        if let Some(owner) = inviting_owner {
            canonical::uuid(owner).map_err(|_| invalid())?;
            if owner == self.machine() {
                return Err(invalid());
            }
        }
        let mut fields = json!({"uid":{"stringValue":self.uid()},"authenticationKey":{"bytesValue":STANDARD.encode(key)},"descriptor":{"bytesValue":STANDARD.encode(descriptor)}});
        if let Some(owner) = inviting_owner {
            fields["invitingOwner"] = json!({"stringValue":owner});
        }
        let existing = self.machine_metadata(io).await?;
        if let Some(old) = &existing {
            if old["fields"]["uid"]["stringValue"] != self.uid()
                || old["fields"]["authenticationKey"] != fields["authenticationKey"]
                || old["fields"].get("invitingOwner") != fields.get("invitingOwner")
            {
                return Err(RemoteError::new(
                    "REMOTE_CLOUD_MACHINE_CONFLICT",
                    "The machine registration belongs to another identity; it was not changed.",
                ));
            }
            if old["fields"]["descriptor"] == fields["descriptor"] {
                return Ok(());
            }
        }
        let name = format!("{}/m/{}", self.root(), self.machine());
        let write = if existing.is_some() {
            json!({"update":{"name":name,"fields":{"descriptor":fields["descriptor"]}},"updateMask":{"fieldPaths":["descriptor"]},"currentDocument":{"exists":true}})
        } else {
            json!({"update":{"name":name,"fields":fields},"currentDocument":{"exists":false}})
        };
        let response = self
            .call(
                Operation::Commit,
                ":commit",
                json!({"writes":[write]}),
                WRITE_LOOKUPS,
                None,
                io,
            )
            .await?
            .ok_or_else(invalid)?;
        if response.status == 200 {
            Ok(())
        } else if response.status == 409 {
            Err(RemoteError::new(
                "REMOTE_CLOUD_MACHINE_CONFLICT",
                "The machine registration belongs to another identity; it was not changed.",
            ))
        } else {
            Err(status(&response))
        }
    }
    pub async fn publish_routes(
        &self,
        rows: &[super::RouteChange],
        invited_machine: Option<&str>,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        if rows.is_empty() {
            return if invited_machine.is_none() {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        let live: Vec<_> = rows.iter().filter(|r| !r.revoked).collect();
        let hash = rows[0].reroute_hash;
        if rows.len() + usize::from(invited_machine.is_some()) + usize::from(hash.is_some())
            > limits::CLOUD_ROUTE_BATCH
            || live.len() > 1
            || invited_machine.is_some_and(|machine| machine != rows[0].peer || live.len() != 1)
            || canonical::uuid(&rows[0].peer).is_err()
            || rows
                .iter()
                .any(|r| !super::uid_ok(&r.uid) || r.generation == 0 || r.generation >= 1 << 53)
            || rows
                .iter()
                .map(|r| &r.uid)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != rows.len()
            || rows.iter().any(|r| {
                r.peer != rows[0].peer
                    || r.project != self.identity().project
                    || r.reroute_hash != hash
            })
        {
            return Err(invalid());
        }
        let mut writes:Vec<_>=rows.iter().map(|r|json!({"update":{"name":format!("{}/m/{}/peers/{}",self.root(),self.machine(),r.uid),"fields":{"peer":{"stringValue":r.peer},"revoked":{"booleanValue":r.revoked},"expiresAt":{"timestampValue":"9999-12-31T23:59:59.999Z"}}}})).collect();
        if let Some(machine) = invited_machine {
            writes.push(json!({"update":{
                "name":format!("{}/m/{machine}",self.root()),
                "fields":{"uid":{"stringValue":live[0].uid}}
            },"updateMask":{"fieldPaths":["uid"]},"currentDocument":{"exists":true}}));
        }
        if let Some(hash) = hash {
            let name = format!(
                "{}/m/{}/reroutes/{}",
                self.root(),
                self.machine(),
                rows[0].peer
            );
            writes.push(if live.is_empty() { json!({"delete":name}) } else {
                json!({"update":{"name":name,"fields":{"tokenHash":{"bytesValue":STANDARD.encode(hash)}}}})
            });
        }
        let charge = WRITE_LOOKUPS * writes.len() as u64;
        let response = self
            .call(
                Operation::Commit,
                ":commit",
                json!({"writes":writes}),
                charge,
                None,
                io,
            )
            .await?
            .ok_or_else(invalid)?;
        if response.status == 200 {
            Ok(())
        } else {
            Err(status(&response))
        }
    }
}
