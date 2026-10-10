//! The deploy plan: every enabled extension's backend declaration for one cloud backend,
//! checked together and reduced to deterministic, digest-addressed bytes (contract:
//! Backends and deploy, "extension backend declarations"). Pure: callers read the
//! installed, owner-approved files and pass bytes in; nothing here provisions, reads a
//! file or grants authority. Authorizing a plan is the deploy command's job.
//!
//! Only the `sharing` profile exists: extension resources and admission fragments, with
//! no operation root. A later profile is an additive variant, so a sharing plan's bytes
//! and digest never change when one is added.
use crate::{
    canonical,
    declaration::{self, Backend, Declaration, Direction, Kind, Reason},
    limits,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudBackend {
    Firestore,
    Cloudflare,
}
impl CloudBackend {
    fn name(self) -> &'static str {
        match self {
            Self::Firestore => "firestore",
            Self::Cloudflare => "cloudflare",
        }
    }
    fn declared(self) -> Backend {
        match self {
            Self::Firestore => Backend::Firestore,
            Self::Cloudflare => Backend::Cloudflare,
        }
    }
}
/// What the deployment target can provide. The caller decides it; the plan only records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub backend: CloudBackend,
    /// Whether the target will run a physical expiry (TTL) policy for `ttlField`.
    pub physical_ttl: bool,
}
/// One enabled extension and the bytes the owner approved for it on this backend.
#[derive(Clone, Copy, Debug)]
pub struct Enabled<'a> {
    pub name: &'a str,
    /// `None` when the extension declares nothing for this backend: it is listed as
    /// unavailable there and the rest of the plan stands.
    pub supplied: Option<Supplied<'a>>,
}
#[derive(Clone, Copy, Debug)]
pub struct Supplied<'a> {
    pub declaration: &'a [u8],
    /// The bytes of the declaration's admission artifact.
    pub artifact: &'a [u8],
}

