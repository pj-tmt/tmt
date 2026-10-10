//! Release-index discovery, separate from cargo-dist archive and activation policy.

use super::{
    artifact, invalid,
    receipt::{GitHubProvenance, Provenance},
};
use semver::Version;
use serde_json::Value;
use std::{collections::BTreeMap, io, time::Instant};
use tmt_core::content_digest::{is_sha256, sha256};
use tmt_core::native_install::Channel;

const INDEX_BASE: &str = "https://raw.githubusercontent.com/pj-tmt/tmt/release-index/";
const RELEASE_BASE: &str = "https://github.com/pj-tmt/tmt/releases/download/";
const POINTER_LIMIT: usize = 16 * 1024;
const RECORD_LIMIT: usize = 256 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MANIFEST_NAME: &str = "dist-manifest.json";
const RECORD_NAME: &str = "tmt-release-record.json";
const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-musl",
];

/// Compatibility for callers reporting a confirmed absence of published releases.
/// An unavailable index pointer is an acquisition failure, never this observation.
#[derive(Debug)]
pub struct ReleaseUnavailable {
    pub product: super::Product,
    pub channel: Channel,
}
impl std::fmt::Display for ReleaseUnavailable {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            output,
            "No published {} release yet in the {} channel.",
            self.product.as_str(),
            self.channel.as_str()
        )
    }
}
impl std::error::Error for ReleaseUnavailable {}

pub(super) struct DownloadedRelease {
    pub version: Version,
    pub manifest: Vec<u8>,
    pub archive: Vec<u8>,
    pub archive_name: String,
    pub provenance: Provenance,
    pub explicit_channel: bool,
}

struct Asset {
    name: String,
    url: String,
    size: usize,
    digest: String,
}
pub(super) struct ReleaseRecord {
    version: Version,
    release_id: u64,
    source_sha: String,
    manifest: Asset,
    archives: BTreeMap<String, Asset>,
}

pub(super) fn download_product(
    product: super::Product,
    channel: Channel,
    exact: Option<&Version>,
    target: &str,
    deadline: Instant,
    mut get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<DownloadedRelease> {
    let record = if let Some(version) = exact {
        canonical_version(&version.to_string())?;
        let tag = format!("{}{version}", product.tag_prefix());
        let response = get(&format!("{RELEASE_BASE}{tag}/{RECORD_NAME}"), "application/json", RECORD_LIMIT, deadline)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    io::Error::new(error.kind(), format!("{tag} cannot be installed directly. Omit --to to get the latest release. Nothing was changed."))
                } else {
                    io::Error::new(error.kind(), format!("Could not read {tag} on github.com: {error}. Nothing was changed."))
                }
            })?;
        parse_record(&response.body, product, channel, Some(version))?
    } else {
        discover_latest(product, channel, deadline, &mut get)?.0
    };
    let selected = record
        .archives
        .get(target)
        .ok_or_else(|| invalid("Unsupported release-index target."))?;
    let manifest = fetch_asset(
        &record.manifest,
        artifact::MANIFEST_LIMIT,
        deadline,
        &mut get,
    )?;
    let (archive_name, manifest_version) = artifact::select(product, &manifest, target)?;
    if manifest_version != record.version || archive_name != selected.name {
        return Err(invalid("Release index and cargo-dist manifest disagree."));
    }
    let archive = fetch_asset(selected, artifact::COMPRESSED_LIMIT, deadline, &mut get)?;
    Ok(DownloadedRelease {
        explicit_channel: false,
        version: record.version,
        archive_name,
        archive,
        manifest,
        provenance: GitHubProvenance {
            release_id: record.release_id,
            manifest_sha256: record.manifest.digest,
        }
        .into(),
    })
}

