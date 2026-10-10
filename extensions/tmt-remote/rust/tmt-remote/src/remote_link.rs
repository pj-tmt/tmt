//! Public configuration carrier, never a device grant or membership credential.
//! Canonical bytes and a corruption checksum have one owner; no provider I/O.
use crate::{canonical, deploy_run::DeployRecord, error::RemoteError, limits, wire};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicConfig {
    api_key: String,
    auth_domain: String,
    project_id: String,
    app_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Descriptor {
    version: u8,
    kind: String,
    project_id: String,
    region: String,
    deployment_id: String,
    plan_digest: String,
    hosted_origin: String,
    public_web_config: PublicConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_host: Option<String>,
    checksum: String,
}
/// Fixed, credential-free owner guidance. A missing link never triggers deployment.
pub fn unavailable() -> RemoteError {
    RemoteError::new(
        "REMOTE_LINK_UNAVAILABLE",
        "No remote link is saved yet. Run tmt remote deploy firestore to create one.",
    )
}
fn hash(bytes: &[u8]) -> String {
    crate::deploy_plan::sha256_hex(bytes)
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Descriptor {
    fn body(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("descriptor serializes");
        value.as_object_mut().expect("object").remove("checksum");
        value.sort_all_objects();
        value
    }
    fn valid(&self) -> bool {
        self.version == 1
            && self.kind == "firestore"
            && crate::deploy_run::project_ok(&self.project_id)
            && crate::deploy_run::location_ok(&self.region)
            && canonical::uuid(&self.deployment_id).is_ok()
            && digest(&self.plan_digest)
            && self.hosted_origin == format!("https://{}.web.app", self.project_id)
            && self.public_web_config.project_id == self.project_id
            && self.public_web_config.auth_domain == format!("{}.firebaseapp.com", self.project_id)
            && [
                &self.public_web_config.api_key,
                &self.public_web_config.app_id,
            ]
            .iter()
            .all(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            && self
                .owner_host
                .as_ref()
                .is_none_or(|s| !s.is_empty() && s.len() <= 80 && !s.chars().any(char::is_control))
            && self.checksum == hash(&serde_json::to_vec(&self.body()).expect("body serializes"))
    }
    fn url(&self) -> Result<String, RemoteError> {
        if !self.valid() {
            return Err(unavailable());
        }
        // The workspace preserves insertion order, so sort explicitly, recursively.
        let mut value = serde_json::to_value(self).expect("descriptor serializes");
        value.sort_all_objects();
        let bytes = serde_json::to_vec(&value).expect("value serializes");
        let url = format!(
            "{}/pair#{}",
            self.hosted_origin,
            URL_SAFE_NO_PAD.encode(bytes)
        );
        if url.len() > limits::REMOTE_LINK_BYTES {
            return Err(unavailable());
        }
        Ok(url)
    }
}
/// Only the typed final joint verification can produce a new carrier.
/// No friendly owner label exists in the current admitted projection, so omit it.
pub fn from_record(record: &DeployRecord) -> Result<String, RemoteError> {
    let publication = record.verified_publication().ok_or_else(unavailable)?;
    let run = record.run.as_ref().ok_or_else(unavailable)?;
    let region = run
        .hosting
        .as_ref()
        .and_then(|h| h.envelope["database"]["location"].as_str())
        .ok_or_else(unavailable)?;
    let public_web_config: PublicConfig =
        serde_json::from_value(publication.public_config().clone()).map_err(|_| unavailable())?;
    let mut descriptor = Descriptor {
        version: 1,
        kind: "firestore".into(),
        project_id: public_web_config.project_id.clone(),
        region: region.into(),
        deployment_id: record.deployment_id.clone(),
        plan_digest: run.plan_digest.clone(),
        hosted_origin: publication.entry_url().into(),
        public_web_config,
        owner_host: None,
        checksum: String::new(),
    };
    descriptor.checksum = hash(&serde_json::to_vec(&descriptor.body()).expect("body serializes"));
    descriptor.url()
}
/// Strict canonical UTF-8/closed schema/checksum validation. The checksum is not authority.
pub fn validate(url: &str) -> Result<(), RemoteError> {
    if url.len() > limits::REMOTE_LINK_BYTES {
        return Err(unavailable());
    }
    let (origin, fragment) = url.split_once("/pair#").ok_or_else(unavailable)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(fragment)
        .map_err(|_| unavailable())?;
    let value = wire::strict_json(&bytes).ok_or_else(unavailable)?;
    let descriptor: Descriptor = serde_json::from_value(value).map_err(|_| unavailable())?;
    if descriptor.hosted_origin != origin || descriptor.url()? != url {
        return Err(unavailable());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vector() -> Descriptor {
        let mut d = Descriptor {
            version: 1,
            kind: "firestore".into(),
            project_id: "demo-remote-1".into(),
            region: "asia-east1".into(),
            deployment_id: "3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c11".into(),
            plan_digest: "a".repeat(64),
            hosted_origin: "https://demo-remote-1.web.app".into(),
            public_web_config: PublicConfig {
                api_key: "public-key".into(),
                auth_domain: "demo-remote-1.firebaseapp.com".into(),
                project_id: "demo-remote-1".into(),
                app_id: "1:123:web:mine".into(),
            },
            owner_host: None,
            checksum: String::new(),
        };
        d.checksum = hash(&serde_json::to_vec(&d.body()).unwrap());
        d
    }
    #[test]
    fn canonical_public_vector_is_stable_and_closed() {
        let d = vector();
        let golden: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/remote_link/descriptor-v1.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(&d).unwrap(), golden);
        let url = d.url().unwrap();
        assert_eq!(url, d.url().unwrap());
        validate(&url).unwrap();
        assert!(url.starts_with("https://demo-remote-1.web.app/pair#"));
        let bytes = URL_SAFE_NO_PAD
            .decode(url.split_once('#').unwrap().1)
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("ownerHost").is_none());
        assert_eq!(value["publicWebConfig"].as_object().unwrap().len(), 4);
        for key in ["token", "uid", "membership", "deviceKey", "extra"] {
            let mut changed = value.clone();
            changed[key] = Value::String("TOKEN_CANARY".into());
            let invalid = format!(
                "{}/pair#{}",
                d.hosted_origin,
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&changed).unwrap())
            );
            let error = validate(&invalid).unwrap_err();
            assert_eq!(error.code, "REMOTE_LINK_UNAVAILABLE");
            assert!(!error.to_string().contains("TOKEN_CANARY"));
        }
        for bytes in [b"\xff".as_slice(), b"{\"version\":1,\"version\":1}"] {
            assert!(
                validate(&format!(
                    "{}/pair#{}",
                    d.hosted_origin,
                    URL_SAFE_NO_PAD.encode(bytes)
                ))
                .is_err()
            );
        }
        assert!(validate(&(url.clone() + "=")).is_err());
        assert!(validate(&"x".repeat(limits::REMOTE_LINK_BYTES + 1)).is_err());
    }
    #[test]
    fn inconsistent_or_unsafe_fields_never_form_a_link() {
        let d = vector();
        for field in [
            "checksum",
            "projectId",
            "region",
            "deploymentId",
            "planDigest",
            "hostedOrigin",
        ] {
            let mut value = serde_json::to_value(&d).unwrap();
            value[field] = Value::String("https://evil.invalid/@TOKEN_CANARY".into());
            let changed: Descriptor = serde_json::from_value(value).unwrap();
            assert!(changed.url().is_err(), "{field}");
        }
        let mut changed = d;
        changed.public_web_config.project_id = "another-project".into();
        changed.checksum = hash(&serde_json::to_vec(&changed.body()).unwrap());
        assert!(changed.url().is_err());
    }
}
