//! Durable attempted costs under a single project writer; no provider effects.
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_remote::{
    firestore_budget::{Decision, Model},
    native_sync::NativeBudgetFile,
    state::Layout,
};
const NOW: u64 = 1_800_000_000_000;
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "tmt-native-budget-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn attempted_costs_flush_once_per_minute_and_stop_survives_restart() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let model = Model::new(1, 1).unwrap();
    let mut budget = NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW, 0).unwrap();
    let path = root.0.join("remote/native-budget/demo-tmt-remote.json");
    assert_eq!(
        budget.authorize(&model, 3, NOW, 0).unwrap(),
        Decision::Allow
    );
    assert!(!path.exists());
    budget.authorize(&model, 258, NOW + 59_999, 59_999).unwrap();
    assert!(!path.exists());
    budget.authorize(&model, 3, NOW + 60_000, 60_000).unwrap();
    let saved = fs::read(&path).unwrap();
    budget.authorize(&model, 3, NOW + 60_001, 60_001).unwrap();
    assert_eq!(fs::read(&path).unwrap(), saved);
    assert!(
        NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW, 0).is_err(),
        "second opener cannot grant a fresh allowance"
    );
    budget.finish().unwrap();
    let restored =
        NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW + 61_000, 61_000).unwrap();
    assert_eq!(restored.usage().reads, 267);
}

#[test]
fn the_owner_checkpoint_persists_costs_without_another_provider_request() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let path = root.0.join("remote/native-budget/demo-tmt-remote.json");
    let mut budget = NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW, 0).unwrap();
    budget
        .authorize(&Model::new(1, 1).unwrap(), 258, NOW, 0)
        .unwrap();
    assert!(!path.exists());
    budget.checkpoint(60_000).unwrap();
    let saved = fs::read(&path).unwrap();
    budget
        .authorize(&Model::new(1, 1).unwrap(), 3, NOW + 60_001, 60_001)
        .unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        saved,
        "checkpoint updates the shared flush cadence"
    );
    drop(budget);
    let restored =
        NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW + 61_000, 61_000).unwrap();
    assert_eq!(restored.usage().reads, 258);
}

#[test]
fn the_persistent_port_refuses_a_different_project_without_charging() {
    use tmt_remote::{firebase_auth::Time, native_sync::NativeBudgetPort, sealed_sync::SyncError};
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let file =
        std::sync::Mutex::new(NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW, 0).unwrap());
    assert_eq!(
        file.charge(
            "another-project",
            &Model::new(1, 1).unwrap(),
            3,
            Time {
                wall_ms: NOW,
                monotonic_ms: 0
            }
        )
        .err(),
        Some(SyncError::Denied)
    );
    assert_eq!(file.lock().unwrap().usage().reads, 0);
    assert!(
        !root
            .0
            .join("remote/native-budget/demo-tmt-remote.json")
            .exists()
    );
}
#[test]
fn crash_exposure_is_bounded_to_the_unsaved_flush_window() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let model = Model::new(1, 1).unwrap();
    let mut budget = NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW, 0).unwrap();
    budget.authorize(&model, 3, NOW + 60_000, 60_000).unwrap();
    budget.authorize(&model, 3, NOW + 61_000, 61_000).unwrap();
    drop(budget); // Abrupt owner loss deliberately has no stop flush.
    let restored =
        NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW + 62_000, 62_000).unwrap();
    assert_eq!(restored.usage().reads, 3);
}
#[test]
fn failed_flush_refuses_network_authorization_and_preserves_charge() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let model = Model::new(1, 1).unwrap();
    let mut budget = NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW, 0).unwrap();
    let path = root.0.join("remote/native-budget/demo-tmt-remote.json");
    fs::create_dir(&path).unwrap();
    assert!(budget.authorize(&model, 258, NOW + 60_000, 60_000).is_err());
    assert_eq!(budget.usage().reads, 258);
    assert!(path.is_dir());
}

#[test]
fn a_changed_persisted_counter_is_never_reset_by_the_live_writer() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let model = Model::new(1, 1).unwrap();
    let mut budget = NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW, 0).unwrap();
    budget.authorize(&model, 3, NOW + 60_000, 60_000).unwrap();
    let path = root.0.join("remote/native-budget/demo-tmt-remote.json");
    fs::write(&path, b"foreign state").unwrap();
    assert!(budget.authorize(&model, 3, NOW + 120_000, 120_000).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"foreign state");
}
