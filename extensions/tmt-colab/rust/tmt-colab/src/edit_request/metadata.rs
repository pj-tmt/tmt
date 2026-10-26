//! Version 2 of an editor request: a closed metadata action, not source or content authority.
use serde::Deserialize;
use serde_json::{Value, json};
use tmt_colab_model::{Invalid, crypto, values};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub version: u8,
    pub kind: String,
    pub operation_id: String,
    pub action: String,
    pub body: Value,
    /// The complete observed creator/legacy recipient pair or registered question entry.
    pub target: Value,
}
impl Request {
    pub fn read(root: &str, key: &str, value: &Value) -> tmt_colab_model::Result<Self> {
        let r: Self = serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
        values::generated_id(&r.operation_id)?;
        if root != "intents"
            || key != r.operation_id
            || r.version != 2
            || r.kind != "edit-request"
            || crate::closed::canonical(value).map_err(|_| Invalid)?.len() > 8192
        {
            return Err(Invalid);
        }
        // These are observed metadata field names, not arbitrary JSON keys. Restricting keys
        // to nonnumeric ASCII fields keeps browser/native ordering identical: browser JSON
        // reorders integer-index keys even after lexical sorting.
        fn field_keys(value: &Value) -> bool {
            match value {
                Value::Object(map) => map.iter().all(|(key, value)| {
                    key.is_ascii()
                        && (key.is_empty() || key.bytes().any(|b| !b.is_ascii_digit()))
                        && field_keys(value)
                }),
                Value::Array(items) => items.iter().all(field_keys),
                _ => true,
            }
        }
        if !field_keys(&r.target) {
            return Err(Invalid);
        }
        match r.action.as_str() {
            "creator.set" => {
                crate::closed::keys(&r.body, &["selector"], &[]).map_err(|_| Invalid)?;
                if !r.body["selector"].as_str().is_some_and(|s| {
                    !s.is_empty()
                        && s.len() <= 256
                        && s.trim() == s
                        && !s.chars().any(char::is_control)
                }) {
                    return Err(Invalid);
                }
                let target = r.target.as_object().ok_or(Invalid)?;
                if target.len() != 2
                    || !target.contains_key("creator")
                    || !target.contains_key("creationRecipient")
                {
                    return Err(Invalid);
                }
            }
            "question.handle" | "question.reopen" => {
                crate::page::metadata::validate(&r.action, &r.body).map_err(|_| Invalid)?;
                if !r.target.is_object() || r.target["id"] != r.body["id"] {
                    return Err(Invalid);
                }
            }
            _ => return Err(Invalid),
        }
        Ok(r)
    }

    /// A request's own append and unrelated conversations do not change its metadata target.
    pub fn current_target(&self, page: &crate::page::Page) -> Value {
        if self.action == "creator.set" {
            json!({"creator":page.creator,"creationRecipient":page.creation_recipient})
        } else {
            page.questions
                .as_ref()
                .and_then(Value::as_array)
                .and_then(|rows| rows.iter().find(|row| row["id"] == self.body["id"]))
                .cloned()
                .unwrap_or(Value::Null)
        }
    }

    pub fn prepare(
        &self,
        open: &crate::page::save::SourceOpener,
        page: &str,
        operation: &str,
    ) -> crate::Result<crate::page::metadata::MetadataEdit> {
        let mut source = open()?;
        let current = crate::page::read(&source.store, &source.keyring, page, &mut source.decoder)?;
        if self.target != self.current_target(&current) {
            return Err(crate::page::Fault::StaleBase.into());
        }
        let body = if self.action == "creator.set" {
            resolve_creator(self.body["selector"].as_str().ok_or(Invalid)?)?
        } else {
            self.body.clone()
        };
        Ok(crate::page::metadata::MetadataEdit {
            page: page.into(),
            operation_id: operation.into(),
            expected_revision: current.revision,
            action: self.action.clone(),
            body,
        })
    }
}

