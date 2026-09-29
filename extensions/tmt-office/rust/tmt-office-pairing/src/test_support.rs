//! Invocation-owned filesystem fixture for Office pairing tests.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) struct TestDirectory {
    pub path: PathBuf,
}

impl TestDirectory {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-office-pairing-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // Refuse to reuse an existing directory; cleanup owns only this creation.
        fs::create_dir(&path).expect("create unique Office pairing test directory");
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            if std::thread::panicking() {
                eprintln!(
                    "Could not remove Office pairing fixture {}: {error}",
                    self.path.display()
                );
            } else {
                panic!(
                    "Could not remove Office pairing fixture {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}
