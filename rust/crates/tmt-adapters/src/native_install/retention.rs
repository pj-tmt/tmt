//! Best-effort retirement of positively verified, superseded generations.
use super::{Product, invalid, publication::Layout, receipt::Receipt};
use semver::Version;
use std::{collections::BTreeSet, fs, io, os::unix::fs::PermissionsExt, path::Path};
use uuid::Uuid;

fn generation(path: &Path) -> Option<Uuid> {
    let name = path.file_name()?.to_str()?;
    let id = Uuid::parse_str(name).ok()?;
    (id.to_string() == name).then_some(id)
}

/// The publisher's caller holds install.lock; repair deliberately never calls this.
pub(super) fn prune(layout: &Layout, active: Uuid, previous: Uuid) -> io::Result<()> {
    if layout.current()?.map(|receipt| receipt.id) != Some(active) {
        return Err(invalid("Native current release changed before retention."));
    }
    let releases = layout.root.join("releases");
    let inventory = fs::symlink_metadata(&releases)?;
    if !inventory.is_dir() {
        return Err(invalid("Release inventory is not a directory."));
    }
    // Do not override an owner's withdrawn inventory permission, even as root.
    if inventory.permissions().mode() & 0o400 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Release inventory is not readable.",
        ));
    }
    for entry in fs::read_dir(&releases)? {
        let entry = entry?;
        let path = entry.path();
        let Some(id) = generation(&path) else {
            continue;
        };
        if id == active || id == previous || !fs::symlink_metadata(&path)?.is_dir() {
            continue;
        }
        let Ok(_) = Receipt::read_product(layout.product, &path, &layout.prefix, id) else {
            // Damage, foreign inventory and historical formats are not deletion authority.
            continue;
        };
        fs::remove_dir_all(path)?;
    }
    fs::File::open(releases)?.sync_all()
}

/// Provider assets for retained CLI releases remain available.
/// An unmanaged/stale executable or uncertain retained receipt grants no cleanup authority.
pub(crate) fn retained_cli_versions(executable: &Path) -> io::Result<BTreeSet<Version>> {
    super::with_active_product(Product::Cli, executable, |installation| {
        let layout = Layout::existing_product(&installation.prefix, Product::Cli)?;
        let mut versions = BTreeSet::new();
        for entry in fs::read_dir(layout.root.join("releases"))? {
            let path = entry?.path();
            let Some(id) = generation(&path) else {
                continue;
            };
            if !fs::symlink_metadata(&path)?.is_dir() {
                return Err(invalid("Retained CLI release is not a directory."));
            }
            let receipt = Receipt::read_product(Product::Cli, &path, &layout.prefix, id)?;
            versions.insert(receipt.state.version);
        }
        Ok(versions)
    })?
}
