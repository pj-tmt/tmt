//! One owner-private file per project. Readers and compare/replace writers share
//! its lock. Replacement follows Remote's deploy-record publication order.
use super::{
    AuthError, AuthErrorKind, CREDENTIAL_BYTES, CREDENTIAL_LIFETIME_MS, Cancellation, Credential,
    CredentialStore, StoreRead,
};
use crate::{state::Layout, store::uuid_v4};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, TryLockError, atomic::AtomicBool},
    time::Duration,
};
use tmt_extension_state::{WRITE_WAIT, WriteWait};

pub struct FileStore {
    remote: PathBuf,
    project: String,
    handle: Mutex<Option<File>>,
    // Tests shorten the shared write wait; production always uses WRITE_WAIT.
    wait: Duration,
    #[cfg(test)]
    file_lock_waits: std::sync::atomic::AtomicUsize,
}
// Callers without their own budget wait the whole write wait.
static NEVER: AtomicBool = AtomicBool::new(false);
// The cached file description stays with FileStore. A guard releases flock
// before its mutex, including error paths, even when a fork still has a copy.
struct CredentialLock<'a> {
    handle: Option<MutexGuard<'a, Option<File>>>,
}
impl std::ops::Deref for CredentialLock<'_> {
    type Target = File;
    fn deref(&self) -> &Self::Target {
        self.handle
            .as_ref()
            .expect("credential lock guard is active")
            .as_ref()
            .expect("credential lock handle was initialized")
    }
}
impl CredentialLock<'_> {
    fn release(mut self) -> Result<(), AuthError> {
        let handle = self.handle.take().expect("credential lock guard is active");
        handle
            .as_ref()
            .expect("credential lock handle was initialized")
            .unlock()
            .map_err(|_| unavailable())
    }
}
impl Drop for CredentialLock<'_> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // Keep the primary operation error on an early return or unwind.
            // Normal completion uses release() and reports an unlock failure.
            let _ = handle
                .as_ref()
                .expect("credential lock handle was initialized")
                .unlock();
        }
    }
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
            handle: Mutex::new(None),
            wait: WRITE_WAIT,
            #[cfg(test)]
            file_lock_waits: Default::default(),
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
    // Load, replace and remove read or change data, so they wait for another
    // holder of the mutex or the file lock within one write wait, or until the
    // caller's `cancel`. A poisoned mutex or a changed lock path still refuses
    // at once and clears the handle.
    fn lock(
        &self,
        dir: &tmt_extension_state::Layout,
        cancel: &dyn Cancellation,
    ) -> Result<CredentialLock<'_>, AuthError> {
        let pending = WriteWait::start(self.wait);
        let mut handle = loop {
            match self.handle.try_lock() {
                Ok(handle) => break handle,
                Err(TryLockError::WouldBlock) => pause(&pending, cancel)?,
                Err(TryLockError::Poisoned(poisoned)) => {
                    let mut handle = poisoned.into_inner();
                    *handle = None;
                    self.handle.clear_poison();
                    return Err(unavailable());
                }
            }
        };
        let path = dir.directory.join(format!("{}.lock", self.project));
        if handle.is_none() {
            *handle = Some(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .mode(0o600)
                    .custom_flags(
                        nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC,
                    )
                    .open(&path)
                    .map_err(|_| invalid())?,
            );
        }
        let file = handle
            .as_ref()
            .expect("credential lock handle was initialized");
        if !same_lock(file, &path) {
            *handle = None;
            return Err(invalid());
        }
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(fs::TryLockError::WouldBlock) => {
                    #[cfg(test)]
                    self.file_lock_waits
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    pause(&pending, cancel)?
                }
                Err(fs::TryLockError::Error(_)) => return Err(unavailable()),
            }
        }
        // The path can change during the wait; a lock on its old inode excludes nobody.
        if !same_lock(file, &path) {
            let _ = file.unlock();
            *handle = None;
            return Err(invalid());
        }
        Ok(CredentialLock {
            handle: Some(handle),
        })
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
// A retained descriptor must never lock an old inode after its path is
// replaced. Any failed admission or metadata read also invalidates it.
fn same_lock(file: &File, path: &Path) -> bool {
    match (
        admit(file, false),
        fs::symlink_metadata(path),
        file.metadata(),
    ) {
        (Ok(_), Ok(current), Ok(opened)) => {
            current.is_file() && current.dev() == opened.dev() && current.ino() == opened.ino()
        }
        _ => false,
    }
}
// One pause between lock attempts: the caller's cancellation (a serving call's
// deadline or stop) ends the wait early, and the write wait bounds it.
fn pause(pending: &WriteWait, cancel: &dyn Cancellation) -> Result<(), AuthError> {
    if cancel.cancelled() {
        Err(AuthError::new(AuthErrorKind::Cancelled))
    } else if pending.pause() {
        Ok(())
    } else {
        Err(AuthError::lock_held())
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
        self.load_within(&NEVER)
    }
    fn replace(&self, expected: Option<&Credential>, next: &Credential) -> Result<(), AuthError> {
        self.replace_within(expected, next, &NEVER)
    }
    fn remove(&self, expected: &Credential) -> Result<(), AuthError> {
        self.remove_within(expected, &NEVER)
    }
    fn load_within(&self, cancel: &dyn Cancellation) -> Result<StoreRead, AuthError> {
        let Some(dir) = self.directory(false)? else {
            return Ok(StoreRead {
                credential: None,
                permissions_repaired: false,
            });
        };
        let lock = self.lock(&dir, cancel)?;
        self.clean_staging(&dir)?;
        let read = self.read(&dir)?;
        lock.release()?;
        Ok(read)
    }
    fn replace_within(
        &self,
        expected: Option<&Credential>,
        next: &Credential,
        cancel: &dyn Cancellation,
    ) -> Result<(), AuthError> {
        if next.project_id != self.project {
            return Err(invalid());
        }
        let dir = self.directory(true)?.ok_or_else(unavailable)?;
        let lock = self.lock(&dir, cancel)?;
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
        dir.sync().map_err(|_| unavailable().uncertain())?;
        lock.release().map_err(AuthError::uncertain)
    }
    fn remove_within(
        &self,
        expected: &Credential,
        cancel: &dyn Cancellation,
    ) -> Result<(), AuthError> {
        let Some(dir) = self.directory(false)? else {
            return Err(AuthError::state_changed());
        };
        let lock = self.lock(&dir, cancel)?;
        if self.read(&dir)?.credential.as_ref() != Some(expected) {
            return Err(AuthError::state_changed());
        }
        self.clean_staging(&dir)?;
        fs::remove_file(self.path(&dir)).map_err(|_| unavailable())?;
        dir.sync().map_err(|_| unavailable().uncertain())?;
        lock.release().map_err(AuthError::uncertain)
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

    struct PrivateRoot(PathBuf);
    impl Drop for PrivateRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (PrivateRoot, FileStore, Credential) {
        let root = PrivateRoot(
            std::env::temp_dir().join(format!("t4965-{}", crate::store::uuid_v4().unwrap())),
        );
        fs::create_dir(&root.0).unwrap();
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
        let layout = Layout::open(&root.0).unwrap();
        let store = FileStore::new(&layout, "demo-credential-lock").unwrap();
        let credential = Credential {
            project_id: "demo-credential-lock".into(),
            uid: "fixture-uid".into(),
            refresh_token: "fixture-refresh".into(),
            created_at_ms: 1,
            expires_at_ms: 1 + CREDENTIAL_LIFETIME_MS,
            last_used_at_ms: 1,
            revoking: false,
        };
        store.replace(None, &credential).unwrap();
        (root, store, credential)
    }
    // Keeps refusal tests fast; the contended success test keeps WRITE_WAIT.
    const SHORT_WAIT: Duration = Duration::from_millis(200);

    struct HeldCopy(std::process::Child);
    impl HeldCopy {
        fn start(file: File) -> Self {
            use std::{
                io::{BufRead, BufReader},
                process::{Command, Stdio},
            };
            let mut child = Self(
                Command::new("/bin/sh")
                    .args(["-c", "printf 'ready\\n'; IFS= read -r release"])
                    .env_clear()
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::from(file))
                    .spawn()
                    .unwrap(),
            );
            let mut ready = String::new();
            BufReader::new(child.0.stdout.take().unwrap())
                .read_line(&mut ready)
                .unwrap();
            assert_eq!(ready, "ready\n");
            child
        }
        fn release(mut self) {
            self.0
                .stdin
                .take()
                .unwrap()
                .write_all(b"release\n")
                .unwrap();
            assert!(self.0.wait().unwrap().success());
        }
    }
    impl Drop for HeldCopy {
        fn drop(&mut self) {
            if matches!(self.0.try_wait(), Ok(None)) {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }
    #[test]
    fn credential_release_ends_exclusion_while_a_fork_copy_remains_alive() {
        if crate::test_isolation::isolated_case(
            "firebase_auth::credential::tests::credential_release_ends_exclusion_while_a_fork_copy_remains_alive",
        ) {
            return;
        }
        let (root, mut store, credential) = fixture();
        store.wait = SHORT_WAIT;
        let dir = store.directory(false).unwrap().unwrap();
        let lock = store.lock(&dir, &NEVER).unwrap();
        let mut other = FileStore::new(
            &Layout::open(root.0.as_path()).unwrap(),
            "demo-credential-lock",
        )
        .unwrap();
        other.wait = SHORT_WAIT;
        assert_eq!(
            other.load().err(),
            Some(AuthError::lock_held()),
            "a genuine live owner must still exclude another credential owner"
        );
        assert_eq!(
            store.load().err(),
            Some(AuthError::lock_held()),
            "the retained description must not bypass another operation on the same store"
        );
        // Fork while the parent holds exclusion. The child's readiness barrier
        // proves it holds the duplicate before the parent ends its operation.
        let mut child = HeldCopy::start(File::try_clone(&lock).unwrap());
        drop(lock);
        let reloaded = store.load();
        let independent = other.load();
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "the duplicate must remain in the child through both acquisition attempts"
        );
        child.release();
        assert_eq!(
            reloaded.unwrap().credential,
            Some(credential.clone()),
            "a fork copy must not extend a completed credential operation"
        );
        assert_eq!(
            independent.unwrap().credential,
            Some(credential),
            "standalone command access must resume while the duplicate still lives"
        );
    }

    /// Waits until `store` has found the file lock held at least once, so the
    /// operation is waiting behind the holder. Returns false if it ended first.
    fn waits_behind_holder(
        store: &FileStore,
        operation: &std::thread::ScopedJoinHandle<'_, impl Sized>,
    ) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if store
                .file_lock_waits
                .load(std::sync::atomic::Ordering::SeqCst)
                > 0
            {
                return true;
            }
            if operation.is_finished() {
                return false;
            }
            std::thread::yield_now();
        }
        panic!("the contended operation neither waited nor ended");
    }

    #[test]
    fn contended_replace_waits_for_the_holder_and_then_succeeds() {
        let (root, store, credential) = fixture();
        let dir = store.directory(false).unwrap().unwrap();
        let other =
            FileStore::new(&Layout::open(&root.0).unwrap(), "demo-credential-lock").unwrap();
        let next = Credential {
            last_used_at_ms: 2,
            ..credential.clone()
        };
        let holder = other.lock(&dir, &NEVER).unwrap();
        std::thread::scope(|scope| {
            let replace = scope.spawn(|| store.replace(Some(&credential), &next));
            let waited = waits_behind_holder(&store, &replace);
            drop(holder);
            let result = replace.join().unwrap();
            assert_eq!(
                result,
                Ok(()),
                "a contended replace must wait for the holder"
            );
            assert!(waited, "the replace must have found the holder's lock");
        });
        assert_eq!(store.load().unwrap().credential, Some(next));
    }

    #[test]
    fn a_lock_path_replaced_during_the_wait_refuses_after_the_lock() {
        let (root, store, credential) = fixture();
        let dir = store.directory(false).unwrap().unwrap();
        let path = dir.directory.join("demo-credential-lock.lock");
        let other =
            FileStore::new(&Layout::open(&root.0).unwrap(), "demo-credential-lock").unwrap();
        let holder = other.lock(&dir, &NEVER).unwrap();
        std::thread::scope(|scope| {
            let load = scope.spawn(|| store.load());
            assert!(waits_behind_holder(&store, &load));
            fs::rename(&path, dir.directory.join("previous.lock")).unwrap();
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&path)
                .unwrap();
            drop(holder);
            assert_eq!(
                load.join().unwrap().err().unwrap().kind,
                AuthErrorKind::InvalidCredential,
                "a lock taken on the old inode must not admit the operation"
            );
        });
        assert_eq!(store.load().unwrap().credential, Some(credential));
    }

    #[test]
    fn a_cancelled_caller_stops_waiting_for_the_holder() {
        let (root, store, credential) = fixture();
        let dir = store.directory(false).unwrap().unwrap();
        let other =
            FileStore::new(&Layout::open(&root.0).unwrap(), "demo-credential-lock").unwrap();
        let holder = other.lock(&dir, &NEVER).unwrap();
        // A serving call whose deadline or stop has already come.
        let started = std::time::Instant::now();
        let error = store.load_within(&AtomicBool::new(true)).err().unwrap();
        assert!(
            started.elapsed() < WRITE_WAIT,
            "the cancel must end the wait"
        );
        assert_eq!(error.kind, AuthErrorKind::Cancelled);
        drop(holder);
        assert_eq!(store.load().unwrap().credential, Some(credential));
    }

    #[test]
    fn a_holder_past_the_write_wait_gets_a_named_refusal() {
        let (root, mut store, credential) = fixture();
        store.wait = SHORT_WAIT;
        let dir = store.directory(false).unwrap().unwrap();
        let other =
            FileStore::new(&Layout::open(&root.0).unwrap(), "demo-credential-lock").unwrap();
        let next = Credential {
            last_used_at_ms: 2,
            ..credential.clone()
        };
        // Another owner holds the file lock, then another operation on this
        // store holds its process mutex. Both waits end in the same refusal.
        for same_store in [false, true] {
            let holder = if same_store {
                store.lock(&dir, &NEVER).unwrap()
            } else {
                other.lock(&dir, &NEVER).unwrap()
            };
            let started = std::time::Instant::now();
            let error = store.replace(Some(&credential), &next).unwrap_err();
            let waited = started.elapsed();
            drop(holder);
            assert!(
                waited >= SHORT_WAIT,
                "refused after {waited:?}, before the wait"
            );
            assert!(
                waited < WRITE_WAIT,
                "the injected wait must bound the refusal"
            );
            assert_eq!(error, AuthError::lock_held());
            assert_eq!(error.kind, AuthErrorKind::Unavailable);
            assert!(!error.may_have_effect());
            assert_eq!(
                error.to_string(),
                "Another tmt operation held the Firestore sign-in lock under remote/firebase-auth for 5 seconds; nothing changed. Run the same command again after it ends. (REMOTE_STATE_UNAVAILABLE)"
            );
            assert_eq!(
                serde_json::to_value(super::super::presentation::diagnostic(&error)).unwrap(),
                serde_json::json!({"state":"busy","expiresAtMs":null})
            );
        }
        assert_eq!(store.load().unwrap().credential, Some(credential));
    }

    #[test]
    fn a_lock_refusal_names_the_wait_in_whole_seconds() {
        let line = |wait| AuthError::lock_held_for(wait).to_string();
        assert!(line(Duration::from_secs(1)).contains(" for 1 second; "));
        assert!(line(Duration::from_millis(200)).contains(" for 1 second; "));
        assert!(line(Duration::from_millis(1500)).contains(" for 2 seconds; "));
        assert!(line(WRITE_WAIT).contains(" for 5 seconds; "));
        assert_eq!(AuthError::lock_held_for(WRITE_WAIT), AuthError::lock_held());
    }

    #[test]
    fn retained_credential_handle_is_close_on_exec() {
        let (_root, store, _) = fixture();
        let cached = store.handle.try_lock().unwrap();
        let file = cached.as_ref().unwrap();
        let flags = nix::fcntl::fcntl(file, nix::fcntl::FcntlArg::F_GETFD).unwrap();
        assert_ne!(flags & nix::libc::FD_CLOEXEC, 0);
    }

    #[test]
    fn retained_credential_handle_refuses_a_replaced_or_symlinked_lock_path() {
        for symlink in [false, true] {
            let (_root, store, credential) = fixture();
            let dir = store.directory(false).unwrap().unwrap();
            let path = dir.directory.join("demo-credential-lock.lock");
            let original = dir.directory.join("original.lock");
            fs::rename(&path, &original).unwrap();
            if symlink {
                std::os::unix::fs::symlink(&original, &path).unwrap();
            } else {
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(&path)
                    .unwrap();
            }
            assert_eq!(
                store.load().err().unwrap().kind,
                AuthErrorKind::InvalidCredential
            );
            assert_eq!(
                store.read(&dir).unwrap().credential,
                Some(credential),
                "refusing the changed lock path must preserve the credential"
            );
        }
    }

    #[test]
    fn credential_handle_recovers_after_an_inode_mismatch_refusal() {
        let (root, mut store, credential) = fixture();
        store.wait = SHORT_WAIT;
        let dir = store.directory(false).unwrap().unwrap();
        let path = dir.directory.join("demo-credential-lock.lock");
        fs::rename(&path, dir.directory.join("previous.lock")).unwrap();
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        assert_eq!(
            store.load().err().unwrap().kind,
            AuthErrorKind::InvalidCredential
        );
        let other =
            FileStore::new(&Layout::open(&root.0).unwrap(), "demo-credential-lock").unwrap();
        let competing = other.lock(&dir, &NEVER).unwrap();
        assert_eq!(
            store.load().err(),
            Some(AuthError::lock_held()),
            "the reopened path must still respect a genuine competing owner"
        );
        drop(competing);
        assert_eq!(store.load().unwrap().credential, Some(credential));
    }

    #[test]
    fn credential_handle_recovers_after_the_lock_path_is_deleted() {
        let (_root, store, credential) = fixture();
        let dir = store.directory(false).unwrap().unwrap();
        let path = dir.directory.join("demo-credential-lock.lock");
        fs::remove_file(&path).unwrap();
        assert_eq!(
            store.load().err().unwrap().kind,
            AuthErrorKind::InvalidCredential
        );
        assert!(
            !path.exists(),
            "the refused operation must not recreate the lock"
        );
        assert_eq!(store.load().unwrap().credential, Some(credential));
        assert!(
            path.is_file(),
            "the next operation must recreate and admit the lock"
        );
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn credential_handle_recovers_after_a_poisoned_mutex_refusal() {
        let (_root, store, credential) = fixture();
        let dir = store.directory(false).unwrap().unwrap();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _lock = store.lock(&dir, &NEVER).unwrap();
            panic!("fixture poisons the credential operation guard");
        }));
        assert!(panic.is_err());
        assert!(store.handle.is_poisoned());
        assert_eq!(store.load().err().unwrap().kind, AuthErrorKind::Unavailable);
        assert_eq!(store.load().unwrap().credential, Some(credential));
    }
}
