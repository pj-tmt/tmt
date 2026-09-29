//! Scoped Firestore block reads and conditional single-document commits.

use super::{AgentCredential, DeploymentMode, OfficeDeployment, OfficeError, valid_token};
use crate::office_http;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Instant;
use tmt_office_model::codec::office_block::BlockSnapshot;
use tmt_office_model::office_block::{BlockLayout, MAX_REVISION};

const DOCUMENT_LIMIT: usize = 8192;

struct Document {
    snapshot: BlockSnapshot,
    update_time: Option<String>,
}

struct BlockResource<'a> {
    deployment: &'a OfficeDeployment,
    token: &'a str,
    block_id: &'a str,
    base: &'static str,
    name: String,
    deadline: Instant,
}

impl AgentCredential {
    pub(in crate::office_pairing) fn block(
        &self,
        deployment: &OfficeDeployment,
        block_id: &str,
        edit: Option<(&BlockLayout, u64)>,
        deadline: Instant,
    ) -> Result<BlockSnapshot, OfficeError> {
        if !super::super::wire::valid_uuid(block_id) || !valid_token(&self.id_token) {
            return Err(OfficeError::CredentialsInvalid);
        }
        let resource = BlockResource {
            deployment,
            token: &self.id_token,
            block_id,
            base: match deployment.target().mode() {
                DeploymentMode::Cloud => "https://firestore.googleapis.com",
                DeploymentMode::Emulator => "http://127.0.0.1:8080",
            },
            name: format!(
                "projects/{}/databases/(default)/documents/worlds/{}/blocks/{block_id}",
                deployment.project_id(),
                deployment.target().world_id()
            ),
            deadline,
        };
        let current = resource.read()?;
        let Some((layout, expected)) = edit else {
            return Ok(current.snapshot);
        };
        if expected >= MAX_REVISION {
            return Err(OfficeError::LayoutInvalid);
        }
        if exact_retry(&current.snapshot, layout, expected) {
            return Ok(current.snapshot);
        }
        if current.snapshot.revision != expected {
            return Err(OfficeError::RevisionConflict);
        }
        match resource.commit(&current, layout, expected + 1) {
            Ok(()) => {
                let confirmed = resource.read()?;
                if confirmed.snapshot.revision < expected + 1
                    || (confirmed.snapshot.revision == expected + 1
                        && &confirmed.snapshot.layout != layout)
                {
                    return Err(OfficeError::RemoteUncertain);
                }
                Ok(confirmed.snapshot)
            }
            Err(error @ (OfficeError::RevisionConflict | OfficeError::RemoteDenied)) => {
                // A concurrent exact retry can win the conditional write. Re-read
                // under current authority; never retry with a new revision.
                let observed = resource.read()?;
                if exact_retry(&observed.snapshot, layout, expected) {
                    Ok(observed.snapshot)
                } else if observed.snapshot.revision != expected {
                    Err(OfficeError::RevisionConflict)
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }
}

fn exact_retry(snapshot: &BlockSnapshot, layout: &BlockLayout, expected: u64) -> bool {
    snapshot.revision == expected + 1 && &snapshot.layout == layout
}

impl BlockResource<'_> {
    fn read(&self) -> Result<Document, OfficeError> {
        let agent = office_http::agent(self.deployment.target().mode(), self.deadline)
            .map_err(|_| OfficeError::RemoteUncertain)?;
        let mut response = agent
            .get(format!("{}/v1/{}", self.base, self.name))
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Accept", "application/json")
            .header("Cache-Control", "no-store")
            .call()
            .map_err(|_| OfficeError::RemoteUncertain)?;
        match response.status().as_u16() {
            200 => {
                let bytes = office_http::json_body(&mut response, DOCUMENT_LIMIT)
                    .map_err(|_| OfficeError::RemoteUncertain)?;
                decode_document(&bytes, &self.name, self.block_id)
            }
            404 => Ok(Document {
                snapshot: BlockSnapshot {
                    block_id: self.block_id.into(),
                    revision: 0,
                    layout: BlockLayout::new(vec![])
                        .map_err(|_| OfficeError::CredentialsInvalid)?,
                },
                update_time: None,
            }),
            401 | 403 => Err(OfficeError::RemoteDenied),
            300..=399 => Err(OfficeError::CredentialsInvalid),
            _ => Err(OfficeError::RemoteUncertain),
        }
    }

    fn commit(
        &self,
        current: &Document,
        layout: &BlockLayout,
        revision: u64,
    ) -> Result<(), OfficeError> {
        let body = commit_body(&self.name, current.update_time.as_deref(), layout, revision);
        let url = format!(
            "{}/v1/projects/{}/databases/(default)/documents:commit",
            self.base,
            self.deployment.project_id()
        );
        let agent = office_http::agent(self.deployment.target().mode(), self.deadline)
            .map_err(|_| OfficeError::RemoteUncertain)?;
        let mut response = agent
            .post(url)
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .send(body.to_string())
            .map_err(|_| OfficeError::RemoteUncertain)?;
        match response.status().as_u16() {
            200 => {
                let bytes = office_http::json_body(&mut response, DOCUMENT_LIMIT)
                    .map_err(|_| OfficeError::RemoteUncertain)?;
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|_| OfficeError::RemoteUncertain)?;
                if value["writeResults"]
                    .as_array()
                    .is_none_or(|items| items.len() != 1)
                    || value["commitTime"].as_str().is_none()
                {
                    return Err(OfficeError::RemoteUncertain);
                }
                Ok(())
            }
            401 | 403 => Err(OfficeError::RemoteDenied),
            409 | 412 => Err(OfficeError::RevisionConflict),
            400 => {
                let bytes = office_http::json_body(&mut response, DOCUMENT_LIMIT)
                    .map_err(|_| OfficeError::RemoteUncertain)?;
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|_| OfficeError::RemoteUncertain)?;
                if value["error"]["status"] == "FAILED_PRECONDITION" {
                    Err(OfficeError::RevisionConflict)
                } else {
                    Err(OfficeError::RemoteUncertain)
                }
            }
            300..=399 => Err(OfficeError::CredentialsInvalid),
            _ => Err(OfficeError::RemoteUncertain),
        }
    }
}

