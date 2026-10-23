//! Whether a remembered provider session still has its transcript file, read
//! only under the driver's own tree and only by the exact session UUID. Any
//! doubt is `Unknown`, which callers treat like today's resume: never a refusal.
use super::transcript;
use std::{fs, os::unix::fs::OpenOptionsExt, path::Path, time::Instant};
use tmt_core::binding::session::ProviderSessionId;

/// The most directory entries one probe visits.
pub const PRESENCE_ENTRIES: usize = 16_384;

/// How long a resume waits for the probe before it counts as `Unknown`.
pub const PRESENCE_BUDGET: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPresence {
    /// A regular transcript file for the session passed the trust boundary.
    Present,
    /// The whole tree was read and no file names the session: gone or expired.
    Missing,
    /// A file names the session but TMT may not or cannot read it.
    Unreadable,
    /// The probe could not decide (no tree, deadline, bound, read error).
    Unknown,
}

/// Where a driver keeps its transcripts: files at `depth` below `root`, whose
/// name `names` accepts for the session UUID.
pub(crate) struct Layout<'a> {
    pub root: &'a Path,
    pub depth: usize,
    pub names: fn(&str, &str) -> bool,
}

pub(crate) fn probe(
    layout: &Layout<'_>,
    session: &ProviderSessionId,
    deadline: Instant,
    entries: usize,
) -> SessionPresence {
    if uuid::Uuid::parse_str(session.as_str()).is_err() {
        return SessionPresence::Unknown;
    }
    let mut pending = vec![(layout.root.to_path_buf(), 0)];
    let mut visited = 0;
    let mut unreadable = false;
    let mut doubt = false;
    while let Some((directory, depth)) = pending.pop() {
        // A tree TMT cannot list completely cannot prove the session is gone.
        let Ok(listing) = fs::read_dir(&directory) else {
            return SessionPresence::Unknown;
        };
        for entry in listing {
            visited += 1;
            if visited > entries || Instant::now() >= deadline {
                return SessionPresence::Unknown;
            }
            let Ok(entry) = entry else {
                return SessionPresence::Unknown;
            };
            // `file_type` does not follow a final symlink.
            let Ok(kind) = entry.file_type() else {
                return SessionPresence::Unknown;
            };
            if kind.is_dir() && depth < layout.depth {
                pending.push((entry.path(), depth + 1));
                continue;
            }
            let named = depth == layout.depth
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| (layout.names)(name, session.as_str()));
            if !named {
                continue;
            }
            if !kind.is_file() {
                unreadable = true;
                continue;
            }
            match admit(layout.root, &entry.path()) {
                SessionPresence::Present => return SessionPresence::Present,
                SessionPresence::Unreadable => unreadable = true,
                _ => doubt = true,
            }
        }
    }
    if unreadable {
        SessionPresence::Unreadable
    } else if doubt {
        SessionPresence::Unknown
    } else {
        SessionPresence::Missing
    }
}

/// Only a denied open is evidence of an unreadable file; any other open error
/// may be transient. An opened file must also pass the transcript boundary.
fn admit(root: &Path, path: &Path) -> SessionPresence {
    let flags = nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK;
    match fs::File::options()
        .read(true)
        .custom_flags(flags.bits())
        .open(path)
    {
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            SessionPresence::Unreadable
        }
        Err(_) => SessionPresence::Unknown,
        Ok(_) if transcript::open(root, path).is_some() => SessionPresence::Present,
        Ok(_) => SessionPresence::Unreadable,
    }
}

#[cfg(test)]
mod tests;
