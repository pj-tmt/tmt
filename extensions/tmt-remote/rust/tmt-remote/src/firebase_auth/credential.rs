//! One owner-private file per project. Readers and compare/replace writers share
//! its lock. Replacement follows Remote's deploy-record publication order.
use super::{
    AuthError, AuthErrorKind, CREDENTIAL_BYTES, CREDENTIAL_LIFETIME_MS, Credential,
    CredentialStore, StoreRead,
};
use crate::{state::Layout, store::uuid_v4};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

pub struct FileStore {
    remote: PathBuf,
    project: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    version: u8,
    project_id: String,
    uid: String,
    provider: String,
    refresh_token: String,
    created_at_ms: u64,
    expires_at_ms: u64,
    last_used_at_ms: u64,
    state: String,
}
fn invalid() -> AuthError {
    AuthError::new(AuthErrorKind::InvalidCredential)
}
fn unavailable() -> AuthError {
    AuthError::unavailable()
}
impl FileStore {
    pub fn new(layout: &Layout, project: &str) -> Result<Self, AuthError> {
        if !crate::deploy_run::project_ok(project) {
            return Err(AuthError::new(AuthErrorKind::InputInvalid));
        }
        Ok(Self {
            remote: layout.directory.clone(),
            project: project.into(),
        })
    }
    fn directory(&self, create: bool) -> Result<Option<tmt_extension_state::Layout>, AuthError> {
        // The flat credential name is validated by the Remote owner. The leaf
        // still applies its unchanged strict admission to the fixed directory.
        if create {
            tmt_extension_state::Layout::open(&self.remote, "firebase-auth", &[])
                .map(Some)
                .map_err(|_| invalid())
        } else {
            tmt_extension_state::Layout::existing(&self.remote, "firebase-auth", &[])
                .map_err(|_| invalid())
        }
    }
    fn path(&self, dir: &tmt_extension_state::Layout) -> PathBuf {
        dir.directory.join(format!("{}.json", self.project))
    }
    fn lock(&self, dir: &tmt_extension_state::Layout) -> Result<File, AuthError> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(dir.directory.join(format!("{}.lock", self.project)))
            .map_err(|_| invalid())?;
        admit(&lock, false)?;
        lock.try_lock().map_err(|_| unavailable())?;
        Ok(lock)
    }
    fn read(&self, dir: &tmt_extension_state::Layout) -> Result<StoreRead, AuthError> {
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(self.path(dir))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoreRead {
                    credential: None,
                    permissions_repaired: false,
                });
            }
            Err(_) => return Err(invalid()),
        };
        let repaired = admit(&file, true)?;
        if file.metadata().map_err(|_| invalid())?.len() > CREDENTIAL_BYTES as u64 {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(CREDENTIAL_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() > CREDENTIAL_BYTES {
            return Err(invalid());
        }
        let document: Document = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let state_ok = matches!(document.state.as_str(), "active" | "revoking");
        if document.version != 1
            || document.project_id != self.project
            || document.provider != "anonymous"
            || !state_ok
            || document.uid.is_empty()
            || document.uid.len() > 128
            || document.uid.chars().any(char::is_control)
            || document.refresh_token.is_empty()
            || document.refresh_token.len() > 8192
            || document.refresh_token.chars().any(char::is_control)
            || document.created_at_ms.checked_add(CREDENTIAL_LIFETIME_MS)
                != Some(document.expires_at_ms)
            || document.last_used_at_ms < document.created_at_ms
            || document.last_used_at_ms > document.expires_at_ms
        {
            return Err(invalid());
        }
        Ok(StoreRead {
            permissions_repaired: repaired,
            credential: Some(Credential {
                project_id: document.project_id,
                uid: document.uid,
                refresh_token: document.refresh_token,
                created_at_ms: document.created_at_ms,
                expires_at_ms: document.expires_at_ms,
                last_used_at_ms: document.last_used_at_ms,
                revoking: document.state == "revoking",
            }),
        })
    }
    fn clean_staging(&self, dir: &tmt_extension_state::Layout) -> Result<(), AuthError> {
        let prefix = format!(".{}-", self.project);
        let mut removed = false;
        for entry in fs::read_dir(&dir.directory).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            let name = entry.file_name();
            let Some(suffix) = name.to_str().and_then(|name| name.strip_prefix(&prefix)) else {
                continue;
            };
            if crate::canonical::uuid(suffix).is_err() {
                continue;
            }
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
                .open(entry.path())
                .map_err(|_| invalid())?;
            admit(&file, false)?;
            if file.metadata().map_err(|_| invalid())?.len() > CREDENTIAL_BYTES as u64 {
                return Err(invalid());
            }
            fs::remove_file(entry.path()).map_err(|_| unavailable())?;
            removed = true;
        }
        if removed {
            dir.sync().map_err(|_| unavailable())?;
        }
        Ok(())
    }
}
fn admit_owner(owner: u32, current: u32) -> Result<(), AuthError> {
    if owner != current {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn admit(file: &File, repair: bool) -> Result<bool, AuthError> {
    let m = file.metadata().map_err(|_| invalid())?;
    admit_owner(m.uid(), nix::unistd::Uid::effective().as_raw())?;
    if !m.is_file() {
        return Err(invalid());
    }
    let mode = m.mode() & 0o777;
    if mode == 0o600 {
        return Ok(false);
    }
    // Only this credential file may be repaired, and only a readable owner
    // file whose mode drifted looser. Strict directory/lock admission stays put.
    if repair && mode == 0o400 {
        return Ok(false);
    }
    if !repair || mode & 0o400 == 0 {
        return Err(invalid());
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| invalid())?;
    file.sync_all().map_err(|_| unavailable())?;
    Ok(true)
}
impl CredentialStore for FileStore {
    fn load(&self) -> Result<StoreRead, AuthError> {
        let Some(dir) = self.directory(false)? else {
            return Ok(StoreRead {
                credential: None,
                permissions_repaired: false,
            });
        };
        let _lock = self.lock(&dir)?;
        self.clean_staging(&dir)?;
        self.read(&dir)
    }
    fn replace(&self, expected: Option<&Credential>, next: &Credential) -> Result<(), AuthError> {
        if next.project_id != self.project {
            return Err(invalid());
        }
        let dir = self.directory(true)?.ok_or_else(unavailable)?;
        let _lock = self.lock(&dir)?;
        if self.read(&dir)?.credential.as_ref() != expected {
            return Err(AuthError::state_changed());
        }
        self.clean_staging(&dir)?;
        let document = Document {
            version: 1,
            project_id: next.project_id.clone(),
            uid: next.uid.clone(),
            provider: "anonymous".into(),
            refresh_token: next.refresh_token.clone(),
            created_at_ms: next.created_at_ms,
            expires_at_ms: next.expires_at_ms,
            last_used_at_ms: next.last_used_at_ms,
            state: if next.revoking { "revoking" } else { "active" }.into(),
        };
        let bytes = serde_json::to_vec(&document).map_err(|_| invalid())?;
        if bytes.len() > CREDENTIAL_BYTES {
            return Err(invalid());
        }
        let mut staged = Staged {
            path: dir.directory.join(format!(
                ".{}-{}",
                self.project,
                uuid_v4().map_err(|_| unavailable())?
            )),
            published: false,
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(&staged.path)
            .map_err(|_| unavailable())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| unavailable())?;
        fs::rename(&staged.path, self.path(&dir)).map_err(|_| unavailable())?;
        staged.published = true;
        // After rename the new generation may be visible even if dir-sync fails.
        dir.sync().map_err(|_| unavailable().uncertain())
    }
    fn remove(&self, expected: &Credential) -> Result<(), AuthError> {
        let Some(dir) = self.directory(false)? else {
            return Err(AuthError::state_changed());
        };
        let _lock = self.lock(&dir)?;
        if self.read(&dir)?.credential.as_ref() != Some(expected) {
            return Err(AuthError::state_changed());
        }
        self.clean_staging(&dir)?;
        fs::remove_file(self.path(&dir)).map_err(|_| unavailable())?;
        dir.sync().map_err(|_| unavailable().uncertain())
    }
}
struct Staged {
    path: PathBuf,
    published: bool,
}
impl Drop for Staged {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_credential_owner_is_an_unreadable_diagnostic() {
        // Inject only the UID comparison: changing ownership of a fixture on
        // a non-root developer host would require privileges outside the test.
        let error = admit_owner(1001, 1002).unwrap_err();
        assert_eq!(error.kind, AuthErrorKind::InvalidCredential);
        assert_eq!(
            serde_json::to_value(super::super::presentation::diagnostic(&error)).unwrap(),
            serde_json::json!({"state":"unreadable","expiresAtMs":null})
        );
        assert!(error.to_string().contains("file ownership"));
        assert!(admit_owner(1001, 1001).is_ok());
    }
}
