//! Revision-2 PR catalog records. Remote records never grant producer trust.

use super::{OFFICIAL_REPOSITORY, Product, artifact, invalid};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io};
use tmt_core::{content_digest::is_sha256, limits::MAX_JS_SAFE_INTEGER, native_install::PrNumber};

pub(super) const CATALOG_LIMIT: usize = 1024 * 1024;
pub(super) const CATALOG_ZIP_LIMIT: usize = 2 * 1024 * 1024;
pub(super) const PAYLOAD_ZIP_LIMIT: usize = 69 * 1024 * 1024;
const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-musl",
];
const DOMAINS: [&str; 4] = [
    "tmt-core-db",
    "tmt-remote-db",
    "tmt-remote-objects-db",
    "tmt-colab-db",
];
const TTL_MS: u64 = 3 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Producer {
    pub workflow_id: u64,
    pub workflow_path: String,
    pub workflow_sha256: String,
    pub tooling_sha: String,
    pub run_id: u64,
    pub run_attempt: u32,
}

/// Installer-owned immutable authority, never deserialized from a candidate.
pub(super) struct ApprovedProducer {
    pub workflow_id: u64,
    pub workflow_sha256: &'static str,
    pub tooling_sha: &'static str,
}

impl Producer {
    pub fn approved(&self, policy: &[ApprovedProducer]) -> bool {
        self.workflow_path == ".github/workflows/pr-rc.yml"
            && policy.iter().any(|approved| {
                self.workflow_id == approved.workflow_id
                    && self.workflow_sha256 == approved.workflow_sha256
                    && self.tooling_sha == approved.tooling_sha
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DatabaseSchema {
    pub domain: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApplicationSchema {
    pub schema_version: u32,
    pub product: String,
    pub source_sha: String,
    pub databases: Vec<DatabaseSchema>,
    pub source_files: Vec<SourceFile>,
}

impl ApplicationSchema {
    pub fn validate(&self, product: &str, head: &str) -> io::Result<()> {
        if self.schema_version != 1
            || self.product != product
            || self.source_sha != head
            || !git_sha(head)
            || self.databases.is_empty()
            || self.databases.len() > 8
            || self.source_files.is_empty()
            || self.source_files.len() > 64
        {
            return Err(invalid("Invalid PR application-schema evidence."));
        }
        let mut previous = None;
        for database in &self.databases {
            if !DOMAINS.contains(&database.domain.as_str())
                || database.version > i32::MAX as u32
                || previous.is_some_and(|previous| previous >= database.domain.as_str())
            {
                return Err(invalid("Invalid or duplicate application-schema domain."));
            }
            previous = Some(database.domain.as_str());
        }
        let mut previous = None;
        for source in &self.source_files {
            if source.path.is_empty()
                || source.path.len() > 256
                || source.path.split('/').any(|part| {
                    part.is_empty()
                        || part == "."
                        || part == ".."
                        || !part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
                })
                || !is_sha256(&source.sha256)
                || previous.is_some_and(|previous| previous >= source.path.as_str())
            {
                return Err(invalid(
                    "Invalid or duplicate application-schema source path.",
                ));
            }
            previous = Some(source.path.as_str());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PayloadArtifact {
    pub id: u64,
    pub name: String,
    pub zip_sha256: String,
    pub zip_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Member {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

impl Member {
    fn validate(&self, maximum: usize) -> bool {
        !self.name.is_empty()
            && self.name.len() <= 160
            && self.name.as_bytes()[0].is_ascii_alphanumeric()
            && self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            && is_sha256(&self.sha256)
            && self.bytes > 0
            && self.bytes <= maximum as u64
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Verification {
    pub prepare_run_id: u64,
    pub prepare_run_attempt: u32,
    pub prepare_tooling_sha: String,
    pub source_sha: String,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Candidate {
    pub product: String,
    pub target: String,
    pub version: String,
    pub application_schema: ApplicationSchema,
    pub payload_artifact: PayloadArtifact,
    pub dist_manifest: Member,
    pub archive: Member,
    pub verification: Verification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    pub schema_version: u32,
    pub kind: String,
    pub repository: String,
    pub pr: u32,
    pub head_sha: String,
    pub producer: Producer,
    pub eligibility: Eligibility,
    pub published_at_ms: u64,
    pub expires_at_ms: u64,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Eligibility {
    pub label: String,
    pub label_id: u64,
    pub enabled_event_id: u64,
    pub enabled_at_ms: u64,
}
impl Eligibility {
    pub fn validate(&self) -> io::Result<()> {
        if self.label != "rc-build"
            || !positive(self.label_id)
            || !positive(self.enabled_event_id)
            || !positive(self.enabled_at_ms)
            || !self.enabled_at_ms.is_multiple_of(1000)
        {
            return Err(invalid("Invalid PR eligibility epoch."));
        }
        Ok(())
    }
}

impl Candidate {
    pub fn validate(&self, pr: PrNumber, head: &str, producer: &Producer) -> io::Result<()> {
        let product = Product::parse(&self.product)
            .filter(|product| *product != Product::Office)
            .ok_or_else(|| invalid("Unsupported PR candidate product."))?;
        if !TARGETS.contains(&self.target.as_str())
            || self.version.is_empty()
            || self.version.len() > 128
            || self.version.parse::<semver::Version>().is_err()
            || !positive(self.payload_artifact.id)
            || !is_sha256(&self.payload_artifact.zip_sha256)
            || self.payload_artifact.zip_bytes == 0
            || self.payload_artifact.zip_bytes > PAYLOAD_ZIP_LIMIT as u64
            || self.payload_artifact.name
                != format!(
                    "tmt-pr-rc-payload-v2-pr{pr}-{}-{}-{}-a{}",
                    self.product, self.target, producer.run_id, producer.run_attempt
                )
            || self.dist_manifest.name != "dist-manifest.json"
            || !self.dist_manifest.validate(artifact::MANIFEST_LIMIT)
            || !self.archive.validate(artifact::COMPRESSED_LIMIT)
            || self.archive.name != format!("{}-{}.tar.gz", product.package(), self.target)
            || !positive(self.verification.prepare_run_id)
            || !(1..=100).contains(&self.verification.prepare_run_attempt)
            || !git_sha(&self.verification.prepare_tooling_sha)
            || self.verification.source_sha != head
            || !self.verification.complete
        {
            return Err(invalid(
                "Invalid, duplicate or incomplete PR candidate evidence.",
            ));
        }
        self.application_schema.validate(&self.product, head)?;
        Ok(())
    }
}

impl Catalog {
    /// Typed structs reject unknown and duplicate keys at every catalog level;
    /// the fixed shape also cannot contain arbitrary recursive JSON values.
    pub fn parse(
        bytes: &[u8],
        pr: PrNumber,
        head: &str,
        now_ms: u64,
        policy: &[ApprovedProducer],
    ) -> io::Result<Self> {
        if bytes.len() > CATALOG_LIMIT {
            return Err(invalid("PR catalog exceeds its bound."));
        }
        let catalog: Self = super::pr_json::parse(bytes, CATALOG_LIMIT)?;
        catalog.validate(pr, head, now_ms, policy)?;
        Ok(catalog)
    }

    fn validate(
        &self,
        pr: PrNumber,
        head: &str,
        now_ms: u64,
        policy: &[ApprovedProducer],
    ) -> io::Result<()> {
        self.eligibility.validate()?;
        if self.schema_version != 2
            || self.kind != "tmt-pr-rc-catalog"
            || self.repository != OFFICIAL_REPOSITORY
            || self.pr != pr.get()
            || self.head_sha != head
            || !git_sha(head)
            || !self.producer.approved(policy)
            || !positive(self.producer.run_id)
            || !positive(self.producer.workflow_id)
            || !(1..=100).contains(&self.producer.run_attempt)
            || !positive(self.published_at_ms)
            || !positive(self.expires_at_ms)
            || self.published_at_ms > now_ms
            || self.eligibility.enabled_at_ms > self.published_at_ms
            || self.expires_at_ms <= now_ms
            || self.expires_at_ms <= self.published_at_ms
            || self.expires_at_ms - self.published_at_ms > TTL_MS
            || self.candidates.is_empty()
            || self.candidates.len() > 16
        {
            return Err(invalid(
                "PR catalog source, producer or availability evidence does not match.",
            ));
        }
        let mut previous = None;
        let mut ids = BTreeSet::new();
        for candidate in &self.candidates {
            let key = (candidate.product.as_str(), candidate.target.as_str());
            if previous.is_some_and(|previous| previous >= key)
                || !ids.insert(candidate.payload_artifact.id)
            {
                return Err(invalid("Duplicate or unordered PR candidate."));
            }
            candidate.validate(pr, head, &self.producer)?;
            previous = Some(key);
        }
        for product in [
            Product::Cli,
            Product::Colab,
            Product::Remote,
            Product::Squad,
        ] {
            let group = self
                .candidates
                .iter()
                .filter(|candidate| candidate.product == product.as_str())
                .collect::<Vec<_>>();
            if group.is_empty() {
                continue;
            }
            if group.len() != TARGETS.len()
                || group.iter().any(|candidate| {
                    candidate.version != group[0].version
                        || candidate.application_schema != group[0].application_schema
                        || candidate.verification != group[0].verification
                })
            {
                return Err(invalid(
                    "PR product verification does not cover all native targets.",
                ));
            }
        }
        Ok(())
    }
}

pub(super) fn positive(value: u64) -> bool {
    value > 0 && value <= MAX_JS_SAFE_INTEGER
}
pub(super) fn git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests;
