//! Crash barriers exist only in this unit-test executable, never in the library.
use super::*;
use std::{
    cell::RefCell,
    sync::atomic::{AtomicUsize, Ordering},
};
#[allow(
    dead_code,
    reason = "the shared child owner also serves integration scenarios"
)]
#[path = "../tests/support/mod.rs"]
mod support;

thread_local! { static STOP_AT: RefCell<Option<String>> = const { RefCell::new(None) }; }
pub(super) fn checkpoint(phase: &str) {
    if STOP_AT.with(|stop| stop.borrow().as_deref() == Some(phase)) {
        println!("STATE_{phase}");
        io::stdout().flush().unwrap();
        let mut line = String::new();
        io::stdin().read_line(&mut line).unwrap();
        panic!("crash barrier must be killed, not released");
    }
}
#[test]
fn process_worker() {
    let Some(root) = std::env::var_os("TMT_STATE_WORKER_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let phase = std::env::var("TMT_STATE_WORKER_MODE").unwrap();
    STOP_AT.with(|stop| *stop.borrow_mut() = Some(phase));
    let document = Document::new(
        &root.join("state"),
        &root.join("project/settings"),
        1024,
        "write-again",
    )
    .unwrap();
    println!("STATE_READY");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    document
        .update(|_| Ok::<_, io::Error>(b"new complete bytes".to_vec()))
        .unwrap();
}
#[test]
fn killed_publication_is_old_or_new_then_recovery_cleans_only_reserved_staging() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    for phase in ["created", "written", "synced", "published"] {
        let root = std::env::temp_dir().join(format!(
            "state-kill-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        // Created before the child so a panic still reaps it before directory cleanup.
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(root.clone());
        fs::create_dir(root.join("project")).unwrap();
        let target = root.join("project/settings");
        fs::write(&target, b"old complete bytes").unwrap();
        fs::write(root.join("project/foreign.tmp"), b"keep").unwrap();
        let mut child = support::Worker::spawn(&root, 0, "document::tests::process_worker", phase);
        child.ready();
        child.release();
        child.until(&format!("STATE_{phase}"));
        child.kill_and_reap();
        let expected = if phase == "published" {
            b"new complete bytes"
        } else {
            b"old complete bytes"
        };
        assert_eq!(fs::read(&target).unwrap(), expected);
        let document = Document::new(&root.join("state"), &target, 1024, "write-again").unwrap();
        if phase == "written" {
            document
                .update(|bytes| Ok::<_, io::Error>(bytes.unwrap().to_vec()))
                .unwrap();
        } else {
            document.recover().unwrap();
        }
        assert!(!staging_path(&target).exists());
        assert_eq!(fs::read(root.join("project/foreign.tmp")).unwrap(), b"keep");
        document
            .update(|bytes| {
                assert_eq!(bytes.unwrap(), expected);
                Ok::<_, io::Error>(b"next complete bytes".to_vec())
            })
            .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"next complete bytes");
        assert_eq!(fs::read_dir(root.join("project")).unwrap().count(), 2);
    }
}

#[test]
fn a_held_document_lock_times_out_before_the_callback() {
    let root = std::env::temp_dir().join(format!("state-document-timeout-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(root.clone());
    let target = root.join("settings");
    let document = Document::new(&root.join("state"), &target, 128, "retry-document").unwrap();
    let destination = document.destination().unwrap();
    let guard = document.acquire(&destination).unwrap();
    let mut invoked = false;
    let started = std::time::Instant::now();
    let second_target = root.join("second-settings");
    let second = Document::new(&root.join("state"), &second_target, 128, "retry-document").unwrap();
    let error = second
        .update(|_| {
            invoked = true;
            Ok::<_, io::Error>(vec![])
        })
        .unwrap_err();
    let UpdateError::State(error) = error else {
        panic!("expected lock timeout")
    };
    assert!(error.is_timeout());
    assert!(!error.may_not_be_saved());
    assert!(!invoked);
    assert!(started.elapsed() >= std::time::Duration::from_secs(4));
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(error.to_string().contains(second_target.to_str().unwrap()));
    assert!(error.to_string().contains("retry-document"));
    assert!(std::error::Error::source(&error).is_some());
    drop(guard);
    document
        .update(|_| Ok::<_, io::Error>(b"saved".to_vec()))
        .unwrap();
    assert_eq!(fs::read(target).unwrap(), b"saved");
}

thread_local! { static FAIL_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(super) fn directory_sync_result(result: io::Result<()>) -> io::Result<()> {
    if FAIL_DIRECTORY_SYNC.with(|fail| fail.replace(false)) {
        Err(io::Error::other("injected directory fsync failure"))
    } else {
        result
    }
}
#[test]
fn after_rename_failure_reports_uncertainty_with_new_bytes_and_no_staging() {
    let root = std::env::temp_dir().join(format!("state-document-sync-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(root.clone());
    let target = root.join("settings");
    fs::write(&target, b"old").unwrap();
    let document = Document::new(&root.join("state"), &target, 128, "retry-exact").unwrap();
    FAIL_DIRECTORY_SYNC.with(|fail| fail.set(true));
    let error = document
        .update(|_| Ok::<_, io::Error>(b"new".to_vec()))
        .unwrap_err();
    let UpdateError::State(error) = error else {
        panic!("expected publication error")
    };
    assert!(error.may_not_be_saved());
    assert!(error.to_string().contains("new bytes may already be saved"));
    assert!(error.to_string().contains("retry-exact"));
    assert_eq!(fs::read(&target).unwrap(), b"new");
    assert!(!staging_path(&target).exists());
    document
        .update(|_| Ok::<_, io::Error>(b"retry".to_vec()))
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"retry");
}

#[test]
fn document_admission_serializes_case_aliases_of_the_same_destination() {
    let root = std::env::temp_dir().join(format!("state-document-alias-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(root.clone());
    let target = root.join("settings");
    let alias = root.join("SETTINGS");
    fs::write(&target, b"old").unwrap();
    match fs::metadata(&alias) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            eprintln!("case-alias test skipped: this test volume is case-sensitive");
            return;
        }
        result => assert_eq!(result.unwrap().ino(), fs::metadata(&target).unwrap().ino()),
    }
    let document = Document::new(&root.join("state"), &target, 128, "retry-exact").unwrap();
    let alternate = Document::new(&root.join("state"), &alias, 128, "retry-exact").unwrap();
    let held = document.acquire(&document.destination().unwrap()).unwrap();
    let mut invoked = false;
    let error = alternate
        .update(|_| {
            invoked = true;
            Ok::<_, io::Error>(b"new".to_vec())
        })
        .unwrap_err();
    assert!(matches!(
        error,
        UpdateError::State(StateError::Timeout { .. })
    ));
    assert!(!invoked);
    assert_eq!(fs::read(&target).unwrap(), b"old");
    drop(held);
    fs::write(staging_path(&target), b"crashed alias staging").unwrap();
    alternate
        .update(|bytes| {
            assert_eq!(bytes.unwrap(), b"old");
            Ok::<_, io::Error>(b"new".to_vec())
        })
        .unwrap();
    assert!(!staging_path(&target).exists());
    assert_eq!(fs::read(target).unwrap(), b"new");
}

#[test]
fn document_admission_rejects_case_insensitive_reserved_staging_names() {
    for name in [
        ".settings.tmt-staging",
        ".Settings.TMT-STAGING",
        ".other.TmT-StAgInG",
    ] {
        let destination = Path::new("/private/project").join(name);
        assert!(matches!(
            Document::new(
                Path::new("/private/state"),
                &destination,
                128,
                "retry-exact"
            ),
            Err(StateError::Invalid { .. })
        ));
    }
    assert!(
        Document::new(
            Path::new("/private/state"),
            Path::new("/private/project/.settings"),
            128,
            "retry-exact"
        )
        .is_ok()
    );
}
