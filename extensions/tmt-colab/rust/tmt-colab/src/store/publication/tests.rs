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

impl Fixture {
    /// A fold of the stream at `seq` in both namespaces, signed by another device.
    fn fold_pair(&mut self, seq: u64, signer: &str) {
        for (index, namespace) in [Namespace::Content, Namespace::Own].into_iter().enumerate() {
            self.store
                .fold(
                    &Envelope {
                        scope: scope(),
                        namespace,
                        seq,
                        hash: [150 + index as u8 + seq as u8; 32],
                        previous: [seq as u8; 32],
                        bytes: format!("fold-{seq}-{}", namespace.name()).as_bytes(),
                    },
                    signer,
                )
                .unwrap();
        }
    }
}

#[test]
fn a_fold_keeps_the_update_it_binds_until_a_later_checkpoint_supersedes_it() {
    let mut f = Fixture::new();
    for seq in 1..=3 {
        f.append(seq);
    }
    f.fold_pair(3, "signer");
    // The prefix is pruned, but the update at the fold's sequence stays: it is the anchor.
    assert_eq!((f.payload(1), f.payload(2)), (None, None));
    assert!(f.payload(3).is_some());
    let anchor = f.store.anchor(scope(), 3).unwrap().unwrap();
    assert_eq!(
        (anchor.namespace, anchor.hash),
        (Namespace::Content, [3; 32])
    );
    assert_eq!(anchor.bytes, b"update-3");
    assert!(f.store.anchor(scope(), 2).unwrap().is_none(), "pruned");
    assert!(
        f.store.anchor(scope(), 9).unwrap().is_none(),
        "never stored"
    );
    // A fold at 5 supersedes the one at 3 and takes the anchor with it.
    f.append(4);
    f.append(5);
    f.fold_pair(5, "signer");
    assert_eq!((f.payload(3), f.payload(4)), (None, None));
    assert!(f.payload(5).is_some());
    // A device's own checkpoint binds the same update but needs no anchor: its signer is the stream.
    f.append(6);
    f.append(7);
    f.pair(7);
    assert_eq!((f.payload(5), f.payload(7)), (None, None));
    assert!(f.store.anchor(scope(), 7).unwrap().is_none());
}

#[test]
fn a_fold_in_one_namespace_alone_prunes_nothing() {
    let mut f = Fixture::new();
    for seq in 1..=3 {
        f.append(seq);
    }
    f.store
        .fold(
            &Envelope {
                scope: scope(),
                namespace: Namespace::Content,
                seq: 3,
                hash: [150; 32],
                previous: [3; 32],
                bytes: b"fold-3-content",
            },
            "signer",
        )
        .unwrap();
    for seq in 1..=3 {
        assert!(
            f.payload(seq).is_some(),
            "update {seq} is kept until the pair is complete"
        );
    }
}

#[test]
fn a_fold_must_bind_an_update_the_stream_holds_and_never_replaces_another_checkpoint() {
    let mut f = Fixture::new();
    f.append(1);
    f.append(2);
    let fold = |f: &mut Fixture, seq, previous: [u8; 32], hash: [u8; 32]| {
        f.store.fold(
            &Envelope {
                scope: scope(),
                namespace: Namespace::Content,
                seq,
                hash,
                previous,
                bytes: b"fold",
            },
            "signer",
        )
    };
    // No update at 3, and an update whose hash is not the one the fold names.
    assert!(matches!(
        fold(&mut f, 3, [3; 32], [150; 32]),
        Err(Fault::Gap)
    ));
    assert!(matches!(
        fold(&mut f, 2, [9; 32], [150; 32]),
        Err(Fault::Gap)
    ));
    // A device's own checkpoint at 2 wins the sequence: the fold that differs is a conflict.
    f.pair(2);
    assert!(matches!(
        fold(&mut f, 2, [2; 32], [150; 32]),
        Err(Fault::Conflict)
    ));
    // Replaying a fold that is stored is not a second one.
    f.append(3);
    f.fold_pair(3, "signer");
    let replay = f.store.fold(
        &Envelope {
            scope: scope(),
            namespace: Namespace::Content,
            seq: 3,
            hash: [150 + 3; 32],
            previous: [3; 32],
            bytes: b"fold-3-content",
        },
        "signer",
    );
    assert!(matches!(replay, Ok(crate::store::Accepted::Replay)));
}

