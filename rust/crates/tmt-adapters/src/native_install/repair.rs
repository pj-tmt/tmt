//! Explicit exact-artifact recovery; damaged release contents are never replaced.

use super::{
    ActivatedInstallation, InstallReport, Product, ReleaseVerifier, artifact, installed_report,
    invalid, publication::Layout, receipt::Receipt, release,
};
use crate::bounded_file;
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct RepairRequired {
    pub product: Product,
    pub prefix: PathBuf,
    pub version: String,
    pub requires_archive: bool,
    cause: io::Error,
}
impl std::fmt::Display for RepairRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.cause.fmt(f)
    }
}
impl std::error::Error for RepairRequired {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

#[derive(Debug)]
pub struct RepairReport {
    pub installation: InstallReport,
    /// The untouched damaged release, retained at its original path.
    pub retained_release: Option<PathBuf>,
}

pub(super) fn required(
    product: Product,
    directory: &Path,
    prefix: &Path,
    receipt: &Receipt,
    cause: io::Error,
) -> io::Error {
    if product == Product::Cli
        || !matches!(
            cause.kind(),
            io::ErrorKind::InvalidData | io::ErrorKind::NotFound
        )
    {
        return cause;
    }
    let admission = (|| {
        let layout = Layout::existing_product(prefix, product)?;
        layout.check_links(true)?;
        safe_layout(&layout, directory)?;
        Ok::<_, io::Error>(())
    })();
    match admission {
        Ok(()) => io::Error::new(
            io::ErrorKind::InvalidData,
            RepairRequired {
                product,
                prefix: prefix.into(),
                version: receipt.state.version.to_string(),
                requires_archive: receipt.provenance.is_none(),
                cause,
            },
        ),
        Err(_) => cause,
    }
}

fn safe_entry(path: &Path, control: bool) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if (!metadata.is_dir() && !metadata.is_file())
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o7000 != 0
        || (control && metadata.mode() & 0o022 != 0)
    {
        return Err(invalid(
            "Repair refused: an installation entry has unsafe ownership, type or permissions.",
        ));
    }
    Ok(metadata)
}

fn safe_layout(layout: &Layout, directory: &Path) -> io::Result<()> {
    for path in [
        layout.prefix.clone(),
        layout.prefix.join("lib"),
        layout.prefix.join("bin"),
        layout.root.clone(),
        layout.root.join("releases"),
        directory.into(),
        directory.join("receipt.json"),
    ] {
        safe_entry(&path, true)?;
    }
    let mut pending = vec![directory.to_path_buf()];
    let mut count = 0;
    // Bound inspection independently of foreign content; nothing in the old tree is read or deleted.
    const MAXIMUM_ENTRIES: usize = 4096;
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            count += 1;
            if count > MAXIMUM_ENTRIES {
                return Err(invalid(
                    "Repair refused: release inventory exceeds the inspection bound.",
                ));
            }
            if safe_entry(&entry.path(), false)?.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

struct Observation {
    directory: PathBuf,
    receipt: Receipt,
    bytes: Vec<u8>,
    damaged: bool,
}
fn observe(layout: &Layout) -> io::Result<Observation> {
    let (directory, id) = layout
        .current_directory()?
        .ok_or_else(|| invalid("Repair requires an existing managed extension release."))?;
    safe_layout(layout, &directory)?;
    layout.check_links(true)?;
    let bytes = bounded_file::read_no_follow(
        &directory.join("receipt.json"),
        super::skills_tree::receipt_limit(layout.product),
    )
    .map_err(io::Error::other)?;
    let receipt = Receipt::parse_metadata(layout.product, &bytes, &layout.prefix, id)?;
    let damaged = match receipt.verify(layout.product, &directory) {
        Ok(()) => false,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::NotFound
            ) =>
        {
            true
        }
        Err(error) => return Err(error),
    };
    Ok(Observation {
        directory,
        receipt,
        bytes,
        damaged,
    })
}
fn revalidate(layout: &Layout, observed: &Observation) -> io::Result<()> {
    let current = observe(layout)?;
    if current.directory != observed.directory || current.bytes != observed.bytes {
        return Err(invalid(
            "The installed release or receipt changed while downloading; repair was refused.",
        ));
    }
    Ok(())
}

