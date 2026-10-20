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

pub mod attachments;
pub mod conversations;

/// One export's conversations never exceed this many bytes (both files together).
pub const CONVERSATIONS_BYTES: usize = 8 * 1024 * 1024;

pub const DISCLOSURE: &str =
    "This creates an unencrypted copy of the page. Anyone with these files can read it.";

/// Publish exact admitted source to a new private file. Like the full export, all writes
/// happen in owned staging, and an existing destination (including a symlink) is never replaced.
pub fn publish_source(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let encoded = path.as_os_str().as_encoded_bytes();
    if encoded.ends_with(b"/") || encoded.ends_with(b"/.") {
        return Err("Source destination needs a filename, not a directory path.".into());
    }
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("Source destination must not contain parent traversal.".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let leaf = absolute
        .file_name()
        .ok_or("Source destination needs a filename.")?;
    let parent_path = std::fs::canonicalize(absolute.parent().ok_or("Missing parent.")?)?;
    let destination = parent_path.join(leaf);
    destination
        .to_str()
        .ok_or("Source destination must be representable as a UTF-8 path.")?;
    let parent = directory(&parent_path)?;
    let mut random = [0; 16];
    getrandom::fill(&mut random)?;
    let stage_name = format!(".tmt-colab-source-{}", hex(&random));
    let mut stage = Staging::new(&parent, &stage_name)?;
    let result = (|| -> Result<()> {
        let file = File::from(openat(
            stage.tree.existing(None),
            "source",
            OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )?);
        stage.files.push(("source".into(), file));
        let file = &mut stage.files[0].1;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        let info = FileInfo::new("source", bytes);
        stage.check()?;
        verify_file(stage.tree.existing(None), "source", &info, bytes)?;
        check_destination(&parent_path, &parent)?;
        linkat(
            stage.tree.existing(None),
            "source",
            &parent,
            leaf,
            AtFlags::empty(),
        )?;
        same_entry(&parent, leaf, &stage.files[0].1)?;
        verify_file(
            &parent,
            leaf.to_str().expect("validated UTF-8 destination"),
            &info,
            bytes,
        )?;
        check_destination(&parent_path, &parent)?;
        parent.sync_all()?;
        Ok(())
    })();
    let cleanup = stage.cleanup();
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(destination),
        (result, cleanup) => Err(Fault::Publication {
            partial_directory: None,
            reason: [result.err(), cleanup.err()]
                .into_iter()
                .flatten()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        }
        .into()),
    }
}

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
    /// The published path: a file name, or `attachments/<id>`.
    pub name: String,
    pub size_bytes: usize,
    pub sha256: String,
}
impl FileInfo {
    fn new(name: impl Into<String>, bytes: &[u8]) -> Self {
        Self {
            name: name.into(),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    page_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_author: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    publisher_agent: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    creation_recipient: Option<&'a crate::decoder::CreationRecipient>,
    exported_at_ms: u64,
    membership_head: MembershipHead,
    epoch: String,
    plaintext: bool,
    discussions: Discussions,
    attachments: &'a [attachments::AttachmentRow],
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
    /// What the command tells the user about the copy it just made.
    disclosure: &'static str,
}
impl Bundle {
    /// Named files to publish in order, the manifest last. Names are generated by the caller:
    /// a file name, or `attachments/<id>`.
    pub fn from_files(disclosure: &'static str, files: Vec<(String, Vec<u8>)>) -> Result<Self> {
        let mut infos = Vec::with_capacity(files.len());
        let mut contents = Vec::with_capacity(files.len());
        for (name, bytes) in files {
            require_relative(&name)?;
            infos.push(FileInfo::new(name, &bytes));
            contents.push(bytes);
        }
        Ok(Self {
            contents,
            files: infos,
            disclosure,
        })
    }
    pub fn capture(
        store: &Store,
        key: &Keyring,
        page: &str,
        decoder: &mut Decoder,
        now: u64,
        attachment_source: &dyn attachments::Source,
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
        let mut view = snapshot.materialize(key, page, decoder)?;
        let (content_name, content) = if let Some(shell) = view.meta.get("shell") {
            (
                "shell.json",
                crate::closed::canonical(shell).map_err(|_| crate::page::Fault::Invalid)?,
            )
        } else {
            ("page.html", std::mem::take(&mut view.source).into_bytes())
        };
        // An ordinary page's work is exported beside its HTML, so a copy never loses its tasks.
        let work = match view.meta.get("work") {
            Some(work) => {
                Some(crate::closed::canonical(work).map_err(|_| crate::page::Fault::Invalid)?)
            }
            None => None,
        };
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
        let attachments::Gathered {
            entries,
            files: attachment_files,
        } = attachments::gather(
            &attachments::PageParts {
                meta: &view.meta,
                own: &view.own,
            },
            &attachments::GatherScope {
                space: &key.space_id,
                page,
                epoch: snapshot.epoch,
                revision: &crate::attachments::revision(key, page, &snapshot)?,
                total_bytes: attachments::TOTAL_BYTES,
                budget: attachments::BUDGET,
            },
            attachment_source,
            std::time::Instant::now(),
        )?;
        let mut files = vec![FileInfo::new(content_name, &content)];
        if let Some(work) = &work {
            files.push(FileInfo::new("work.json", work));
        }
        files.push(FileInfo::new("conversations.json", &json));
        files.push(FileInfo::new("conversations.md", &markdown));
        let manifest = serde_json::to_vec(&Manifest {
            format: "tmt-colab-page-export",
            version: 1,
            space_id: &key.space_id,
            page_id: page,
            title: &view.title,
            page_type: view.meta.get("shell").map(|_| "shell"),
            original_author: view.original_author.as_deref(),
            publisher_agent: view.publisher_agent.as_deref(),
            creation_recipient: view.creation_recipient.as_ref(),
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
            attachments: &entries,
            files: &files,
        })?;
        let mut contents = vec![content];
        contents.extend(work);
        contents.extend([json, markdown]);
        for (name, bytes) in attachment_files {
            files.push(FileInfo::new(name, &bytes));
            contents.push(bytes);
        }
        // The manifest lists the page files (the work among them when there is some) and every
        // attachment entry, never itself.
        files.push(FileInfo::new("manifest.json", &manifest));
        contents.push(manifest);
        Ok(Self {
            contents,
            files,
            disclosure: DISCLOSURE,
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
        for info in &self.files {
            require_relative(&info.name)?;
        }
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
                let (dir, leaf) = split(&info.name);
                let holder = stage.tree.holder(dir)?;
                let file = File::from(openat(
                    holder,
                    leaf,
                    OFlag::O_RDWR
                        | OFlag::O_CREAT
                        | OFlag::O_EXCL
                        | OFlag::O_NOFOLLOW
                        | OFlag::O_CLOEXEC,
                    Mode::S_IRUSR | Mode::S_IWUSR,
                )?);
                // Track an exclusive file before writing, including partial writes.
                let index = stage.files.len();
                stage.files.push((info.name.clone(), file));
                let file = &mut stage.files[index].1;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                file.write_all(bytes)?;
                file.sync_all()?;
                verify_file(stage.tree.existing(dir), leaf, info, bytes)?;
            }
            stage.tree.sync()?;
            before_publish(&parent, 0)?;
            stage.check()?;
            check_destination(&path, &parent)?;
            mkdirat(&parent, id, Mode::S_IRWXU)?;
            partial_directory = Some(path.join(id));
            let mut output = Tree::new(child_directory(&parent, id)?);
            output
                .root
                .set_permissions(std::fs::Permissions::from_mode(0o700))?;
            check_directory(&output.root)?;
            for (index, (info, bytes)) in self.files.iter().zip(&self.contents).enumerate() {
                let (dir, leaf) = split(&info.name);
                check_destination(&path, &parent)?;
                same_entry(&parent, id, &output.root)?;
                stage.check()?;
                verify_file(stage.tree.existing(dir), leaf, info, bytes)?;
                let holder = output.holder(dir)?;
                linkat(
                    stage.tree.existing(dir),
                    leaf,
                    holder,
                    leaf,
                    AtFlags::empty(),
                )?;
                same_entry(holder, leaf, &stage.files[index].1)?;
                verify_file(holder, leaf, info, bytes)?;
                holder.sync_all()?;
                output.root.sync_all()?;
                before_publish(&parent, index + 1)?;
            }
            check_destination(&path, &parent)?;
            same_entry(&parent, id, &output.root)?;
            output.check()?;
            parent.sync_all()?;
            Ok(())
        })();
        let cleanup = stage.cleanup();
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(Published {
                directory: path.join(id),
                files: self.files.clone(),
                disclosure: self.disclosure,
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
/// A published name is one file, or one file inside one fixed subdirectory, built only from
/// generated names: never a caller's path.
fn split(name: &str) -> (Option<&str>, &str) {
    match name.split_once('/') {
        Some((directory, leaf)) => (Some(directory), leaf),
        None => (None, name),
    }
}
fn require_relative(name: &str) -> Result<()> {
    let (directory, leaf) = split(name);
    let plain = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    };
    if directory.is_none_or(plain) && plain(leaf) {
        Ok(())
    } else {
        Err("Export entry name is not a generated name.".into())
    }
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
/// `leaf` is read inside `parent`; the manifest-visible name stays on `info`.
fn verify_file(parent: &File, leaf: &str, info: &FileInfo, expected: &[u8]) -> Result<()> {
    let mut file = File::from(openat(
        parent,
        leaf,
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
    if bytes != expected || FileInfo::new(&info.name, &bytes) != *info {
        return Err("Export bytes changed during publication.".into());
    }
    Ok(())
}
/// A directory this invocation created and holds open, with the subdirectories made in it.
struct Tree {
    root: File,
    directories: Vec<(String, File)>,
}
impl Tree {
    fn new(root: File) -> Self {
        Self {
            root,
            directories: Vec::new(),
        }
    }
    /// The directory a file goes in, creating its private subdirectory on first use.
    fn holder(&mut self, directory: Option<&str>) -> Result<&File> {
        let Some(name) = directory else {
            return Ok(&self.root);
        };
        if !self.directories.iter().any(|(known, _)| known == name) {
            mkdirat(&self.root, name, Mode::S_IRWXU)?;
            let file = child_directory(&self.root, name)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
            check_directory(&file)?;
            self.directories.push((name.to_owned(), file));
        }
        Ok(self.existing(directory))
    }
    /// A directory made earlier by `holder`.
    fn existing(&self, directory: Option<&str>) -> &File {
        match directory {
            None => &self.root,
            Some(name) => {
                &self
                    .directories
                    .iter()
                    .find(|(known, _)| known == name)
                    .expect("subdirectory made before use")
                    .1
            }
        }
    }
    fn check(&self) -> Result<()> {
        check_directory(&self.root)?;
        for (name, file) in &self.directories {
            same_entry(&self.root, name, file)?;
            check_directory(file)?;
        }
        Ok(())
    }
    fn sync(&self) -> Result<()> {
        for (_, file) in &self.directories {
            file.sync_all()?;
        }
        self.root.sync_all()?;
        Ok(())
    }
}
struct Staging<'a> {
    parent: &'a File,
    name: &'a str,
    tree: Tree,
    files: Vec<(String, File)>,
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
            tree: Tree::new(file),
            files: Vec::new(),
        })
    }
    fn check(&self) -> Result<()> {
        self.tree.check()?;
        same_entry(self.parent, self.name, &self.tree.root)?;
        for (name, file) in &self.files {
            let (directory, leaf) = split(name);
            same_entry(self.tree.existing(directory), leaf, file)?;
        }
        Ok(())
    }
    fn cleanup(self) -> Result<()> {
        self.check()?;
        for (name, _) in &self.files {
            let (directory, leaf) = split(name);
            unlinkat(
                self.tree.existing(directory),
                leaf,
                UnlinkatFlags::NoRemoveDir,
            )?;
        }
        for (name, _) in &self.tree.directories {
            unlinkat(&self.tree.root, name.as_str(), UnlinkatFlags::RemoveDir)?;
        }
        unlinkat(self.parent, self.name, UnlinkatFlags::RemoveDir)?;
        self.parent.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
