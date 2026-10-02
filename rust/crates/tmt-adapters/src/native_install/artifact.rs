//! Cargo-dist metadata and bounded, allowlisted archive acquisition.
//!
//! Never use generic archive unpacking: archive paths are not filesystem targets.

use super::{Product, invalid, skills_tree};
use crate::bounded_file;
use flate2::read::MultiGzDecoder;
use semver::Version;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    path::Path,
};
pub(super) use tmt_core::content_digest::sha256 as digest;

pub(super) const COMPRESSED_LIMIT: usize = 64 * 1024 * 1024;
pub(super) const MANIFEST_LIMIT: usize = 4 * 1024 * 1024;
const EXPANDED_LIMIT: usize = 128 * 1024 * 1024;
#[cfg(test)]
pub(super) const FILES: [&str; 4] = Product::Cli.files();

#[derive(Debug)]
pub(super) struct Artifact {
    pub name: String,
    pub version: Version,
    pub target: String,
    pub sha256: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Artifact {
    pub fn file_hashes(&self) -> BTreeMap<String, String> {
        self.files
            .iter()
            .map(|(name, bytes)| (name.clone(), digest(bytes)))
            .collect()
    }
}

#[cfg(test)]
pub(super) fn acquire(manifest: &Path, archive: &Path, target: &str) -> io::Result<Artifact> {
    acquire_product(Product::Cli, manifest, archive, target)
}

pub(super) fn acquire_product(
    product: Product,
    manifest: &Path,
    archive: &Path,
    target: &str,
) -> io::Result<Artifact> {
    let bytes = bounded_file::read_no_follow(manifest, MANIFEST_LIMIT).map_err(io::Error::other)?;
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("Native archive requires an ASCII filename."))?;
    let listed = metadata_bytes(product, &bytes, name, target)?;
    let compressed =
        bounded_file::read_no_follow(archive, COMPRESSED_LIMIT).map_err(io::Error::other)?;
    verified_archive(product, name, target, listed, &compressed)
}

pub(super) fn acquire_bytes(
    product: Product,
    manifest: &[u8],
    name: &str,
    compressed: &[u8],
    target: &str,
) -> io::Result<Artifact> {
    let listed = metadata_bytes(product, manifest, name, target)?;
    verified_archive(product, name, target, listed, compressed)
}

fn metadata_bytes(product: Product, bytes: &[u8], name: &str, target: &str) -> io::Result<Listed> {
    if bytes.len() > MANIFEST_LIMIT {
        return Err(invalid("Native manifest exceeds its bound."));
    }
    let manifest: Value = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    metadata(product, &manifest, name, target)
}

/// What the manifest lists for one archive: its version, checksum and whether
/// it declares a skills tree. cargo-dist records an included directory as one
/// asset named after it, so the tree's files are inventoried from the
/// checksum-verified archive, never from the manifest.
struct Listed {
    version: Version,
    sha256: String,
    skills: bool,
    /// The optional companion executables this archive declares.
    companions: Vec<String>,
}

fn verified_archive(
    product: Product,
    name: &str,
    target: &str,
    listed: Listed,
    compressed: &[u8],
) -> io::Result<Artifact> {
    let Listed {
        version,
        sha256,
        skills,
        companions,
    } = listed;
    if compressed.len() > COMPRESSED_LIMIT {
        return Err(invalid("Native archive exceeds its bound."));
    }
    if digest(compressed) != sha256 {
        return Err(invalid("Native archive checksum mismatch."));
    }
    let files = decode(
        product,
        compressed,
        archive_root(name)?,
        skills,
        &companions,
    )?;
    Ok(Artifact {
        name: name.into(),
        version,
        target: target.into(),
        sha256,
        files,
    })
}

