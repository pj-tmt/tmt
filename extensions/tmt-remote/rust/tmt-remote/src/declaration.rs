//! Strict version-1 extension backend declarations (contract: Backends and deploy,
//! "extension backend declarations"). Pure parsing: no I/O, no clock, no storage.
//! A declaration names resources an extension needs; it grants nothing and carries
//! no executable command or authority.
use crate::{
    canonical,
    hosting::{HostingManifest, HostingRefusal},
    limits, mount, routes, wire,
};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Local,
    Firestore,
    Cloudflare,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Log,
    Checkpoint,
    Blob,
    Awareness,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}
/// Positive bounds the extension asks for; the plan checks them against the backend's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    pub max_object_bytes: u64,
    pub max_namespace_bytes: u64,
    pub max_entries: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub field: String,
    pub direction: Direction,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub name: String,
    pub kind: Kind,
    /// Relative to the extension's namespace root; validated segments, joined by `/`.
    pub path: String,
    pub bounds: ResourceLimits,
    pub ttl_field: Option<String>,
    pub indexes: Vec<Index>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    pub artifact: String,
    /// Lowercase SHA-256 hex of the artifact; the plan checks it against the bytes it is given.
    pub digest: String,
    pub entry_point: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub extension: String,
    pub backend: Backend,
    pub resources: Vec<Resource>,
    pub admission: Admission,
    pub hosting: Option<HostingManifest>,
}

/// Why a declaration was refused. Codes are fixed words; no declared text is echoed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    TooLarge,
    NotStrictJson,
    Shape,
    Version,
    ExtensionName,
    Backend,
    TooManyResources,
    ResourceName,
    DuplicateResourceName,
    Kind,
    Path,
    ReservedPath,
    Limit,
    AwarenessStores,
    TtlField,
    TooManyIndexes,
    IndexField,
    IndexDirection,
    DuplicateIndex,
    ArtifactPath,
    Digest,
    EntryPoint,
    Hosting(HostingRefusal),
}
impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Self::TooLarge => "too-large",
            Self::NotStrictJson => "not-strict-json",
            Self::Shape => "shape",
            Self::Version => "version",
            Self::ExtensionName => "extension-name",
            Self::Backend => "backend",
            Self::TooManyResources => "too-many-resources",
            Self::ResourceName => "resource-name",
            Self::DuplicateResourceName => "duplicate-resource-name",
            Self::Kind => "kind",
            Self::Path => "path",
            Self::ReservedPath => "reserved-path",
            Self::Limit => "limit",
            Self::AwarenessStores => "awareness-stores",
            Self::TtlField => "ttl-field",
            Self::TooManyIndexes => "too-many-indexes",
            Self::IndexField => "index-field",
            Self::IndexDirection => "index-direction",
            Self::DuplicateIndex => "duplicate-index",
            Self::ArtifactPath => "artifact-path",
            Self::Digest => "digest",
            Self::EntryPoint => "entry-point",
            Self::Hosting(_) => "hosting",
        }
    }
}
/// A refusal with the position of the resource it concerns, when there is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeclarationError {
    pub resource: Option<usize>,
    pub reason: Reason,
}
type Result<T> = std::result::Result<T, DeclarationError>;
fn refuse<T>(reason: Reason) -> Result<T> {
    Err(DeclarationError {
        resource: None,
        reason,
    })
}
fn refuse_resource<T>(position: usize, reason: Reason) -> Result<T> {
    Err(DeclarationError {
        resource: Some(position),
        reason,
    })
}

