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
    prepare_with(
        current,
        home,
        explicit,
        xdg,
        || crate::native_install::inspect(&std::env::current_exe()?).map(|_| ()),
        rename_exclusive,
        |former, current| {
            eprintln!("tmt: moved Core data directory {former:?} to {current:?}");
        },
    )
}

fn prepare_with(
    current: &Path,
    home: &Path,
    explicit: Option<&Path>,
    xdg: Option<&Path>,
    admit: impl FnOnce() -> io::Result<()>,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
    report: impl FnOnce(&Path, &Path),
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
            "Cannot move Core data directory {} to {}: {error}. Refusing to initialize separate state.",
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
    admit().map_err(|error| {
        ConfigError::internal(format!(
            "Refusing to move Core data directory {} to {} from an unmanaged executable: {error}. Run the managed release or set TMT_HOME to an isolated directory.",
            former.display(),
            current.display(),
        ))
    })?;
    fs::create_dir_all(
        current
            .parent()
            .ok_or_else(|| failure(io::Error::other("destination has no parent")))?,
    )
    .map_err(failure)?;
    match rename(&former, current) {
        Ok(()) => {
            report(&former, current);
            Ok(())
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound
            ) && matches!(fs::symlink_metadata(&former), Err(error) if error.kind() == io::ErrorKind::NotFound)
                && fs::symlink_metadata(current).is_ok_and(|metadata| metadata.is_dir()) =>
        {
            Ok(())
        }
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
        let mut reports = Vec::new();
        prepare_with(
            &current,
            &temp.path,
            None,
            None,
            || Ok(()),
            rename_exclusive,
            |old, new| {
                reports.push((old.to_path_buf(), new.to_path_buf()));
            },
        )
        .unwrap();
        assert_eq!(reports, vec![(former.clone(), current.clone())]);
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
        prepare_with(
            &current,
            &temp.path,
            None,
            None,
            || panic!("second admission"),
            |_, _| panic!("second move"),
            |_, _| panic!("second report"),
        )
        .unwrap();
        let paths = ConfigPaths::resolve(&temp.path, &temp.path, None, None);
        assert_eq!(paths.database, current.join("tmux-team.db"));
        assert!(paths.global_config.exists());
    }

    #[test]
    fn managed_move_preserves_the_former_default_selection() {
        for scenario in ["legacy-empty", "legacy-config", "both-config", "xdg-empty"] {
            let temp = TestDirectory::new();
            let legacy = temp.path.join(".tmux-team");
            let preferred = temp.path.join(".config/tmux-team");
            let current = temp.path.join(".config/tmt");
            fs::create_dir(&legacy).unwrap();
            if scenario != "legacy-empty" {
                fs::write(legacy.join("config.json"), b"legacy").unwrap();
            }
            if matches!(scenario, "both-config" | "xdg-empty") {
                fs::create_dir_all(&preferred).unwrap();
            }
            if scenario == "both-config" {
                fs::write(preferred.join("config.json"), b"preferred").unwrap();
            }
            let expected = if scenario == "both-config" {
                &preferred
            } else {
                &legacy
            };
            prepare_with(
                &current,
                &temp.path,
                None,
                None,
                || Ok(()),
                rename_exclusive,
                |old, new| {
                    assert_eq!(old, expected);
                    assert_eq!(new, current);
                },
            )
            .unwrap();
            assert!(!expected.exists());
            assert!(current.is_dir());
            if scenario != "legacy-empty" {
                let bytes: &[u8] = if scenario == "both-config" {
                    b"preferred"
                } else {
                    b"legacy"
                };
                assert_eq!(fs::read(current.join("config.json")).unwrap(), bytes);
            }
            assert!(!current.join("tmux-team.db").exists());
        }
    }

    #[test]
    fn unmanaged_executable_refuses_before_creating_the_destination_parent() {
        let temp = TestDirectory::new();
        let former = temp.path.join(".tmux-team");
        let current = temp.path.join(".config/tmt");
        fs::create_dir(&former).unwrap();
        fs::write(former.join("config.json"), b"owned settings").unwrap();
        let error = prepare(&current, &temp.path, None, None).unwrap_err();
        assert!(error.message.contains(&former.display().to_string()));
        assert!(error.message.contains(&current.display().to_string()));
        assert!(error.message.contains("TMT_HOME"));
        assert!(!current.parent().unwrap().exists());
        assert_eq!(
            fs::read(former.join("config.json")).unwrap(),
            b"owned settings"
        );
    }

    #[test]
    fn completed_concurrent_move_succeeds_without_reporting_another_move() {
        for outcome in [io::ErrorKind::NotFound, io::ErrorKind::AlreadyExists] {
            let temp = TestDirectory::new();
            let former = temp.path.join(".config/tmux-team");
            let current = temp.path.join(".config/tmt");
            fs::create_dir_all(&former).unwrap();
            fs::write(former.join("kept"), b"same store").unwrap();
            prepare_with(
                &current,
                &temp.path,
                None,
                None,
                || Ok(()),
                |old, new| {
                    rename_exclusive(old, new)?;
                    Err(io::Error::from(outcome))
                },
                |_, _| panic!("peer move must stay quiet"),
            )
            .unwrap();
            assert!(!former.exists());
            assert_eq!(fs::read(current.join("kept")).unwrap(), b"same store");
        }
    }

    #[test]
    fn source_not_found_without_a_completed_destination_refuses() {
        let temp = TestDirectory::new();
        let former = temp.path.join(".config/tmux-team");
        let current = temp.path.join(".config/tmt");
        fs::create_dir_all(&former).unwrap();
        let result = prepare_with(
            &current,
            &temp.path,
            None,
            None,
            || Ok(()),
            |_, _| Err(io::Error::from(io::ErrorKind::NotFound)),
            |_, _| panic!("missing source is not delivery evidence"),
        );
        assert!(result.is_err());
        assert!(!current.exists());
        assert!(former.exists());
    }

    #[test]
    fn absent_former_default_has_no_admission_or_report_work() {
        let temp = TestDirectory::new();
        let current = temp.path.join(".config/tmt");
        prepare_with(
            &current,
            &temp.path,
            None,
            None,
            || panic!("no former store"),
            |_, _| panic!("no rename"),
            |_, _| panic!("no report"),
        )
        .unwrap();
        assert!(!current.exists());
    }

    #[test]
    fn destination_racer_is_not_replaced() {
        let temp = TestDirectory::new();
        let xdg = temp.path.join("xdg");
        let former = xdg.join("tmux-team");
        let current = xdg.join("tmt");
        fs::create_dir_all(&former).unwrap();
        fs::write(former.join("kept"), b"former").unwrap();
        let error = prepare_with(
            &current,
            &temp.path,
            None,
            Some(&xdg),
            || Ok(()),
            |old, new| {
                fs::create_dir(new)?;
                let error = rename_exclusive(old, new).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
                Err(error)
            },
            |_, _| panic!("racer did not move our store"),
        )
        .unwrap_err();
        assert!(error.message.contains(&former.display().to_string()));
        assert!(error.message.contains(&current.display().to_string()));
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
        let error = prepare_with(
            &current,
            &temp.path,
            None,
            None,
            || Ok(()),
            |_, _| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "fixture refusal",
                ))
            },
            |_, _| panic!("failed move report"),
        )
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
        prepare_with(
            &current,
            &temp.path,
            Some(&current),
            None,
            || panic!("explicit admission"),
            |_, _| panic!("explicit move"),
            |_, _| panic!("explicit report"),
        )
        .unwrap();
        assert!(!current.exists());
        assert!(temp.path.join(".config/tmux-team").exists());
        assert_eq!(
            ConfigPaths::resolve(&temp.path, &temp.path, Some(&current), None).global_dir,
            current
        );
    }
}