/// Why no plan exists. A plan either composes completely or not at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanReason {
    /// The declaration itself is refused.
    Declaration(Reason),
    ExtensionName,
    DuplicateExtension,
    TooManyExtensions,
    /// The declaration names another extension than the one enabled.
    ExtensionMismatch,
    /// The declaration is for a different backend than the target.
    BackendMismatch,
    /// A resource asks for more than the backend offers: an unsupported requirement.
    LimitAboveBackend,
    ArtifactTooLarge,
    /// The artifact bytes do not hash to the declared digest.
    ArtifactDigest,
    /// Two resources share a namespace path or one lies inside the other.
    Overlap,
    /// On Firestore a resource path names a collection: an odd number of segments, at most
    /// seven, because `x/<extension>` is a document. An unsupported requirement.
    PathNotCollection,
}
impl PlanReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Declaration(reason) => reason.code(),
            Self::ExtensionName => "extension-name",
            Self::DuplicateExtension => "duplicate-extension",
            Self::TooManyExtensions => "too-many-extensions",
            Self::ExtensionMismatch => "extension-mismatch",
            Self::BackendMismatch => "backend-mismatch",
            Self::LimitAboveBackend => "limit-above-backend",
            Self::ArtifactTooLarge => "artifact-too-large",
            Self::ArtifactDigest => "artifact-digest",
            Self::Overlap => "overlap",
            Self::PathNotCollection => "path-not-collection",
        }
    }
}
/// The refusal, with the enabled extension and the position of the resource it concerns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanError {
    pub extension: Option<String>,
    pub resource: Option<usize>,
    pub reason: PlanReason,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    pub version: u8,
    pub profile: &'static str,
    pub backend: &'static str,
    pub capabilities: Capabilities,
    pub extensions: Vec<ExtensionPlan>,
    pub unavailable: Vec<Unavailable>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub physical_ttl: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPlan {
    pub name: String,
    /// SHA-256 of the exact declaration bytes, so authorization names those bytes.
    pub declaration_digest: String,
    pub admission: AdmissionPlan,
    pub resources: Vec<ResourcePlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosting: Option<crate::hosting::HostingManifest>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionPlan {
    pub artifact_digest: String,
    pub entry_point: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePlan {
    pub name: String,
    pub kind: &'static str,
    /// `x/<extension>/<declared path>`: the root is the extension's, never chosen by it.
    pub path: String,
    pub limits: LimitsPlan,
    pub ttl_field: Option<String>,
    /// `none` without a `ttlField`, `provisioned`, or `not-provisioned` when the target
    /// has no physical TTL: expiry is then by Rules and nothing is provisioned for it.
    pub ttl: &'static str,
    pub indexes: Vec<IndexPlan>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitsPlan {
    pub max_object_bytes: u64,
    pub max_namespace_bytes: u64,
    pub max_entries: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct IndexPlan {
    pub field: String,
    pub direction: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub struct Unavailable {
    pub name: String,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct Plan {
    view: PlanView,
    bytes: Vec<u8>,
    digest: String,
}
impl Plan {
    pub fn view(&self) -> &PlanView {
        &self.view
    }
    /// The canonical bytes: compact JSON in field order, extensions and resources sorted.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Lowercase SHA-256 hex of `bytes`.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// What a backend offers a resource, from the object-channel bounds the contract states
/// (`payloadBytes`) and the storage proposal's namespace bounds. The same for every cloud
/// backend until a backend ticket states its own.
const MAX_OBJECT_BYTES: u64 = limits::OBJECT_PAYLOAD_BYTES;
const MAX_NAMESPACE_BYTES: u64 = limits::OBJECT_NAMESPACE_BYTES;
/// Longest resource path that names a Firestore collection under `x/<extension>`.
const FIRESTORE_COLLECTION_SEGMENTS: usize = 7;
const MAX_ENTRIES: u64 = limits::OBJECT_NAMESPACE_ENTRIES as u64;

pub fn compose(target: Target, enabled: &[Enabled<'_>]) -> Result<Plan, PlanError> {
    let fail = |extension: Option<&str>, resource: Option<usize>, reason| PlanError {
        extension: extension.map(str::to_owned),
        resource,
        reason,
    };
    if enabled.len() > limits::PLAN_EXTENSIONS {
        return Err(fail(None, None, PlanReason::TooManyExtensions));
    }
    // Sorted by name, so the first refusal does not depend on the order of the input.
    let mut ordered: Vec<&Enabled<'_>> = enabled.iter().collect();
    ordered.sort_by(|a, b| a.name.cmp(b.name));
    let mut names = BTreeSet::new();
    for item in &ordered {
        if !canonical::extension_name(item.name) {
            return Err(fail(None, None, PlanReason::ExtensionName));
        }
        if !names.insert(item.name) {
            return Err(fail(Some(item.name), None, PlanReason::DuplicateExtension));
        }
    }
    let mut extensions = Vec::new();
    let mut unavailable = Vec::new();
    for item in ordered {
        let Some(supplied) = item.supplied else {
            unavailable.push(Unavailable {
                name: item.name.to_owned(),
                reason: "no-declaration",
            });
            continue;
        };
        let planned = extension(target, item.name, supplied)
            .map_err(|(resource, reason)| fail(Some(item.name), resource, reason))?;
        extensions.push(planned);
    }
    check_overlap(&extensions)
        .map_err(|(name, resource)| fail(Some(&name), Some(resource), PlanReason::Overlap))?;
    let view = PlanView {
        version: 1,
        profile: "sharing",
        backend: target.backend.name(),
        capabilities: Capabilities {
            physical_ttl: target.physical_ttl,
        },
        extensions: extensions.into_iter().map(|(plan, _)| plan).collect(),
        unavailable,
    };
    let bytes = serde_json::to_vec(&view).expect("a plan view serializes");
    let digest = sha256_hex(&bytes);
    Ok(Plan {
        view,
        bytes,
        digest,
    })
}

type Refusal = (Option<usize>, PlanReason);
/// One extension's plan with the declaration positions of its resources, for error reports.
type Planned = (ExtensionPlan, Vec<usize>);

fn extension(target: Target, name: &str, supplied: Supplied<'_>) -> Result<Planned, Refusal> {
    let parsed: Declaration = declaration::parse(supplied.declaration)
        .map_err(|error| (error.resource, PlanReason::Declaration(error.reason)))?;
    if parsed.extension != name {
        return Err((None, PlanReason::ExtensionMismatch));
    }
    if parsed.backend != target.backend.declared() {
        return Err((None, PlanReason::BackendMismatch));
    }
    if supplied.artifact.len() > limits::DECLARATION_ARTIFACT_BYTES {
        return Err((None, PlanReason::ArtifactTooLarge));
    }
    let artifact_digest = sha256_hex(supplied.artifact);
    if artifact_digest != parsed.admission.digest {
        return Err((None, PlanReason::ArtifactDigest));
    }
    let mut resources = Vec::with_capacity(parsed.resources.len());
    for (position, resource) in parsed.resources.iter().enumerate() {
        let bounds = resource.bounds;
        if target.backend == CloudBackend::Firestore {
            let segments = resource.path.split('/').count();
            if segments % 2 == 0 || segments > FIRESTORE_COLLECTION_SEGMENTS {
                return Err((Some(position), PlanReason::PathNotCollection));
            }
        }
        if bounds.max_object_bytes > MAX_OBJECT_BYTES
            || bounds.max_namespace_bytes > MAX_NAMESPACE_BYTES
            || bounds.max_entries > MAX_ENTRIES
        {
            return Err((Some(position), PlanReason::LimitAboveBackend));
        }
        let ttl = match (&resource.ttl_field, target.physical_ttl) {
            (None, _) => "none",
            (Some(_), true) => "provisioned",
            (Some(_), false) => "not-provisioned",
        };
        let mut indexes: Vec<IndexPlan> = resource
            .indexes
            .iter()
            .map(|index| IndexPlan {
                field: index.field.clone(),
                direction: match index.direction {
                    Direction::Asc => "asc",
                    Direction::Desc => "desc",
                },
            })
            .collect();
        indexes.sort_by(|a, b| (&a.field, a.direction).cmp(&(&b.field, b.direction)));
        resources.push((
            position,
            ResourcePlan {
                name: resource.name.clone(),
                kind: match resource.kind {
                    Kind::Log => "log",
                    Kind::Checkpoint => "checkpoint",
                    Kind::Blob => "blob",
                    Kind::Awareness => "awareness",
                },
                path: format!("{}/{name}/{}", crate::mount::MOUNT_SEGMENT, resource.path),
                limits: LimitsPlan {
                    max_object_bytes: bounds.max_object_bytes,
                    max_namespace_bytes: bounds.max_namespace_bytes,
                    max_entries: bounds.max_entries,
                },
                ttl_field: resource.ttl_field.clone(),
                ttl,
                indexes,
            },
        ));
    }
    resources.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    let (positions, resources) = resources.into_iter().unzip();
    Ok((
        ExtensionPlan {
            name: name.to_owned(),
            declaration_digest: sha256_hex(supplied.declaration),
            admission: AdmissionPlan {
                artifact_digest,
                entry_point: parsed.admission.entry_point,
            },
            resources,
            hosting: parsed.hosting,
        },
        positions,
    ))
}

/// Refuse two resources whose paths are equal or nested, as whole segments. In sorted
/// order a path's descendants follow it directly, so adjacent pairs find every overlap.
/// Returns the later resource's extension and declaration position.
fn check_overlap(extensions: &[Planned]) -> Result<(), (String, usize)> {
    let mut paths: Vec<(Vec<&str>, &str, usize)> = Vec::new();
    for (plan, positions) in extensions {
        for (resource, position) in plan.resources.iter().zip(positions) {
            paths.push((resource.path.split('/').collect(), &plan.name, *position));
        }
    }
    paths.sort();
    for pair in paths.windows(2) {
        if pair[1].0.starts_with(&pair[0].0) {
            return Err((pair[1].1.to_owned(), pair[1].2));
        }
    }
    Ok(())
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}
