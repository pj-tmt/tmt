//! One durable release directory and one atomic activation pointer.

use super::{Product, ReleaseVerifier, artifact::Artifact, invalid, receipt::Receipt};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug)]
pub(super) struct ActivatedError(io::Error);

impl std::fmt::Display for ActivatedError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            output,
            "Release activated, but installation finalization failed; retry to repair command links: {}",
            self.0
        )
    }
}

impl std::error::Error for ActivatedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

pub(super) struct Layout {
    pub product: Product,
    pub prefix: PathBuf,
    pub root: PathBuf,
}

struct ActivationCheck<F> {
    has_current: bool,
    validate: F,
}

fn directory(path: &Path, create: bool) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(invalid(
            "Native installation directory is occupied by another entry.",
        )),
        Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(path)?;
            if let Some(parent) = path.parent() {
                File::open(parent)?.sync_all()?;
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

impl Layout {
    #[cfg(test)]
    pub fn existing(prefix: &Path) -> io::Result<Self> {
        Self::existing_product(prefix, Product::Cli)
    }

    pub fn existing_product(prefix: &Path, product: Product) -> io::Result<Self> {
        directory(prefix, false)?;
        let prefix = fs::canonicalize(prefix)?;
        let root = prefix.join(product.namespace());
        for path in [
            prefix.join("lib"),
            prefix.join("bin"),
            root.clone(),
            root.join("releases"),
        ] {
            directory(&path, false)?;
        }
        Ok(Self {
            prefix,
            root,
            product,
        })
    }

    #[cfg(test)]
    pub fn open(prefix: &Path) -> io::Result<Self> {
        Self::open_product(prefix, Product::Cli)
    }

    pub fn open_product(prefix: &Path, product: Product) -> io::Result<Self> {
        if prefix.as_os_str().is_empty() {
            return Err(invalid("Installation prefix must not be empty."));
        }
        // Preserve the leaf entry: only ancestors may be symlink aliases.
        // Components also remove a trailing separator that could make lstat
        // follow a symlink supplied as the installation prefix.
        let requested: PathBuf = std::env::current_dir()?.join(prefix).components().collect();
        if let Some(parent) = requested.parent() {
            create_prefix_ancestors(parent)?;
        }
        directory(&requested, true)?;
        let prefix = fs::canonicalize(&requested)?;
        directory(&prefix.join("lib"), true)?;
        directory(&prefix.join("bin"), true)?;
        let root = prefix.join(product.namespace());
        directory(&root, true)?;
        directory(&root.join("releases"), true)?;
        Ok(Self {
            prefix,
            root,
            product,
        })
    }

    pub fn current(&self) -> io::Result<Option<Receipt>> {
        let Some((release, id)) = self.current_directory()? else {
            return Ok(None);
        };
        Receipt::read_product(self.product, &release, &self.prefix, id).map(Some)
    }

    pub(super) fn current_directory(&self) -> io::Result<Option<(PathBuf, Uuid)>> {
        let pointer = self.root.join("current");
        let target = match fs::read_link(&pointer) {
            Ok(target) => target,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let id = target
            .to_str()
            .and_then(|target| target.strip_prefix("releases/"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .filter(|id| target == Path::new(&format!("releases/{id}")))
            .ok_or_else(|| invalid("Native current pointer is not an owned release."))?;
        let release = self.root.join(&target);
        if !fs::symlink_metadata(&release)?.file_type().is_dir() {
            return Err(invalid("Native current release is not a real directory."));
        }
        Ok(Some((release, id)))
    }

    pub fn check_links(&self, has_current: bool) -> io::Result<()> {
        for name in self.product.links() {
            let path = self.prefix.join("bin").join(name);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
                Ok(metadata)
                    if has_current
                        && metadata.file_type().is_symlink()
                        && fs::read_link(&path)? == Path::new(&self.product.link_target()) => {}
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "Refusing to replace {}. Remove it using its original package manager or choose another prefix.",
                            path.display()
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn ensure_links(&self) -> io::Result<()> {
        self.check_links(true)?;
        for name in self.product.links() {
            let path = self.prefix.join("bin").join(name);
            match symlink(self.product.link_target(), &path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    self.check_links(true)?
                }
                Err(error) => return Err(error),
            }
        }
        File::open(self.prefix.join("bin"))?.sync_all()
    }

    pub fn publish(
        &self,
        artifact: &Artifact,
        receipt: &Receipt,
        expected_current: Option<Uuid>,
        verifier: Option<ReleaseVerifier<'_>>,
        checkpoint: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.publish_with_finalization(
            artifact,
            receipt,
            expected_current,
            verifier,
            checkpoint,
            || self.ensure_links(),
        )
    }

    pub(super) fn publish_with_finalization(
        &self,
        artifact: &Artifact,
        receipt: &Receipt,
        expected_current: Option<Uuid>,
        verifier: Option<ReleaseVerifier<'_>>,
        checkpoint: &mut impl FnMut() -> io::Result<()>,
        finalize: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        self.publish_checked(
            artifact,
            receipt,
            verifier,
            checkpoint,
            ActivationCheck {
                has_current: expected_current.is_some(),
                validate: || {
                    if self.current()?.map(|receipt| receipt.id) != expected_current {
                        return Err(invalid(
                            "Native current release changed during installation.",
                        ));
                    }
                    Ok(())
                },
            },
            finalize,
        )
    }

    pub(super) fn publish_repair(
        &self,
        artifact: &Artifact,
        receipt: &Receipt,
        verifier: Option<ReleaseVerifier<'_>>,
        checkpoint: &mut impl FnMut() -> io::Result<()>,
        validate_current: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        self.publish_checked(
            artifact,
            receipt,
            verifier,
            checkpoint,
            ActivationCheck {
                has_current: true,
                validate: validate_current,
            },
            || self.ensure_links(),
        )
    }

    fn publish_checked(
        &self,
        artifact: &Artifact,
        receipt: &Receipt,
        verifier: Option<ReleaseVerifier<'_>>,
        checkpoint: &mut impl FnMut() -> io::Result<()>,
        current: ActivationCheck<impl FnOnce() -> io::Result<()>>,
        finalize: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        checkpoint()?;
        let release = self.root.join("releases").join(receipt.id.to_string());
        fs::DirBuilder::new().mode(0o700).create(&release)?;
        let pointer = self.root.join(format!(".current-{}", receipt.id));
        let mut pointer_created = false;
        let mut activated = false;
        let result = (|| {
            for name in self.product.files() {
                checkpoint()?;
                write(
                    &release.join(name),
                    &artifact.files[name],
                    if name == self.product.executable() {
                        0o755
                    } else {
                        0o644
                    },
                )?;
            }
            // Companion executables the release carries sit beside the
            // product's own.
            for name in self.product.companions() {
                if let Some(bytes) = artifact.files.get(*name) {
                    checkpoint()?;
                    write(&release.join(name), bytes, 0o755)?;
                }
            }
            // A verified skills tree is published with its release, read-only
            // content under directories only this release owns.
            let skills = artifact
                .files
                .iter()
                .filter(|(name, _)| super::skills_tree::is_skill_path(name))
                .collect::<Vec<_>>();
            if !skills.is_empty() {
                checkpoint()?;
                for (name, bytes) in skills {
                    let path = release.join(name);
                    fs::DirBuilder::new()
                        .recursive(true)
                        .mode(0o755)
                        .create(path.parent().expect("skill file parent"))?;
                    write(&path, bytes, 0o644)?;
                }
            }
            if let Some(verify) = verifier {
                verify(&release.join(self.product.executable()), &artifact.version)?;
            }
            write(
                &release.join("receipt.json"),
                &receipt.encode(&self.prefix)?,
                0o600,
            )?;
            checkpoint()?;
            File::open(&release)?.sync_all()?;
            File::open(self.root.join("releases"))?.sync_all()?;
            symlink(format!("releases/{}", receipt.id), &pointer)?;
            pointer_created = true;
            checkpoint()?;
            (current.validate)()?;
            self.check_links(current.has_current)?;
            fs::rename(&pointer, self.root.join("current"))?;
            activated = true;
            File::open(&self.root)?.sync_all()?;
            finalize()
        })();
        if !activated {
            // A UUID name is not proof that this invocation created an entry.
            let pointer_cleanup = if pointer_created {
                fs::remove_file(&pointer)
            } else {
                Ok(())
            };
            let release_cleanup = fs::remove_dir_all(&release);
            if let Err(cleanup) = pointer_cleanup.and(release_cleanup) {
                return Err(io::Error::new(
                    result
                        .as_ref()
                        .err()
                        .map_or(cleanup.kind(), io::Error::kind),
                    format!(
                        "{}; owned staging cleanup failed: {cleanup}",
                        result.as_ref().err().map_or_else(
                            || "Native installation failed".into(),
                            ToString::to_string
                        )
                    ),
                ));
            }
        }
        result.map_err(|error| {
            if activated {
                io::Error::new(error.kind(), ActivatedError(error))
            } else {
                error
            }
        })
    }
}

/// Prefix ancestors may be aliases; managed leaf entries may not. Track only
/// missing ancestors so their directory links can be synced after creation.
fn create_prefix_ancestors(parent: &Path) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut cursor = parent;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(cursor);
                cursor = cursor.parent().ok_or(error)?;
            }
            Err(error) => return Err(error),
        }
    }
    fs::create_dir_all(parent)?;
    for directory in missing {
        File::open(directory)?.sync_all()?;
        if let Some(parent) = directory.parent() {
            File::open(parent)?.sync_all()?;
        }
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(mode)
        .open(path)?;
    file.write_all(bytes)?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.sync_all()
}
