//! Deactivate only verified extension links; retained releases, skills and
//! application data are untouched.

use super::{Product, invalid, publication::Layout};
use std::{fs, io, path::Path};

pub fn uninstall_office(prefix: &Path) -> io::Result<bool> {
    uninstall_extension(prefix, Product::Office)
}

/// Remove every command link of an installed extension after validating that
/// each still points into its managed release, then its `current` pointer.
/// A foreign same-named command is refused, never removed. The CLI is not an
/// extension and cannot be uninstalled here.
pub fn uninstall_extension(prefix: &Path, product: Product) -> io::Result<bool> {
    if product == Product::Cli {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The TMT CLI is not an extension.",
        ));
    }
    let root = prefix.join(product.namespace());
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            for name in product.links() {
                match fs::symlink_metadata(prefix.join("bin").join(name)) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                    Ok(_) => {
                        return Err(invalid(&format!(
                            "Unmanaged {name} command; refusing removal."
                        )));
                    }
                }
            }
            return Ok(false);
        }
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let layout = Layout::existing_product(prefix, product)?;
    let _lock = crate::file_lock::exclusive(&layout.root.join("install.lock"))?;
    let current = layout.current()?;
    layout.check_links(current.is_some())?;
    if current.is_none() {
        return Ok(false);
    }
    for name in product.links() {
        match fs::remove_file(layout.prefix.join("bin").join(name)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
    }
    fs::File::open(layout.prefix.join("bin"))?.sync_all()?;
    fs::remove_file(layout.root.join("current"))?;
    fs::File::open(&layout.root)?.sync_all()?;
    Ok(true)
}