fn commit_body(
    name: &str,
    update_time: Option<&str>,
    layout: &BlockLayout,
    revision: u64,
) -> Value {
    let precondition = update_time.map_or_else(
        || json!({"exists":false}),
        |time| json!({"updateTime":time}),
    );
    json!({"writes":[{
        "update":{"name":name,"fields":{
            "version":{"integerValue":"1"},
            "revision":{"integerValue":revision.to_string()},
            "objects":{"arrayValue":{"values":layout.encode().iter().map(|token| json!({"stringValue":token})).collect::<Vec<_>>()}}
        }},
        "currentDocument":precondition,
        "updateTransforms":[{"fieldPath":"updatedAt","setToServerValue":"REQUEST_TIME"}]
    }]})
}

fn decode_document(bytes: &[u8], name: &str, block_id: &str) -> Result<Document, OfficeError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct WireDocument {
        name: String,
        fields: Value,
        update_time: String,
    }
    let document: WireDocument =
        serde_json::from_slice(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
    let fields = document
        .fields
        .as_object()
        .ok_or(OfficeError::CredentialsInvalid)?;
    if document.name != name
        || fields.len() != 4
        || document.update_time.is_empty()
        || fields.get("version") != Some(&json!({"integerValue":"1"}))
    {
        return Err(OfficeError::CredentialsInvalid);
    }
    let revision = fields
        .get("revision")
        .and_then(|field| field["integerValue"].as_str())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (1..=MAX_REVISION).contains(value))
        .ok_or(OfficeError::CredentialsInvalid)?;
    if fields["revision"] != json!({"integerValue":revision.to_string()}) {
        return Err(OfficeError::CredentialsInvalid);
    }
    let timestamp = fields
        .get("updatedAt")
        .and_then(|field| field["timestampValue"].as_str())
        .ok_or(OfficeError::CredentialsInvalid)?;
    if timestamp.is_empty() || fields["updatedAt"] != json!({"timestampValue":timestamp}) {
        return Err(OfficeError::CredentialsInvalid);
    }
    let array = fields
        .get("objects")
        .and_then(|field| field.get("arrayValue"))
        .ok_or(OfficeError::CredentialsInvalid)?;
    let values = array.get("values").cloned().unwrap_or_else(|| json!([]));
    if fields["objects"] != json!({"arrayValue": {"values":values}})
        && fields["objects"] != json!({"arrayValue":{}})
    {
        return Err(OfficeError::CredentialsInvalid);
    }
    let tokens = values
        .as_array()
        .ok_or(OfficeError::CredentialsInvalid)?
        .iter()
        .map(|value| {
            let token = value["stringValue"]
                .as_str()
                .ok_or(OfficeError::CredentialsInvalid)?;
            if value != &json!({"stringValue":token}) {
                return Err(OfficeError::CredentialsInvalid);
            }
            Ok(token.to_owned())
        })
        .collect::<Result<Vec<_>, OfficeError>>()?;
    let layout = BlockLayout::decode(&tokens).map_err(|_| OfficeError::CredentialsInvalid)?;
    Ok(Document {
        snapshot: BlockSnapshot {
            block_id: block_id.into(),
            revision,
            layout,
        },
        update_time: Some(document.update_time),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Value {
        json!({"name":"expected", "updateTime":"2026-09-13T00:00:00Z", "fields":{
            "version":{"integerValue":"1"}, "revision":{"integerValue":"7"},
            "objects":{"arrayValue":{"values":[{"stringValue":"d1us"}]}},
            "updatedAt":{"timestampValue":"2026-09-13T00:00:00Z"}
        }})
    }

    #[test]
    fn firestore_decoder_rejects_other_resources_and_invalid_document_fields() {
        let good = document();
        let read = |value: &Value| {
            decode_document(&serde_json::to_vec(value).unwrap(), "expected", "block")
        };
        let parsed = read(&good).unwrap();
        assert_eq!(parsed.snapshot.revision, 7);
        assert_eq!(parsed.snapshot.layout.encode(), ["d1us"]);
        for (pointer, replacement) in [
            ("/name", json!("another")),
            ("/fields/version/integerValue", json!("2")),
            ("/fields/revision/integerValue", json!("0")),
            ("/fields/revision/integerValue", json!("9007199254740992")),
            (
                "/fields/objects/arrayValue/values/0/stringValue",
                json!("d1ut"),
            ),
            (
                "/fields/updatedAt",
                json!({"stringValue":"not a timestamp"}),
            ),
        ] {
            let mut invalid = good.clone();
            *invalid.pointer_mut(pointer).unwrap() = replacement;
            assert!(read(&invalid).is_err(), "{pointer}");
        }
        let mut extra = good.clone();
        extra["fields"]["owner"] = json!({"stringValue":"forged"});
        assert!(read(&extra).is_err());
    }

    #[test]
    fn commit_has_exact_precondition_and_server_timestamp_not_client_time() {
        let layout = BlockLayout::decode(&["d1us".into()]).unwrap();
        let create = commit_body("expected", None, &layout, 1);
        assert_eq!(
            create["writes"][0]["currentDocument"],
            json!({"exists":false})
        );
        let update = commit_body("expected", Some("server-update-time"), &layout, 8);
        let write = &update["writes"][0];
        assert_eq!(
            write["currentDocument"],
            json!({"updateTime":"server-update-time"})
        );
        assert_eq!(
            write["updateTransforms"],
            json!([{"fieldPath":"updatedAt","setToServerValue":"REQUEST_TIME"}])
        );
        assert_eq!(
            write["update"]["fields"]["revision"],
            json!({"integerValue":"8"})
        );
        assert!(write["update"]["fields"].get("updatedAt").is_none());
    }

    #[test]
    fn exact_retry_requires_both_revision_and_ordered_content() {
        let layout = BlockLayout::decode(&["d000".into(), "p022".into()]).unwrap();
        let snapshot = BlockSnapshot {
            block_id: "block".into(),
            revision: 7,
            layout: layout.clone(),
        };
        assert!(exact_retry(&snapshot, &layout, 6));
        assert!(!exact_retry(&snapshot, &layout, 7));
        let reordered = BlockLayout::decode(&["p022".into(), "d000".into()]).unwrap();
        assert!(!exact_retry(&snapshot, &reordered, 6));
    }
}
