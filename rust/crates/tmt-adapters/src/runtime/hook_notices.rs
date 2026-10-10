//! Disposable restart advisories, separate from delivery claims and settlement.
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
    time::{Duration, Instant, SystemTime},
};
use tmt_core::binding::session::{HarnessId, ProviderSessionId};

const RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Only the successful creator may publish. A failed later write loses this
/// best-effort advisory for the session; it never changes delivery evidence.
pub fn claim(
    directory: &Path,
    provider: &HarnessId,
    session: &ProviderSessionId,
    deadline: Instant,
) -> bool {
    let result = (|| -> std::io::Result<()> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir()
            || metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || Instant::now() >= deadline
        {
            return Err(std::io::Error::other(
                "Unavailable private notice directory",
            ));
        }
        let digest = tmt_core::content_digest::sha256(session.as_str().as_bytes());
        let marker = directory.join(format!("{}-{}", provider.as_str(), &digest[..32]));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags((nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK).bits())
            .open(marker)?;
        remove_old_markers(directory, SystemTime::now(), deadline);
        Ok(())
    })();
    result.is_ok()
}

fn remove_old_markers(directory: &Path, now: SystemTime, deadline: Instant) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if Instant::now() >= deadline {
            break;
        }
        let path = entry.path();
        let owned_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.rsplit_once('-').is_some_and(|(provider, digest)| {
                    HarnessId::new(provider).is_ok()
                        && digest.len() == 32
                        && digest
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
            });
        if owned_name
            && fs::symlink_metadata(&path).is_ok_and(|metadata| {
                metadata.is_file()
                    && metadata.uid() == nix::unistd::geteuid().as_raw()
                    && metadata.len() == 0
                    && metadata.mode() & 0o077 == 0
                    && metadata
                        .modified()
                        .ok()
                        .and_then(|modified| now.duration_since(modified).ok())
                        .is_some_and(|age| age > RETENTION)
            })
        {
            let _ = fs::remove_file(path);
        }
    }
}

/// Both first-party Stop protocols expose this user-visible field. Preserve all
/// existing decision fields and decline an occupied field or non-object output.
pub fn system_message(message: &str, decision: Option<&str>) -> Option<String> {
    let mut value = match decision {
        Some(decision) => serde_json::from_str::<serde_json::Value>(decision).ok()?,
        None => serde_json::json!({}),
    };
    let object = value.as_object_mut()?;
    if object.contains_key("systemMessage") {
        return None;
    }
    object.insert("systemMessage".into(), message.into());
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Barrier},
        thread,
    };
    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("tmt-hook-notices-{}", uuid::Uuid::new_v4())))
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn claim_session(path: &Path, provider: &str, session: &str) -> bool {
        claim(
            path,
            &HarnessId::new(provider).unwrap(),
            &ProviderSessionId::new(session).unwrap(),
            Instant::now() + Duration::from_secs(1),
        )
    }
    #[test]
    fn concurrent_callbacks_elect_one_writer_and_keep_sessions_and_providers_separate() {
        let scratch = Scratch::new();
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = scratch.0.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    claim_session(&path, "claude", "session-a")
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| usize::from(h.join().unwrap()))
                .sum::<usize>(),
            1
        );
        assert!(!claim_session(&scratch.0, "claude", "session-a"));
        assert!(claim_session(&scratch.0, "codex", "session-a"));
        assert!(claim_session(&scratch.0, "claude", "session-b"));
        assert_eq!(fs::metadata(&scratch.0).unwrap().mode() & 0o777, 0o700);
        for entry in fs::read_dir(&scratch.0).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_name().to_string_lossy().contains("session"));
            let metadata = entry.metadata().unwrap();
            assert_eq!(metadata.len(), 0);
            assert_eq!(metadata.mode() & 0o777, 0o600);
        }
    }
    #[test]
    fn unsafe_directory_and_expired_budget_skip_without_a_marker() {
        let scratch = Scratch::new();
        fs::create_dir(&scratch.0).unwrap();
        fs::set_permissions(
            &scratch.0,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        assert!(!claim_session(&scratch.0, "claude", "session"));
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
        fs::set_permissions(
            &scratch.0,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        assert!(!claim(
            &scratch.0,
            &HarnessId::new("claude").unwrap(),
            &ProviderSessionId::new("session").unwrap(),
            Instant::now()
        ));
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
    }
    #[test]
    fn cleanup_removes_only_old_empty_owned_markers() {
        let scratch = Scratch::new();
        assert!(claim_session(&scratch.0, "claude", "old"));
        let old = fs::read_dir(&scratch.0)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let file = OpenOptions::new().write(true).open(&old).unwrap();
        file.set_times(
            std::fs::FileTimes::new()
                .set_modified(SystemTime::now() - RETENTION - Duration::from_secs(1)),
        )
        .unwrap();
        fs::write(scratch.0.join("user-file"), "keep").unwrap();
        let link = scratch.0.join(format!("claude-{}", "a".repeat(32)));
        std::os::unix::fs::symlink(&old, &link).unwrap();
        assert!(claim_session(&scratch.0, "claude", "new"));
        assert!(!old.exists());
        assert!(fs::symlink_metadata(link).unwrap().is_symlink());
        assert_eq!(
            fs::read_to_string(scratch.0.join("user-file")).unwrap(),
            "keep"
        );
    }
    #[test]
    fn notice_merges_into_one_object_without_changing_the_decision() {
        let value: serde_json::Value = serde_json::from_str(
            &system_message("restart", Some(r#"{"decision":"block","reason":"held"}"#)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"decision":"block","reason":"held","systemMessage":"restart"})
        );
        assert_eq!(
            system_message("restart", None).unwrap(),
            r#"{"systemMessage":"restart"}"#
        );
        assert!(system_message("restart", Some("{} {}")).is_none());
        assert!(system_message("restart", Some("[]")).is_none());
        assert!(system_message("restart", Some(r#"{"systemMessage":"owned"}"#)).is_none());
    }
}
