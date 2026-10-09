//! One-release default-directory cutover (#2270), removed after the owner cut.
//! Move the directory as one unit; tmux-team.db and its live journals stay named.

use super::{ConfigError, normalize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(super) fn prepare(
    current: &Path,
    home: &Path,
    explicit: Option<&Path>,
    xdg: Option<&Path>,
) -> Result<(), ConfigError> {
    prepare_with(current, home, explicit, xdg, rename_exclusive)
}

fn prepare_with(
    current: &Path,
    home: &Path,
    explicit: Option<&Path>,
    xdg: Option<&Path>,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), ConfigError> {
    if explicit.is_some() {
        return Ok(());
    }
    // The normal path costs exactly one stat and never reads the former store.
    match fs::symlink_metadata(current) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ConfigError::internal(format!(
                "Cannot inspect {}: {error}",
                current.display()
            )));
        }
    }
    let preferred = normalize(&xdg.unwrap_or(&home.join(".config")).join("tmux-team"));
    let legacy = normalize(&home.join(".tmux-team"));
    // Preserve the former default selection only during this one-shot move.
    let former = if xdg.is_none()
        && ((legacy.join("config.json").exists() && !preferred.join("config.json").exists())
            || (!preferred.exists() && legacy.exists()))
    {
        legacy
    } else {
        preferred
    };
    let failure = |error: io::Error| {
        ConfigError::internal(format!(
            "Cannot move Core data directory {} to {}: {error}. No replacement directory was created.",
            former.display(),
            current.display(),
        ))
    };
    match fs::symlink_metadata(&former) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(failure(io::Error::other(
                "former default is not a directory",
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(error)),
    }
    fs::create_dir_all(
        current
            .parent()
            .ok_or_else(|| failure(io::Error::other("destination has no parent")))?,
    )
    .map_err(failure)?;
    match rename(&former, current) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(failure(error)),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rename_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    tmt_sys::rename_exclusive(from, to)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn rename_exclusive(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic exclusive rename is unavailable",
    ))
}

/// Translate only former default-root source coordinates after the atomic move.
/// This does not read the former store or establish ownership; the skill owner
/// must still prove the complete digest/inventory at the returned current path.
pub(crate) fn relocated_skill_source(current: &Path, source: &Path) -> Option<PathBuf> {
    if current.file_name()? != "tmt" {
        return None;
    }
    let parent = current.parent()?;
    let mut former = vec![parent.join("tmux-team")];
    if parent.file_name().is_some_and(|name| name == ".config") {
        former.push(parent.parent()?.join(".tmux-team"));
    }
    former.into_iter().find_map(|old| {
        if !matches!(fs::symlink_metadata(&old), Err(error) if error.kind() == io::ErrorKind::NotFound) {
            return None;
        }
        let suffix = source.strip_prefix(old.join("skill-assets")).ok()?;
        Some(current.join("skill-assets").join(suffix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::ConfigPaths, test_support::TestDirectory};

    #[test]
    fn moves_the_complete_default_once_and_keeps_open_file_incarnations() {
        use std::{
            io::{Read, Seek},
            os::unix::fs::MetadataExt,
        };
        let temp = TestDirectory::new();
        let former = temp.path.join(".config/tmux-team");
        let current = temp.path.join(".config/tmt");
        fs::create_dir_all(&former).unwrap();
        for name in [
            "config.json",
            "tmux-team.db",
            "tmux-team.db-wal",
            "tmux-team.db-shm",
        ] {
            fs::write(former.join(name), name.as_bytes()).unwrap();
        }
        let mut held = fs::File::open(former.join("tmux-team.db-wal")).unwrap();
        let inode = held.metadata().unwrap().ino();
        prepare(&current, &temp.path, None, None).unwrap();
        assert!(!former.exists());
        assert_eq!(
            fs::metadata(current.join("tmux-team.db-wal"))
                .unwrap()
                .ino(),
            inode
        );
        held.rewind().unwrap();
        let mut bytes = Vec::new();
        held.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"tmux-team.db-wal");
        prepare_with(&current, &temp.path, None, None, |_, _| {
            panic!("second move")
        })
        .unwrap();
        let paths = ConfigPaths::resolve(&temp.path, &temp.path, None, None);
        assert_eq!(paths.database, current.join("tmux-team.db"));
        assert!(paths.global_config.exists());
    }

    #[test]
    fn destination_racer_is_not_replaced() {
        let temp = TestDirectory::new();
        let xdg = temp.path.join("xdg");
        let former = xdg.join("tmux-team");
        let current = xdg.join("tmt");
        fs::create_dir_all(&former).unwrap();
        fs::write(former.join("kept"), b"former").unwrap();
        prepare_with(&current, &temp.path, None, Some(&xdg), |old, new| {
            fs::create_dir(new)?;
            let error = rename_exclusive(old, new).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            Err(error)
        })
        .unwrap();
        assert_eq!(fs::read(former.join("kept")).unwrap(), b"former");
        assert!(current.is_dir());
        assert!(!current.join("kept").exists());
    }

    #[test]
    fn failed_move_refuses_without_creating_empty_state() {
        let temp = TestDirectory::new();
        let former = temp.path.join(".config/tmux-team");
        let current = temp.path.join(".config/tmt");
        fs::create_dir_all(&former).unwrap();
        fs::write(former.join("config.json"), b"opaque config").unwrap();
        let error = prepare_with(&current, &temp.path, None, None, |_, _| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fixture refusal",
            ))
        })
        .unwrap_err();
        assert!(error.message.contains(&former.display().to_string()));
        assert!(error.message.contains(&current.display().to_string()));
        assert!(!current.exists());
        assert_eq!(
            fs::read(former.join("config.json")).unwrap(),
            b"opaque config"
        );
    }

    #[test]
    fn explicit_home_is_exact_and_untouched() {
        let temp = TestDirectory::new();
        let current = temp.path.join("explicit");
        fs::create_dir_all(temp.path.join(".config/tmux-team")).unwrap();
        prepare_with(&current, &temp.path, Some(&current), None, |_, _| {
            panic!("explicit move")
        })
        .unwrap();
        assert!(!current.exists());
        assert!(temp.path.join(".config/tmux-team").exists());
        assert_eq!(
            ConfigPaths::resolve(&temp.path, &temp.path, Some(&current), None).global_dir,
            current
        );
    }
}