/// One fixed channel pointer, then its size/hash-bound record; no API fallback.
pub(super) fn discover_latest(
    product: super::Product,
    channel: Channel,
    deadline: Instant,
    get: &mut impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<(ReleaseRecord, Version)> {
    let mut resolve = || {
        if !Channel::ALL.contains(&channel) {
            return Err(invalid("Published release channel required."));
        }
        let pointer = get(
            &format!(
                "{INDEX_BASE}channels/{}/{}.json",
                product.as_str(),
                channel.as_str()
            ),
            "application/json",
            POINTER_LIMIT,
            deadline,
        )?;
        let pointer: Value = super::pr_json::parse(&pointer.body, POINTER_LIMIT)?;
        let version = identity(&pointer, product, channel)?;
        if pointer["channel"] != channel.as_str().as_ref() {
            return Err(invalid("Release index channel mismatch."));
        }
        let tag = format!("{}{version}", product.tag_prefix());
        let url = pointer["record"]["url"]
            .as_str()
            .ok_or_else(|| invalid("Release index record URL missing."))?;
        if url != format!("{RELEASE_BASE}{tag}/{RECORD_NAME}")
            && url != format!("{INDEX_BASE}records/{tag}.json")
        {
            return Err(invalid("Release index record URL mismatch."));
        }
        let descriptor = descriptor(&pointer["record"], RECORD_NAME, url, RECORD_LIMIT)?;
        let bytes = fetch_asset(&descriptor, RECORD_LIMIT, deadline, get)?;
        let record = parse_record(&bytes, product, channel, Some(&version))?;
        Ok((record, version))
    };
    resolve().map_err(|error: io::Error| io::Error::new(error.kind(), format!("Could not check for {} updates on raw.githubusercontent.com: {error}. Nothing was changed.", product.as_str())))
}

fn canonical_version(text: &str) -> io::Result<Version> {
    let version: Version = text
        .parse()
        .map_err(|_| invalid("Release index requires a canonical stable or alpha version."))?;
    let alpha = version
        .pre
        .as_str()
        .strip_prefix("alpha.")
        .and_then(|part| part.parse::<u64>().ok());
    if version.to_string() != text
        || !version.build.is_empty()
        || [version.major, version.minor, version.patch]
            .iter()
            .any(|part| *part > MAX_SAFE_INTEGER)
        || (!version.pre.is_empty()
            && alpha.is_none_or(|part| {
                part > MAX_SAFE_INTEGER || version.pre.as_str() != format!("alpha.{part}")
            }))
    {
        return Err(invalid(
            "Release index requires a canonical stable or alpha version.",
        ));
    }
    Ok(version)
}

fn identity(value: &Value, product: super::Product, channel: Channel) -> io::Result<Version> {
    if !value.is_object() || value["schemaVersion"] != 1 {
        return Err(invalid(
            "Unsupported release-index schema; rerun install.sh",
        ));
    }
    let version = canonical_version(
        value["version"]
            .as_str()
            .ok_or_else(|| invalid("Release index version missing."))?,
    )?;
    if value["product"] != product.as_str()
        || !Channel::ALL.contains(&channel)
        || !channel.accepts(&version)
        || value["tag"] != format!("{}{version}", product.tag_prefix())
    {
        return Err(invalid(
            "Release index product, channel or tag/version mismatch.",
        ));
    }
    Ok(version)
}

fn descriptor(value: &Value, name: &str, url: &str, maximum: usize) -> io::Result<Asset> {
    let size = value["size"]
        .as_u64()
        .filter(|size| *size > 0 && *size <= maximum as u64)
        .ok_or_else(|| invalid("Release index asset size outside bounds."))?
        as usize;
    let digest = value["sha256"]
        .as_str()
        .filter(|hash| is_sha256(hash))
        .ok_or_else(|| invalid("Release index requires SHA-256."))?;
    if !value.is_object() || value["url"] != url {
        return Err(invalid("Release index asset URL mismatch."));
    }
    Ok(Asset {
        name: name.into(),
        url: url.into(),
        size,
        digest: digest.into(),
    })
}

fn parse_record(
    bytes: &[u8],
    product: super::Product,
    channel: Channel,
    exact: Option<&Version>,
) -> io::Result<ReleaseRecord> {
    let record: Value = super::pr_json::parse(bytes, RECORD_LIMIT)?;
    let version = identity(&record, product, channel)?;
    if exact.is_some_and(|expected| *expected != version) {
        return Err(invalid("Release index record version mismatch."));
    }
    let release_id = record["releaseId"]
        .as_u64()
        .filter(|id| *id > 0 && *id <= MAX_SAFE_INTEGER)
        .ok_or_else(|| invalid("Invalid release index release ID."))?;
    let source_sha = record["sourceSha"]
        .as_str()
        .filter(|sha| {
            sha.len() == 40
                && sha
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| invalid("Invalid release index source SHA."))?;
    let tag = format!("{}{version}", product.tag_prefix());
    let asset = |value: &Value, name: &str, limit| {
        if value["name"] != name {
            return Err(invalid("Release index asset name mismatch."));
        }
        descriptor(value, name, &format!("{RELEASE_BASE}{tag}/{name}"), limit)
    };
    let manifest = asset(&record["manifest"], MANIFEST_NAME, artifact::MANIFEST_LIMIT)?;
    let entries = record["archives"]
        .as_object()
        .filter(|entries| entries.len() == 4)
        .ok_or_else(|| invalid("Release index requires four native targets."))?;
    let mut archives = BTreeMap::new();
    for target in TARGETS {
        let value = entries
            .get(target)
            .ok_or_else(|| invalid("Release index requires four native targets."))?;
        let name = format!("{}-{target}.tar.gz", product.package());
        archives.insert(
            target.to_owned(),
            asset(value, &name, artifact::COMPRESSED_LIMIT)?,
        );
    }
    Ok(ReleaseRecord {
        version,
        release_id,
        source_sha: source_sha.into(),
        manifest,
        archives,
    })
}

fn fetch_asset(
    asset: &Asset,
    maximum: usize,
    deadline: Instant,
    get: &mut impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<Vec<u8>> {
    let host = if asset.url.starts_with(INDEX_BASE) {
        "raw.githubusercontent.com"
    } else {
        "github.com"
    };
    let bytes = get(
        &asset.url,
        "application/octet-stream",
        asset.size.min(maximum),
        deadline,
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Could not download release bytes from {host}: {error}"),
        )
    })?
    .body;
    if bytes.len() != asset.size || sha256(&bytes) != asset.digest {
        return Err(invalid(
            "Downloaded bytes do not match the release index size and SHA-256.",
        ));
    }
    Ok(bytes)
}

/// Manifest schema remains authoritative; the index binds its release/source/bytes.
pub(super) fn schema_evidence(
    product: super::Product,
    target: &str,
    deadline: Instant,
    get: &mut impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<super::pr_receipt::AlphaEvidence> {
    let (record, version) = discover_latest(product, Channel::Alpha, deadline, get)?;
    let manifest = fetch_asset(&record.manifest, artifact::MANIFEST_LIMIT, deadline, get)?;
    if artifact::select(product, &manifest, target)?.1 != version {
        return Err(invalid(
            "Alpha manifest version disagrees with the release index.",
        ));
    }
    let value: Value = super::pr_json::parse(&manifest, artifact::MANIFEST_LIMIT)?;
    let schema = value
        .get("tmt_application_schema")
        .ok_or_else(|| io::Error::other(tmt_core::native_install::SchemaError::Unknown))?;
    let schema: super::pr_catalog::ApplicationSchema = serde_json::from_value(schema.clone())
        .map_err(|_| io::Error::other(tmt_core::native_install::SchemaError::Unknown))?;
    schema.validate(product.as_str(), &record.source_sha)?;
    Ok(super::pr_receipt::AlphaEvidence {
        release_id: record.release_id,
        version: version.to_string(),
        manifest_sha256: record.manifest.digest,
        application_schema: schema,
    })
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod release_tests;
#[cfg(test)]
pub(super) use release_tests::{
    companion_fixture, indexed_download, indexed_routes, product_fixture, valid_fixture,
};
