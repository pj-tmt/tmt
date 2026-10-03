//! Canonical GitHub release discovery, separate from cargo-dist archive policy.

use super::{OFFICIAL_REPOSITORY, artifact, invalid, receipt::GitHubProvenance};
use semver::Version;
use serde_json::Value;
use std::{io, time::Instant};
use tmt_core::content_digest::sha256;
use tmt_core::native_install::{Channel, latest_in_channel};

const METADATA_LIMIT: usize = 2 * 1024 * 1024;
const MANIFEST_NAME: &str = "dist-manifest.json";

/// Complete product/channel discovery found no published release. Keep this
/// distinct from missing files or assets after a release has been selected.
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
    pub provenance: GitHubProvenance,
}

#[cfg(test)]
pub(super) fn download(
    channel: Channel,
    exact: Option<&Version>,
    target: &str,
    deadline: Instant,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<DownloadedRelease> {
    download_product(super::Product::Cli, channel, exact, target, deadline, get)
}

pub(super) fn download_product(
    product: super::Product,
    channel: Channel,
    exact: Option<&Version>,
    target: &str,
    deadline: Instant,
    mut get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<DownloadedRelease> {
    let endpoint = format!("https://api.github.com/repos/{OFFICIAL_REPOSITORY}/releases");
    let document = if let Some(version) = exact {
        json(
            &get(
                &format!("{endpoint}/tags/{}{version}", product.tag_prefix()),
                "application/vnd.github+json",
                METADATA_LIMIT,
                deadline,
            )?
            .body,
        )?
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

/// Highest-precedence published release in the product's channel. Complete ref
/// discovery precedes selection; release lookups stop after the first published
/// precedence group, so the ordinary metadata path costs two requests.
pub(super) fn discover_latest(
    product: super::Product,
    channel: Channel,
    deadline: Instant,
    get: &mut impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<(Value, Version)> {
    let repository = format!("https://api.github.com/repos/{OFFICIAL_REPOSITORY}");
    let endpoint = format!(
        "{repository}/git/matching-refs/tags/{}",
        product.tag_prefix()
    );
    let prefix = format!("refs/tags/{}", product.tag_prefix());
    let mut url = format!("{endpoint}?per_page=100&page=1");
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut remaining_bytes = METADATA_LIMIT;
    for page in 1..=MAX_REF_PAGES {
        let response = get(
            &url,
            "application/vnd.github+json",
            remaining_bytes,
            deadline,
        )?;
        remaining_bytes = remaining_bytes
            .checked_sub(response.body.len())
            .ok_or_else(discovery_bound)?;
        let document = json(&response.body)?;
        let entries = document
            .as_array()
            .ok_or_else(|| invalid("Invalid release discovery response."))?;
        for entry in entries {
            let reference = entry["ref"]
                .as_str()
                .ok_or_else(|| invalid("Invalid release discovery reference."))?;
            let suffix = reference.strip_prefix(&prefix).ok_or_else(|| {
                invalid("Release discovery reference has the wrong product prefix.")
            })?;
            if !seen.insert(reference.to_owned()) {
                return Err(invalid("Duplicate release discovery reference."));
            }
            if let Ok(version) = suffix.parse::<Version>()
                && channel.accepts(&version)
            {
                candidates.push((format!("{}{suffix}", product.tag_prefix()), version));
            }
        }
        let Some(next) = next_page(response.link.as_deref())? else {
            break;
        };
        if page == MAX_REF_PAGES || remaining_bytes == 0 {
            return Err(discovery_bound());
        }
        // GitHub currently returns all matching refs without Link, including
        // arrays larger than 1,000. If it paginates, admit only the next page of
        // this same product lookup, not a header-selected host/path or a cycle.
        if next != format!("{endpoint}?per_page=100&page={}", page + 1)
            && next != format!("{endpoint}?page={}&per_page=100", page + 1)
        {
            return Err(invalid("Invalid release discovery pagination URL."));
        }
        url = next.to_owned();
    }
    candidates.sort_by(|(_, a), (_, b)| b.cmp_precedence(a));
    let mut published: Vec<(Value, Version)> = Vec::new();
    for (requests, (tag, expected)) in candidates.into_iter().enumerate() {
        if published
            .first()
            .is_some_and(|(_, version)| version.cmp_precedence(&expected).is_ne())
        {
            break;
        }
        if requests == MAX_RELEASE_LOOKUPS {
            return Err(discovery_bound());
        }
        let response = match get(
            &format!("{repository}/releases/tags/{tag}"),
            "application/vnd.github+json",
            METADATA_LIMIT,
            deadline,
        ) {
            Ok(response) => response,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let release = json(&response.body)?;
        if version(product, &release)? != expected {
            return Err(invalid(
                "Release version does not match its discovery reference.",
            ));
        }
        match release["draft"].as_bool() {
            Some(true) => continue,
            Some(false) => published.push((release, expected)),
            None => return Err(invalid("Invalid release discovery draft metadata.")),
        }
    }
    let versions = published
        .iter()
        .map(|(_, version)| version.clone())
        .collect::<Vec<_>>();
    latest_in_channel(&versions, channel).map_err(io::Error::other)?;
    published.into_iter().next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            ReleaseUnavailable { product, channel },
        )
    })
}

const MAX_REF_PAGES: usize = 10;
const MAX_RELEASE_LOOKUPS: usize = 32;

fn discovery_bound() -> io::Error {
    invalid("Release discovery exceeds its bound; select an exact version with --to.")
}

/// The optional Link header is untrusted evidence of incomplete discovery.
/// Malformed or ambiguous links must not become an apparent final page.
fn next_page(link: Option<&str>) -> io::Result<Option<&str>> {
    let Some(link) = link else { return Ok(None) };
    let mut next = None;
    for part in link.split(',') {
        let (url, parameters) = part
            .trim()
            .split_once('>')
            .ok_or_else(|| invalid("Invalid release discovery pagination header."))?;
        let url = url
            .strip_prefix('<')
            .filter(|url| !url.is_empty())
            .ok_or_else(|| invalid("Invalid release discovery pagination header."))?;
        if !parameters.trim_start().starts_with(';') {
            return Err(invalid("Invalid release discovery pagination header."));
        }
        let mut relation = None;
        for parameter in parameters.split(';').skip(1) {
            if let Some(value) = parameter.trim().strip_prefix("rel=") {
                if relation.is_some() {
                    return Err(invalid("Invalid release discovery pagination header."));
                }
                let value = if value.starts_with('"') {
                    value
                        .strip_prefix('"')
                        .and_then(|value| value.strip_suffix('"'))
                        .ok_or_else(|| invalid("Invalid release discovery pagination header."))?
                } else {
                    value
                };
                if value.is_empty()
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b' ')
                {
                    return Err(invalid("Invalid release discovery pagination header."));
                }
                relation = Some(value);
            }
        }
        let relation =
            relation.ok_or_else(|| invalid("Invalid release discovery pagination header."))?;
        if relation
            .split_ascii_whitespace()
            .any(|relation| relation.eq_ignore_ascii_case("next"))
            && next.replace(url).is_some()
        {
            return Err(invalid("Invalid release discovery pagination header."));
        }
    }
    Ok(next)
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
    get: &mut impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<Vec<u8>> {
    let bytes = get(
        &format!("{endpoint}/assets/{}", asset.id),
        "application/octet-stream",
        maximum,
        deadline,
    )?
    .body;
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
