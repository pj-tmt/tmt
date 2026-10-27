//! Setup publication is separate from consent and provider document mapping.

use super::{SETTINGS_LIMIT, SetupPlan};
use crate::{
    bounded_file::{self, FileReadError},
    drivers::{DriverDefinition, HookFormat},
};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Serializes setup publications to the settings files in one directory.
const LOCK: &str = ".tmt-setup.lock";

pub const BACKUP_DIRECTORY: &str = ".tmt-setup-backups";
const BACKUP_WARNING_THRESHOLD: usize = 32;

pub fn backup_directory(settings: &Path) -> Option<PathBuf> {
    settings
        .parent()
        .map(|parent| parent.join(BACKUP_DIRECTORY))
}

/// Advisory housekeeping only; never prevents publication or deletes a backup.
pub fn backup_warning(settings: &Path) -> Option<String> {
    let directory = backup_directory(settings)?;
    let entries = fs::read_dir(&directory).ok()?;
    let count = entries
        .flatten()
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                name.starts_with("settings.tmt-backup-") && name.ends_with(".json")
            })
        })
        .take(BACKUP_WARNING_THRESHOLD + 1)
        .count();
    (count > BACKUP_WARNING_THRESHOLD).then(|| format!(
        "More than {BACKUP_WARNING_THRESHOLD} settings backups in {}. Review recovery copies and manually remove only those you no longer need; TMT never deletes or migrates backups automatically.",
        directory.display()
    ))
}

fn allows_symlink_settings(provider: &DriverDefinition) -> bool {
    matches!(provider.descriptor.hooks, Some(HookFormat::NamedHooksJson))
}

/// When `provider` permits a symlinked settings file (such as `agy` sharing
/// `~/.gemini/config/hooks.json`) and `path` is a symlink, resolves and
/// validates the target regular file owned by the current user.
pub fn resolve_provider_settings_target(
    provider: &DriverDefinition,
    path: &Path,
) -> io::Result<Option<PathBuf>> {
    if !allows_symlink_settings(provider) {
        return Ok(None);
    }
    resolve_symlink_target(path)
}

fn resolve_symlink_target(path: &Path) -> io::Result<Option<PathBuf>> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(None);
    };
    if !metadata.is_symlink() {
        return Ok(None);
    }
    let link = fs::read_link(path)?;
    let resolved = fs::canonicalize(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "Provider settings {} is a symlink to {}, which could not be resolved: {error}",
                path.display(),
                link.display()
            ),
        )
    })?;
    let target_meta = fs::symlink_metadata(&resolved)?;
    if !target_meta.is_file() || target_meta.uid() != nix::unistd::geteuid().as_raw() {
        return Err(io::Error::other(format!(
            "Provider settings {} resolves to {}, which must be a regular file owned by the current user.",
            path.display(),
            resolved.display()
        )));
    }
    Ok(Some(resolved))
}

pub fn read_provider_settings(
    provider: &DriverDefinition,
    path: &Path,
) -> io::Result<Option<String>> {
    if let Some(target) = resolve_provider_settings_target(provider, path)? {
        return read_regular_settings(&target);
    }
    read_settings(path)
}

pub fn read_settings(path: &Path) -> io::Result<Option<String>> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink()) {
        let target = fs::read_link(path)?;
        return Err(io::Error::other(format!(
            "Provider settings {} is a symlink to {}; TMT will not follow or replace it. Edit the target manually to add or remove TMT hooks, preserving your other settings; relative targets are relative to the link directory. Review the target and make a byte-exact backup before editing.",
            path.display(),
            target.display()
        )));
    }
    read_regular_settings(path)
}

