//! Canonical GitHub release discovery, separate from cargo-dist archive policy.

use super::{OFFICIAL_REPOSITORY, artifact, invalid, receipt::GitHubProvenance};
use semver::Version;
use serde_json::Value;
use std::{io, time::Instant};
use tmt_core::content_digest::sha256;
use tmt_core::native_install::{Channel, latest_in_channel};

const METADATA_LIMIT: usize = 2 * 1024 * 1024;
const MANIFEST_NAME: &str = "dist-manifest.json";

pub(super) struct DownloadedRelease {
    pub version: Version,
    pub manifest: Vec<u8>,
    pub archive: Vec<u8>,
    pub archive_name: String,
    pub provenance: GitHubProvenance,
}

#[cfg(test)]
pub(super) fn download(
    channel: Channel,
    exact: Option<&Version>,
    target: &str,
    deadline: Instant,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<Vec<u8>>,
) -> io::Result<DownloadedRelease> {
    download_product(super::Product::Cli, channel, exact, target, deadline, get)
}

pub(super) fn download_product(
    product: super::Product,
    channel: Channel,
    exact: Option<&Version>,
    target: &str,
    deadline: Instant,
    mut get: impl FnMut(&str, &str, usize, Instant) -> io::Result<Vec<u8>>,
) -> io::Result<DownloadedRelease> {
    let endpoint = format!("https://api.github.com/repos/{OFFICIAL_REPOSITORY}/releases");
    let document = if let Some(version) = exact {
        json(&get(
            &format!("{endpoint}/tags/{}{version}", product.tag_prefix()),
            "application/vnd.github+json",
            METADATA_LIMIT,
            deadline,
        )?)?
    } else {
        discover_latest(product, channel, deadline, &mut get)?.0
    };
    let version = version(product, &document)?;
    if !channel.accepts(&version) || exact.is_some_and(|expected| expected != &version) {
        return Err(invalid(
            "Release version does not match the selected channel or exact version.",
        ));
    }
    let flag_accepted = document["prerelease"]
        .as_bool()
        .is_some_and(|flagged| product.accepts_prerelease_flag(&version, flagged));
    if document["draft"] != false || document["immutable"] != true || !flag_accepted {
        return Err(invalid(
            "Native updates require a non-draft immutable release with matching channel metadata.",
        ));
    }
    let release_id = document["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("Invalid native release ID."))?;
    let manifest_asset = asset(&document, MANIFEST_NAME, artifact::MANIFEST_LIMIT)?;
    let manifest = fetch_asset(
        &endpoint,
        &manifest_asset,
        artifact::MANIFEST_LIMIT,
        deadline,
        &mut get,
    )?;
    let (archive_name, manifest_version) = artifact::select(product, &manifest, target)?;
    if manifest_version != version {
        return Err(invalid(
            "Release and cargo-dist manifest versions disagree.",
        ));
    }
    let archive_asset = asset(&document, &archive_name, artifact::COMPRESSED_LIMIT)?;
    let archive = fetch_asset(
        &endpoint,
        &archive_asset,
        artifact::COMPRESSED_LIMIT,
        deadline,
        &mut get,
    )?;
    Ok(DownloadedRelease {
        version,
        archive_name,
        archive,
        manifest,
        provenance: GitHubProvenance {
            release_id,
            manifest_sha256: manifest_asset.digest,
        },
    })
}

/// The newest non-draft release of `product` in `channel`, found by bounded
/// discovery (at most three pages), with its parsed version. No asset is read.
pub(super) fn discover_latest(
    product: super::Product,
    channel: Channel,
    deadline: Instant,
    get: &mut impl FnMut(&str, &str, usize, Instant) -> io::Result<Vec<u8>>,
) -> io::Result<(Value, Version)> {
    let endpoint = format!("https://api.github.com/repos/{OFFICIAL_REPOSITORY}/releases");
    let mut releases = Vec::new();
    for page in 1..=3 {
        let document = json(&get(
            &format!("{endpoint}?per_page=100&page={page}"),
            "application/vnd.github+json",
            METADATA_LIMIT,
            deadline,
        )?)?;
        let entries = document
            .as_array()
            .ok_or_else(|| invalid("Invalid release discovery response."))?;
        if entries.len() > 100 {
            return Err(invalid("Release discovery exceeds its page bound."));
        }
        releases.extend(
            entries
                .iter()
                .filter(|entry| entry["draft"] == false)
                .cloned(),
        );
        if entries.len() < 100 {
            break;
        }
        if page == 3 {
            return Err(invalid(
                "Release discovery exceeds its bound; select an exact version with --to.",
            ));
        }
    }
    let candidates = releases
        .iter()
        .filter_map(|release| {
            version(product, release)
                .ok()
                .map(|version| (release, version))
        })
        .collect::<Vec<_>>();
    let versions = candidates
        .iter()
        .map(|(_, version)| version.clone())
        .collect::<Vec<_>>();
    let selected = latest_in_channel(&versions, channel)
        .map_err(io::Error::other)?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "No release is available in the selected native channel.",
            )
        })?;
    let (release, version) = candidates
        .iter()
        .find(|(_, version)| version == selected)
        .expect("selected release exists");
    Ok(((*release).clone(), version.clone()))
}

fn json(bytes: &[u8]) -> io::Result<Value> {
    serde_json::from_slice(bytes).map_err(|_| invalid("Invalid release metadata JSON."))
}

fn version(product: super::Product, release: &Value) -> io::Result<Version> {
    release["tag_name"]
        .as_str()
        .and_then(|tag| tag.strip_prefix(product.tag_prefix()))
        .and_then(|version| version.parse().ok())
        .ok_or_else(|| invalid("Release tag is not a canonical native version."))
}

struct Asset {
    id: u64,
    size: usize,
    digest: String,
}

fn asset(release: &Value, name: &str, maximum: usize) -> io::Result<Asset> {
    let entries = release["assets"]
        .as_array()
        .ok_or_else(|| invalid("Release assets are missing."))?;
    let matches = entries
        .iter()
        .filter(|asset| asset["name"] == name)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(invalid(
            "Selected release does not contain exactly one required native asset.",
        ));
    }
    let value = matches[0];
    if value["state"] != "uploaded" {
        return Err(invalid("Native release asset is not fully uploaded."));
    }
    let id = value["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("Invalid native release asset ID."))?;
    let size = value["size"]
        .as_u64()
        .filter(|size| *size > 0 && *size <= maximum as u64)
        .ok_or_else(|| invalid("Native release asset exceeds its size bound."))?
        as usize;
    let digest = value["digest"]
        .as_str()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hash| tmt_core::content_digest::is_sha256(hash))
        .ok_or_else(|| invalid("Native release asset has no valid GitHub SHA-256 digest."))?
        .into();
    Ok(Asset { id, size, digest })
}

fn fetch_asset(
    endpoint: &str,
    asset: &Asset,
    maximum: usize,
    deadline: Instant,
    get: &mut impl FnMut(&str, &str, usize, Instant) -> io::Result<Vec<u8>>,
) -> io::Result<Vec<u8>> {
    let bytes = get(
        &format!("{endpoint}/assets/{}", asset.id),
        "application/octet-stream",
        maximum,
        deadline,
    )?;
    if bytes.len() != asset.size || sha256(&bytes) != asset.digest {
        return Err(invalid(
            "Downloaded asset does not match GitHub's recorded size and digest.",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod release_tests;

#[cfg(test)]
pub(super) use release_tests::{companion_fixture, product_fixture, valid_fixture};