/// Parse and validate one declaration. ResourceLimits against a particular backend are the
/// plan's decision, not this parser's.
pub fn parse(bytes: &[u8]) -> Result<Declaration> {
    if bytes.len() > limits::DECLARATION_BYTES {
        return refuse(Reason::TooLarge);
    }
    let Some(value) = wire::strict_json(bytes) else {
        return refuse(Reason::NotStrictJson);
    };
    let top = value.as_object().ok_or(DeclarationError {
        resource: None,
        reason: Reason::Shape,
    })?;
    let keys = ["version", "extension", "backend", "resources", "admission"];
    if !keys.iter().all(|key| top.contains_key(*key))
        || top
            .keys()
            .any(|key| !keys.contains(&key.as_str()) && key != "hosting")
    {
        return refuse(Reason::Shape);
    }
    let hosting = top
        .get("hosting")
        .map(HostingManifest::parse)
        .transpose()
        .map_err(|reason| DeclarationError {
            resource: None,
            reason: Reason::Hosting(reason),
        })?;
    if top["version"].as_u64() != Some(1) {
        return refuse(Reason::Version);
    }
    let extension = text(&top["extension"], Reason::ExtensionName)?;
    if !canonical::extension_name(extension) {
        return refuse(Reason::ExtensionName);
    }
    let backend = match text(&top["backend"], Reason::Backend)? {
        "local" => Backend::Local,
        "firestore" => Backend::Firestore,
        "cloudflare" => Backend::Cloudflare,
        _ => return refuse(Reason::Backend),
    };
    if hosting.is_some() && backend != Backend::Firestore {
        return refuse(Reason::Backend);
    }
    let Some(listed) = top["resources"].as_array() else {
        return refuse(Reason::Shape);
    };
    if listed.len() > limits::DECLARATION_RESOURCES {
        return refuse(Reason::TooManyResources);
    }
    let mut resources = Vec::with_capacity(listed.len());
    let mut names = BTreeSet::new();
    for (position, item) in listed.iter().enumerate() {
        let resource = resource(position, item)?;
        if !names.insert(resource.name.clone()) {
            return refuse_resource(position, Reason::DuplicateResourceName);
        }
        resources.push(resource);
    }
    Ok(Declaration {
        extension: extension.to_owned(),
        backend,
        resources,
        admission: admission(&top["admission"])?,
        hosting,
    })
}

fn resource(position: usize, value: &Value) -> Result<Resource> {
    let at = |error: DeclarationError| DeclarationError {
        resource: Some(position),
        ..error
    };
    resource_inner(value).map_err(at)
}
fn resource_inner(value: &Value) -> Result<Resource> {
    let top = members(
        value,
        &["name", "kind", "path", "limits", "ttlField", "indexes"],
    )?;
    let name = text(&top["name"], Reason::ResourceName)?;
    if !canonical::extension_name(name) {
        return refuse(Reason::ResourceName);
    }
    let kind = match text(&top["kind"], Reason::Kind)? {
        "log" => Kind::Log,
        "checkpoint" => Kind::Checkpoint,
        "blob" => Kind::Blob,
        "awareness" => Kind::Awareness,
        _ => return refuse(Reason::Kind),
    };
    let path = namespace_path(text(&top["path"], Reason::Path)?)?;
    let bounds = bounds(&top["limits"])?;
    let ttl_field = match &top["ttlField"] {
        Value::Null => None,
        other => {
            let field = text(other, Reason::TtlField)?;
            if !field_name(field) {
                return refuse(Reason::TtlField);
            }
            Some(field.to_owned())
        }
    };
    let Some(listed) = top["indexes"].as_array() else {
        return refuse(Reason::Shape);
    };
    if listed.len() > limits::DECLARATION_INDEXES {
        return refuse(Reason::TooManyIndexes);
    }
    let mut indexes: Vec<Index> = Vec::with_capacity(listed.len());
    for item in listed {
        let entry = members(item, &["field", "direction"])?;
        let field = text(&entry["field"], Reason::IndexField)?;
        if !field_name(field) {
            return refuse(Reason::IndexField);
        }
        let direction = match text(&entry["direction"], Reason::IndexDirection)? {
            "asc" => Direction::Asc,
            "desc" => Direction::Desc,
            _ => return refuse(Reason::IndexDirection),
        };
        if indexes
            .iter()
            .any(|seen| seen.field == field && seen.direction == direction)
        {
            return refuse(Reason::DuplicateIndex);
        }
        indexes.push(Index {
            field: field.to_owned(),
            direction,
        });
    }
    // Awareness is ephemeral: nothing stored, so nothing to expire or index.
    if kind == Kind::Awareness && (ttl_field.is_some() || !indexes.is_empty()) {
        return refuse(Reason::AwarenessStores);
    }
    Ok(Resource {
        name: name.to_owned(),
        kind,
        path,
        bounds,
        ttl_field,
        indexes,
    })
}

