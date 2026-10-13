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
    former: bool,
}

struct ActivationCheck<F> {
    has_current: bool,
    validate: F,
}

/// A namespace directory that already exists may hold only what the publisher
/// writes there; anything else is not ours to publish into.
pub(super) fn require_installation_entries(root: &Path) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let name = entry?.file_name();
        let known = matches!(name.to_str(), Some("releases" | "current" | "install.lock"))
            || name
                .to_str()
                .is_some_and(|name| name.starts_with(".current-"));
        if !known {
            return Err(invalid(
                "Native installation directory holds files that TMT did not write; refusing to use it.",
            ));
        }
    }
    Ok(())
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
    pub(super) fn is_former(&self) -> bool {
        self.former
    }

    #[cfg(test)]
    pub fn existing(prefix: &Path) -> io::Result<Self> {
        Self::existing_product(prefix, Product::Cli)
    }

    pub fn existing_product(prefix: &Path, product: Product) -> io::Result<Self> {
        Self::existing_identity(prefix, product, false)
    }

    pub(super) fn existing_former(prefix: &Path, product: Product) -> io::Result<Self> {
        Self::existing_identity(prefix, product, true)
    }

    fn existing_identity(prefix: &Path, product: Product, former: bool) -> io::Result<Self> {
        directory(prefix, false)?;
        let prefix = fs::canonicalize(prefix)?;
        let namespace = if former {
            product
                .former()
                .ok_or_else(|| invalid("Product has no former installation."))?
                .namespace
        } else {
            product.namespace()
        };
        let root = prefix.join(namespace);
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
            former,
        })
    }

    /// A layout under the former namespace, as an installer from before the
    /// move left it.
    #[cfg(test)]
    pub fn open_former(prefix: &Path) -> io::Result<Self> {
        let former = Product::Cli.former().expect("the CLI has a former prefix");
        directory(prefix, true)?;
        let prefix = fs::canonicalize(prefix)?;
        directory(&prefix.join("lib"), true)?;
        directory(&prefix.join("bin"), true)?;
        let root = prefix.join(former.namespace);
        directory(&root, true)?;
        directory(&root.join("releases"), true)?;
        Ok(Self {
            prefix,
            root,
            product: Product::Cli,
            former: true,
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
        // A directory that is not yet an installation must hold nothing foreign.
        if fs::symlink_metadata(root.join("releases")).is_err() {
            require_installation_entries(&root)?;
        }
        directory(&root.join("releases"), true)?;
        Ok(Self {
            prefix,
            root,
            product,
            former: false,
        })
    }

    pub fn current(&self) -> io::Result<Option<Receipt>> {
        let Some((release, id)) = self.current_directory()? else {
            return Ok(None);
        };
        if self.former {
            Receipt::read_former(self.product, &release, &self.prefix, id).map(Some)
        } else {
            Receipt::read_product(self.product, &release, &self.prefix, id).map(Some)
        }
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

    fn namespace(&self) -> &'static str {
        match self.product.former() {
            Some(former) if self.former => former.namespace,
            _ => self.product.namespace(),
        }
    }

    /// Where this layout's command links point.
    pub fn link_target(&self) -> String {
        let executable = match self.product.former() {
            Some(former) if self.former => former.executable,
            _ => self.product.executable(),
        };
        format!("../{}/current/{executable}", self.namespace())
    }

    /// Where the former installation pointed the command link `name`, when
    /// this layout replaces that installation and keeps the command.
    fn former_link_target(&self, name: &str) -> Option<String> {
        self.product
            .former_link_target(name)
            .filter(|_| !self.former)
    }

    pub fn check_links(&self, has_current: bool) -> io::Result<()> {
        for name in self.product.links() {
            let path = self.prefix.join("bin").join(name);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
                Ok(metadata)
                    if metadata.file_type().is_symlink() && {
                        let target = fs::read_link(&path)?;
                        (has_current && target == Path::new(&self.link_target()))
                            || self
                                .former_link_target(name)
                                .is_some_and(|former| target == Path::new(&former))
                    } => {}
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

    /// Creates missing command links and moves a link that still points into
    /// the former installation. Anything else was refused by `check_links`.
    pub fn ensure_links(&self) -> io::Result<()> {
        self.check_links(true)?;
        for name in self.product.links() {
            let path = self.prefix.join("bin").join(name);
            match fs::read_link(&path) {
                Ok(target) if target == Path::new(&self.link_target()) => {}
                Ok(_) => self.retarget_link(name)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match symlink(self.link_target(), &path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            self.check_links(true)?
                        }
                        Err(error) => return Err(error),
                    }
                }
                Err(error) => return Err(error),
            }
        }
        File::open(self.prefix.join("bin"))?.sync_all()
    }

    /// Atomic replacement: a reader sees the former or the new target, never none.
    fn retarget_link(&self, name: &str) -> io::Result<()> {
        let bin = self.prefix.join("bin");
        let temporary = bin.join(format!(".{name}.{}", Uuid::new_v4()));
        symlink(self.link_target(), &temporary)?;
        fs::rename(&temporary, bin.join(name)).inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
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
        )?;
        // Finalization succeeded. Retain the exact predecessor, never guess from mtime.
        // A cleanup refusal must not turn a successful activation into an upgrade failure.
        if let Some(previous) = expected_current {
            let _ = super::retention::prune(self, receipt.id, previous);
        }
        Ok(())
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
            // Optional plain files (such as the declaration of optional uses).
            for name in self.product.optional_files() {
                if let Some(bytes) = artifact.files.get(*name) {
                    checkpoint()?;
                    write(&release.join(name), bytes, 0o644)?;
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
