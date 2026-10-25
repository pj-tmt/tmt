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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hosting_site: Option<String>,
    checksum: String,
}
/// Fixed, credential-free owner guidance. A missing link never triggers deployment.
pub fn unavailable() -> RemoteError {
    RemoteError::new(
        "REMOTE_LINK_UNAVAILABLE",
        "No remote link is available yet. Run tmt remote deploy firestore to create one.",
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
            && self.hosting_site.as_ref().is_none_or(|site| {
                crate::hosting::site_id_ok(site) && site != &self.public_web_config.project_id
            })
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
            self.hosting_site
                .as_deref()
                .unwrap_or(&self.public_web_config.project_id),
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
    let site = publication
        .entry_url()
        .strip_prefix("https://")
        .and_then(|origin| origin.strip_suffix(".web.app"))
        .filter(|site| crate::hosting::site_id_ok(site))
        .ok_or_else(unavailable)?;
    let hosting_site = (site != public_web_config.project_id).then(|| site.to_owned());
    let mut descriptor = Descriptor {
        version: 1,
        kind: "firestore".into(),
        public_web_config,
        hosting_site,
        checksum: String::new(),
    };
    descriptor.checksum = hash(&serde_json::to_vec(&descriptor.body()).expect("body serializes"));
    descriptor.url()
}
/// Strict canonical UTF-8/closed schema/checksum validation. The checksum is not authority.
fn parse(url: &str) -> Result<Descriptor, RemoteError> {
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
            descriptor
                .hosting_site
                .as_deref()
                .unwrap_or(&descriptor.public_web_config.project_id)
        )
        || descriptor.url()? != url
    {
        return Err(unavailable());
    }
    Ok(descriptor)
}
/// Check the complete closed public reference without granting any authority.
pub fn validate(url: &str) -> Result<(), RemoteError> {
    parse(url).map(|_| ())
}
/// Only the four verified public configuration fields, never deployment identity.
pub fn public_config(url: &str) -> Result<Value, RemoteError> {
    serde_json::to_value(parse(url)?.public_web_config).map_err(|_| unavailable())
}

/// Public reference identity is project/app/auth domain, never catalog import authority.
pub fn matches_record(url: &str, record: &DeployRecord) -> bool {
    let Ok(descriptor) = parse(url) else {
        return false;
    };
    record.verified_publication().is_some_and(|publication| {
        let config = publication.public_config();
        config["projectId"] == descriptor.public_web_config.project_id
            && config["appId"] == descriptor.public_web_config.app_id
            && config["authDomain"] == descriptor.public_web_config.auth_domain
            && publication.entry_url()
                == format!(
                    "https://{}.web.app",
                    descriptor
                        .hosting_site
                        .as_deref()
                        .unwrap_or(&descriptor.public_web_config.project_id)
                )
    })
}
/// Extract a project only after validating the complete public reference.
pub fn project(url: &str) -> Result<String, RemoteError> {
    Ok(parse(url)?.public_web_config.project_id)
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
            hosting_site: None,
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
    fn selected_site_vector_is_bound_to_its_origin_and_keeps_config_project_identity() {
        let mut descriptor = vector();
        descriptor.hosting_site = Some("tmt-selected-site".into());
        descriptor.checksum = hash(&serde_json::to_vec(&descriptor.body()).unwrap());
        // Python json(sort_keys=True, separators=(',', ':')) + hashlib.sha256,
        // independent of the native/SDK descriptor serializers.
        let golden: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/remote_link/descriptor-v1-selected-site.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(&descriptor).unwrap(), golden);
        let link = descriptor.url().unwrap();
        assert!(link.starts_with("https://tmt-selected-site.web.app/pair#"));
        assert_eq!(public_config(&link).unwrap()["projectId"], "demo-remote-1");
        assert!(
            validate(&link.replacen("tmt-selected-site.web.app", "demo-remote-1.web.app", 1))
                .is_err()
        );
        for invalid in [
            "",
            "demo-remote-1",
            "site/other",
            "UPPER",
            "abcdefghijklmnopqrstuvwxyz12345",
        ] {
            let mut changed = descriptor.clone();
            changed.hosting_site = Some(invalid.into());
            changed.checksum = hash(&serde_json::to_vec(&changed.body()).unwrap());
            assert!(changed.url().is_err(), "{invalid}");
        }
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