fn read_regular_settings(path: &Path) -> io::Result<Option<String>> {
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
    let write_path = plan.change.resolved_target.as_ref().unwrap_or(path);
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Settings path has no parent."))?;
    let write_parent = write_path
        .parent()
        .ok_or_else(|| io::Error::other("Settings target path has no parent."))?;
    fs::create_dir_all(parent)?;
    // The lock lives beside the link, never in the directory a link points into, which may be
    // another checkout: a link retargeted or a target changed meanwhile is what
    // `ensure_unchanged` refuses, under this lock and again just before the rename.
    let _lock = crate::file_lock::exclusive(&parent.join(LOCK))?;
    ensure_unchanged(plan)?;
    if plan.change.resolved_target.is_some() {
        // The stage is written beside the target so that the rename keeps the link: prove that
        // directory takes a file before the backup or anything else changes.
        let probe = write_parent.join(format!(".tmt-setup-probe-{}", Uuid::new_v4()));
        write_new(&probe, b"").map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "Cannot write beside the settings target {}: {error}",
                    write_path.display()
                ),
            )
        })?;
        fs::remove_file(&probe)?;
    }
    let backup = if let Some(before) = &plan.change.before {
        let directory = parent.join(BACKUP_DIRECTORY);
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(&directory)?;
        if !metadata.is_dir()
            || metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(io::Error::other(format!(
                "Settings backup directory {} must be a private directory owned by the current user, without a symlink.",
                directory.display()
            )));
        }
        let backup = directory.join(format!("settings.tmt-backup-{}.json", Uuid::new_v4()));
        write_new(&backup, before.as_bytes())?;
        fs::File::open(&directory)?.sync_all()?;
        fs::File::open(parent)?.sync_all()?;
        Some(backup)
    } else {
        None
    };
    let stage = write_parent.join(format!(".tmt-setup-{}", Uuid::new_v4()));
    let pending = (|| {
        write_new(&stage, plan.change.after.as_bytes())?;
        ensure_unchanged(plan)?;
        fs::rename(&stage, write_path)?;
        std::fs::File::open(write_parent)?.sync_all()
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

pub(super) fn ensure_unchanged(plan: &SetupPlan) -> io::Result<()> {
    let current = match &plan.change.resolved_target {
        Some(expected_target) => {
            let actual_target = resolve_symlink_target(&plan.change.path)?;
            if actual_target.as_ref() != Some(expected_target) {
                return Err(io::Error::other(
                    "Provider settings changed after planning; run setup again to review the new plan.",
                ));
            }
            read_regular_settings(expected_target)?
        }
        None => read_settings(&plan.change.path)?,
    };
    if current != plan.change.before {
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
    use crate::{
        drivers::claude,
        setup::{UsageHook, plan},
    };

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
            UsageHook::Keep,
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
            UsageHook::Keep,
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
            UsageHook::Keep,
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
        let message = read_settings(&link).unwrap_err().to_string();
        assert!(message.contains("symlink"));
        assert!(message.contains("Edit the target manually"));
        assert!(message.contains(original.to_str().unwrap()));
        assert_eq!(fs::read_link(&link).unwrap(), original);
        assert_eq!(fs::read_to_string(&original).unwrap(), "{}");
    }
    #[test]
    fn backups_are_private_and_over_threshold_publication_preserves_every_copy() {
        let root = TestDirectory::new();
        let path = root.path.join("settings.json");
        let original = "{\r\n \"user\": 1.000e+100\r\n}\r\n";
        fs::write(&path, original).unwrap();
        let legacy = root.path.join("settings.tmt-backup-legacy.json");
        fs::write(&legacy, b"legacy bytes").unwrap();
        let planned = plan(
            &claude::DRIVER,
            path.clone(),
            read_settings(&path).unwrap(),
            "/stable/tmt".into(),
            false,
            UsageHook::Keep,
        )
        .unwrap();
        let backup = apply(&planned).unwrap().unwrap();
        let directory = backup.parent().unwrap();
        assert_eq!(directory, root.path.join(BACKUP_DIRECTORY));
        assert_eq!(fs::metadata(directory).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&backup).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::read(&backup).unwrap(), original.as_bytes());
        for index in 1..32 {
            fs::write(
                directory.join(format!("settings.tmt-backup-{index}.json")),
                b"retained",
            )
            .unwrap();
        }
        assert!(backup_warning(&path).is_none());
        let removing = plan(
            &claude::DRIVER,
            path.clone(),
            read_settings(&path).unwrap(),
            "/stable/tmt".into(),
            true,
            UsageHook::Keep,
        )
        .unwrap();
        let installed = fs::read(&path).unwrap();
        let removed_backup = apply(&removing).unwrap().unwrap();
        assert_eq!(fs::read(&removed_backup).unwrap(), installed);
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
        let warning = backup_warning(&path).unwrap();
        assert!(warning.contains(directory.to_str().unwrap()));
        assert!(warning.contains("manually remove only"));
        assert_eq!(fs::read_dir(directory).unwrap().count(), 33);
        assert_eq!(fs::read(&backup).unwrap(), original.as_bytes());
        assert_eq!(fs::read(&legacy).unwrap(), b"legacy bytes");
        for index in 1..32 {
            assert_eq!(
                fs::read(directory.join(format!("settings.tmt-backup-{index}.json"))).unwrap(),
                b"retained"
            );
        }
    }

    #[test]
    fn settings_changed_to_a_link_after_consent_preserves_link_and_target() {
        let root = TestDirectory::new();
        let path = root.path.join("settings.json");
        fs::write(&path, "{}").unwrap();
        let planned = plan(
            &claude::DRIVER,
            path.clone(),
            read_settings(&path).unwrap(),
            "/stable/tmt".into(),
            false,
            UsageHook::Keep,
        )
        .unwrap();
        let target = root.path.join("target.json");
        fs::write(&target, b"target bytes").unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(apply(&planned).unwrap_err().to_string().contains("symlink"));
        assert_eq!(fs::read_link(&path).unwrap(), target);
        assert_eq!(fs::read(&target).unwrap(), b"target bytes");
        assert!(!root.path.join(BACKUP_DIRECTORY).exists());
    }

    #[test]
    fn occupied_backup_directory_never_redirects_or_publishes_settings() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for linked in [false, true] {
            let root = TestDirectory::new();
            let path = root.path.join("settings.json");
            fs::write(&path, "{}").unwrap();
            let planned = plan(
                &claude::DRIVER,
                path.clone(),
                read_settings(&path).unwrap(),
                "/stable/tmt".into(),
                false,
                UsageHook::Keep,
            )
            .unwrap();
            let directory = root.path.join(BACKUP_DIRECTORY);
            let target = root.path.join("outside");
            fs::create_dir(&target).unwrap();
            if linked {
                symlink(&target, &directory).unwrap();
            } else {
                fs::create_dir(&directory).unwrap();
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
            }
            assert!(
                apply(&planned)
                    .unwrap_err()
                    .to_string()
                    .contains("private directory")
            );
            assert_eq!(fs::read(&path).unwrap(), b"{}");
            assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        }
    }

    #[test]
    fn named_hooks_provider_reads_and_applies_through_symlink_preserving_link_and_backup() {
        use crate::drivers::agy;
        let root = TestDirectory::new();
        let checkout = root.path.join("checkout");
        let config = root.path.join(".gemini/config");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&config).unwrap();
        let target = checkout.join("hooks.json");
        let original = "{\n  \"hooks\": {\"SessionStart\": []}\n}\n";
        fs::write(&target, original).unwrap();
        let link = config.join("hooks.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let before = read_provider_settings(&agy::DRIVER, &link).unwrap();
        assert_eq!(before.as_deref(), Some(original));
        let planned = plan(
            &agy::DRIVER,
            link.clone(),
            before,
            "/stable/tmt".into(),
            false,
            UsageHook::Default,
        )
        .unwrap();
        assert_eq!(
            planned.change.resolved_target.as_deref(),
            Some(fs::canonicalize(&target).unwrap().as_path())
        );
        let backup = apply(&planned).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).unwrap(), target);
        assert_eq!(fs::read_to_string(&target).unwrap(), planned.change.after);
        assert!(planned.change.after.contains("\"tmt\""));
        assert!(planned.change.after.contains("\"hooks\""));

        // Removing TMT hooks preserves the symlink and restores the original content.
        let removing = plan(
            &agy::DRIVER,
            link.clone(),
            read_provider_settings(&agy::DRIVER, &link).unwrap(),
            "/stable/tmt".into(),
            true,
            UsageHook::Keep,
        )
        .unwrap();
        apply(&removing).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
    }

    /// A link in `config` to `target`, the way `agy` shares gemini-cli's hooks file.
    fn linked(root: &TestDirectory, original: &str) -> (PathBuf, PathBuf) {
        let checkout = root.path.join("checkout");
        let config = root.path.join(".gemini/config");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&config).unwrap();
        let target = checkout.join("hooks.json");
        fs::write(&target, original).unwrap();
        let link = config.join("hooks.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        (link, target)
    }

    fn planned(link: &Path, before: Option<String>) -> Result<SetupPlan, super::super::PlanError> {
        use crate::drivers::agy;
        plan(
            &agy::DRIVER,
            link.to_path_buf(),
            before,
            "/stable/tmt".into(),
            false,
            UsageHook::Default,
        )
    }

    #[test]
    fn a_link_that_does_not_resolve_to_a_regular_file_is_refused_and_never_replaced() {
        use crate::drivers::agy;
        let root = TestDirectory::new();
        let config = root.path.join(".gemini/config");
        fs::create_dir_all(&config).unwrap();
        let link = config.join("hooks.json");

        // Dangling.
        std::os::unix::fs::symlink(root.path.join("missing.json"), &link).unwrap();
        assert!(read_provider_settings(&agy::DRIVER, &link).is_err());
        assert_eq!(
            planned(&link, None).unwrap_err(),
            super::super::PlanError::UnresolvedLink
        );
        // Looping.
        fs::remove_file(&link).unwrap();
        let other = config.join("other.json");
        std::os::unix::fs::symlink(&other, &link).unwrap();
        std::os::unix::fs::symlink(&link, &other).unwrap();
        assert!(read_provider_settings(&agy::DRIVER, &link).is_err());
        assert_eq!(
            planned(&link, None).unwrap_err(),
            super::super::PlanError::UnresolvedLink
        );
        // A directory.
        fs::remove_file(&link).unwrap();
        fs::remove_file(&other).unwrap();
        let directory = root.path.join("a-directory");
        fs::create_dir(&directory).unwrap();
        std::os::unix::fs::symlink(&directory, &link).unwrap();
        assert!(read_provider_settings(&agy::DRIVER, &link).is_err());
        assert_eq!(
            planned(&link, None).unwrap_err(),
            super::super::PlanError::UnresolvedLink
        );
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn claude_and_codex_keep_refusing_a_symlinked_settings_file() {
        use crate::drivers::{claude, codex};
        let root = TestDirectory::new();
        let (link, target) = linked(&root, "{}\n");
        for driver in [&claude::DRIVER, &codex::DRIVER] {
            let error = read_provider_settings(driver, &link).unwrap_err();
            assert!(error.to_string().contains("will not follow"), "{error}");
        }
        assert_eq!(fs::read_to_string(&target).unwrap(), "{}\n");
    }

    #[test]
    fn a_link_retargeted_after_planning_is_refused_with_nothing_written() {
        use crate::drivers::agy;
        let root = TestDirectory::new();
        let original = "{\n  \"hooks\": {}\n}\n";
        let (link, target) = linked(&root, original);
        let before = read_provider_settings(&agy::DRIVER, &link).unwrap();
        let plan = planned(&link, before).unwrap();
        let elsewhere = root.path.join("elsewhere.json");
        fs::write(&elsewhere, original).unwrap();
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        let error = apply(&plan).unwrap_err();
        assert!(
            error.to_string().contains("changed after planning"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), original);
        assert!(!link.parent().unwrap().join(BACKUP_DIRECTORY).exists());
    }

    #[test]
    fn a_target_directory_that_takes_no_file_is_refused_before_any_backup() {
        use crate::drivers::agy;
        use std::os::unix::fs::PermissionsExt;
        let root = TestDirectory::new();
        let original = "{\n  \"hooks\": {}\n}\n";
        let (link, target) = linked(&root, original);
        let before = read_provider_settings(&agy::DRIVER, &link).unwrap();
        let plan = planned(&link, before).unwrap();
        let directory = target.parent().unwrap();
        fs::set_permissions(directory, fs::Permissions::from_mode(0o500)).unwrap();
        let result = apply(&plan);
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.unwrap_err();
        assert!(error.to_string().contains("Cannot write beside"), "{error}");
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
        assert!(!link.parent().unwrap().join(BACKUP_DIRECTORY).exists());
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn the_hooks_key_beside_tmts_named_entry_is_left_byte_for_byte() {
        use crate::drivers::agy;
        let root = TestDirectory::new();
        let original = "{\n  \"hooks\": {\"SessionStart\": [{\"command\": \"gemini-own\"}]}\n}\n";
        let (link, target) = linked(&root, original);
        let before = read_provider_settings(&agy::DRIVER, &link).unwrap();
        apply(&planned(&link, before).unwrap()).unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
        let untouched: serde_json::Value = serde_json::from_str(original).unwrap();
        // gemini-cli reads only the top-level "hooks" key; TMT's entries sit under "tmt".
        assert_eq!(written["hooks"], untouched["hooks"]);
        assert!(written["tmt"].is_object());
    }
}
