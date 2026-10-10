//! Public configuration carrier, never a device grant or membership credential.
//! Canonical bytes and a corruption checksum have one owner; no provider I/O.
use crate::{deploy_run::DeployRecord, error::RemoteError, limits, wire};
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
    public_web_config: PublicConfig,
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
            && crate::deploy_run::project_ok(&self.public_web_config.project_id)
            && self.public_web_config.auth_domain
                == format!("{}.firebaseapp.com", self.public_web_config.project_id)
            && [
                &self.public_web_config.api_key,
                &self.public_web_config.app_id,
            ]
            .iter()
            .all(|s| {
                !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| (0x20..=0x7e).contains(&b))
            })
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
            "https://{}.web.app/pair#{}",
            self.public_web_config.project_id,
            URL_SAFE_NO_PAD.encode(bytes)
        );
        if url.len() > limits::REMOTE_LINK_BYTES {
            return Err(unavailable());
        }
        Ok(url)
    }
}
/// Only the typed final joint verification can produce a new carrier.
/// The carrier has no administrative identity or friendly-label fields.
pub fn from_record(record: &DeployRecord) -> Result<String, RemoteError> {
    let publication = record.verified_publication().ok_or_else(unavailable)?;
    let public_web_config: PublicConfig =
        serde_json::from_value(publication.public_config().clone()).map_err(|_| unavailable())?;
    let mut descriptor = Descriptor {
        version: 1,
        kind: "firestore".into(),
        public_web_config,
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
    if origin
        != format!(
            "https://{}.web.app",
            descriptor.public_web_config.project_id
        )
        || descriptor.url()? != url
    {
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
            public_web_config: PublicConfig {
                api_key: "public-key".into(),
                auth_domain: "demo-remote-1.firebaseapp.com".into(),
                project_id: "demo-remote-1".into(),
                app_id: "1:123:web:mine".into(),
            },
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
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert_eq!(value["publicWebConfig"].as_object().unwrap().len(), 4);
        for key in [
            "token",
            "uid",
            "membership",
            "deviceKey",
            "extra",
            "region",
            "deploymentId",
            "planDigest",
            "hostedOrigin",
            "ownerHost",
            "projectId",
        ] {
            let mut changed = value.clone();
            changed[key] = Value::String("TOKEN_CANARY".into());
            let invalid = format!(
                "https://{}.web.app/pair#{}",
                d.public_web_config.project_id,
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&changed).unwrap())
            );
            let error = validate(&invalid).unwrap_err();
            assert_eq!(error.code, "REMOTE_LINK_UNAVAILABLE");
            assert!(!error.to_string().contains("TOKEN_CANARY"));
        }
        for bytes in [b"\xff".as_slice(), b"{\"version\":1,\"version\":1}"] {
            assert!(
                validate(&format!(
                    "https://{}.web.app/pair#{}",
                    d.public_web_config.project_id,
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
        for field in ["checksum", "kind"] {
            let mut value = serde_json::to_value(&d).unwrap();
            value[field] = Value::String("https://evil.invalid/@TOKEN_CANARY".into());
            let changed: Descriptor = serde_json::from_value(value).unwrap();
            assert!(changed.url().is_err(), "{field}");
        }
        let mut non_ascii = d.clone();
        non_ascii.public_web_config.api_key = "public-\u{e9}".into();
        non_ascii.checksum = hash(&serde_json::to_vec(&non_ascii.body()).unwrap());
        assert!(non_ascii.url().is_err());
        let mut changed = d;
        changed.public_web_config.project_id = "another-project".into();
        changed.checksum = hash(&serde_json::to_vec(&changed.body()).unwrap());
        assert!(changed.url().is_err());
    }
}
