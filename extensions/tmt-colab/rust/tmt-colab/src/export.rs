//! Root-local plaintext export. Only the admitted fold supplies page bytes.
use crate::{Result, decoder::Decoder, fold::Snapshot, keyring::Keyring, store::Store};
use nix::{
    fcntl::{AtFlags, OFlag, openat},
    sys::stat::{Mode, fstat, fstatat, mkdirat},
    unistd::{Uid, UnlinkatFlags, linkat, unlinkat},
};
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};
use tmt_colab_model::{crypto, values};

pub mod conversations;

/// One export's conversations never exceed this many bytes (both files together).
pub const CONVERSATIONS_BYTES: usize = 8 * 1024 * 1024;

pub const DISCLOSURE: &str =
    "This creates an unencrypted copy of the page. Anyone with these files can read it.";

#[derive(Debug)]
pub enum Fault {
    Inactive,
    MissingState,
    TooLarge,
    Publication {
        partial_directory: Option<PathBuf>,
        reason: String,
    },
}
impl Fault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Inactive => "COLAB_EXPORT_INACTIVE",
            Self::MissingState => "COLAB_EXPORT_STATE_MISSING",
            Self::TooLarge => "COLAB_EXPORT_TOO_LARGE",
            Self::Publication { .. } => "COLAB_EXPORT_FAILED",
        }
    }
    pub fn partial_directory(&self) -> Option<&Path> {
        match self {
            Self::Publication {
                partial_directory, ..
            } => partial_directory.as_deref(),
            _ => None,
        }
    }
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inactive => f.write_str("archived or deleted pages cannot be exported yet"),
            Self::MissingState => f.write_str("No existing Colab state to export."),
            Self::TooLarge => f.write_str("The page conversations are too large to export."),
            Self::Publication {
                partial_directory,
                reason,
            } => {
                write!(f, "Export failed: {reason}")?;
                if let Some(path) = partial_directory {
                    write!(f, "; partial output was created at {}", path.display())?;
                }
                Ok(())
            }
        }
    }
}
impl std::error::Error for Fault {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub name: &'static str,
    pub size_bytes: usize,
    pub sha256: String,
}
impl FileInfo {
    fn new(name: &'static str, bytes: &[u8]) -> Self {
        Self {
            name,
            size_bytes: bytes.len(),
            sha256: hex(&crypto::digest(bytes)),
        }
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
    format: &'static str,
    version: u8,
    space_id: &'a str,
    page_id: &'a str,
    title: &'a str,
    exported_at_ms: u64,
    membership_head: MembershipHead,
    epoch: String,
    plaintext: bool,
    discussions: Discussions,
    files: &'a [FileInfo],
}
#[derive(Serialize)]
struct Discussions {
    included: bool,
    scope: &'static str,
    format: &'static str,
    version: u8,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MembershipHead {
    revision: String,
    statement_hash: String,
}
/// Immutable byte bundle: no roots, wraps, sessions or renderer markup.
pub struct Bundle {
    /// Every file's bytes in `files` order; the manifest is last.
    contents: Vec<Vec<u8>>,
    files: Vec<FileInfo>,
}
impl Bundle {
    pub fn capture(
        store: &Store,
        key: &Keyring,
        page: &str,
        decoder: &mut Decoder,
        now: u64,
    ) -> Result<Self> {
        values::generated_id(page)?;
        values::time(now)?;
        let snapshot = Snapshot::capture(store, key, page).map_err(|error| {
            // The current fold deliberately denies both archive and deletion.
            // Resolve this diagnostic only on failure, never replace fold admission.
            let inactive = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
                let (states, _) = crate::fold::verify_log(&tx.log()?, key, page)?;
                Ok(states.last().is_some_and(|state| !state.policy.writable()))
            });
            if matches!(inactive, Ok(true)) {
                Box::new(Fault::Inactive) as _
            } else {
                error
            }
        })?;
        let view = snapshot.materialize(key, page, decoder)?;
        let html = view.source.into_bytes();
        let conversations = conversations::Conversations::project(
            conversations::Capture {
                space_id: &key.space_id,
                page_id: page,
                title: &view.title,
                epoch: &snapshot.epoch.to_string(),
                head: conversations::Head {
                    revision: snapshot.authority.head.revision.to_string(),
                    statement_hash: hex(&snapshot.authority.head.hash),
                },
            },
            &view.own,
            &view.signing_keys,
            &view.status_writers,
        );
        let json = conversations.json();
        let markdown = conversations.markdown().into_bytes();
        if json.len() + markdown.len() > CONVERSATIONS_BYTES {
            return Err(Box::new(Fault::TooLarge));
        }
        let mut files = vec![
            FileInfo::new("page.html", &html),
            FileInfo::new("conversations.json", &json),
            FileInfo::new("conversations.md", &markdown),
        ];
        let manifest = serde_json::to_vec(&Manifest {
            format: "tmt-colab-page-export",
            version: 1,
            space_id: &key.space_id,
            page_id: page,
            title: &view.title,
            exported_at_ms: now,
            membership_head: MembershipHead {
                revision: snapshot.authority.head.revision.to_string(),
                statement_hash: hex(&snapshot.authority.head.hash),
            },
            epoch: snapshot.epoch.to_string(),
            plaintext: true,
            discussions: Discussions {
                included: true,
                scope: "current-epoch",
                format: conversations::FORMAT,
                version: 1,
            },
            files: &files,
        })?;
        files.push(FileInfo::new("manifest.json", &manifest));
        Ok(Self {
            contents: vec![html, json, markdown, manifest],
            files,
        })
    }
    pub fn publish(&self, parent: &Path) -> Result<Published> {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)?;
        random[6] = (random[6] & 15) | 64;
        random[8] = (random[8] & 63) | 128;
        let text = hex(&random);
        let id = format!(
            "{}-{}-{}-{}-{}",
            &text[..8],
            &text[8..12],
            &text[12..16],
            &text[16..20],
            &text[20..]
        );
        self.publish_as(parent, &id, |_, _| Ok(()))
    }
    fn publish_as(
        &self,
        path: &Path,
        id: &str,
        mut before_publish: impl FnMut(&File, usize) -> Result<()>,
    ) -> Result<Published> {
        values::generated_id(id)?;
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err("Export destination must not contain parent traversal.".into());
        }
        // The user-selected parent may have aliases (including macOS /tmp).
        // Resolve once; every created entry is still admitted without following links.
        let path = std::fs::canonicalize(path)?;
        let parent = directory(&path)?;
        let stage_name = format!(".tmt-colab-export-{id}");
        let mut stage = Staging::new(&parent, &stage_name)?;
        let mut partial_directory = None;
        let result = (|| -> Result<()> {
            for (info, bytes) in self.files.iter().zip(&self.contents) {
                let file = File::from(openat(
                    &stage.file,
                    info.name,
                    OFlag::O_RDWR
                        | OFlag::O_CREAT
                        | OFlag::O_EXCL
                        | OFlag::O_NOFOLLOW
                        | OFlag::O_CLOEXEC,
                    Mode::S_IRUSR | Mode::S_IWUSR,
                )?);
                // Track an exclusive file before writing, including partial writes.
                let index = stage.files.len();
                stage.files.push((info.name, file));
                let file = &mut stage.files[index].1;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                file.write_all(bytes)?;
                file.sync_all()?;
                verify_file(&stage.file, info, bytes)?;
            }
            stage.file.sync_all()?;
            before_publish(&parent, 0)?;
            stage.check()?;
            check_destination(&path, &parent)?;
            mkdirat(&parent, id, Mode::S_IRWXU)?;
            partial_directory = Some(path.join(id));
            let output = child_directory(&parent, id)?;
            output.set_permissions(std::fs::Permissions::from_mode(0o700))?;
            check_directory(&output)?;
            for (index, (info, bytes)) in self.files.iter().zip(&self.contents).enumerate() {
                check_destination(&path, &parent)?;
                same_entry(&parent, id, &output)?;
                stage.check()?;
                verify_file(&stage.file, info, bytes)?;
                linkat(&stage.file, info.name, &output, info.name, AtFlags::empty())?;
                same_entry(&output, info.name, &stage.files[index].1)?;
                verify_file(&output, info, bytes)?;
                output.sync_all()?;
                before_publish(&parent, index + 1)?;
            }
            check_destination(&path, &parent)?;
            same_entry(&parent, id, &output)?;
            parent.sync_all()?;
            Ok(())
        })();
        let cleanup = stage.cleanup();
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(Published {
                directory: path.join(id),
                files: self.files.clone(),
                disclosure: DISCLOSURE,
            }),
            (result, cleanup) => Err(Fault::Publication {
                partial_directory,
                reason: [result.err(), cleanup.err()]
                    .into_iter()
                    .flatten()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("; staging cleanup: "),
            }
            .into()),
        }
    }
}
#[derive(Serialize)]
pub struct Published {
    pub directory: PathBuf,
    pub files: Vec<FileInfo>,
    pub disclosure: &'static str,
}
fn directory(path: &Path) -> Result<File> {
    let mut file = File::open("/")?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => file = child_directory(&file, Path::new(name))?,
            _ => return Err("Export destination must not contain parent traversal.".into()),
        }
    }
    Ok(file)
}
fn check_destination(path: &Path, parent: &File) -> Result<()> {
    let current = directory(path)?.metadata()?;
    let original = parent.metadata()?;
    if current.dev() != original.dev() || current.ino() != original.ino() {
        return Err("Export destination changed during publication.".into());
    }
    Ok(())
}
fn child_directory(parent: &File, name: impl AsRef<Path>) -> Result<File> {
    Ok(File::from(openat(
        parent,
        name.as_ref(),
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?))
}
fn check_directory(file: &File) -> Result<()> {
    let m = file.metadata()?;
    if !m.is_dir() || m.uid() != Uid::effective().as_raw() || m.mode() & 0o777 != 0o700 {
        return Err("Unsafe export directory.".into());
    }
    Ok(())
}
fn same_entry(parent: &File, name: impl AsRef<Path>, file: &File) -> Result<()> {
    let metadata = fstat(file)?;
    let linked = fstatat(parent, name.as_ref(), AtFlags::AT_SYMLINK_NOFOLLOW)?;
    if linked.st_dev != metadata.st_dev || linked.st_ino != metadata.st_ino {
        return Err("Export entry changed during publication.".into());
    }
    Ok(())
}
fn verify_file(parent: &File, info: &FileInfo, expected: &[u8]) -> Result<()> {
    let mut file = File::from(openat(
        parent,
        info.name,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?);
    let m = file.metadata()?;
    if !m.is_file()
        || m.uid() != Uid::effective().as_raw()
        || m.mode() & 0o777 != 0o600
        || m.len() != expected.len() as u64
    {
        return Err("Unsafe export file.".into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(expected.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes != expected || FileInfo::new(info.name, &bytes) != *info {
        return Err("Export bytes changed during publication.".into());
    }
    Ok(())
}
struct Staging<'a> {
    parent: &'a File,
    name: &'a str,
    file: File,
    files: Vec<(&'static str, File)>,
}
impl<'a> Staging<'a> {
    fn new(parent: &'a File, name: &'a str) -> Result<Self> {
        mkdirat(parent, name, Mode::S_IRWXU)?;
        let file = child_directory(parent, name)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        check_directory(&file)?;
        Ok(Self {
            parent,
            name,
            file,
            files: Vec::new(),
        })
    }
    fn check(&self) -> Result<()> {
        check_directory(&self.file)?;
        same_entry(self.parent, self.name, &self.file)?;
        for (name, file) in &self.files {
            same_entry(&self.file, name, file)?;
        }
        Ok(())
    }
    fn cleanup(self) -> Result<()> {
        self.check()?;
        for (name, file) in self.files {
            same_entry(&self.file, name, &file)?;
            unlinkat(&self.file, name, UnlinkatFlags::NoRemoveDir)?;
        }
        unlinkat(self.parent, self.name, UnlinkatFlags::RemoveDir)?;
        self.parent.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
