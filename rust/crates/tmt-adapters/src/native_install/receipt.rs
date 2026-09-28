//! Installation evidence is rooted in a release directory, not application config.

use super::{
    Product,
    artifact::{Artifact, digest},
    invalid,
};
use crate::bounded_file;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io, os::unix::fs::PermissionsExt, path::Path};
use tmt_core::native_install::{Channel, InstalledVersion};
use uuid::Uuid;

#[derive(Debug)]
pub(super) struct Receipt {
    pub id: Uuid,
    pub state: InstalledVersion,
    pub archive_name: String,
    pub archive_sha256: String,
    pub target: String,
    pub file_hashes: BTreeMap<String, String>,
    pub provenance: Option<GitHubProvenance>,
}

#[derive(Debug, Clone)]
pub(super) struct GitHubProvenance {
    pub release_id: u64,
    pub manifest_sha256: String,
}

impl Receipt {
    pub fn new(artifact: &Artifact, state: InstalledVersion) -> Self {
        Self {
            id: Uuid::new_v4(),
            state,
            archive_name: artifact.name.clone(),
            archive_sha256: artifact.sha256.clone(),
            target: artifact.target.clone(),
            file_hashes: artifact.file_hashes(),
            provenance: None,
        }
    }

    pub fn encode(&self, prefix: &Path) -> io::Result<Vec<u8>> {
        let prefix = prefix
            .to_str()
            .ok_or_else(|| invalid("Installation prefix must be UTF-8."))?;
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1, "release_id": self.id.to_string(), "prefix": prefix,
            "version": self.state.version.to_string(), "channel": self.state.channel.as_str(),
            "pinned_version": self.state.pinned_version.as_ref().map(ToString::to_string),
            "archive": self.archive_name, "archive_sha256": self.archive_sha256,
            "target": self.target, "file_sha256": self.file_hashes,
            "source": self.provenance.as_ref().map_or_else(|| json!("local-archive"), |source| json!({
                "kind": "github-release", "repository": super::OFFICIAL_REPOSITORY,
                "release_id": source.release_id, "manifest_sha256": source.manifest_sha256,
            }))
        }))
        .map_err(io::Error::other)
    }

    #[cfg(test)]
    pub fn read(directory: &Path, prefix: &Path, id: Uuid) -> io::Result<Self> {
        Self::read_product(Product::Cli, directory, prefix, id)
    }

    pub fn read_product(
        product: Product,
        directory: &Path,
        prefix: &Path,
        id: Uuid,
    ) -> io::Result<Self> {
        let mut inventory = fs::read_dir(directory)?
            .take(product.files().len() + 2)
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        inventory.sort();
        let mut expected = product
            .files()
            .into_iter()
            .chain(["receipt.json"])
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>();
        expected.sort();
        if inventory != expected {
            return Err(invalid("Installed release inventory has changed."));
        }
        let bytes = bounded_file::read_no_follow(&directory.join("receipt.json"), 16 * 1024)
            .map_err(io::Error::other)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let text = |key: &str| {
            value[key]
                .as_str()
                .ok_or_else(|| invalid("Invalid native installation receipt."))
        };
        if value["schema_version"] != 1
            || text("release_id")? != id.to_string()
            || Some(text("prefix")?) != prefix.to_str()
        {
            return Err(invalid(
                "Native receipt ownership does not match its installation.",
            ));
        }
        if value.get("pinned_version").is_none()
            || !tmt_core::content_digest::is_sha256(text("archive_sha256")?)
        {
            return Err(invalid("Native receipt digest or pin metadata is invalid."));
        }
        let state = InstalledVersion {
            version: text("version")?
                .parse()
                .map_err(|_| invalid("Invalid installed version."))?,
            channel: Channel::parse(text("channel")?)
                .ok_or_else(|| invalid("Invalid installed channel."))?,
            pinned_version: match &value["pinned_version"] {
                Value::Null => None,
                Value::String(version) => Some(
                    version
                        .parse()
                        .map_err(|_| invalid("Invalid installed pin."))?,
                ),
                _ => return Err(invalid("Invalid installed pin.")),
            },
        };
        state.validate().map_err(io::Error::other)?;
        let provenance = if value["source"] == "local-archive" {
            None
        } else {
            let source = &value["source"];
            if source.as_object().is_none_or(|fields| fields.len() != 4)
                || source["kind"] != "github-release"
                || source["repository"] != super::OFFICIAL_REPOSITORY
            {
                return Err(invalid("Invalid native release provenance."));
            }
            let release_id = source["release_id"]
                .as_u64()
                .filter(|id| *id > 0)
                .ok_or_else(|| invalid("Invalid native release provenance."))?;
            let manifest_sha256 = source["manifest_sha256"]
                .as_str()
                .filter(|hash| tmt_core::content_digest::is_sha256(hash))
                .ok_or_else(|| invalid("Invalid native release provenance."))?
                .into();
            Some(GitHubProvenance {
                release_id,
                manifest_sha256,
            })
        };
        let hashes = value["file_sha256"]
            .as_object()
            .ok_or_else(|| invalid("Missing installed file digests."))?;
        if hashes.len() != product.files().len() {
            return Err(invalid("Unexpected installed file digest inventory."));
        }
        let mut file_hashes = BTreeMap::new();
        for name in product.files() {
            let metadata = fs::symlink_metadata(directory.join(name))?;
            let mode = metadata.permissions().mode();
            if !metadata.file_type().is_file()
                || mode & 0o7000 != 0
                || (name == product.executable() && mode & 0o111 == 0)
            {
                return Err(invalid(
                    "Installed release file type or permissions have changed.",
                ));
            }
            let expected = hashes
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Missing installed file digest."))?;
            let bytes = bounded_file::read_no_follow(&directory.join(name), 128 * 1024 * 1024)
                .map_err(io::Error::other)?;
            if digest(&bytes) != expected {
                return Err(invalid(
                    "Installed release file has changed; refusing replacement.",
                ));
            }
            file_hashes.insert(name.into(), expected.into());
        }
        Ok(Self {
            id,
            state,
            archive_name: text("archive")?.into(),
            archive_sha256: text("archive_sha256")?.into(),
            target: text("target")?.into(),
            file_hashes,
            provenance,
        })
    }
}