pub(super) fn select(
    product: Product,
    manifest: &[u8],
    target: &str,
) -> io::Result<(String, Version)> {
    if manifest.len() > MANIFEST_LIMIT {
        return Err(invalid("Native manifest exceeds its bound."));
    }
    let manifest: Value = serde_json::from_slice(manifest).map_err(io::Error::other)?;
    let releases = manifest["releases"]
        .as_array()
        .ok_or_else(|| invalid("Native manifest releases are missing."))?;
    let matches = manifest["artifacts"]
        .as_object()
        .ok_or_else(|| invalid("Native manifest artifacts are missing."))?
        .iter()
        .filter(|(name, value)| {
            value["kind"] == "executable-zip"
                && value["target_triples"] == serde_json::json!([target])
                && releases.iter().any(|release| {
                    release["app_name"] == product.package()
                        && release["artifacts"].as_array().is_some_and(|assets| {
                            assets
                                .iter()
                                .any(|asset| asset.as_str() == Some(name.as_str()))
                        })
                })
        })
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(invalid(
            "Native manifest must select exactly one target archive.",
        ));
    }
    let name = matches[0];
    let listed = metadata(product, &manifest, name, target)?;
    Ok((name.clone(), listed.version))
}

fn archive_root(name: &str) -> io::Result<&str> {
    name.strip_suffix(".tar.gz")
        .filter(|root| {
            root.as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
                && root
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
        .ok_or_else(|| invalid("Invalid native archive filename."))
}

fn metadata(product: Product, manifest: &Value, name: &str, target: &str) -> io::Result<Listed> {
    archive_root(name)?;
    let metadata = &manifest["artifacts"][name];
    if metadata["kind"] != "executable-zip"
        || metadata["name"] != name
        || metadata["target_triples"] != serde_json::json!([target])
    {
        return Err(invalid("Native archive metadata or target does not match."));
    }
    let sha256 = metadata["checksums"]["sha256"]
        .as_str()
        .filter(|hash| tmt_core::content_digest::is_sha256(hash))
        .ok_or_else(|| invalid("Native archive requires a SHA-256 checksum."))?;
    let mut inventory = metadata["assets"]
        .as_array()
        .ok_or_else(|| invalid("Native archive asset inventory is missing."))?
        .iter()
        .map(|asset| {
            asset["path"]
                .as_str()
                .ok_or_else(|| invalid("Invalid archive asset path."))
        })
        .collect::<io::Result<Vec<_>>>()?;
    inventory.sort_unstable();
    let declarations = inventory
        .iter()
        .filter(|path| **path == skills_tree::ROOT)
        .count();
    if declarations > 1 {
        return Err(invalid("Unexpected native archive asset inventory."));
    }
    let skills = declarations == 1;
    if skills && product == Product::Cli {
        return Err(invalid("The TMT CLI release carries no agent skills."));
    }
    let companions: Vec<String> = inventory
        .iter()
        .filter(|path| product.companions().contains(path))
        .map(|path| (*path).to_owned())
        .collect();
    if companions.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid("Unexpected native archive asset inventory."));
    }
    let mut required: Vec<&str> = inventory
        .into_iter()
        .filter(|path| *path != skills_tree::ROOT && !product.companions().contains(path))
        .collect();
    let mut expected = product.files();
    expected.sort_unstable();
    required.sort_unstable();
    if required != expected {
        return Err(invalid("Unexpected native archive asset inventory."));
    }
    let releases = manifest["releases"]
        .as_array()
        .ok_or_else(|| invalid("Native manifest releases are missing."))?
        .iter()
        .filter(|release| {
            release["artifacts"]
                .as_array()
                .is_some_and(|assets| assets.iter().any(|asset| asset == name))
        })
        .collect::<Vec<_>>();
    if releases.len() != 1 || releases[0]["app_name"] != product.package() {
        return Err(invalid(
            "Native archive must belong to exactly one TMT release.",
        ));
    }
    let version = releases[0]["app_version"]
        .as_str()
        .ok_or_else(|| invalid("Native release version is missing."))?
        .parse()
        .map_err(|_| invalid("Native release version is invalid."))?;
    Ok(Listed {
        version,
        sha256: sha256.into(),
        skills,
        companions,
    })
}