pub fn repair_product(
    product: Product,
    prefix: &Path,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<RepairReport> {
    let client = crate::release_http::Https::new();
    repair_with(
        product,
        prefix,
        verifier,
        checkpoint,
        |url, accept, limit, deadline| client.get(url, accept, limit, deadline),
    )
}
fn repair_with(
    product: Product,
    prefix: &Path,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
    get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> io::Result<RepairReport> {
    repair_using(product, prefix, verifier, checkpoint, |old, prefix| {
        let Some(provenance) = &old.provenance else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                RepairRequired {
                    product,
                    prefix: prefix.into(),
                    version: old.state.version.to_string(),
                    requires_archive: true,
                    cause: invalid(
                        "Local-archive repair requires the original archive and matching manifest.",
                    ),
                },
            ));
        };
        let downloaded = release::download_product(
            product,
            old.state.channel,
            Some(&old.state.version),
            &old.target,
            Instant::now() + Duration::from_secs(60),
            get,
        )?;
        if downloaded.provenance.release_id != provenance.release_id
            || downloaded.provenance.manifest_sha256 != provenance.manifest_sha256
        {
            return Err(invalid(
                "The original release provenance does not match its receipt; repair was refused.",
            ));
        }
        let artifact = artifact::acquire_bytes(
            product,
            &downloaded.manifest,
            &downloaded.archive_name,
            &downloaded.archive,
            &old.target,
        )?;
        Ok((artifact, Some(downloaded.provenance)))
    })
}

/// Local archives restore only local-archive receipts; they never override GitHub provenance.
pub fn repair_product_from_archive(
    product: Product,
    prefix: &Path,
    archive: &Path,
    manifest: &Path,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<RepairReport> {
    repair_using(product, prefix, verifier, checkpoint, |old, _| {
        if old.provenance.is_some() {
            return Err(invalid(
                "GitHub-provenance repair must acquire the original GitHub release, not a local archive.",
            ));
        }
        let artifact = artifact::acquire_product(product, manifest, archive, &old.target)?;
        Ok((artifact, None))
    })
}

fn repair_using(
    product: Product,
    prefix: &Path,
    verifier: Option<ReleaseVerifier<'_>>,
    mut checkpoint: impl FnMut() -> io::Result<()>,
    acquire: impl FnOnce(
        &Receipt,
        &Path,
    )
        -> io::Result<(artifact::Artifact, Option<super::receipt::GitHubProvenance>)>,
) -> io::Result<RepairReport> {
    if product == Product::Cli {
        return Err(invalid("Repair is only supported for official extensions."));
    }
    if product.requires_release_verifier() && verifier.is_none() {
        return Err(invalid(
            "This product's releases require a verifier; refusing publication.",
        ));
    }
    checkpoint()?;
    let layout = Layout::existing_product(prefix, product)?;
    let observed = {
        let _lock = crate::file_lock::exclusive(&layout.root.join("install.lock"))?;
        observe(&layout)?
    };
    if !observed.damaged {
        return Ok(RepairReport {
            installation: installed_report(
                &layout,
                &observed.receipt.state.version.to_string(),
                observed.receipt.id,
                false,
            ),
            retained_release: None,
        });
    }
    let old = &observed.receipt;
    let (artifact, provenance) = acquire(old, &layout.prefix)?;
    if artifact.version != old.state.version
        || artifact.target != old.target
        || artifact.name != old.archive_name
        || artifact.sha256 != old.archive_sha256
        || artifact.file_hashes() != old.file_hashes
    {
        return Err(invalid(
            "The original release artifact does not match its receipt; repair was refused.",
        ));
    }
    checkpoint()?;
    let _lock = crate::file_lock::exclusive(&layout.root.join("install.lock"))?;
    let mut next = Receipt::new(&artifact, old.state.clone());
    next.provenance = provenance;
    let report = installed_report(&layout, &artifact.version.to_string(), next.id, true);
    layout
        .publish_repair(&artifact, &next, verifier, &mut checkpoint, || {
            revalidate(&layout, &observed)
        })
        .map_err(|cause| {
            if cause
                .get_ref()
                .is_some_and(|error| error.is::<super::publication::ActivatedError>())
            {
                io::Error::new(
                    cause.kind(),
                    ActivatedInstallation {
                        report: report.clone(),
                        cause,
                    },
                )
            } else {
                cause
            }
        })?;
    Ok(RepairReport {
        installation: report,
        retained_release: Some(observed.directory),
    })
}

#[cfg(test)]
mod tests;
