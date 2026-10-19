//! Retiring the former CLI prefix (`lib/tmux-team`) once the installation runs
//! from `lib/tmt`. The former prefix is the rollback source while the move is
//! new; it goes away on the next upgrade that finds nothing pointing into it.

use super::{
    Product, invalid,
    managed::release_prefix,
    publication::{Layout, require_installation_entries},
};
use crate::config::normalize;
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

const SETTINGS_LIMIT: u64 = 4 * 1024 * 1024;

/// Provider settings files where TMT may have written an absolute path. Unit
/// tests never read the host's provider settings; `blockers` is tested directly.
#[cfg(test)]
fn hook_settings() -> Vec<PathBuf> {
    Vec::new()
}

/// Provider settings files where TMT may have written an absolute path.
#[cfg(not(test))]
fn hook_settings() -> Vec<PathBuf> {
    use crate::{drivers::Registry, setup::provider_settings};
    Registry::builtin()
        .with_hooks()
        .filter_map(|driver| provider_settings(driver).ok())
        .collect()
}

/// What still points into `former_root`, as plain descriptions: command links
/// in `bin/` and provider settings that name the former prefix. Receipts hold
/// only the installation prefix, and skills are published as copies, so
/// neither can point there.
pub(super) fn blockers(
    prefix: &Path,
    former_root: &Path,
    settings: &[PathBuf],
) -> io::Result<Vec<String>> {
    let mut found = Vec::new();
    let bin = prefix.join("bin");
    for entry in fs::read_dir(&bin)? {
        let path = entry?.path();
        if !fs::symlink_metadata(&path)?.file_type().is_symlink() {
            continue;
        }
        let target = fs::read_link(&path)?;
        if normalize(&bin.join(target)).starts_with(former_root) {
            found.push(format!("the command link {}", path.display()));
        }
    }
    let needle = former_root.to_string_lossy().into_owned();
    for path in settings {
        let mut text = Vec::new();
        match fs::File::open(path) {
            Ok(file) => {
                file.take(SETTINGS_LIMIT).read_to_end(&mut text)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
        if String::from_utf8_lossy(&text).contains(&needle) {
            found.push(format!("the hook settings {}", path.display()));
        }
    }
    Ok(found)
}

/// Remove the former prefix when the new layout holds a verified active
/// release and nothing managed points into the old one; otherwise keep it and
/// say why in the error. The caller holds the new installation's lock, so the
/// former lock is taken after it, as everywhere else.
pub(super) fn retire(new: &Layout) -> io::Result<()> {
    new.current()?
        .ok_or_else(|| invalid("New installation has no verified activation."))?;
    let former = Layout::existing_former(&new.prefix, new.product)?;
    let _lock = crate::file_lock::exclusive(&former.root.join("install.lock"))?;
    require_installation_entries(&former.root)?;
    let found = blockers(&new.prefix, &former.root, &hook_settings())?;
    if !found.is_empty() {
        return Err(invalid(&format!(
            "{} still points into the former installation.",
            found.join(" and ")
        )));
    }
    fs::remove_dir_all(&former.root)?;
    fs::File::open(new.prefix.join("lib"))?.sync_all()
}

/// Whether the product's former installation is the same product under an
/// older prefix (the CLI).
pub(super) fn is_move(product: Product) -> bool {
    product
        .former()
        .is_some_and(|former| former.executable == product.executable())
}

/// Whether `executable` is a release of the moved CLI prefix.
pub fn in_moved_prefix(executable: &Path) -> bool {
    executable
        .ancestors()
        .nth(3)
        .is_some_and(|root| root.ends_with(Product::Cli.namespace()))
}

/// After an upgrade that ran from the moved prefix, why the former prefix is
/// still there. `None` when it is gone, or when the upgrade is the one that
/// moved the installation, which keeps it on purpose as the rollback source.
pub fn retained_former_hint(active: &Path) -> Option<String> {
    let former = Product::Cli.former()?;
    let prefix = release_prefix(active)?;
    let root = prefix.join(former.namespace);
    fs::symlink_metadata(&root).ok()?;
    Some(match blockers(prefix, &root, &hook_settings()) {
        Ok(found) if !found.is_empty() => format!(
            "The previous installation at {} is kept because {} still points into it. The next upgrade removes it once nothing does.",
            root.display(),
            found.join(" and ")
        ),
        _ => format!(
            "The previous installation at {} could not be removed. Remove it by hand when you no longer need to roll back.",
            root.display()
        ),
    })
}
