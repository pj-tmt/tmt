use super::*;
use crate::keyring::Layout;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Fixture {
    root: PathBuf,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "colab-auth-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let mut store = Store::open(&layout).unwrap();
        store.start_auth().unwrap();
        Self { root, store }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn enrollment<'a>(code: &'a str, device: &'a str, hash: &'a [u8; 32]) -> Enrollment<'a> {
    Enrollment {
        code,
        space: "pinned-space",
        device,
        signing_key: &[3; 32],
        encryption_key: &[4; 32],
        certificate: b"verified-certificate",
        token_hash: hash,
        now: 100,
        expires: 1000,
    }
}
#[test]
fn consumption_session_scope_expiry_and_revocation_are_durable() {
    let mut fixture = Fixture::new();
    let store = &mut fixture.store;
    store.issue_signin("code", "pinned-space", 90, 200).unwrap();
    let hash = [5; 32];
    store.enroll(enrollment("code", "device", &hash)).unwrap();
    assert!(
        store
            .enroll(enrollment("code", "other-device", &[6; 32]))
            .is_err()
    );
    let layout = Layout::open(&fixture.root).unwrap();
    *store = Store::open(&layout).unwrap();
    assert_eq!(
        store.session(&hash, "pinned-space", 999).unwrap(),
        Some("device".into())
    );
    assert_eq!(store.session(&hash, "different-space", 999).unwrap(), None);
    assert_eq!(store.session(&hash, "pinned-space", 1000).unwrap(), None);
    assert_eq!(store.session(&[0; 32], "pinned-space", 100).unwrap(), None);
    let persisted: Vec<u8> = store
        .connection
        .query_row("SELECT token_hash FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(persisted, hash);
    store.start_auth().unwrap();
    assert_eq!(
        store.session(&hash, "pinned-space", 100).unwrap(),
        Some("device".into())
    );
    store.revoke_device("device").unwrap();
    assert_eq!(store.session(&hash, "pinned-space", 100).unwrap(), None);
    let remaining: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, 0);
}
#[test]
fn expiry_restart_and_injected_failure_do_not_issue_a_session() {
    let mut fixture = Fixture::new();
    let store = &mut fixture.store;
    assert!(
        store
            .issue_signin("overlong", "pinned-space", 0, 600001)
            .is_err()
    );
    store
        .issue_signin("expired", "pinned-space", 0, 100)
        .unwrap();
    assert!(
        store
            .enroll(enrollment("expired", "device", &[5; 32]))
            .is_err()
    );
    store.issue_signin("old", "pinned-space", 100, 200).unwrap();
    store.start_auth().unwrap();
    assert!(store.enroll(enrollment("old", "device", &[5; 32])).is_err());
    store.issue_signin("new", "pinned-space", 100, 200).unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_session BEFORE INSERT ON sessions BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store.enroll(enrollment("new", "device", &[5; 32])).is_err());
    let consumed: i64 = store
        .connection
        .query_row(
            "SELECT consumed FROM signin_codes WHERE code='new'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let devices: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))
        .unwrap();
    assert_eq!((consumed, devices), (0, 0));
    store
        .connection
        .execute_batch("DROP TRIGGER fail_session")
        .unwrap();
    store.enroll(enrollment("new", "device", &[5; 32])).unwrap();
}
