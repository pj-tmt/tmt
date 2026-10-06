//! Durable PR provenance and the single acquisition admission observation.

use super::{
    OFFICIAL_REPOSITORY, Product, artifact, invalid,
    pr_catalog::{
        ApplicationSchema, Candidate, DatabaseSchema, Eligibility, Producer, git_sha, positive,
    },
};
use serde::{Deserialize, Serialize};
use std::io;
use tmt_core::{
    content_digest::is_sha256,
    native_install::{Channel, InstalledVersion, PrCandidateIdentity, PrNumber, admit_schema},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AlphaEvidence {
    pub release_id: u64,
    pub version: String,
    pub manifest_sha256: String,
    pub application_schema: ApplicationSchema,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Admission {
    pub observed_at_ms: u64,
    pub published_at_ms: u64,
    pub expires_at_ms: u64,
    pub pull_sha256: String,
    pub timeline_sha256: String,
    pub initial_pull_sha256: String,
    pub initial_timeline_sha256: String,
    pub catalog_inventory_sha256: String,
    pub producer_run_sha256: String,
    pub catalog_metadata_sha256: String,
    pub payload_metadata_sha256: String,
    pub latest_alpha: AlphaEvidence,
    pub local_schemas: Vec<DatabaseSchema>,
    pub schema_ahead_opt_in: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrProvenance {
    pub repository: String,
    pub repository_id: u64,
    pub pr: u32,
    pub head_sha: String,
    pub producer: Producer,
    pub eligibility: Eligibility,
    pub catalog_artifact_id: u64,
    pub catalog_zip_sha256: String,
    pub catalog_sha256: String,
    pub candidate: Candidate,
    pub admission: Admission,
}

impl PrProvenance {
    pub fn identity(&self) -> io::Result<PrCandidateIdentity> {
        let pr = PrNumber::parse(&self.pr.to_string())
            .ok_or_else(|| invalid("Invalid installed PR identity."))?;
        PrCandidateIdentity::new(pr, self.head_sha.clone(), self.producer.run_id)
            .ok_or_else(|| invalid("Invalid installed PR source identity."))
    }

    /// Historical receipt availability is judged at its captured observation,
    /// not against today's remote state, clock or approved producer set.
    pub fn validate(
        &self,
        product: Product,
        target: &str,
        state: &InstalledVersion,
        archive_name: &str,
        archive_sha256: &str,
    ) -> io::Result<()> {
        let identity = self.identity()?;
        self.eligibility.validate()?;
        if self.repository != OFFICIAL_REPOSITORY
            || !positive(self.repository_id)
            || !positive(self.catalog_artifact_id)
            || !is_sha256(&self.catalog_zip_sha256)
            || !is_sha256(&self.catalog_sha256)
            || state.channel != Channel::Pr(identity.pr())
            || self.candidate.product != product.as_str()
            || self.candidate.target != target
            || self.candidate.version != state.version.to_string()
            || self.candidate.archive.name != archive_name
            || self.candidate.archive.sha256 != archive_sha256
            || self.producer.workflow_path != ".github/workflows/pr-rc.yml"
            || !positive(self.producer.workflow_id)
            || !positive(self.producer.run_id)
            || !(1..=100).contains(&self.producer.run_attempt)
            || !git_sha(&self.producer.tooling_sha)
            || !is_sha256(&self.producer.workflow_sha256)
        {
            return Err(invalid(
                "Installed PR provenance does not match its release.",
            ));
        }
        self.candidate
            .validate(identity.pr(), &self.head_sha, &self.producer)?;
        let observed = &self.admission;
        if !positive(observed.observed_at_ms)
            || !positive(observed.published_at_ms)
            || !positive(observed.expires_at_ms)
            || observed.published_at_ms > observed.observed_at_ms
            || self.eligibility.enabled_at_ms > observed.published_at_ms
            || observed.expires_at_ms <= observed.observed_at_ms
            || observed.expires_at_ms - observed.published_at_ms > 3 * 24 * 60 * 60 * 1000
            || [
                &observed.pull_sha256,
                &observed.timeline_sha256,
                &observed.initial_pull_sha256,
                &observed.initial_timeline_sha256,
                &observed.catalog_inventory_sha256,
                &observed.producer_run_sha256,
                &observed.catalog_metadata_sha256,
                &observed.payload_metadata_sha256,
            ]
            .into_iter()
            .any(|hash| !is_sha256(hash))
            || !positive(observed.latest_alpha.release_id)
            || !is_sha256(&observed.latest_alpha.manifest_sha256)
            || !observed
                .latest_alpha
                .version
                .parse::<semver::Version>()
                .is_ok_and(|version| Channel::Alpha.accepts(&version))
        {
            return Err(invalid("Invalid installed PR admission observation."));
        }
        let alpha = &observed.latest_alpha.application_schema;
        alpha.validate(product.as_str(), &alpha.source_sha)?;
        admit_databases(
            product,
            &self.candidate.application_schema,
            alpha,
            &observed.local_schemas,
            observed.schema_ahead_opt_in,
        )?;
        Ok(())
    }

    pub fn manifest_sha256(&self) -> &str {
        &self.candidate.dist_manifest.sha256
    }

    pub fn verify_manifest(&self, bytes: &[u8]) -> io::Result<()> {
        let manifest: serde_json::Value = super::pr_json::parse(bytes, artifact::MANIFEST_LIMIT)?;
        if artifact::digest(bytes) != self.manifest_sha256()
            || manifest.get("tmt_application_schema")
                != Some(
                    &serde_json::to_value(&self.candidate.application_schema)
                        .map_err(io::Error::other)?,
                )
        {
            return Err(invalid(
                "PR manifest application-schema evidence does not match its provenance.",
            ));
        }
        Ok(())
    }
}

/// Database domains are assigned by their product owner. A remote record
/// cannot omit a required domain or invent a local version. Extension-owner
/// exports must be reviewed before those products acquire PR candidates.
pub(super) fn admit_databases(
    product: Product,
    candidate: &ApplicationSchema,
    alpha: &ApplicationSchema,
    local: &[DatabaseSchema],
    opt_in: bool,
) -> io::Result<()> {
    let unknown = || io::Error::other(tmt_core::native_install::SchemaError::Unknown);
    if product != Product::Cli
        || candidate.databases.len() != 1
        || candidate.databases[0].domain != "tmt-core-db"
        || alpha.databases.len() != 1
        || local.len() != 1
        || alpha.databases[0].domain != "tmt-core-db"
        || local[0].domain != "tmt-core-db"
        || local[0].version > i32::MAX as u32
    {
        return Err(unknown());
    }
    admit_schema(
        Some(candidate.databases[0].version),
        Some(alpha.databases[0].version),
        Some(local[0].version),
        opt_in,
    )
    .map_err(io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod tests;