#[test]
fn the_retained_folds_a_device_signed_are_listed_sorted_and_without_the_superseded() {
    let mut f = Fixture::new();
    for seq in 1..=5 {
        f.append(seq);
    }
    f.fold_pair(2, "signer");
    f.fold_pair(4, "signer");
    let owner = ed25519_dalek::SigningKey::from_bytes(&[7; 32])
        .verifying_key()
        .to_bytes();
    let space = crypto::space_id(&owner).unwrap();
    let signed = |f: &Fixture, device: &str| {
        f.store
            .owner_read(&space, &owner, |tx| tx.committed_folds(device))
            .unwrap()
    };
    // Only the fold at 4 is retained (the one at 2 was superseded), in the byte order a revoke
    // lists them.
    let encode = |hash: [u8; 32]| values::encode_binary(&hash);
    assert_eq!(
        signed(&f, "signer"),
        [encode([154; 32]), encode([155; 32])],
        "the content and the own fold at 4"
    );
    assert!(signed(&f, "another").is_empty());
    // A later checkpoint of the device itself supersedes them; nothing is left to list.
    f.append(6);
    f.pair(6);
    assert!(signed(&f, "signer").is_empty());
}

#[test]
fn a_device_that_signed_more_folds_than_a_revoke_can_list_is_refused() {
    let mut f = Fixture::new();
    f.append(1);
    let owner = ed25519_dalek::SigningKey::from_bytes(&[7; 32])
        .verifying_key()
        .to_bytes();
    let space = crypto::space_id(&owner).unwrap();
    let count = |f: &Fixture, n: usize| {
        let rows: Vec<(String, Vec<u8>)> = (0..n)
            .map(|i| (format!("{}", 100 + i), (i as u32).to_be_bytes().repeat(8)))
            .collect();
        f.store.connection.execute_batch("BEGIN").unwrap();
        for (seq, hash) in rows {
            f.store
                .connection
                .execute(
                    "INSERT INTO checkpoints(page,epoch,stream,namespace,seq,hash,digest,head,payload,signer)
                     SELECT page,epoch,stream,'content',?1,?2,x'00',x'00',x'01','signer' FROM streams LIMIT 1",
                    rusqlite::params![seq, hash],
                )
                .unwrap();
        }
        f.store.connection.execute_batch("COMMIT").unwrap();
    };
    let list = |f: &Fixture| {
        f.store
            .owner_read(&space, &owner, |tx| tx.committed_folds("signer"))
    };
    count(&f, 512);
    assert_eq!(list(&f).unwrap().len(), 512, "exactly the limit is listed");
    f.store
        .connection
        .execute(
            "INSERT INTO checkpoints(page,epoch,stream,namespace,seq,hash,digest,head,payload,signer)
             SELECT page,epoch,stream,'content','999',x'ff',x'00',x'00',x'01','signer' FROM streams LIMIT 1",
            [],
        )
        .unwrap();
    assert!(
        list(&f).is_err(),
        "one more is refused rather than left out of the revoke"
    );
}
#[test]
fn a_snapshot_is_given_the_anchor_of_each_retained_fold_and_of_nothing_else() {
    let mut f = Fixture::new();
    for seq in 1..=3 {
        f.append(seq);
    }
    f.pair(1);
    f.append(4);
    f.fold_pair(4, "signer");
    let owner = ed25519_dalek::SigningKey::from_bytes(&[7; 32])
        .verifying_key()
        .to_bytes();
    let space = crypto::space_id(&owner).unwrap();
    let anchors = f
        .store
        .owner_read(&space, &owner, |tx| tx.fold_anchors(PAGE, 1))
        .unwrap();
    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors[&(STREAM.to_owned(), 4)], b"update-4");
}
