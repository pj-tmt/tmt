//! The process-private cache root of a test build (TEST_ISOLATION): a create-new
//! directory under the temporary directory that no other process shares, and the
//! sweep of directories whose owner has exited.

use std::{
    fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};

const PREFIX: &str = "tmt-ops-cache-";

/// Sweeps the directories of finished test processes, then creates this one's.
pub(super) fn create() -> PathBuf {
    let parent = std::env::temp_dir();
    let own = std::process::id();
    sweep(&parent, own, owner_is_alive);
    create_new(&parent, own).expect("create the test cache root")
}

/// `<parent>/tmt-ops-cache-<pid>-<n>` for the first `n` this call creates. An
/// existing directory, such as a crashed run's under a reused pid, is never adopted.
fn create_new(parent: &Path, pid: u32) -> io::Result<PathBuf> {
    for attempt in 0u32.. {
        let candidate = parent.join(format!("{PREFIX}{pid}-{attempt}"));
        match fs::DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    unreachable!("the attempt counter does not run out")
}

/// Removes the cache directories in `parent` whose owner is neither `own` nor
/// `alive`. Only a real directory of this user goes: never a link, a file or
/// another user's. A live owner whose pid was reused keeps a stale directory,
/// which a later sweep removes.
fn sweep(parent: &Path, own: u32, alive: impl Fn(u32) -> bool) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    let user = nix::unistd::geteuid().as_raw();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(owner) else {
            continue;
        };
        if pid == own || alive(pid) {
            continue;
        }
        let path = entry.path();
        if fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_dir() && metadata.uid() == user)
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// The pid in `tmt-ops-cache-<pid>` or `tmt-ops-cache-<pid>-<n>`.
fn owner(name: &str) -> Option<u32> {
    let digits = name.strip_prefix(PREFIX)?.split('-').next()?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Only a process that is certainly gone (`ESRCH`) counts as finished.
fn owner_is_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) != Err(nix::errno::Errno::ESRCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{fs::PermissionsExt, fs::symlink};

    /// A scratch directory inside this process's own cache root, never the host's.
    fn scratch(name: &str) -> PathBuf {
        let directory = super::super::directory(name).unwrap();
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn a_created_root_is_new_private_and_never_an_existing_directory() {
        let parent = scratch("test-root-create");
        let stale = parent.join("tmt-ops-cache-4242-0");
        fs::create_dir(&stale).unwrap();
        fs::write(stale.join("marker"), b"stale").unwrap();
        let first = create_new(&parent, 4242).unwrap();
        let second = create_new(&parent, 4242).unwrap();
        assert_eq!(first, parent.join("tmt-ops-cache-4242-1"));
        assert_eq!(second, parent.join("tmt-ops-cache-4242-2"));
        assert_eq!(fs::read(stale.join("marker")).unwrap(), b"stale");
        assert_eq!(
            fs::metadata(&first).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(fs::read_dir(&first).unwrap().count(), 0);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn a_sweep_removes_only_the_directories_of_finished_owners() {
        let parent = scratch("test-root-sweep");
        let outside = scratch("test-root-sweep-outside");
        fs::write(outside.join("marker"), b"kept").unwrap();
        for name in [
            "tmt-ops-cache-111",
            "tmt-ops-cache-111-0",
            "tmt-ops-cache-222-0",
            "tmt-ops-cache-333-1",
            "other-111",
            "tmt-ops-cache-",
            "tmt-ops-cache-x-0",
            "tmt-ops-cache-+111",
        ] {
            fs::create_dir(parent.join(name)).unwrap();
        }
        symlink(&outside, parent.join("tmt-ops-cache-444-0")).unwrap();
        fs::write(parent.join("tmt-ops-cache-555"), b"a file").unwrap();
        sweep(&parent, 333, |pid| pid == 222);
        let mut left: Vec<_> = fs::read_dir(&parent)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "other-111",
                "tmt-ops-cache-",
                "tmt-ops-cache-+111",
                "tmt-ops-cache-222-0",
                "tmt-ops-cache-333-1",
                "tmt-ops-cache-444-0",
                "tmt-ops-cache-555",
                "tmt-ops-cache-x-0",
            ]
        );
        assert_eq!(fs::read(outside.join("marker")).unwrap(), b"kept");
        fs::remove_dir_all(parent).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn the_owner_is_the_leading_pid_and_nothing_looser() {
        assert_eq!(owner("tmt-ops-cache-7"), Some(7));
        assert_eq!(owner("tmt-ops-cache-7-12"), Some(7));
        for name in [
            "tmt-ops-cache-",
            "tmt-ops-cache--7",
            "tmt-ops-cache-+7",
            "tmt-ops-cache-7x-0",
            "x-7",
        ] {
            assert_eq!(owner(name), None, "{name}");
        }
    }

    #[test]
    fn this_process_is_alive_and_an_unrepresentable_pid_is_kept() {
        assert!(owner_is_alive(std::process::id()));
        assert!(owner_is_alive(u32::MAX));
    }
}
