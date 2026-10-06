//! Bounded authenticated Actions acquisition; remote JSON cannot approve a writer.

use super::{
    OFFICIAL_REPOSITORY, Product, artifact, invalid, pr_catalog as wire, pr_json,
    pr_receipt::{Admission, PrProvenance},
    pr_zip,
    receipt::Provenance,
    release::{self, DownloadedRelease},
};
use crate::release_http::Response;
use base64::Engine;
use serde_json::Value;
use std::{collections::BTreeSet, io, time::Instant};
use tmt_core::native_install::{Channel, PrNumber};

const ROOT: &str = "https://api.github.com/repos/pj-tmt/tmt";
const METADATA_LIMIT: usize = 2 * 1024 * 1024;

/// Populated only by a review of actual publisher code and its frozen inputs.
/// Fixture identities are intentionally absent. Eligibility is also owned by
/// the reviewed producer contract, never inferred from a surviving artifact.
pub(super) const APPROVED: &[wire::ApprovedProducer] = &[];

#[derive(Debug)]
pub(super) struct Unavailable(pub &'static str);
impl std::fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for Unavailable {}

pub(super) struct Policy<'a> {
    pub repository_id: u64,
    pub producers: &'a [wire::ApprovedProducer],
}

struct Api<F> {
    get: F,
    deadline: Instant,
    requests: usize,
    remaining: usize,
}
impl<F> Api<F>
where
    F: FnMut(&str, &str, usize, Instant) -> io::Result<Response>,
{
    fn metadata(&mut self, route: &str) -> io::Result<(Value, Vec<u8>)> {
        if self.requests == 32 || self.remaining == 0 || Instant::now() >= self.deadline {
            return Err(invalid(
                "PR candidate metadata discovery exceeds its bound.",
            ));
        }
        self.requests += 1;
        let response = (self.get)(
            &format!("{ROOT}/{route}"),
            "application/vnd.github+json",
            self.remaining,
            self.deadline,
        )?;
        if response
            .link
            .as_deref()
            .is_some_and(|link| !link.is_empty())
        {
            return Err(invalid("PR candidate metadata inventory is incomplete."));
        }
        self.remaining = self
            .remaining
            .checked_sub(response.body.len())
            .ok_or_else(|| invalid("PR candidate metadata exceeds its byte bound."))?;
        let value = pr_json::parse(&response.body, METADATA_LIMIT)?;
        Ok((value, response.body))
    }
    fn zip(&mut self, metadata: &Value, maximum: usize) -> io::Result<Vec<u8>> {
        let artifact_id = id(&metadata["id"])?;
        let size = id(&metadata["size_in_bytes"])?;
        let digest = digest(metadata)?;
        if size > maximum as u64 {
            return Err(invalid("Actions artifact exceeds its ZIP bound."));
        }
        let bytes = (self.get)(
            &format!("{ROOT}/actions/artifacts/{artifact_id}/zip"),
            "application/octet-stream",
            maximum,
            self.deadline,
        )?
        .body;
        if bytes.len() as u64 != size || artifact::digest(&bytes) != digest {
            return Err(invalid(
                "Actions ZIP does not match authenticated artifact metadata.",
            ));
        }
        Ok(bytes)
    }
}

fn id(value: &Value) -> io::Result<u64> {
    value
        .as_u64()
        .filter(|id| wire::positive(*id))
        .ok_or_else(|| invalid("Invalid PR candidate API identity."))
}
fn text(value: &Value) -> io::Result<&str> {
    value
        .as_str()
        .ok_or_else(|| invalid("Invalid PR candidate API text."))
}
fn digest(metadata: &Value) -> io::Result<&str> {
    text(&metadata["digest"])?
        .strip_prefix("sha256:")
        .filter(|hash| tmt_core::content_digest::is_sha256(hash))
        .ok_or_else(|| invalid("Actions artifact has no authenticated SHA-256 digest."))
}
fn pull(value: &Value, pr: PrNumber, policy: &Policy<'_>) -> io::Result<String> {
    if value["number"] != pr.get()
        || value["state"] != "open"
        || value["base"]["repo"]["id"] != policy.repository_id
        || value["head"]["repo"]["id"] != policy.repository_id
        || value["head"]["repo"]["full_name"] != OFFICIAL_REPOSITORY
        || !value["labels"].as_array().is_some_and(|labels| {
            labels
                .iter()
                .filter(|entry| entry["name"] == "rc-build")
                .count()
                == 1
        })
    {
        return Err(invalid(
            "PR is closed, disabled, ineligible or outside the official repository.",
        ));
    }
    let head = text(&value["head"]["sha"])?;
    if !wire::git_sha(head) {
        return Err(invalid("PR head identity is unknown."));
    }
    Ok(head.into())
}