/// Domain-separated from owner operations and other links, including across page epochs.
pub fn operation(page: &str, epoch: &str, writer: &str, request: &str) -> String {
    let mut bytes = crypto::digest(
        &serde_json::to_vec(&json!([
            "tmt-colab-editor-metadata-v1",
            page,
            epoch,
            writer,
            request
        ]))
        .expect("strings serialize"),
    );
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let h: String = bytes[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

pub fn outcome_key(writer: &str, operation: &str) -> String {
    format!("{writer}:{operation}:edit-result")
}

/// Own-record syntax grants no authority. Consumers must additionally require owner-member
/// provenance, the exact request writer/operation and this digest of the immutable request.
pub fn validate_result(root: &str, key: &str, value: &Value) -> tmt_colab_model::Result<()> {
    crate::closed::keys(
        value,
        &[
            "version",
            "kind",
            "writer",
            "operationId",
            "requestSha256",
            "result",
        ],
        &[],
    )
    .map_err(|_| Invalid)?;
    let writer = value["writer"].as_str().ok_or(Invalid)?;
    let operation = value["operationId"].as_str().ok_or(Invalid)?;
    values::generated_id(writer)?;
    values::generated_id(operation)?;
    if root != "messages"
        || value["kind"] != "edit-result"
        || value["version"] != 1
        || key != outcome_key(writer, operation)
        || values::binary(value["requestSha256"].as_str().ok_or(Invalid)?, 32)?.len() != 32
    {
        return Err(Invalid);
    }
    let r = &value["result"];
    crate::closed::keys(
        r,
        &["operationId", "state"],
        &["revision", "code", "message"],
    )
    .map_err(|_| Invalid)?;
    if r["operationId"] != operation {
        return Err(Invalid);
    }
    match r["state"].as_str() {
        Some("committed" | "unchanged") => {
            if !r["revision"]
                .as_str()
                .is_some_and(crate::publication::valid_revision)
                || r.get("code").is_some()
                || r.get("message").is_some()
            {
                return Err(Invalid);
            }
        }
        Some("rejected") => {
            if r.get("revision").is_some()
                || !r["code"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("COLAB_") && s.len() <= 80)
                || !r["message"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 2048)
            {
                return Err(Invalid);
            }
        }
        _ => return Err(Invalid),
    }
    Ok(())
}

/// Refusals are browser-readable; no owner-store CLI commands reach a link holder.
#[derive(Debug)]
pub struct IdentityRefused(pub &'static str);
impl std::fmt::Display for IdentityRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            "COLAB_IDENTITY_UNKNOWN" => "That identity was not found. Check the name or ID and try again.",
            _ => "The identity could not be verified. Try again when the owner's identity directory is available.",
        })
    }
}
impl std::error::Error for IdentityRefused {}

pub fn resolve_creator(selector: &str) -> crate::Result<Value> {
    resolve_observed(crate::core::identity_lookup(selector))
}
fn resolve_observed(lookup: crate::core::IdentityLookup) -> crate::Result<Value> {
    match lookup {
        crate::core::IdentityLookup::Found(agent) => agent
            .agent_id
            .map(|id| json!({"agent":{"id":id,"name":agent.name}}))
            .ok_or_else(|| IdentityRefused("COLAB_IDENTITY_UNVERIFIED").into()),
        crate::core::IdentityLookup::NotFound => {
            Err(IdentityRefused("COLAB_IDENTITY_UNKNOWN").into())
        }
        crate::core::IdentityLookup::Unavailable => {
            Err(IdentityRefused("COLAB_IDENTITY_UNVERIFIED").into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_authenticated_resolved_identity_is_bound() {
        use crate::core::{CallerSnapshot, IdentityLookup};
        let id = "10000000-0000-4000-8000-000000000001";
        assert_eq!(
            resolve_observed(IdentityLookup::Found(CallerSnapshot {
                agent_id: Some(id.into()),
                name: "reviewer".into(),
            }))
            .unwrap(),
            json!({"agent":{"id":id,"name":"reviewer"}})
        );
        for state in [
            IdentityLookup::NotFound,
            IdentityLookup::Unavailable,
            IdentityLookup::Found(CallerSnapshot {
                agent_id: None,
                name: "reviewer".into(),
            }),
        ] {
            assert!(resolve_observed(state).is_err());
        }
    }
}
