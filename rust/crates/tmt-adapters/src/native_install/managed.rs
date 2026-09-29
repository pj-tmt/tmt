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
}

impl ManagedInstallation {
    pub fn release_id(&self) -> Uuid {
        self.id
    }
}

pub fn inspect(executable: &Path) -> io::Result<ManagedInstallation> {
    inspect_product(Product::Cli, executable)
}

pub fn inspect_product(product: Product, executable: &Path) -> io::Result<ManagedInstallation> {
    let executable = fs::canonicalize(executable)?;
    let prefix = executable.ancestors().nth(5).ok_or_else(unmanaged)?;
    let layout = Layout::existing_product(prefix, product).map_err(|_| unmanaged())?;
    let current = layout.current()?.ok_or_else(unmanaged)?;
    let active = layout
        .root
        .join("releases")
        .join(current.id.to_string())
        .join(product.executable());
    if executable != active {
        return Err(invalid(
            "This executable is not the active managed release. Run the current native installation, or update using its original package manager.",
        ));
    }
    layout.check_links(true)?;
    Ok(ManagedInstallation {
        executable: layout.prefix.join("bin").join(product.executable()),
        active_executable: active,
        state: current.state,
        target: current.target,
        prefix: layout.prefix,
        id: current.id,
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

fn unmanaged() -> io::Error {
    invalid(
        "This executable is not a managed native installation. Use its original package manager (for example brew upgrade tmux-team), or install the official native release into a separate prefix. Do not overwrite npm/pnpm/brew files.",
    )
}
