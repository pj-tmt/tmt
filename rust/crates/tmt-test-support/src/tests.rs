use super::*;
use std::{
    fs,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

struct PublicationDirectory(PathBuf);
impl PublicationDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-fixture-publication-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for PublicationDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn exact_bytes_and_caller_modes_survive_paths_with_shell_metacharacters() {
    let root = PublicationDirectory::new();
    for mode in [0o700, 0o755] {
        let path = root
            .0
            .join(format!("fixture {mode:o} ' \" ; $(touch injected)"));
        let bytes = b"#!/bin/sh\nprintf '%s' 'fixture ran'\n";
        write_executable(&path, bytes, mode).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            mode
        );
        assert_eq!(
            fs::read_dir(&root.0).unwrap().count(),
            if mode == 0o700 { 1 } else { 2 }
        );
        let output = Command::new(&path).current_dir(&root.0).output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"fixture ran");
        assert!(output.stderr.is_empty());
        assert!(!root.0.join("injected").exists());
    }
    let bytes = b"\0\xff\r\n";
    let path = root.0.join("opaque");
    write_executable(&path, bytes, 0o700).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn writer_failure_preserves_existing_directory_and_reports_the_destination() {
    let root = PublicationDirectory::new();
    let path = root.0.join("missing/fixture");
    let error = write_executable(&path, b"payload", 0o755).unwrap_err();
    assert!(error.to_string().contains(&path.display().to_string()));
    assert!(!path.exists());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    let path = root.0.join("directory");
    fs::create_dir(&path).unwrap();
    let error = write_executable(&path, b"payload", 0o700).unwrap_err();
    assert!(error.to_string().contains(&path.display().to_string()));
    assert!(path.is_dir());
    assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
}

#[test]
fn blocked_fifo_writer_times_out_with_confirmed_group_cleanup() {
    let root = PublicationDirectory::new();
    let path = root.0.join("blocked");
    assert!(
        Command::new("/usr/bin/mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let started = Instant::now();
    let error = write_executable(&path, b"payload", 0o700).unwrap_err();
    let cause = error
        .get_ref()
        .unwrap()
        .downcast_ref::<tmt_invoke::InvokeError>()
        .unwrap();
    assert!(matches!(cause.kind, tmt_invoke::FailureKind::Deadline));
    assert!(matches!(cause.cleanup, tmt_invoke::Cleanup::Confirmed));
    assert!(started.elapsed() < Duration::from_secs(10));
    // No reader existed: the shell was blocked before opening the FIFO for writing.
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn an_independent_held_writer_still_refuses_exec_without_retry() {
    let root = PublicationDirectory::new();
    let path = root.0.join("fixture");
    let writer = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    write_executable(&path, b"#!/bin/sh\nexit 0\n", 0o700).unwrap();
    let error = Command::new(&path)
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ExecutableFileBusy);
    drop(writer);
    assert!(Command::new(&path).status().unwrap().success());
}
