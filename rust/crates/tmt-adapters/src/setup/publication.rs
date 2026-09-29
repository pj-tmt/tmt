//! Setup publication is separate from consent and provider document mapping.

use super::{SETTINGS_LIMIT, SetupPlan};
use crate::bounded_file::{self, FileReadError};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Serializes setup publications to the settings files in one directory.
const LOCK: &str = ".tmt-setup.lock";

pub fn read_settings(path: &Path) -> io::Result<Option<String>> {
    match bounded_file::read_no_follow(path, SETTINGS_LIMIT) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| io::Error::other("Provider settings must be UTF-8 JSON.")),
        Err(FileReadError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io::Error::other(error)),
    }
}

/// Call only after consent to this exact plan. A changed input requires a new
/// plan/approval; setup never merges an unseen concurrent user edit.
pub fn apply(plan: &SetupPlan) -> io::Result<Option<PathBuf>> {
    if !plan.change.changed() {
        return Ok(None);
    }
    let path = &plan.change.path;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Settings path has no parent."))?;
    fs::create_dir_all(parent)?;
    let _lock = crate::file_lock::exclusive(&parent.join(LOCK))?;
    ensure_unchanged(plan)?;
    let backup = if let Some(before) = &plan.change.before {
        let backup = parent.join(format!("settings.tmt-backup-{}.json", Uuid::new_v4()));
        write_new(&backup, before.as_bytes())?;
        Some(backup)
    } else {
        None
    };
    let stage = parent.join(format!(".tmt-setup-{}", Uuid::new_v4()));
    let pending = (|| {
        write_new(&stage, plan.change.after.as_bytes())?;
        ensure_unchanged(plan)?;
        fs::rename(&stage, path)?;
        std::fs::File::open(parent)?.sync_all()
    })();
    if stage.exists() {
        fs::remove_file(&stage)?;
    }
    pending.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "Setup publication failed: {error}. Recoverable backup: {}",
                backup.as_ref().map_or_else(
                    || "none (new settings)".into(),
                    |path| path.display().to_string()
                )
            ),
        )
    })?;
    Ok(backup)
}

/// Removes the setup lock beside `settings` once TMT has no hooks left in
/// that directory. It is held while it is unlinked, so it never disappears
/// under a publication that is still running.
pub(super) fn remove_lock(settings: &Path) -> io::Result<()> {
    let Some(parent) = settings.parent() else {
        return Ok(());
    };
    let lock = parent.join(LOCK);
    match fs::symlink_metadata(&lock) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        result => result?,
    };
    let _held = crate::file_lock::exclusive(&lock)?;
    fs::remove_file(&lock)
}

fn ensure_unchanged(plan: &SetupPlan) -> io::Result<()> {
    if read_settings(&plan.change.path)? != plan.change.before {
        return Err(io::Error::other(
            "Provider settings changed after planning; run setup again to review the new plan.",
        ));
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use crate::{drivers::claude, setup::plan};

    #[test]
    fn consented_plan_backs_up_exact_bytes_and_rejects_a_later_edit() {
        let root = TestDirectory::new();
        let path = root.path.join("settings.json");
        let original = "{\n  \"permissions\": {\"allow\": []}\n}\n";
        fs::write(&path, original).unwrap();
        let planned = plan(
            &claude::DRIVER,
            path.clone(),
            read_settings(&path).unwrap(),
            "/stable/tmt".into(),
            false,
        )
        .unwrap();
        let backup = apply(&planned).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);
        assert_eq!(fs::read_to_string(&path).unwrap(), planned.change.after);
        let again = plan(
            &claude::DRIVER,
            path.clone(),
            read_settings(&path).unwrap(),
            "/stable/tmt".into(),
            false,
        )
        .unwrap();
        assert!(!again.change.changed());
        assert_eq!(apply(&again).unwrap(), None);
        let remove = plan(
            &claude::DRIVER,
            path.clone(),
            read_settings(&path).unwrap(),
            "/stable/tmt".into(),
            true,
        )
        .unwrap();
        fs::write(&path, "{\"user\":true}").unwrap();
        assert!(
            apply(&remove)
                .unwrap_err()
                .to_string()
                .contains("changed after planning")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"user\":true}");
    }

    #[test]
    fn settings_symlinks_are_not_replaced_or_followed() {
        let root = TestDirectory::new();
        let original = root.path.join("original.json");
        let link = root.path.join("settings.json");
        fs::write(&original, "{}").unwrap();
        std::os::unix::fs::symlink(&original, &link).unwrap();
        assert!(read_settings(&link).is_err());
        assert_eq!(fs::read_to_string(&original).unwrap(), "{}");
    }
}
