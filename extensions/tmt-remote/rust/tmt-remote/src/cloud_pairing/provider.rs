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

pub struct EnrollmentDocument {
    pub id: String,
    pub uid: String,
    pub kind: String,
    pub body: Vec<u8>,
    pub created_ms: u64,
    pub cursor: Value,
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
        .filter(|f| f.len() == 4)
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
    })
}
impl Provider {
    pub fn uid(&self) -> &str {
        &self.identity().uid
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
        let value = json!({"writes":[{"update":{"name":name,"fields":{"uid":{"stringValue":uid},"kind":{"stringValue":kind},"body":{"bytesValue":STANDARD.encode(body)}}},"currentDocument":{"exists":false},"updateTransforms":[{"fieldPath":"createdAt","setToServerValue":"REQUEST_TIME"}]}]});
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
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        let fields = json!({"uid":{"stringValue":self.uid()},"authenticationKey":{"bytesValue":STANDARD.encode(key)},"descriptor":{"bytesValue":STANDARD.encode(descriptor)}});
        let existing = self.machine_metadata(io).await?;
        if let Some(old) = &existing {
            if old["fields"]["uid"]["stringValue"] != self.uid()
                || old["fields"]["authenticationKey"] != fields["authenticationKey"]
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
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        if rows.is_empty() {
            return Ok(());
        }
        if rows.len() > limits::CLOUD_ROUTE_BATCH
            || rows
                .iter()
                .any(|r| r.peer != rows[0].peer || r.project != self.identity().project)
        {
            return Err(invalid());
        }
        let writes:Vec<_>=rows.iter().map(|r|json!({"update":{"name":format!("{}/m/{}/peers/{}",self.root(),self.machine(),r.uid),"fields":{"peer":{"stringValue":r.peer},"revoked":{"booleanValue":r.revoked},"expiresAt":{"timestampValue":"9999-12-31T23:59:59.999Z"}}}})).collect();
        let response = self
            .call(
                Operation::Commit,
                ":commit",
                json!({"writes":writes}),
                WRITE_LOOKUPS * rows.len() as u64,
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