fn eligibility(pull: &Value, timeline: &Value) -> io::Result<wire::Eligibility> {
    let bad = || invalid("PR eligibility timeline is missing, ambiguous or inconsistent.");
    let label = pull["labels"]
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .find(|entry| entry["name"] == "rc-build")
        .ok_or_else(bad)?;
    let label_id = id(&label["id"])?;
    let events = timeline
        .as_array()
        .filter(|events| events.len() <= 100)
        .ok_or_else(bad)?;
    let mut seen = BTreeSet::new();
    let mut transitions = Vec::new();
    for event in events {
        if event["label"]["name"] != "rc-build" {
            continue;
        }
        let kind = text(&event["event"])?;
        if kind != "labeled" && kind != "unlabeled" {
            continue;
        }
        let event_id = id(&event["id"])?;
        if !seen.insert(event_id) {
            return Err(bad());
        }
        let timestamp = text(&event["created_at"])?;
        // Only the documented UTC second form supplies this conservative order.
        if timestamp.len() != 20 || !timestamp.ends_with('Z') {
            return Err(bad());
        }
        let millis = utc_millis(timestamp)?;
        transitions.push((millis, event_id, kind));
    }
    transitions.sort_by_key(|entry| entry.0);
    let &(enabled_at_ms, enabled_event_id, kind) = transitions.last().ok_or_else(bad)?;
    if kind != "labeled"
        || transitions
            .iter()
            .filter(|entry| entry.0 == enabled_at_ms)
            .count()
            != 1
    {
        return Err(bad());
    }
    let epoch = wire::Eligibility {
        label: "rc-build".into(),
        label_id,
        enabled_event_id,
        enabled_at_ms,
    };
    epoch.validate()?;
    Ok(epoch)
}