fn bounds(value: &Value) -> Result<ResourceLimits> {
    let top = members(
        value,
        &["maxObjectBytes", "maxNamespaceBytes", "maxEntries"],
    )?;
    let positive = |value: &Value| -> Result<u64> {
        // `as_u64` is None for fractions, exponents, signs and values beyond u64.
        match value.as_u64() {
            Some(number) if (1..=MAX_SAFE_INTEGER).contains(&number) => Ok(number),
            _ => refuse(Reason::Limit),
        }
    };
    let bounds = ResourceLimits {
        max_object_bytes: positive(&top["maxObjectBytes"])?,
        max_namespace_bytes: positive(&top["maxNamespaceBytes"])?,
        max_entries: positive(&top["maxEntries"])?,
    };
    // An object larger than its namespace could never be stored.
    if bounds.max_object_bytes > bounds.max_namespace_bytes {
        return refuse(Reason::Limit);
    }
    Ok(bounds)
}
/// The largest integer every JSON implementation represents exactly (2^53 - 1).
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

fn admission(value: &Value) -> Result<Admission> {
    let top = members(value, &["artifact", "digest", "entryPoint"])?;
    let artifact = text(&top["artifact"], Reason::ArtifactPath)?;
    if !artifact_path(artifact) {
        return refuse(Reason::ArtifactPath);
    }
    let digest = text(&top["digest"], Reason::Digest)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return refuse(Reason::Digest);
    }
    let entry_point = text(&top["entryPoint"], Reason::EntryPoint)?;
    if !entry_point_name(entry_point) {
        return refuse(Reason::EntryPoint);
    }
    Ok(Admission {
        artifact: artifact.to_owned(),
        digest: digest.to_owned(),
        entry_point: entry_point.to_owned(),
    })
}

/// An object holding exactly `keys`: a missing or extra member is a shape error.
fn members<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>> {
    match value.as_object() {
        Some(object)
            if object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key)) =>
        {
            Ok(object)
        }
        _ => refuse(Reason::Shape),
    }
}
fn text(value: &Value, otherwise: Reason) -> Result<&str> {
    value.as_str().map_or_else(|| refuse(otherwise), Ok)
}

/// A namespace path: 1 to 8 segments of `[a-z0-9_-]`, none empty, `.`/`..`, wildcard or
/// escape, and the first not an operation route or the mount segment. It is always
/// resolved under the extension's own root, so it cannot name another root.
fn namespace_path(value: &str) -> Result<String> {
    let segments: Vec<&str> = value.split('/').collect();
    if segments.len() > limits::DECLARATION_PATH_SEGMENTS
        || !segments.iter().all(|segment| {
            (1..=limits::DECLARATION_IDENTIFIER_BYTES).contains(&segment.len())
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        })
    {
        return refuse(Reason::Path);
    }
    if reserved_segment(segments[0]) {
        return refuse(Reason::ReservedPath);
    }
    Ok(value.to_owned())
}
/// The names an extension namespace may not start with: the operation routes of the
/// binding, and the mount segment that separates extensions from them.
pub fn reserved_segment(segment: &str) -> bool {
    segment == mount::MOUNT_SEGMENT
        || routes::ROUTES
            .iter()
            .any(|route| route.trim_start_matches('/') == segment)
}
/// An installed artifact path: 1 to 8 segments of `[A-Za-z0-9._-]`, each starting with
/// a letter or digit, so there is no absolute, dot, dotfile or escaping segment.
fn artifact_path(value: &str) -> bool {
    let segments: Vec<&str> = value.split('/').collect();
    segments.len() <= limits::DECLARATION_PATH_SEGMENTS
        && segments.iter().all(|segment| {
            (1..=limits::DECLARATION_IDENTIFIER_BYTES).contains(&segment.len())
                && segment.starts_with(|c: char| c.is_ascii_alphanumeric())
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}
/// A stored field name: a letter, then letters or digits.
fn field_name(value: &str) -> bool {
    (1..=limits::DECLARATION_IDENTIFIER_BYTES).contains(&value.len())
        && value.starts_with(|c: char| c.is_ascii_alphabetic())
        && value.bytes().all(|b| b.is_ascii_alphanumeric())
}
/// An admission entry point: a letter or underscore, then letters, digits or underscores.
fn entry_point_name(value: &str) -> bool {
    (1..=limits::DECLARATION_IDENTIFIER_BYTES).contains(&value.len())
        && value.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
