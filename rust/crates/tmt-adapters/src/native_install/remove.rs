//! Deactivate only verified extension links; retained releases, skills and
//! application data are untouched.

use super::{Product, invalid, publication::Layout};
use std::{fs, io, path::Path};

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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProductRemoval {
    /// Command links and the product's directory that were removed.
    pub removed: Vec<std::path::PathBuf>,
    /// Same-named commands that are not TMT's links, left in place.
    pub kept: Vec<std::path::PathBuf>,
}

/// Read-only: the links and directory [`remove_product`] would remove, and the
/// foreign commands it would keep.
pub fn plan_product_removal(prefix: &Path, product: Product) -> io::Result<ProductRemoval> {
    let mut plan = ProductRemoval::default();
    for name in product.links() {
        let link = prefix.join("bin").join(name);
        match fs::symlink_metadata(&link) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    && fs::read_link(&link)? == Path::new(&product.link_target()) =>
            {
                plan.removed.push(link)
            }
            Ok(_) => plan.kept.push(link),
        }
    }
    let root = prefix.join(product.namespace());
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
        Ok(metadata) if metadata.is_dir() => plan.removed.push(root),
        Ok(_) => {
            return Err(invalid(&format!(
                "{} is not a TMT directory.",
                root.display()
            )));
        }
    }
    Ok(plan)
}

/// A full uninstall of one product, the CLI included: its own command links,
/// then its whole directory (releases, receipts, `current`). A same-named
/// command that does not link into the product is kept.
pub fn remove_product(prefix: &Path, product: Product) -> io::Result<ProductRemoval> {
    let plan = plan_product_removal(prefix, product)?;
    let root = prefix.join(product.namespace());
    if plan.removed.contains(&root) {
        // Validates the layout: real directories, not symlink aliases.
        let layout = Layout::existing_product(prefix, product)?;
        let _lock = crate::file_lock::exclusive(&layout.root.join("install.lock"))?;
        for link in plan.removed.iter().filter(|path| **path != root) {
            match fs::remove_file(link) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                result => result?,
            }
        }
        fs::remove_dir_all(&layout.root)?;
    } else {
        for link in &plan.removed {
            fs::remove_file(link)?;
        }
    }
    Ok(plan)
}
