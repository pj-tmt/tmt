//! Nonsecret stable installation identity. Binding/secret state belongs to the vault.

use super::{OfficeError, wire::valid_uuid};
use crate::office_deployment::WorldTarget;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};
use tmt_adapters::bounded_file;
use tmt_adapters::bounded_file::FileReadError;
use tmt_adapters::config::ConfigPaths;
use tmt_adapters::file_lock;
use tmt_core::content_digest::sha256;

pub struct OfficeInstallation {
    directory: PathBuf,
    id: String,
}

impl OfficeInstallation {
    pub fn open(paths: &ConfigPaths, create: bool) -> Result<Option<Self>, OfficeError> {
        let directory = paths.office_directory();
        let mut created = false;
        if create {
            fs::create_dir_all(&paths.global_dir).map_err(unavailable)?;
            match fs::DirBuilder::new().mode(0o700).create(&directory) {
                Ok(()) => {
                    created = true;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(unavailable(error)),
            }
        }
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.is_dir() && metadata.permissions().mode() & 0o077 == 0 => {}
            Err(error) if !create && error.kind() == io::ErrorKind::NotFound => return Ok(None),
            _ => return Err(OfficeError::CredentialsUnavailable),
        }
        // Read-only operations do not create missing metadata or lock files.
        if !create {
            let id = read_id(&directory)?.ok_or(OfficeError::CredentialsInvalid)?;
            return Ok(Some(Self { directory, id }));
        }
        let _lock =
            file_lock::exclusive(&directory.join("installation.lock")).map_err(unavailable)?;
        let id = match read_id(&directory)? {
            Some(id) => id,
            None => {
                if !created {
                    // Missing metadata in an existing installation is not
                    // permission to orphan its protected entries under a new ID.
                    return Err(OfficeError::CredentialsInvalid);
                }
                let id = uuid::Uuid::new_v4().to_string();
                let staging = directory.join(format!(".installation-{}.tmp", uuid::Uuid::new_v4()));
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(&staging)
                    .map_err(unavailable)?;
                let result = file
                    .write_all(id.as_bytes())
                    .and_then(|()| file.sync_all())
                    .and_then(|()| fs::rename(&staging, directory.join("installation-id")))
                    .and_then(|()| File::open(&directory)?.sync_all());
                if result.is_err() && staging.exists() {
                    // This unique path belongs to this invocation, never a glob.
                    fs::remove_file(&staging).map_err(unavailable)?;
                }
                result.map_err(unavailable)?;
                id
            }
        };
        Ok(Some(Self { directory, id }))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn with_scope<T>(
        &self,
        target: &WorldTarget,
        identity_id: &str,
        operation: impl FnOnce(&str) -> Result<T, OfficeError>,
    ) -> Result<T, OfficeError> {
        let key = self.scope_key(target, identity_id)?;
        self.with_key(&key, || operation(&key))
    }

    pub(super) fn with_key<T>(
        &self,
        key: &str,
        operation: impl FnOnce() -> Result<T, OfficeError>,
    ) -> Result<T, OfficeError> {
        if !tmt_core::content_digest::is_sha256(key) {
            return Err(OfficeError::CredentialsInvalid);
        }
        let _lock = file_lock::exclusive(&self.directory.join(format!("{key}.lock")))
            .map_err(unavailable)?;
        operation()
    }

    pub fn scope_key(
        &self,
        target: &WorldTarget,
        identity_id: &str,
    ) -> Result<String, OfficeError> {
        if !valid_uuid(identity_id) {
            return Err(OfficeError::CredentialsInvalid);
        }
        let key = sha256(
            &serde_json::to_vec(&(
                target.origin(),
                target.world_id(),
                target.mode(),
                &self.id,
                identity_id,
            ))
            .map_err(|_| OfficeError::CredentialsInvalid)?,
        );
        Ok(key)
    }
}

fn read_id(directory: &std::path::Path) -> Result<Option<String>, OfficeError> {
    let bytes = match bounded_file::read_no_follow(&directory.join("installation-id"), 36) {
        Ok(bytes) => bytes,
        Err(FileReadError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(_) => return Err(OfficeError::CredentialsInvalid),
    };
    let id = String::from_utf8(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
    if !valid_uuid(&id) {
        return Err(OfficeError::CredentialsInvalid);
    }
    Ok(Some(id))
}

fn unavailable(error: io::Error) -> OfficeError {
    if error.kind() == io::ErrorKind::WouldBlock {
        OfficeError::Busy
    } else {
        OfficeError::CredentialsUnavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{office_deployment::DeploymentMode, test_support::TestDirectory};

    fn paths(root: &TestDirectory) -> ConfigPaths {
        ConfigPaths::resolve(&root.path, &root.path, Some(&root.path.join("app")), None)
    }

    #[test]
    fn missing_status_is_read_only_and_stable_id_survives_reopen() {
        let root = TestDirectory::new();
        let paths = paths(&root);
        assert!(OfficeInstallation::open(&paths, false).unwrap().is_none());
        assert!(!paths.global_dir.exists());
        let created = OfficeInstallation::open(&paths, true).unwrap().unwrap();
        let bytes = fs::read(paths.office_directory().join("installation-id")).unwrap();
        assert_eq!(bytes, created.id().as_bytes());
        assert_eq!(
            OfficeInstallation::open(&paths, false)
                .unwrap()
                .unwrap()
                .id(),
            created.id()
        );
        assert_eq!(
            OfficeInstallation::open(&paths, true)
                .unwrap()
                .unwrap()
                .id(),
            created.id()
        );
        assert!(!paths.database.exists());
    }

    #[test]
    fn missing_installation_id_is_not_regenerated() {
        let root = TestDirectory::new();
        let paths = paths(&root);
        OfficeInstallation::open(&paths, true).unwrap();
        let id_path = paths.office_directory().join("installation-id");
        fs::remove_file(&id_path).unwrap();
        assert!(matches!(
            OfficeInstallation::open(&paths, false),
            Err(OfficeError::CredentialsInvalid)
        ));
        assert!(matches!(
            OfficeInstallation::open(&paths, true),
            Err(OfficeError::CredentialsInvalid)
        ));
        assert!(!id_path.exists());
    }

    #[test]
    fn malformed_metadata_and_symlinks_are_never_replaced() {
        let root = TestDirectory::new();
        let paths = paths(&root);
        OfficeInstallation::open(&paths, true).unwrap();
        let id_path = paths.office_directory().join("installation-id");
        fs::write(&id_path, b"malformed").unwrap();
        assert!(matches!(
            OfficeInstallation::open(&paths, true),
            Err(OfficeError::CredentialsInvalid)
        ));
        assert_eq!(fs::read(&id_path).unwrap(), b"malformed");
        fs::remove_file(&id_path).unwrap();
        std::os::unix::fs::symlink(root.path.join("outside"), &id_path).unwrap();
        assert!(matches!(
            OfficeInstallation::open(&paths, true),
            Err(OfficeError::CredentialsInvalid)
        ));
        assert!(
            fs::symlink_metadata(&id_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!root.path.join("outside").exists());
    }

    #[test]
    fn scope_is_stable_isolated_and_exclusively_locked() {
        let root = TestDirectory::new();
        let installation = OfficeInstallation::open(&paths(&root), true)
            .unwrap()
            .unwrap();
        let target = WorldTarget::parse(
            "https://office.example/worlds/abcdefghijklmnopqrst",
            DeploymentMode::Cloud,
        )
        .unwrap();
        let identity = "00000000-0000-4000-8000-000000000001";
        let first = installation
            .with_scope(&target, identity, |key| {
                assert_eq!(
                    installation.with_scope(&target, identity, |_| Ok(())),
                    Err(OfficeError::Busy)
                );
                Ok(key.to_string())
            })
            .unwrap();
        assert_eq!(
            installation
                .with_scope(&target, identity, |key| Ok(key.to_string()))
                .unwrap(),
            first
        );
        assert_ne!(
            installation
                .with_scope(&target, "00000000-0000-4000-8000-000000000002", |key| Ok(
                    key.to_string()
                ))
                .unwrap(),
            first
        );
    }
}
