//! Opaque Store tests, not a substitute for B3's carrier/crypto admission proof.
use super::*;
use crate::{keyring::Layout, store::Envelope};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
const PAGE: &str = "page";
const STREAM: &str = "writer";
fn scope() -> StreamScope<'static> {
    StreamScope {
        page: PAGE,
        epoch: 1,
        stream: STREAM,
    }
}
fn binding() -> CloudBinding<'static> {
    CloudBinding {
        project: "fixture",
        declaration_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        space: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        uid: "owner",
    }
}
struct Fixture {
    root: PathBuf,
    layout: Layout,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-2452-retention-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        Self {
            root,
            layout,
            store,
        }
    }
    fn append(&mut self, seq: u64) {
        let bytes = format!("update-{seq}");
        self.store
            .append(&Envelope {
                scope: scope(),
                namespace: if seq.is_multiple_of(2) {
                    Namespace::Own
                } else {
                    Namespace::Content
                },
                seq,
                hash: [seq as u8; 32],
                previous: if seq == 1 {
                    [0; 32]
                } else {
                    [(seq - 1) as u8; 32]
                },
                bytes: bytes.as_bytes(),
            })
            .unwrap();
    }
    fn pair(&mut self, seq: u64) -> ([u8; 32], [u8; 32]) {
        let mut digests = Vec::new();
        for (index, namespace) in [Namespace::Content, Namespace::Own].into_iter().enumerate() {
            let bytes = format!("checkpoint-{seq}-{}", namespace.name());
            self.store
                .checkpoint(&Envelope {
                    scope: scope(),
                    namespace,
                    seq,
                    hash: [100 + index as u8; 32],
                    previous: [seq as u8; 32],
                    bytes: bytes.as_bytes(),
                })
                .unwrap();
            digests.push(crypto::digest(bytes.as_bytes()));
        }
        (digests[0], digests[1])
    }
    fn payload(&self, seq: u64) -> Option<Vec<u8>> {
        self.store.payload(scope(), seq).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn an_existing_compacted_page_starts_from_a_pending_pair_and_retained_tail() {
    let mut f = Fixture::new();
    for seq in 1..=3 {
        f.append(seq);
    }
    let original = f.pair(3);
    assert_eq!(f.payload(1), None);
    for seq in 4..=5 {
        f.append(seq);
    }
    assert_eq!(
        f.store.track_publication(scope(), &binding()).unwrap(),
        PublicationProgress {
            bootstrap_seq: 3,
            confirmed_seq: 0
        }
    );
    let next = f.pair(5);
    assert!(
        f.payload(4).is_some() && f.payload(5).is_some(),
        "unconfirmed checkpoint grants no pruning"
    );
    assert_eq!(
        f.store
            .publication_progress(scope(), &binding())
            .unwrap()
            .unwrap(),
        PublicationProgress {
            bootstrap_seq: 5,
            confirmed_seq: 0
        }
    );
    // A delayed older confirmation is safe but does not claim the newer cut.
    let p = f
        .store
        .confirm_checkpoint_publication(scope(), &binding(), 3, &original.0, &original.1)
        .unwrap();
    assert_eq!(p.confirmed_seq, 3);
    assert!(f.payload(4).is_some());
    let p = f
        .store
        .confirm_checkpoint_publication(scope(), &binding(), 5, &next.0, &next.1)
        .unwrap();
    assert_eq!(p.confirmed_seq, 5);
    assert_eq!(f.payload(4), None);
    assert_eq!(f.payload(5), None);
    f.append(6);
    f.store
        .confirm_publication(scope(), &binding(), 6, &[6; 32])
        .unwrap();
    assert!(
        f.payload(6).is_some(),
        "a later local checkpoint still owns compaction"
    );
    let read = Store::read(&f.layout).unwrap();
    assert_eq!(
        read.publication_progress(scope(), &binding())
            .unwrap()
            .unwrap()
            .confirmed_seq,
        6
    );
}
#[test]
fn wrong_hash_binding_uid_sequence_and_epoch_never_advance_progress() {
    let mut f = Fixture::new();
    f.store.track_publication(scope(), &binding()).unwrap();
    f.append(1);
    f.append(2);
    assert!(matches!(
        f.store
            .confirm_publication(scope(), &binding(), 1, &[9; 32]),
        Err(Fault::Conflict)
    ));
    assert!(matches!(
        f.store
            .confirm_publication(scope(), &binding(), 2, &[2; 32]),
        Err(Fault::Gap)
    ));
    let mut other = binding();
    other.uid = "other";
    assert!(matches!(
        f.store.track_publication(scope(), &other),
        Err(Fault::Conflict)
    ));
    other = binding();
    other.project = "different";
    assert!(matches!(
        f.store.confirm_publication(scope(), &other, 1, &[1; 32]),
        Err(Fault::Conflict)
    ));
    let old = StreamScope {
        epoch: 2,
        ..scope()
    };
    assert!(matches!(
        f.store.confirm_publication(old, &binding(), 1, &[1; 32]),
        Err(Fault::StaleEpoch)
    ));
    assert_eq!(
        f.store
            .publication_progress(scope(), &binding())
            .unwrap()
            .unwrap()
            .confirmed_seq,
        0
    );
    assert!(f.payload(1).is_some() && f.payload(2).is_some());
    assert_eq!(
        f.store
            .confirm_publication(scope(), &binding(), 1, &[1; 32])
            .unwrap()
            .confirmed_seq,
        1
    );
    assert_eq!(
        f.store
            .confirm_publication(scope(), &binding(), 1, &[1; 32])
            .unwrap()
            .confirmed_seq,
        1
    );
}
#[test]
fn a_wrong_checkpoint_digest_keeps_the_pair_and_tail_pending() {
    let mut f = Fixture::new();
    f.store.track_publication(scope(), &binding()).unwrap();
    for seq in 1..=3 {
        f.append(seq);
    }
    let digests = f.pair(3);
    assert!(matches!(
        f.store
            .confirm_checkpoint_publication(scope(), &binding(), 3, &digests.0, &[9; 32]),
        Err(Fault::Conflict)
    ));
    assert_eq!(
        f.store
            .publication_progress(scope(), &binding())
            .unwrap()
            .unwrap()
            .confirmed_seq,
        0
    );
    for seq in 1..=3 {
        assert!(f.payload(seq).is_some());
    }
    f.store
        .confirm_checkpoint_publication(scope(), &binding(), 3, &digests.0, &digests.1)
        .unwrap();
    for seq in 1..=3 {
        assert_eq!(f.payload(seq), None);
    }
}
#[test]
fn missing_or_corrupt_history_is_not_treated_as_published() {
    for corruption in [
        "DELETE FROM receipts WHERE seq='00000000000000000002'",
        "UPDATE receipts SET payload=NULL WHERE seq='00000000000000000002'",
    ] {
        let mut f = Fixture::new();
        for seq in 1..=3 {
            f.append(seq);
        }
        f.store.connection.execute_batch(corruption).unwrap();
        assert!(matches!(
            f.store.track_publication(scope(), &binding()),
            Err(Fault::ResyncRequired)
        ));
        let count: i64 = f
            .store
            .connection
            .query_row("SELECT count(*) FROM cloud_publications", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
#[test]
fn an_unpaired_checkpoint_does_not_drop_the_protected_bootstrap() {
    let mut f = Fixture::new();
    for seq in 1..=3 {
        f.append(seq);
    }
    let original = f.pair(3);
    f.store.track_publication(scope(), &binding()).unwrap();
    f.append(4);
    f.store
        .checkpoint(&Envelope {
            scope: scope(),
            namespace: Namespace::Content,
            seq: 4,
            hash: [99; 32],
            previous: [4; 32],
            bytes: b"one namespace only",
        })
        .unwrap();
    assert_eq!(
        f.store
            .publication_progress(scope(), &binding())
            .unwrap()
            .unwrap()
            .bootstrap_seq,
        3
    );
    f.store
        .confirm_checkpoint_publication(scope(), &binding(), 3, &original.0, &original.1)
        .unwrap();
    assert!(f.payload(4).is_some());
}
#[test]
fn tracking_metadata_is_bounded_and_capacity_failure_rolls_it_back() {
    let mut f = Fixture::new();
    for i in 0..crate::limits::SYNC_NAMESPACES {
        let stream = format!("writer-{i}");
        f.store
            .track_publication(
                StreamScope {
                    stream: &stream,
                    ..scope()
                },
                &binding(),
            )
            .unwrap();
    }
    assert!(matches!(
        f.store.track_publication(scope(), &binding()),
        Err(Fault::Capacity)
    ));
    let mut f = Fixture::new();
    f.append(1);
    f.store
        .connection
        .execute(
            "UPDATE receipts SET payload=zeroblob(?)",
            [crate::limits::PAGE_BYTES as i64],
        )
        .unwrap();
    assert!(matches!(
        f.store.track_publication(scope(), &binding()),
        Err(Fault::Capacity)
    ));
    assert_eq!(
        f.store
            .connection
            .query_row("SELECT count(*) FROM cloud_publications", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
