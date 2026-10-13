//! The root-local stop request: `tmt colab stop` asks the serving process to shut down through
//! the owner-only serve socket. Browsers never reach this route (the mount refuses `/.tmt/`),
//! and a request carrying a forwarded device context is denied like every other local route.
use crate::{Result, keyring::Layout};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, atomic::AtomicBool};

pub const STOP_PATH: &str = "/.tmt/colab/local/stop";
/// How a serving process holds its Remote door; the door itself is described in `colab-v1`.
pub const DOORS: [&str; 3] = ["attached", "started", "unavailable"];

/// What a serving socket needs to answer a stop request.
#[derive(Clone)]
pub struct Control {
    /// One of [`DOORS`], so the stopping command can say what happens to the door.
    pub door: &'static str,
    /// Raised by a stop request; the accept loop treats it like SIGTERM.
    pub stopping: Arc<AtomicBool>,
}
impl Control {
    pub fn new() -> Self {
        Self {
            door: "unavailable",
            stopping: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopReply {
    stopping: bool,
    door: String,
}
impl StopReply {
    pub fn new(control: &Control) -> Self {
        Self {
            stopping: true,
            door: control.door.to_owned(),
        }
    }
    /// The door state the serving process reported, or `None` for an unrecognized reply.
    pub fn door(&self) -> Option<&'static str> {
        DOORS.into_iter().find(|door| *door == self.door)
    }
}

#[derive(Debug)]
pub enum StopFault {
    /// The serving process did not accept the request.
    Unavailable,
    /// It accepted the request but is still running after the bounded wait.
    Slow,
    /// The serve exited but cleanup is unknown, or its private failure cannot be read.
    CleanupUnconfirmed(String),
}
impl StopFault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable => "COLAB_UNAVAILABLE",
            Self::Slow | Self::CleanupUnconfirmed(_) => "COLAB_OUTCOME_UNKNOWN",
        }
    }
}
impl std::fmt::Display for StopFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Self::CleanupUnconfirmed(detail) = self {
            return write!(
                f,
                "Colab stopped, but Remote cleanup is unconfirmed (cleanup): {detail}. Check tmt remote status --json before retrying."
            );
        }
        f.write_str(match self {
            Self::Unavailable => "The serving Colab did not accept the stop request.",
            Self::Slow => {
                "Colab was asked to stop but is still running; check again before retrying."
            }
            Self::CleanupUnconfirmed(_) => unreachable!(),
        })
    }
}
impl std::error::Error for StopFault {}

/// Ask the serving process of this data root to stop. One bounded request, never retried.
pub fn request_stop(layout: &Layout) -> Result<StopReply> {
    let (code, body) =
        crate::ipc::exchange(layout, STOP_PATH, &[]).map_err(|_| StopFault::Unavailable)?;
    let reply: StopReply = serde_json::from_slice(&body).map_err(|_| StopFault::Unavailable)?;
    if code != 200 || !reply.stopping || reply.door().is_none() {
        return Err(StopFault::Unavailable.into());
    }
    Ok(reply)
}

/// A released serve lock is not proof of owned Remote cleanup if a failure was recorded.
pub fn confirm_cleanup(layout: &Layout) -> Result<()> {
    let unknown =
        || StopFault::CleanupUnconfirmed("the private cleanup record is unreadable".into());
    let bytes = layout.serve_failure().map_err(|_| unknown())?;
    let Some(bytes) = bytes.filter(|bytes| !bytes.is_empty()) else {
        return Ok(());
    };
    let record: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| unknown())?;
    if record["version"] != 1 || !record["phase"].is_string() {
        return Err(unknown().into());
    }
    if record["phase"] == "cleanup" {
        let detail = record["message"].as_str().ok_or_else(unknown)?;
        return Err(StopFault::CleanupUnconfirmed(detail.to_owned()).into());
    }
    Ok(())
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    struct Root(std::path::PathBuf);
    impl Root {
        fn new() -> Self {
            use std::{
                os::unix::fs::DirBuilderExt,
                sync::atomic::{AtomicUsize, Ordering},
            };
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "tmt-2899-cleanup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn missing_old_serve_record_and_empty_cleared_record_confirm_cleanup() {
        let root = Root::new();
        let layout = Layout::open(root.path()).unwrap();
        assert!(confirm_cleanup(&layout).is_ok());
        assert!(!root.path().join("colab/serve-error.json").exists());
        layout.file("serve-error.json").unwrap();
        assert!(confirm_cleanup(&layout).is_ok());
    }

    #[test]
    fn cleanup_failure_and_unreadable_record_refuse_success_with_manual_check() {
        let root = Root::new();
        let layout = Layout::open(root.path()).unwrap();
        let mut file = layout.file("serve-error.json").unwrap();
        file.write_all(br#"{"version":1,"phase":"cleanup","code":"COLAB_OUTCOME_UNKNOWN","message":"worker join timed out"}"#).unwrap();
        let error = confirm_cleanup(&layout).unwrap_err();
        assert_eq!(
            error.downcast_ref::<StopFault>().unwrap().code(),
            "COLAB_OUTCOME_UNKNOWN"
        );
        assert!(error.to_string().contains("cleanup"));
        assert!(error.to_string().contains("worker join timed out"));
        assert!(error.to_string().contains("tmt remote status --json"));
        file.set_len(0).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"not a record").unwrap();
        let error = confirm_cleanup(&layout).unwrap_err();
        assert!(error.to_string().contains("unreadable"));
        assert_eq!(
            error.downcast_ref::<StopFault>().unwrap().code(),
            "COLAB_OUTCOME_UNKNOWN"
        );
        drop(file);
        let path = root.path().join("colab/serve-error.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let error = confirm_cleanup(&layout).unwrap_err();
        assert!(error.to_string().contains("unreadable"));
        assert_eq!(
            error.downcast_ref::<StopFault>().unwrap().code(),
            "COLAB_OUTCOME_UNKNOWN"
        );
    }
}
