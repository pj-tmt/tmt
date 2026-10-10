//! Offline test support only: the production discovery owner parses and composes these bytes.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};
use tmt_remote::{
    deploy_discovery::{self, DeclarationSource, DiscoveryRefusal},
    limits,
};

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub const OUTPUTS: [&str; 3] = ["firestore.rules", "firestore.indexes.json", "plan.json"];

pub fn read_envelope(path: &Path) -> Result<Vec<u8>, &'static str> {
    let file = File::open(path).map_err(|_| "input-unavailable")?;
    if !file.metadata().map_err(|_| "input-unavailable")?.is_file() {
        return Err("input-unavailable");
    }
    let mut bytes = Vec::new();
    file.take(limits::DEPLOY_DECLARATION_REPLY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "input-unavailable")?;
    if bytes.len() > limits::DEPLOY_DECLARATION_REPLY_BYTES {
        return Err("invalid-envelope");
    }
    Ok(bytes)
}

struct VectorDeclaration<'a>(&'a [u8]);
impl DeclarationSource for VectorDeclaration<'_> {
    fn declaration(&mut self, _: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
        Ok(Some(self.0.to_vec()))
    }
}

pub struct VectorComposition {
    pub files: [String; 3],
    pub summary: String,
}

pub fn compose(bytes: &[u8]) -> Result<VectorComposition, &'static str> {
    if bytes.len() > limits::DEPLOY_DECLARATION_REPLY_BYTES {
        return Err("invalid-envelope");
    }
    // This only selects the requested extension. discover re-parses strictly (including
    // duplicate fields), verifies both input digests and owns all declaration/Rules grammar.
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "invalid-envelope")?;
    let extension = value["extension"].as_str().ok_or("invalid-envelope")?;
    let result = deploy_discovery::discover(&mut VectorDeclaration(bytes), &[extension])
        .map_err(|_| "composition-refused")?;
    assert_eq!(
        result.extensions.digest(),
        digest(result.extensions.bytes())
    );
    let identity = result
        .extensions
        .view()
        .extensions
        .first()
        .ok_or("composition-refused")?;
    let plan = json!({
        "version": 1,
        "planDigest": result.extensions.digest(),
        "declarationDigest": identity.declaration_digest,
        "artifactDigest": identity.admission.artifact_digest,
        "plan": result.extensions.view(),
    });
    let summary = json!({
        "version": 1,
        "planDigest": result.extensions.digest(),
        "rulesDigest": digest(result.artifacts.rules.as_bytes()),
        "indexesDigest": digest(result.artifacts.indexes.as_bytes()),
    });
    Ok(VectorComposition {
        files: [
            result.artifacts.rules,
            result.artifacts.indexes,
            serde_json::to_string_pretty(&plan).expect("a plan serializes") + "\n",
        ],
        summary: serde_json::to_string(&summary).expect("a summary serializes") + "\n",
    })
}
