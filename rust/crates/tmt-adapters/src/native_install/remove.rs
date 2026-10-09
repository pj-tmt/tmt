//! Deactivate extension links or remove managed product installations, including
//! verified former products. Application data is untouched.

use super::{Product, invalid, publication::Layout};
use std::{fs, io, path::Path};

/// Finalize a rename only after the new installation verifies. Historical
/// command names are never overwrite authority: foreign links stay untouched.
pub fn finish_product_replacement(prefix: &Path, product: Product) -> io::Result<ProductRemoval> {
    let Some(former) = product.former() else {
        return Ok(ProductRemoval::default());
    };
    match fs::symlink_metadata(prefix.join(former.namespace)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ProductRemoval::default());
        }
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let new = Layout::existing_product(prefix, product)?;
    let _new_lock = crate::file_lock::exclusive(&new.root.join("install.lock"))?;
    new.current()?
        .ok_or_else(|| invalid("New product has no verified activation."))?;
    new.check_links(true)?;
    former_product_removal(prefix, product, true)
}

/// Full uninstall uses the same verified former identity and link fence as
/// replacement, without requiring a successor installation to exist.
fn former_product_removal(
    prefix: &Path,
    product: Product,
    remove: bool,
) -> io::Result<ProductRemoval> {
    let Some(former) = product.former() else {
        return Ok(ProductRemoval::default());
    };
    match fs::symlink_metadata(prefix.join(former.namespace)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ProductRemoval::default());
        }
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let old = Layout::existing_former(prefix, product)?;
    let _old_lock = crate::file_lock::exclusive(&old.root.join("install.lock"))?;
    old.current()?
        .ok_or_else(|| invalid("Former product has no verified activation."))?;
    let mut report = ProductRemoval::default();
    for name in former.links {
        let link = old.prefix.join("bin").join(name);
        match fs::symlink_metadata(&link) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
            Ok(metadata)
                if metadata.is_symlink()
                    && fs::canonicalize(&link)
                        .is_ok_and(|target| target.starts_with(&old.root)) =>
            {
                if remove {
                    fs::remove_file(&link)?;
                }
                report.removed.push(link);
            }
            Ok(_) => report.kept.push(link),
        }
    }
    if remove {
        fs::File::open(old.prefix.join("bin"))?.sync_all()?;
        fs::remove_dir_all(&old.root)?;
        fs::File::open(old.root.parent().expect("lib parent"))?.sync_all()?;
    }
    report.removed.push(old.root);
    Ok(report)
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
    let mut plan = plan_current_product_removal(prefix, product)?;
    let former = former_product_removal(prefix, product, false)?;
    plan.removed.extend(former.removed);
    plan.kept.extend(former.kept);
    Ok(plan)
}

fn plan_current_product_removal(prefix: &Path, product: Product) -> io::Result<ProductRemoval> {
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
    // Validate a retained former installation before removing the current one.
    // Reverify under its lock again when that later step actually executes.
    former_product_removal(prefix, product, false)?;
    let mut plan = plan_current_product_removal(prefix, product)?;
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
    let former = former_product_removal(prefix, product, true)?;
    plan.removed.extend(former.removed);
    plan.kept.extend(former.kept);
    Ok(plan)
}
