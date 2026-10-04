//! Durable server time and advisory expiry through real creation/write/projections.
mod support;
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use tmt_colab::{
    decoder::Decoder,
    inspection,
    keyring::{Keyring, Layout},
    page,
    registration::Registration,
    store::{Accepted, Store},
    transitions::{Engine, OwnerAction, OwnerRequest},
};
use tmt_colab_model::values;
const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const NOW: u64 = 1_790_000_000_000;
const DAY: u64 = 86_400_000;
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
struct Fixture {
    root: PathBuf,
    layout: Layout,
    key: Keyring,
    store: Store,
    clock: Arc<AtomicU64>,
    engine: Engine,
    next: u64,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "colab-expiry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let clock = Arc::new(AtomicU64::new(NOW));
        let captured = clock.clone();
        let store = Store::open(&layout)
            .unwrap()
            .with_clock(move || Ok(captured.load(Ordering::SeqCst)));
        Self {
            root,
            layout,
            key,
            store,
            clock,
            engine: Engine::with_decoder_config(support::decoder_config(BINARY.into())).unwrap(),
            next: 1,
        }
    }
    fn apply(&mut self, action: OwnerAction<'_>) {
        let id = format!("20000000-0000-4000-8000-{:012}", self.next);
        self.next += 1;
        let expected_revision = self
            .store
            .owner_head(&self.key.space_id, &self.key.owner_public())
            .unwrap()
            .map_or(0, |h| h.revision);
        self.engine
            .apply(
                &mut self.store,
                &self.key,
                OwnerRequest {
                    operation_id: &id,
                    expected_revision,
                    action,
                    transport_digest: None,
                    scope: None,
                },
                NOW,
            )
            .unwrap();
    }
    fn create(&mut self) {
        self.apply(OwnerAction::Create {
            page: PAGE,
            title: "Expiry",
            source: "<p>Initial</p>",
            publisher_agent: None,
        });
    }
    fn sql(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.layout.directory.join("space.db")).unwrap()
    }
    fn projection(&self) -> Value {
        let cli = inspection::catalog(&self.store, &self.key).unwrap();
        let clock = self.clock.clone();
        let api = Registration::with_decoder_config(
            Store::read(&self.layout)
                .unwrap()
                .with_clock(move || Ok(clock.load(Ordering::SeqCst))),
            Keyring::read(&self.layout).unwrap(),
            support::decoder_config(BINARY.into()),
        )
        .unwrap();
        let context = json!({"deviceId":"30000000-0000-4000-8000-000000000001", "kind":"browser", "origin":"http://127.0.0.1:1", "name":"Owner", "owner":true, "grantRevision":1,
            "publicKey":values::encode_binary(SigningKey::from_bytes(&[9;32]).verifying_key().as_bytes())}).to_string();
        let api_pages: Value = serde_json::from_slice(&api.pages(Some(&context)).unwrap()).unwrap();
        assert_eq!(cli["pages"], api_pages["pages"]);
        api.close().unwrap();
        cli["pages"][0].clone()
    }
    fn decoder(&self) -> Decoder {
        Decoder::with_config(support::decoder_config(BINARY.into())).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn verified_projection_uses_same_durable_time_for_defaults_override_and_boundaries() {
    let mut f = Fixture::new();
    f.create();
    let p = f.projection();
    assert_eq!(p["lastUpdateAtMs"], NOW);
    assert_eq!(p["retentionDays"], 30);
    assert_eq!(p["expiresAtMs"], NOW + 30 * DAY);
    assert_eq!(p["warnings"], json!([]));
    f.clock.store(NOW + 23 * DAY - 1, Ordering::SeqCst);
    assert_eq!(f.projection()["warnings"], json!([]));
    f.clock.store(NOW + 23 * DAY, Ordering::SeqCst);
    assert_eq!(f.projection()["warnings"], json!(["expires-soon"]));
    f.clock.store(NOW + 30 * DAY - 1, Ordering::SeqCst);
    assert_eq!(f.projection()["warnings"], json!(["expires-soon"]));
    f.clock.store(NOW + 30 * DAY, Ordering::SeqCst);
    assert_eq!(f.projection()["warnings"], json!(["expired"]));
    // Advisory expiry leaves readable content and edit authority in place.
    assert_eq!(
        page::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .source,
        "<p>Initial</p>"
    );
    f.apply(OwnerAction::Retention {
        page: PAGE,
        days: Some(60),
    });
    let p = f.projection();
    assert_eq!(p["lastUpdateAtMs"], NOW);
    assert_eq!(p["expiresAtMs"], NOW + 60 * DAY);
    assert_eq!(p["warnings"], json!([]));
    f.apply(OwnerAction::Retention {
        page: PAGE,
        days: None,
    });
    let p = f.projection();
    assert_eq!(p["lastUpdateAtMs"], NOW);
    assert!(p["expiresAtMs"].is_null());
    assert_eq!(p["warnings"], json!([]));
    f.apply(OwnerAction::Retention {
        page: PAGE,
        days: Some(9_007_199_254_740_991),
    });
    let p = f.projection();
    assert!(p["expiresAtMs"].is_null());
    assert_eq!(p["warnings"], json!(["expiry-out-of-range"]));
    f.sql()
        .execute("UPDATE pages SET last_update_at_ms=NULL", [])
        .unwrap();
    assert_eq!(f.projection()["warnings"], json!(["expiry-unavailable"]));
    f.apply(OwnerAction::Retention {
        page: PAGE,
        days: None,
    });
    assert_eq!(f.projection()["warnings"], json!([]));
    f.sql()
        .execute(
            "UPDATE membership_log SET envelope=x'00' WHERE revision='00000000000000000001'",
            [],
        )
        .unwrap();
    assert!(inspection::catalog(&f.store, &f.key).is_err());
}
#[test]
fn content_samples_append_clock_and_failed_late_receipt_rolls_back_time() {
    let mut f = Fixture::new();
    f.create();
    let p = page::prepare(
        &f.store,
        &f.key,
        PAGE,
        tmt_colab::decoder::ContentEdit {
            source: "<p>New</p>",
            publisher_agent: None,
        },
        None,
        &mut f.decoder(),
        NOW,
    )
    .unwrap();
    f.clock.store(NOW + 40 * DAY, Ordering::SeqCst);
    f.sql().execute_batch("CREATE TRIGGER deny_receipt BEFORE INSERT ON owner_operations BEGIN SELECT RAISE(FAIL,'late rollback'); END;").unwrap();
    assert!(page::commit(&mut f.store, &f.key, &p, NOW).is_err());
    assert_eq!(f.projection()["lastUpdateAtMs"], NOW);
    assert_eq!(
        page::read(&f.store, &f.key, PAGE, &mut f.decoder())
            .unwrap()
            .source,
        "<p>Initial</p>"
    );
    f.sql().execute_batch("DROP TRIGGER deny_receipt").unwrap();
    assert_eq!(
        page::commit(&mut f.store, &f.key, &p, NOW)
            .unwrap()
            .accepted,
        Accepted::New
    );
    assert_eq!(f.projection()["lastUpdateAtMs"], NOW + 40 * DAY);
    f.clock.store(NOW + 50 * DAY, Ordering::SeqCst);
    assert_eq!(
        page::commit(&mut f.store, &f.key, &p, NOW)
            .unwrap()
            .accepted,
        Accepted::Replay
    );
    assert_eq!(f.projection()["lastUpdateAtMs"], NOW + 40 * DAY);
    f.apply(OwnerAction::EpochAdvance { page: PAGE });
    assert_eq!(f.projection()["lastUpdateAtMs"], NOW + 40 * DAY);
    f.apply(OwnerAction::Archive { page: PAGE });
    let p = f.projection();
    assert_eq!(p["lastUpdateAtMs"], NOW + 40 * DAY);
    assert_eq!(p["archived"], true);
}