fn utc_millis(timestamp: &str) -> io::Result<u64> {
    let bad = || invalid("Eligibility timestamp must be canonical UTC seconds.");
    let bytes = timestamp.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| ![4, 7, 10, 13, 16, 19].contains(&index) && !byte.is_ascii_digit())
    {
        return Err(bad());
    }
    let number = |start, end| {
        timestamp
            .get(start..end)
            .ok_or_else(bad)?
            .parse::<u8>()
            .map_err(|_| bad())
    };
    let year = timestamp[..4].parse::<i32>().map_err(|_| bad())?;
    let date = time::Date::from_calendar_date(
        year,
        time::Month::try_from(number(5, 7)?).map_err(|_| bad())?,
        number(8, 10)?,
    )
    .map_err(|_| bad())?;
    let clock = time::Time::from_hms(number(11, 13)?, number(14, 16)?, number(17, 19)?)
        .map_err(|_| bad())?;
    u64::try_from(date.with_time(clock).assume_utc().unix_timestamp_nanos() / 1_000_000)
        .map_err(|_| bad())
}
fn inventory(value: &Value, name: &str, repository_id: u64) -> io::Result<Vec<Value>> {
    let entries = value["artifacts"]
        .as_array()
        .ok_or_else(|| invalid("Missing named PR catalog inventory."))?;
    if entries.len() > 100 || value["total_count"].as_u64() != Some(entries.len() as u64) {
        return Err(invalid(
            "Named PR catalog inventory is incomplete or excessive.",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut runs = BTreeSet::new();
    for entry in entries {
        if !ids.insert(id(&entry["id"])?)
            || entry["name"] != name
            || entry["expired"].as_bool().is_none()
            || entry["workflow_run"]["repository_id"] != repository_id
            || entry["workflow_run"]["head_repository_id"] != repository_id
        {
            return Err(invalid("Ambiguous PR catalog artifact inventory."));
        }
        id(&entry["size_in_bytes"])?;
        digest(entry)?;
        runs.insert(id(&entry["workflow_run"]["id"])?);
    }
    if runs.len() > 8 {
        return Err(invalid("PR catalog run inventory exceeds its bound."));
    }
    let mut result = entries.clone();
    result.sort_by_key(|entry| std::cmp::Reverse(entry["workflow_run"]["id"].as_u64()));
    Ok(result)
}
fn run_producer(
    value: &Value,
    run_id: u64,
    policy: &Policy<'_>,
) -> io::Result<Option<wire::Producer>> {
    if value["id"] != run_id
        || value["repository"]["id"] != policy.repository_id
        || value["head_repository"]["id"] != policy.repository_id
    {
        return Err(invalid("Producer run repository identity is inconsistent."));
    }
    let workflow_id = id(&value["workflow_id"])?;
    let tooling = text(&value["head_sha"])?;
    let path = text(&value["path"])?;
    let attempt = id(&value["run_attempt"])?;
    if !wire::git_sha(tooling) || attempt > 100 {
        return Err(invalid("Malformed producer run identity."));
    }
    let Some(approved) = policy.producers.iter().find(|entry| {
        entry.workflow_id == workflow_id
            && entry.tooling_sha == tooling
            && path == ".github/workflows/pr-rc.yml"
    }) else {
        return Ok(None);
    };
    if value["event"] != "workflow_dispatch"
        || value["head_branch"] != "main"
        || value["status"] != "completed"
        || value["conclusion"] != "success"
    {
        return Err(invalid(
            "Newest trusted PR producer is not complete and successful.",
        ));
    }
    Ok(Some(wire::Producer {
        workflow_id,
        workflow_path: path.into(),
        workflow_sha256: approved.workflow_sha256.into(),
        tooling_sha: tooling.into(),
        run_id,
        run_attempt: attempt as u32,
    }))
}
fn artifact_metadata(
    value: &Value,
    name: &str,
    producer: &wire::Producer,
    repository_id: u64,
    maximum: usize,
) -> io::Result<()> {
    id(&value["id"])?;
    digest(value)?;
    if value["name"] != name
        || value["expired"] != false
        || id(&value["size_in_bytes"])? > maximum as u64
        || value["workflow_run"]["id"] != producer.run_id
        || value["workflow_run"]["repository_id"] != repository_id
        || value["workflow_run"]["head_repository_id"] != repository_id
        || value["workflow_run"]["head_branch"] != "main"
        || value["workflow_run"]["head_sha"] != producer.tooling_sha
    {
        return Err(invalid(
            "Selected PR artifact membership or availability does not match.",
        ));
    }
    Ok(())
}

pub(super) struct Request<'a> {
    pub product: Product,
    pub pr: PrNumber,
    pub exact: Option<&'a semver::Version>,
    pub target: &'a str,
    pub opt_in: bool,
    pub deadline: Instant,
}

/// Live acquisition uses the same compiled authority and local owner for first
/// installation and later updates. Tests inject policy only at `download`.
pub(super) fn download_current(
    request: Request<'_>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<Response>,
) -> io::Result<DownloadedRelease> {
    let product = request.product;
    download(
        request,
        Policy {
            repository_id: 1_118_285_740,
            producers: APPROVED,
        },
        get,
        || super::local_application_schema(product),
        || {
            u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(io::Error::other)?
                    .as_millis(),
            )
            .map_err(io::Error::other)
        },
    )
}

pub(super) fn download(
    request: Request<'_>,
    policy: Policy<'_>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<Response>,
    mut local: impl FnMut() -> io::Result<Vec<wire::DatabaseSchema>>,
    mut now: impl FnMut() -> io::Result<u64>,
) -> io::Result<DownloadedRelease> {
    let Request {
        product,
        pr,
        exact,
        target,
        opt_in,
        deadline,
    } = request;
    if policy.producers.is_empty() || policy.producers.len() > 16 {
        return Err(io::Error::other(Unavailable(
            "PR candidate publisher authority is unknown; no producer tuple is approved.",
        )));
    }
    let mut api = Api {
        get,
        deadline,
        requests: 0,
        remaining: METADATA_LIMIT,
    };
    let pull_route = format!("pulls/{pr}");
    let (initial_pull, initial_pull_bytes) = api.metadata(&pull_route)?;
    let head = pull(&initial_pull, pr, &policy)?;
    let timeline_route = format!("issues/{pr}/timeline?per_page=100&page=1");
    let (initial_timeline, initial_timeline_bytes) = api.metadata(&timeline_route)?;
    let initial_epoch = eligibility(&initial_pull, &initial_timeline)?;
    let name = format!("tmt-pr-rc-catalog-v2-pr{pr}");
    let inventory_route = format!("actions/artifacts?name={name}&per_page=100&page=1");
    let (initial_inventory, _) = api.metadata(&inventory_route)?;
    let entries = inventory(&initial_inventory, &name, policy.repository_id)?;
    let mut selected = None;
    let mut seen = BTreeSet::new();
    for entry in &entries {
        let run_id = id(&entry["workflow_run"]["id"])?;
        if !seen.insert(run_id) {
            continue;
        }
        let (run, _) = api.metadata(&format!("actions/runs/{run_id}"))?;
        let Some(producer) = run_producer(&run, run_id, &policy)? else {
            continue;
        };
        if entries
            .iter()
            .filter(|entry| entry["workflow_run"]["id"] == run_id)
            .count()
            != 1
        {
            return Err(invalid("Newest trusted generation has duplicate catalogs."));
        }
        selected = Some((entry.clone(), run, producer));
        break;
    }
    let (listed, initial_run, producer) = selected
        .ok_or_else(|| invalid("No approved PR candidate is available for the current head."))?;
    let (workflow, _) = api.metadata(&format!(
        "contents/{}?ref={}",
        producer.workflow_path, producer.tooling_sha
    ))?;
    if workflow["type"] != "file" || workflow["encoding"] != "base64" {
        return Err(invalid(
            "Approved publisher workflow bytes are unavailable.",
        ));
    }
    let content = text(&workflow["content"])?.replace(['\n', '\r'], "");
    let blob = base64::engine::general_purpose::STANDARD
        .decode(content)
        .map_err(|_| invalid("Invalid approved workflow blob."))?;
    if artifact::digest(&blob) != producer.workflow_sha256 {
        return Err(invalid(
            "Publisher workflow does not match its externally approved tuple.",
        ));
    }
    let catalog_id = id(&listed["id"])?;
    let catalog_route = format!("actions/artifacts/{catalog_id}");
    let (catalog_metadata, _) = api.metadata(&catalog_route)?;
    if catalog_metadata != listed {
        return Err(invalid("Catalog metadata changed during discovery."));
    }
    artifact_metadata(
        &catalog_metadata,
        &name,
        &producer,
        policy.repository_id,
        wire::CATALOG_ZIP_LIMIT,
    )?;
    let catalog_zip = api.zip(&catalog_metadata, wire::CATALOG_ZIP_LIMIT)?;
    let raw_catalog = pr_zip::catalog(&catalog_zip)?;
    let catalog = wire::Catalog::parse(&raw_catalog, pr, &head, now()?, policy.producers)?;
    if catalog.producer != producer || catalog.eligibility != initial_epoch {
        return Err(invalid(
            "Catalog producer or eligibility epoch does not match current API evidence.",
        ));
    }
    let candidate = catalog
        .candidates
        .iter()
        .find(|entry| entry.product == product.as_str() && entry.target == target)
        .cloned()
        .ok_or_else(|| invalid("Current PR candidate does not contain this product and target."))?;
    let version = candidate
        .version
        .parse::<semver::Version>()
        .map_err(io::Error::other)?;
    if exact.is_some_and(|exact| exact != &version) {
        return Err(invalid(
            "Current PR archive does not match the requested exact version.",
        ));
    }
    let payload_route = format!("actions/artifacts/{}", candidate.payload_artifact.id);
    let (payload_metadata, _) = api.metadata(&payload_route)?;
    artifact_metadata(
        &payload_metadata,
        &candidate.payload_artifact.name,
        &producer,
        policy.repository_id,
        wire::PAYLOAD_ZIP_LIMIT,
    )?;
    if digest(&payload_metadata)? != candidate.payload_artifact.zip_sha256
        || payload_metadata["size_in_bytes"] != candidate.payload_artifact.zip_bytes
    {
        return Err(invalid(
            "Payload metadata does not match the selected catalog.",
        ));
    }
    let payload_zip = api.zip(&payload_metadata, wire::PAYLOAD_ZIP_LIMIT)?;
    let mut members = pr_zip::members(
        &payload_zip,
        wire::PAYLOAD_ZIP_LIMIT,
        &[candidate.dist_manifest.clone(), candidate.archive.clone()],
    )?;
    let manifest = members
        .remove("dist-manifest.json")
        .ok_or_else(|| invalid("Missing PR manifest."))?;
    let archive = members
        .remove(&candidate.archive.name)
        .ok_or_else(|| invalid("Missing PR native archive."))?;
    let (archive_name, manifest_version) = artifact::select(product, &manifest, target)?;
    if archive_name != candidate.archive.name || manifest_version != version {
        return Err(invalid("Catalog and native cargo-dist selection disagree."));
    }
    // A single admission observation: no retries, fallback or reselection.
    let (fresh_pull, raw_pull) = api.metadata(&pull_route)?;
    if pull(&fresh_pull, pr, &policy)? != head {
        return Err(invalid("PR head changed before admission."));
    }
    let (fresh_timeline, raw_timeline) = api.metadata(&timeline_route)?;
    if eligibility(&fresh_pull, &fresh_timeline)? != initial_epoch {
        return Err(invalid("PR eligibility epoch changed before admission."));
    }
    let (fresh_inventory, raw_inventory) = api.metadata(&inventory_route)?;
    inventory(&fresh_inventory, &name, policy.repository_id)?;
    let (fresh_run, raw_run) = api.metadata(&format!("actions/runs/{}", producer.run_id))?;
    let (fresh_catalog, raw_metadata) = api.metadata(&catalog_route)?;
    let (fresh_payload, raw_payload) = api.metadata(&payload_route)?;
    if fresh_inventory != initial_inventory
        || fresh_run != initial_run
        || fresh_catalog != catalog_metadata
        || fresh_payload != payload_metadata
    {
        return Err(invalid(
            "PR candidate changed before its admission snapshot.",
        ));
    }
    // Existing release discovery retains ownership of latest-alpha selection.
    let alpha = release::schema_evidence(
        product,
        target,
        deadline,
        &mut |url, accept, maximum, deadline| {
            if url.starts_with(ROOT) && accept == "application/vnd.github+json" {
                api.metadata(
                    url.strip_prefix(&format!("{ROOT}/"))
                        .ok_or_else(|| invalid("Invalid alpha API route."))?,
                )
                .map(|(_, body)| Response { body, link: None })
            } else {
                (api.get)(url, accept, maximum, deadline)
            }
        },
    )?;
    let local_schemas = local()?;
    super::pr_receipt::admit_databases(
        product,
        &candidate.application_schema,
        &alpha.application_schema,
        &local_schemas,
        opt_in,
    )?;
    let provenance = PrProvenance {
        repository: OFFICIAL_REPOSITORY.into(),
        repository_id: policy.repository_id,
        pr: pr.get(),
        head_sha: head,
        producer,
        eligibility: initial_epoch,
        catalog_artifact_id: catalog_id,
        catalog_zip_sha256: artifact::digest(&catalog_zip),
        catalog_sha256: artifact::digest(&raw_catalog),
        candidate,
        admission: Admission {
            observed_at_ms: now()?,
            published_at_ms: catalog.published_at_ms,
            expires_at_ms: catalog.expires_at_ms,
            pull_sha256: artifact::digest(&raw_pull),
            timeline_sha256: artifact::digest(&raw_timeline),
            initial_pull_sha256: artifact::digest(&initial_pull_bytes),
            initial_timeline_sha256: artifact::digest(&initial_timeline_bytes),
            catalog_inventory_sha256: artifact::digest(&raw_inventory),
            producer_run_sha256: artifact::digest(&raw_run),
            catalog_metadata_sha256: artifact::digest(&raw_metadata),
            payload_metadata_sha256: artifact::digest(&raw_payload),
            latest_alpha: alpha,
            local_schemas,
            schema_ahead_opt_in: opt_in,
        },
    };
    provenance.validate(
        product,
        target,
        &tmt_core::native_install::InstalledVersion {
            version: version.clone(),
            channel: Channel::Pr(pr),
            pinned_version: None,
        },
        &archive_name,
        &artifact::digest(&archive),
    )?;
    provenance.verify_manifest(&manifest)?;
    Ok(DownloadedRelease {
        version,
        manifest,
        archive,
        archive_name,
        provenance: Provenance::Pr(Box::new(provenance)),
        explicit_channel: false,
    })
}

#[cfg(test)]
pub(super) mod tests;
