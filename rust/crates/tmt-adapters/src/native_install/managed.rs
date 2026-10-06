//! Read-only ownership of the executing binary, independent of PATH/app state.

use super::{Product, invalid, publication::Layout};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tmt_core::native_install::InstalledVersion;
use uuid::Uuid;

#[derive(Debug)]
pub struct ManagedInstallation {
    pub executable: PathBuf,
    pub active_executable: PathBuf,
    pub state: InstalledVersion,
    pub target: String,
    pub(super) prefix: PathBuf,
    pub(super) id: Uuid,
    pub(super) provenance: Option<super::receipt::Provenance>,
}

impl ManagedInstallation {
    pub fn release_id(&self) -> Uuid {
        self.id
    }
}

impl ManagedInstallation {
    /// The installation prefix this release belongs to.
    pub fn prefix(&self) -> &Path {
        &self.prefix
    }
}

pub fn inspect(executable: &Path) -> io::Result<ManagedInstallation> {
    inspect_product(Product::Cli, executable)
}

pub fn inspect_product(product: Product, executable: &Path) -> io::Result<ManagedInstallation> {
    let executable = fs::canonicalize(executable)?;
    let prefix = executable.ancestors().nth(5).ok_or_else(unmanaged)?;
    let layout = Layout::existing_product(prefix, product).map_err(|_| unmanaged())?;
    let installation = inspect_layout(layout)?;
    if executable != installation.active_executable {
        return Err(invalid(
            "This executable is not the active managed release. Run the current native installation, or update using its original package manager.",
        ));
    }
    Ok(installation)
}

/// Strict prefix-based inspection, including a missing or damaged payload.
/// Does not grant update authority to an arbitrary executing binary.
pub fn inspect_product_prefix(product: Product, prefix: &Path) -> io::Result<ManagedInstallation> {
    let layout = Layout::existing_product(prefix, product)?;
    inspect_layout(layout)
}

fn inspect_layout(layout: Layout) -> io::Result<ManagedInstallation> {
    let product = layout.product;
    let current = layout.current()?.ok_or_else(unmanaged)?;
    layout.check_links(true)?;
    Ok(ManagedInstallation {
        executable: layout.prefix.join("bin").join(product.executable()),
        active_executable: layout
            .root
            .join("releases")
            .join(current.id.to_string())
            .join(product.executable()),
        state: current.state,
        target: current.target,
        prefix: layout.prefix,
        id: current.id,
        provenance: current.provenance,
    })
}

/// Keep the selected release current while its bounded skill-refresh child
/// runs. Lock order is installation then skills; acquisition never holds either.
pub fn with_active_release<T>(executable: &Path, operation: impl FnOnce() -> T) -> io::Result<T> {
    with_active_product(Product::Cli, executable, |_| operation())
}

/// Run a bounded operation while the verified product remains the active release.
pub fn with_active_product<T>(
    product: Product,
    executable: &Path,
    operation: impl FnOnce(&ManagedInstallation) -> T,
) -> io::Result<T> {
    let observed = inspect_product(product, executable)?;
    let layout = Layout::existing_product(&observed.prefix, product)?;
    let _lock = crate::file_lock::exclusive(&layout.root.join("install.lock"))?;
    if layout.current()?.map(|receipt| receipt.id) != Some(observed.id) {
        return Err(invalid(
            "The active release changed before the managed operation. Retry from the current native executable.",
        ));
    }
    layout.check_links(true)?;
    Ok(operation(&observed))
}

/// A companion executable of the running release, and the SHA-256 its
/// receipt records for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Companion {
    pub path: PathBuf,
    pub sha256: String,
}

/// The companion `name` that the active CLI release carries, when
/// `executable` is that release's own `tmt`. Cheap enough for every command:
/// it reads the receipt, not the release's files; whoever runs the companion
/// checks its bytes against `sha256`. `None` when the release carries no such
/// companion; an error when `executable` is not the active managed release.
pub fn active_companion(executable: &Path, name: &str) -> io::Result<Option<Companion>> {
    if !Product::Cli.companions().contains(&name) {
        return Ok(None);
    }
    let executable = fs::canonicalize(executable)?;
    let prefix = executable.ancestors().nth(5).ok_or_else(unmanaged)?;
    let layout = Layout::existing_product(prefix, Product::Cli).map_err(|_| unmanaged())?;
    let (release, id) = layout.current_directory()?.ok_or_else(unmanaged)?;
    if executable != release.join(Product::Cli.executable()) {
        return Err(invalid(
            "This executable is not the active managed release. Run the current native installation, or update using its original package manager.",
        ));
    }
    let receipt =
        super::receipt::Receipt::read_metadata(Product::Cli, &release, &layout.prefix, id)?;
    Ok(receipt.file_hashes.get(name).map(|sha256| Companion {
        path: release.join(name),
        sha256: sha256.clone(),
    }))
}

fn unmanaged() -> io::Error {
    invalid(
        "This executable is not a managed native installation. Use its original package manager (for example brew upgrade tmux-team), or install the official native release into a separate prefix. Do not overwrite npm/pnpm/brew files.",
    )
}

/// The agent skills the active release carries, read under the installation
/// lock and checked byte for byte against its receipt, so what is published
/// is exactly what was verified. A damaged tree yields an error, not a subset.
pub fn release_skills(
    product: Product,
    executable: &Path,
) -> io::Result<Vec<crate::skill_installation::OwnedSkill>> {
    with_release_receipt(product, executable, |directory, receipt| {
        let mut skills = std::collections::BTreeMap::<String, Vec<(String, Vec<u8>)>>::new();
        for (path, digest) in &receipt.file_hashes {
            let Some((name, file)) = release_skill_file(path) else {
                continue;
            };
            let bytes = crate::bounded_file::read_no_follow(
                &directory.join(path),
                crate::skill_installation::MAXIMUM_FILE_BYTES,
            )
            .map_err(io::Error::other)?;
            if &super::artifact::digest(&bytes) != digest {
                return Err(invalid(
                    "An installed agent skill file has changed; TMT will not use or replace this release.",
                ));
            }
            skills
                .entry(name.to_owned())
                .or_default()
                .push((file.to_owned(), bytes));
        }
        Ok(skills
            .into_iter()
            .map(|(name, files)| crate::skill_installation::OwnedSkill { name, files })
            .collect())
    })
}

/// The names of the agent skills the active release's receipt records. The
/// receipt read itself verifies the tree; the files are not loaded again.
pub fn release_skill_names(product: Product, executable: &Path) -> io::Result<Vec<String>> {
    with_release_receipt(product, executable, |_, receipt| {
        let names: std::collections::BTreeSet<&str> = receipt
            .file_hashes
            .keys()
            .filter_map(|path| release_skill_file(path).map(|(name, _)| name))
            .collect();
        Ok(names.into_iter().map(str::to_owned).collect())
    })
}

/// Splits a receipt path under `skills/` into the skill name and its file.
fn release_skill_file(path: &str) -> Option<(&str, &str)> {
    path.strip_prefix("skills/")
        .and_then(|rest| rest.split_once('/'))
}

fn with_release_receipt<T>(
    product: Product,
    executable: &Path,
    read: impl FnOnce(&Path, &super::receipt::Receipt) -> io::Result<T>,
) -> io::Result<T> {
    with_active_product(product, executable, |installation| {
        let directory = installation
            .active_executable
            .parent()
            .ok_or_else(unmanaged)?;
        let receipt = super::receipt::Receipt::read_product(
            product,
            directory,
            &installation.prefix,
            installation.id,
        )?;
        read(directory, &receipt)
    })?
}
