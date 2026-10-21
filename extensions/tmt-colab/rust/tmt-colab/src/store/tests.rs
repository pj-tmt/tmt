//! Opaque Store tests of the stream position and of `bootstrap`, not a substitute for the
//! carrier and crypto admission proof that calls them.
use super::*;
use crate::keyring::Layout;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
const PAGE: &str = "page";
const STREAM: &str = "writer";
const OTHER: &str = "other";
const SIGNER: &str = "signer";

fn scope(stream: &'static str) -> StreamScope<'static> {
    StreamScope {
        page: PAGE,
        epoch: 1,
        stream,
    }
}
struct Fixture {
    root: PathBuf,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-4113-position-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        Self { root, store }
    }
    fn append(&mut self, stream: &'static str, seq: u64) -> StoreResult<Accepted> {
        let bytes = format!("update-{seq}");
        self.store.append(&Envelope {
            scope: scope(stream),
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
    }
    fn checkpoint(
        &mut self,
        stream: &'static str,
        seq: u64,
        namespace: Namespace,
    ) -> StoreResult<Accepted> {
        let bytes = format!("checkpoint-{seq}-{}", namespace.name());
        self.store.checkpoint(&Envelope {
            scope: scope(stream),
            namespace,
            seq,
            hash: [100 + seq as u8 + namespace as u8; 32],
            previous: [seq as u8; 32],
            bytes: bytes.as_bytes(),
        })
    }
    fn pair(&mut self, stream: &'static str, seq: u64) {
        self.checkpoint(stream, seq, Namespace::Content).unwrap();
        self.checkpoint(stream, seq, Namespace::Own).unwrap();
    }
    /// The position as the Store defined it before bootstrap existed: the latest receipt.
    fn latest_receipt(&self, stream: &'static str) -> Option<(u64, [u8; 32])> {
        self.store
            .connection
            .query_row(
                "SELECT seq,hash FROM receipts WHERE page=? AND epoch='1' AND stream=? ORDER BY seq DESC LIMIT 1",
                params![PAGE, stream],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .unwrap()
            .map(|(seq, hash)| (seq.parse().unwrap(), hash.try_into().unwrap()))
    }
    /// Checkpoints that hold no receipt at their own sequence.
    fn receiptless_checkpoints(&self) -> i64 {
        self.store
            .connection
            .query_row(
                "SELECT count(*) FROM checkpoints c WHERE NOT EXISTS(SELECT 1 FROM receipts r WHERE r.page=c.page AND r.epoch=c.epoch AND r.stream=c.stream AND r.seq=c.seq)",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn rows(&self, table: &str) -> i64 {
        self.store
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
    fn head(&self, stream: &'static str) -> Option<(u64, [u8; 32])> {
        self.store.stream_head(scope(stream)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn envelope<'a>(
    stream: &'static str,
    namespace: Namespace,
    seq: u64,
    bytes: &'a [u8],
) -> Envelope<'a> {
    Envelope {
        scope: scope(stream),
        namespace,
        seq,
        hash: [100 + namespace as u8; 32],
        previous: [seq as u8; 32],
        bytes,
    }
}
fn install(
    f: &mut Fixture,
    stream: &'static str,
    seq: u64,
    signer: Option<&str>,
) -> StoreResult<Accepted> {
    let content = format!("boot-{seq}-content");
    let own = format!("boot-{seq}-own");
    f.store.bootstrap(
        None,
        &[
            Bootstrapped {
                envelope: envelope(stream, Namespace::Content, seq, content.as_bytes()),
                signer,
            },
            Bootstrapped {
                envelope: envelope(stream, Namespace::Own, seq, own.as_bytes()),
                signer,
            },
        ],
    )
}

#[test]
fn the_position_is_the_latest_receipt_on_every_store_built_the_ordinary_way() {
    let mut f = Fixture::new();
    assert_eq!(f.head(STREAM), None);
    let check = |f: &Fixture, what: &str| {
        assert_eq!(f.head(STREAM), f.latest_receipt(STREAM), "{what}");
        assert_eq!(f.head(OTHER), f.latest_receipt(OTHER), "{what}");
        assert_eq!(f.receiptless_checkpoints(), 0, "{what}");
    };
    for seq in 1..=5 {
        f.append(STREAM, seq).unwrap();
        check(&f, "appends");
    }
    f.checkpoint(STREAM, 5, Namespace::Content).unwrap();
    check(&f, "an unpaired checkpoint");
    f.checkpoint(STREAM, 5, Namespace::Own).unwrap();
    check(&f, "a paired checkpoint prunes the prefix");
    for seq in 6..=8 {
        f.append(STREAM, seq).unwrap();
        check(&f, "appends after a compaction");
    }
    f.pair(STREAM, 8);
    check(&f, "a second pair");
    // An owner fold of another stream: its checkpoints are signed by another device.
    f.append(OTHER, 1).unwrap();
    f.append(OTHER, 2).unwrap();
    for namespace in [Namespace::Content, Namespace::Own] {
        let bytes = format!("fold-2-{}", namespace.name());
        f.store
            .fold(
                &Envelope {
                    scope: scope(OTHER),
                    namespace,
                    seq: 2,
                    hash: [150 + namespace as u8; 32],
                    previous: [2; 32],
                    bytes: bytes.as_bytes(),
                },
                SIGNER,
            )
            .unwrap();
        check(&f, "a fold");
    }
    f.append(OTHER, 3).unwrap();
    check(&f, "an append after a fold");
}

#[test]
fn a_checkpoint_needs_its_receipt_so_only_a_bootstrap_holds_one_without() {
    let mut f = Fixture::new();
    // The ordinary compaction path writes the receipt first: a checkpoint with none is refused.
    assert!(f.checkpoint(STREAM, 3, Namespace::Content).is_err());
    f.append(STREAM, 1).unwrap();
    f.append(STREAM, 2).unwrap();
    assert!(matches!(
        f.checkpoint(STREAM, 3, Namespace::Content),
        Err(Fault::Gap)
    ));
    assert_eq!((f.rows("checkpoints"), f.receiptless_checkpoints()), (0, 0));
    assert_eq!(install(&mut f, OTHER, 4, None).unwrap(), Accepted::New);
    assert_eq!((f.rows("checkpoints"), f.receiptless_checkpoints()), (2, 2));
}

#[test]
fn a_bootstrapped_stream_resumes_at_the_next_sequence_and_still_needs_continuity() {
    let mut f = Fixture::new();
    install(&mut f, STREAM, 4, None).unwrap();
    assert_eq!(f.head(STREAM), Some((4, [4; 32])));
    // Nothing before the cut is accepted, a gap is refused, and the next update must name the head.
    assert!(matches!(f.append(STREAM, 3), Err(Fault::Gap)));
    assert!(matches!(f.append(STREAM, 6), Err(Fault::Gap)));
    let wrong = f.store.append(&Envelope {
        scope: scope(STREAM),
        namespace: Namespace::Content,
        seq: 5,
        hash: [5; 32],
        previous: [9; 32],
        bytes: b"update-5",
    });
    assert!(matches!(wrong, Err(Fault::Conflict)));
    assert_eq!(f.head(STREAM), Some((4, [4; 32])));
    f.append(STREAM, 5).unwrap();
    assert_eq!(f.head(STREAM), Some((5, [5; 32])));
    // A browser on this machine is handed the pair, then the tail.
    for namespace in [Namespace::Content, Namespace::Own] {
        let first = f
            .store
            .namespace_next(scope(STREAM), namespace, NamespaceCursor::default())
            .unwrap()
            .unwrap();
        assert!(first.checkpoint);
        assert_eq!(first.cursor.seq, 4);
    }
    let tail = f
        .store
        .namespace_next(
            scope(STREAM),
            Namespace::Content,
            NamespaceCursor {
                seq: 4,
                hash: [100 + Namespace::Content as u8; 32],
            },
        )
        .unwrap()
        .unwrap();
    assert!(!tail.checkpoint);
    assert_eq!(tail.cursor.seq, 5);
}

#[test]
fn the_same_pair_again_is_a_replay_and_anything_else_is_a_conflict_that_writes_nothing() {
    let mut f = Fixture::new();
    assert_eq!(install(&mut f, STREAM, 4, None).unwrap(), Accepted::New);
    assert_eq!(install(&mut f, STREAM, 4, None).unwrap(), Accepted::Replay);
    f.append(STREAM, 5).unwrap();
    assert_eq!(install(&mut f, STREAM, 4, None).unwrap(), Accepted::Replay);
    let before = (f.rows("checkpoints"), f.rows("receipts"));
    // Another pair of the same stream, later or earlier, is not merged in.
    assert!(matches!(
        install(&mut f, STREAM, 9, None),
        Err(Fault::Conflict)
    ));
    assert!(matches!(
        install(&mut f, STREAM, 2, None),
        Err(Fault::Conflict)
    ));
    let different = f.store.bootstrap(
        None,
        &[Bootstrapped {
            envelope: envelope(STREAM, Namespace::Content, 4, b"another"),
            signer: None,
        }],
    );
    assert!(matches!(different, Err(Fault::Conflict)));
    assert_eq!((f.rows("checkpoints"), f.rows("receipts")), before);
    assert_eq!(f.head(STREAM), Some((5, [5; 32])));
}

#[test]
fn a_bootstrap_never_merges_into_a_held_prefix_or_a_frozen_stream() {
    let mut f = Fixture::new();
    f.append(STREAM, 1).unwrap();
    f.append(STREAM, 2).unwrap();
    assert!(matches!(
        install(&mut f, STREAM, 4, None),
        Err(Fault::Conflict)
    ));
    assert_eq!(
        (f.rows("checkpoints"), f.head(STREAM)),
        (0, Some((2, [2; 32])))
    );
    // A fork freezes a stream; a frozen stream takes nothing.
    f.store
        .append(&Envelope {
            scope: scope(OTHER),
            namespace: Namespace::Content,
            seq: 1,
            hash: [1; 32],
            previous: [0; 32],
            bytes: b"update-1",
        })
        .unwrap();
    let fork = f.store.append(&Envelope {
        scope: scope(OTHER),
        namespace: Namespace::Content,
        seq: 1,
        hash: [7; 32],
        previous: [0; 32],
        bytes: b"forked",
    });
    assert!(matches!(fork, Err(Fault::Conflict)));
    assert!(matches!(
        install(&mut f, OTHER, 4, None),
        Err(Fault::Conflict)
    ));
    assert_eq!(f.rows("checkpoints"), 0);
}

#[test]
fn a_fold_brings_its_anchor_as_the_receipt_and_its_signer_is_kept() {
    let mut f = Fixture::new();
    let anchor = Envelope {
        scope: scope(STREAM),
        namespace: Namespace::Own,
        seq: 4,
        hash: [4; 32],
        previous: [3; 32],
        bytes: b"update-4",
    };
    let content = b"fold-4-content".to_vec();
    let own = b"fold-4-own".to_vec();
    let objects = [
        Bootstrapped {
            envelope: envelope(STREAM, Namespace::Content, 4, &content),
            signer: Some(SIGNER),
        },
        Bootstrapped {
            envelope: envelope(STREAM, Namespace::Own, 4, &own),
            signer: Some(SIGNER),
        },
    ];
    assert_eq!(
        f.store.bootstrap(Some(&anchor), &objects).unwrap(),
        Accepted::New
    );
    // The anchor is a real receipt, so the fold finds it and the stream resumes after it.
    let kept = f.store.anchor(scope(STREAM), 4).unwrap().unwrap();
    assert_eq!((kept.namespace, kept.hash), (Namespace::Own, [4; 32]));
    assert_eq!(f.head(STREAM), Some((4, [4; 32])));
    let signers: i64 = f
        .store
        .connection
        .query_row(
            "SELECT count(*) FROM checkpoints WHERE signer=?",
            [SIGNER],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(signers, 2);
    assert_eq!(
        f.store.bootstrap(Some(&anchor), &objects).unwrap(),
        Accepted::Replay
    );
    f.append(STREAM, 5).unwrap();
}

#[test]
fn a_bootstrap_that_is_malformed_stale_or_over_budget_writes_nothing() {
    let mut f = Fixture::new();
    let bytes = b"x".to_vec();
    let same_namespace = f.store.bootstrap(
        None,
        &[
            Bootstrapped {
                envelope: envelope(STREAM, Namespace::Content, 4, &bytes),
                signer: None,
            },
            Bootstrapped {
                envelope: envelope(STREAM, Namespace::Content, 4, &bytes),
                signer: None,
            },
        ],
    );
    assert!(matches!(same_namespace, Err(Fault::Invalid)));
    let other_seq = f.store.bootstrap(
        None,
        &[
            Bootstrapped {
                envelope: envelope(STREAM, Namespace::Content, 4, &bytes),
                signer: None,
            },
            Bootstrapped {
                envelope: envelope(STREAM, Namespace::Own, 5, &bytes),
                signer: None,
            },
        ],
    );
    assert!(matches!(other_seq, Err(Fault::Invalid)));
    let wrong_anchor = Envelope {
        scope: scope(STREAM),
        namespace: Namespace::Own,
        seq: 4,
        hash: [9; 32],
        previous: [3; 32],
        bytes: b"update-4",
    };
    let anchored = f.store.bootstrap(
        Some(&wrong_anchor),
        &[Bootstrapped {
            envelope: envelope(STREAM, Namespace::Content, 4, &bytes),
            signer: Some(SIGNER),
        }],
    );
    assert!(matches!(anchored, Err(Fault::Invalid)));
    assert!(matches!(f.store.bootstrap(None, &[]), Err(Fault::Invalid)));
    // A page that moved to another epoch takes nothing for the old one.
    let mut stale = scope(STREAM);
    stale.epoch = 2;
    let moved = f.store.bootstrap(
        None,
        &[Bootstrapped {
            envelope: Envelope {
                scope: stale,
                ..envelope(STREAM, Namespace::Content, 4, &bytes)
            },
            signer: None,
        }],
    );
    assert!(matches!(moved, Err(Fault::StaleEpoch)));
    assert_eq!((f.rows("checkpoints"), f.rows("receipts")), (0, 0));
    // The page budget is counted inside the transaction: a pair that does not fit rolls back.
    let big = vec![7u8; limits::OBJECT_BYTES - 1024];
    for seq in 1..=3 {
        f.store
            .append(&Envelope {
                scope: scope(OTHER),
                namespace: Namespace::Content,
                seq,
                hash: [seq as u8; 32],
                previous: if seq == 1 {
                    [0; 32]
                } else {
                    [(seq - 1) as u8; 32]
                },
                bytes: &big,
            })
            .unwrap();
    }
    let before = (f.rows("checkpoints"), f.rows("receipts"));
    let over = f.store.bootstrap(
        None,
        &[
            Bootstrapped {
                envelope: envelope(STREAM, Namespace::Content, 4, &big),
                signer: None,
            },
            Bootstrapped {
                envelope: envelope(STREAM, Namespace::Own, 4, &big),
                signer: None,
            },
        ],
    );
    assert!(matches!(over, Err(Fault::Capacity)));
    assert_eq!((f.rows("checkpoints"), f.rows("receipts")), before);
    assert_eq!(f.head(STREAM), None);
}

mod page_titles {
    use super::*;
    use crate::keyring::Keyring;
    use crate::store::owner::OwnerTransaction;

    const TITLED: &str = "10000000-0000-4000-8000-000000000001";

    struct Titled {
        fixture: Fixture,
        key: Keyring,
    }
    impl Titled {
        fn new() -> Self {
            let fixture = Fixture::new();
            let key = Keyring::open(&Layout::open(&fixture.root).unwrap()).unwrap();
            fixture.store.create_page(TITLED).unwrap();
            Self { fixture, key }
        }
        /// An admitted update: the page's content counts move, so its fingerprint does too.
        fn edit(&mut self, seq: u64) {
            self.fixture
                .store
                .append(&Envelope {
                    scope: StreamScope {
                        page: TITLED,
                        epoch: 1,
                        stream: STREAM,
                    },
                    namespace: Namespace::Content,
                    seq,
                    hash: [seq as u8; 32],
                    previous: if seq == 1 {
                        [0; 32]
                    } else {
                        [(seq - 1) as u8; 32]
                    },
                    bytes: b"update",
                })
                .unwrap();
        }
        fn fingerprint(&mut self) -> String {
            self.tx(|tx| Ok(tx.page_fingerprint(TITLED)?.unwrap()))
        }
        fn record(&mut self, expected: &str, title: &str) -> bool {
            self.tx(|tx| tx.record_page_title(TITLED, expected, title))
        }
        fn titles(&mut self) -> Vec<(String, String)> {
            self.tx(|tx| Ok(tx.page_titles()?.into_iter().collect()))
        }
        fn tx<T>(&mut self, apply: impl FnOnce(&mut OwnerTransaction<'_>) -> Result<T>) -> T {
            self.fixture
                .store
                .project_transaction(&self.key.space_id, &self.key.owner_public(), apply)
                .unwrap()
        }
        fn rows(&self) -> i64 {
            self.fixture.rows("page_titles")
        }
    }

    #[test]
    fn a_title_is_stored_at_the_fingerprint_its_fold_started_from() {
        let mut t = Titled::new();
        t.edit(1);
        let fingerprint = t.fingerprint();
        assert!(t.record(&fingerprint, "First title"));
        assert_eq!(t.titles(), [(TITLED.into(), "First title".into())]);
        // A second fold of the same page replaces the row.
        assert!(t.record(&fingerprint, "Renamed"));
        assert_eq!(t.titles(), [(TITLED.into(), "Renamed".into())]);
        assert_eq!(t.rows(), 1);
    }

    #[test]
    fn a_page_that_changed_during_its_fold_installs_nothing_and_its_old_row_goes_stale() {
        let mut t = Titled::new();
        t.edit(1);
        let first = t.fingerprint();
        assert!(t.record(&first, "Old"));
        t.edit(2);
        // The fold that started at the old fingerprint cannot install its result.
        assert!(!t.record(&first, "Racing"));
        assert_eq!(t.rows(), 1);
        // The old row is no longer the page's: it is not read, and the next fold replaces it.
        assert_eq!(t.titles(), []);
        let second = t.fingerprint();
        assert!(t.record(&second, "New"));
        assert_eq!(t.titles(), [(TITLED.into(), "New".into())]);
    }

    #[test]
    fn an_empty_or_over_long_title_is_not_stored_and_removes_the_older_row() {
        let mut t = Titled::new();
        t.edit(1);
        let fingerprint = t.fingerprint();
        let longest = "é".repeat(crate::limits::PAGE_TITLE_BYTES / 2);
        assert!(t.record(&fingerprint, &longest));
        assert!(!t.record(&fingerprint, &format!("{longest}x")));
        assert_eq!(t.rows(), 0);
        assert!(t.record(&fingerprint, "Back"));
        assert!(!t.record(&fingerprint, ""));
        assert_eq!(t.rows(), 0);
    }

    #[test]
    fn an_archive_and_a_delete_take_the_row_and_the_title_of_a_page_the_store_lacks_is_refused() {
        let mut t = Titled::new();
        t.edit(1);
        let fingerprint = t.fingerprint();
        assert!(t.record(&fingerprint, "Kept"));
        t.tx(|tx| tx.drop_page_title(TITLED));
        assert_eq!(t.rows(), 0);
        assert!(t.record(&fingerprint, "Again"));
        t.tx(|tx| tx.delete_page_data(TITLED));
        assert_eq!(t.rows(), 0);
        // A page ID that is not a generated one never reaches a statement.
        let refused =
            t.fixture
                .store
                .project_transaction(&t.key.space_id, &t.key.owner_public(), |tx| {
                    tx.record_page_title("not-an-id", "x", "Title")
                });
        assert!(refused.is_err());
    }
}