/// Only the product's required files, the companion executables the manifest
/// declares and, when the manifest declares it, one bounded skills tree. Every limit applies while decoding: the expansion
/// bound, each skill file's size before its bytes are read, and the file count
/// before the tree is validated as a whole.
fn decode(
    product: Product,
    compressed: &[u8],
    root: &str,
    skills: bool,
    companions: &[String],
) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let mut expanded = Vec::new();
    MultiGzDecoder::new(compressed)
        .take((EXPANDED_LIMIT + 1) as u64)
        .read_to_end(&mut expanded)?;
    if expanded.len() > EXPANDED_LIMIT {
        return Err(invalid("Native archive expansion exceeds its bound."));
    }
    let tree = format!("{root}/{}", skills_tree::ROOT);
    let mut archive = tar::Archive::new(expanded.as_slice());
    let mut files = BTreeMap::new();
    let mut directories = BTreeSet::new();
    let mut skill_files = 0;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path_bytes().into_owned();
        let path = std::str::from_utf8(&path)
            .map_err(|_| invalid("Native archive path must be ASCII."))?;
        let below_tree = path.strip_prefix(&tree);
        let tree_directory =
            skills && below_tree.is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
        // Only files strictly below the tree: `<root>/skills` itself is the
        // tree's directory, never a file.
        let in_tree =
            skills && below_tree.is_some_and(|rest| rest.len() > 1 && rest.starts_with('/'));
        // Archivers record directories with or without a trailing slash.
        if entry.header().entry_type().is_dir() && (path == format!("{root}/") || tree_directory) {
            if entry.header().mode()? & 0o7000 != 0 || entry.size() != 0 {
                return Err(invalid("Native archive directory metadata is invalid."));
            }
            if !directories.insert(path.trim_end_matches('/').to_owned()) {
                return Err(invalid("Duplicate native archive directory."));
            }
            continue;
        }
        let name = path
            .strip_prefix(root)
            .and_then(|p| p.strip_prefix('/'))
            .filter(|name| {
                product.files().contains(name)
                    || companions.iter().any(|companion| companion == name)
                    || in_tree
            })
            .ok_or_else(|| invalid("Unexpected native archive path."))?;
        let mode = entry.header().mode()?;
        // Regular files only: never a link, device or special permission.
        if !entry.header().entry_type().is_file()
            || mode & 0o7000 != 0
            || entry.size() == 0
            || ((name == product.executable() || product.companions().contains(&name))
                && mode & 0o111 == 0)
            || (in_tree && !skills_tree::file_fits(entry.size()))
        {
            return Err(invalid(
                "Native archive requires nonempty regular files with safe permissions.",
            ));
        }
        if in_tree {
            skill_files += 1;
            if skill_files > skills_tree::MAXIMUM_TREE_FILES {
                return Err(invalid("A release carries too many agent skill files."));
            }
        }
        if files.contains_key(name) {
            return Err(invalid("Duplicate native archive file."));
        }
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents)?;
        files.insert(name.into(), contents);
    }
    if product
        .files()
        .iter()
        .any(|file| !files.contains_key(*file))
        || companions.iter().any(|file| !files.contains_key(file))
    {
        return Err(invalid("Native archive is missing required files."));
    }
    let tree_files: Vec<&str> = files
        .keys()
        .map(String::as_str)
        .filter(|name| skills_tree::is_skill_path(name))
        .collect();
    if skills {
        if tree_files.is_empty() {
            return Err(invalid(
                "The release declares agent skills but carries none.",
            ));
        }
        skills_tree::validate(product, tree_files.iter().copied())?;
    }
    // A recorded directory must hold a kept file: the root, or an ancestor of
    // a validated skill file.
    for directory in &directories {
        let Some(relative) = directory.strip_prefix(&format!("{root}/")) else {
            continue;
        };
        let holds_a_file = tree_files.iter().any(|file| {
            file.strip_prefix(relative)
                .is_some_and(|rest| rest.starts_with('/'))
        });
        if !holds_a_file {
            return Err(invalid("Unexpected native archive directory."));
        }
    }
    Ok(files)
}
