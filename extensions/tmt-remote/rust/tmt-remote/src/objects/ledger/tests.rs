//! The ledger's own footprint: the largest rollback journal any production
//! transition writes, and the empty ledger, must fit the installation base charge.
use super::*;
use crate::state::Layout;
use std::{fs, path::PathBuf};

/// A disposable root removed even when an assertion unwinds.
struct Root(PathBuf);
impl Root {
    fn new(tag: &str) -> Self {
        let path = PathBuf::from(format!("/tmp/t1850{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

static NEVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn io() -> IoBudget<'static> {
    IoBudget {
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
        cancelled: &NEVER,
    }
}

fn maximal_rows(connection: &Connection, namespace: u8, count: usize, phase: &str) {
    for n in 0..count {
        let id = |fill: u8| {
            let mut id = vec![fill; 32];
            id[..8].copy_from_slice(&(n as u64 + 1).to_be_bytes());
            id
        };
        connection
            .execute(
                "INSERT INTO intents VALUES (?1,?2,?3,?4,?5,4096,?6,1,2,?7,1,4096,4096,1,0)",
                params![
                    "a-maximal-extension-name-32-chars",
                    id(2),
                    vec![namespace; 32],
                    id(3),
                    id(4),
                    vec![9u8; limits::OBJECT_BINDING_BYTES],
                    phase
                ],
            )
            .unwrap();
    }
}

#[test]
fn the_largest_journal_of_any_transition_and_the_empty_ledger_fit_the_base_charge() {
    let guard = Root::new("j");
    let root = guard.0.clone();
    let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
    let ledger = Ledger::open(&serving).unwrap();
    let database = root.join("remote/objects.db");
    let empty = fs::metadata(&database).unwrap().len();
    // A namespace at its entry ceiling, every row maximal, all awaiting release.
    let ns = NamespaceId([1; 32]);
    let rows = limits::OBJECT_NAMESPACE_ENTRIES as usize;
    {
        let connection = ledger.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO namespaces VALUES ('a-maximal-extension-name-32-chars',?1,'removing')",
                [ns.0.as_slice()],
            )
            .unwrap();
        maximal_rows(&connection, 1, rows, "removing");
        // PERSIST keeps the journal file, so its length is the high-water mark of every
        // transaction below.
        connection
            .query_row("PRAGMA journal_mode=PERSIST", [], |r| r.get::<_, String>(0))
            .unwrap();
    }
    let ext = "a-maximal-extension-name-32-chars";
    ledger.removal_confirmed(ext, &ns, &io()).unwrap();
    // Every other production transition on a full ledger.
    let q = Quotas {
        namespace_entries: u32::MAX,
        extension_entries: u32::MAX,
        installation_entries: u32::MAX,
        retained_extension: u32::MAX,
        retained_installation: u32::MAX,
        active_intents: u32::MAX,
        ..Quotas::contract()
    };
    let spec = BeginSpec {
        intent: IntentId([7; 32]),
        key: BlobKey {
            namespace: NamespaceId([2; 32]),
            object: OpaqueKey([7; 32]),
        },
        payload_sha256: [3; 32],
        payload_bytes: limits::OBJECT_CHUNK_BYTES as u64 + 1,
        binding: vec![5; limits::OBJECT_BINDING_BYTES],
    };
    let Adoption::Fresh(row) = ledger.adopt(ext, &spec, 10, &q, &io()).unwrap() else {
        panic!("fresh adoption");
    };
    let row = ledger
        .advance(ext, &row, limits::OBJECT_CHUNK_BYTES as u64, &io())
        .unwrap();
    ledger.advance(ext, &row, 1, &io()).unwrap();
    let Gate::Go(_) = ledger.commit_adopt(ext, &spec.intent, 11, &io()).unwrap() else {
        panic!("commit adopts");
    };
    ledger.commit_receipt(ext, &spec.intent, &io()).unwrap();
    ledger.unstage(ext, &spec.intent, &io()).unwrap();
    ledger.fence(ext, &NamespaceId([2; 32]), &q, &io()).unwrap();
    ledger.mark_removing(ext, &spec.intent, &io()).unwrap();
    ledger
        .removal_confirmed(ext, &NamespaceId([2; 32]), &io())
        .unwrap();
    let journal = fs::metadata(root.join("remote/objects.db-journal"))
        .unwrap()
        .len();
    assert!(
        empty + journal <= limits::OBJECT_LEDGER_BASE_BYTES,
        "empty ledger {empty} B + largest journal {journal} B exceed the base charge {} B",
        limits::OBJECT_LEDGER_BASE_BYTES
    );
    drop(ledger);
    drop(serving);
}

#[test]
fn removal_releases_one_bounded_batch_at_a_time_and_resumes_where_it_stopped() {
    let guard = Root::new("b");
    let root = guard.0.clone();
    let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
    let ledger = Ledger::open(&serving).unwrap();
    let ext = "a-maximal-extension-name-32-chars";
    let ns = NamespaceId([1; 32]);
    let total = 3 * RELEASE_BATCH as usize + 1;
    {
        let connection = ledger.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO namespaces VALUES (?1,?2,'removing')",
                params![ext, ns.0.as_slice()],
            )
            .unwrap();
        maximal_rows(&connection, 1, total, "removing");
    }
    let state = |ledger: &Ledger| -> (i64, i64, String) {
        let connection = ledger.connection.lock().unwrap();
        let (deleted, charged): (i64, i64) = connection
            .query_row(
                "SELECT COUNT(*),COALESCE(SUM(payload_charged),0) FROM intents WHERE phase='deleted'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let namespace = connection
            .query_row("SELECT state FROM namespaces", [], |r| r.get(0))
            .unwrap();
        (deleted, charged, namespace)
    };
    // One batch, then the process "stops": the rest stay removing and charged.
    assert_eq!(
        ledger.release_removed(ext, &ns, &io()).unwrap(),
        RELEASE_BATCH as usize
    );
    assert_eq!(state(&ledger), (RELEASE_BATCH, 0, "removing".into()));
    let charged: i64 = ledger
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT SUM(payload_charged) FROM intents WHERE phase='removing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        charged,
        (total as i64 - RELEASE_BATCH) * 4096,
        "still charged"
    );
    // Resuming releases exactly the remainder and then closes the namespace.
    ledger.removal_confirmed(ext, &ns, &io()).unwrap();
    assert_eq!(state(&ledger), (total as i64, 0, "removed".into()));
    ledger.removal_confirmed(ext, &ns, &io()).unwrap();
    drop(ledger);
    drop(serving);
}

#[test]
fn a_ledger_wait_that_outlives_the_budget_neither_reads_nor_transitions() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let guard = Root::new("w");
    let root = guard.0.clone();
    let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
    let ledger = Ledger::open(&serving).unwrap();
    let ext = "alpha";
    let spec = BeginSpec {
        intent: IntentId([1; 32]),
        key: BlobKey {
            namespace: NamespaceId([1; 32]),
            object: OpaqueKey([1; 32]),
        },
        payload_sha256: [0; 32],
        payload_bytes: 10,
        binding: vec![],
    };
    let Adoption::Fresh(row) = ledger
        .adopt(ext, &spec, 10, &Quotas::contract(), &io())
        .unwrap()
    else {
        panic!("fresh adoption");
    };
    let cancelled = AtomicBool::new(false);
    let budget = IoBudget {
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
        cancelled: &cancelled,
    };
    // Hold the ledger, start every kind of ledger call, spend the budget while they
    // wait, then release: each must refuse and leave the row exactly as it was.
    let held = ledger.connection.lock().unwrap();
    let outcomes = std::thread::scope(|scope| {
        let workers = [
            scope.spawn(|| ledger.row(ext, &spec.intent, &budget).map(drop)),
            scope.spawn(|| ledger.usage(ext, UsageScope::Extension, &budget).map(drop)),
            scope.spawn(|| ledger.close_confirmed(ext, &spec.intent, &budget)),
            scope.spawn(|| ledger.advance(ext, &row, 1, &budget).map(drop)),
            scope.spawn(|| ledger.settle_unknown(ext, &spec.intent, 1, &budget)),
        ];
        cancelled.store(true, Ordering::SeqCst);
        drop(held);
        workers.map(|worker| worker.join().unwrap())
    });
    for outcome in outcomes {
        assert_eq!(outcome, Err(BackendError::Cancelled));
    }
    let after = ledger.row(ext, &spec.intent, &io()).unwrap().unwrap();
    assert_eq!(
        (after.phase, after.next_index, after.received),
        (Phase::Staging, 0, 0)
    );
    drop(ledger);
    drop(serving);
}

#[test]
fn an_unknown_charge_never_drops_and_an_unrepresentable_one_stops_adoption() {
    let guard = Root::new("u2");
    let root = guard.0.clone();
    let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
    let ledger = Ledger::open(&serving).unwrap();
    let ext = "alpha";
    let spec = |n: u8| BeginSpec {
        intent: IntentId([n; 32]),
        key: BlobKey {
            namespace: NamespaceId([1; 32]),
            object: OpaqueKey([n; 32]),
        },
        payload_sha256: [0; 32],
        payload_bytes: 3 * 4096,
        binding: vec![],
    };
    let quotas = Quotas::contract();
    for n in [1, 2] {
        ledger.adopt(ext, &spec(n), 10, &quotas, &io()).unwrap();
    }
    let charged = |intent: u8| -> i64 {
        ledger
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT payload_charged FROM intents WHERE intent=?1",
                [vec![intent; 32]],
                |r| r.get(0),
            )
            .unwrap()
    };
    // The reservation is a floor: a smaller observation never lowers it.
    ledger
        .settle_unknown(ext, &IntentId([1; 32]), 1, &io())
        .unwrap();
    assert_eq!(charged(1), 3 * 4096);
    ledger
        .settle_unknown(ext, &IntentId([1; 32]), 5 * 4096, &io())
        .unwrap();
    assert_eq!(charged(1), 5 * 4096);
    ledger
        .settle_unknown(ext, &IntentId([1; 32]), 4096, &io())
        .unwrap();
    assert_eq!(
        charged(1),
        5 * 4096,
        "and a later smaller observation neither"
    );
    // A charge no signed integer holds is unavailable and changes nothing.
    assert_eq!(
        ledger.settle_unknown(ext, &IntentId([2; 32]), u64::MAX, &io()),
        Err(BackendError::Unavailable)
    );
    let row = ledger.row(ext, &IntentId([2; 32]), &io()).unwrap().unwrap();
    assert_eq!((row.phase, charged(2)), (Phase::Staging, 3 * 4096));
    // From then on this live ledger adopts nothing new, but repeats still observe.
    assert_eq!(
        ledger.adopt(ext, &spec(3), 10, &quotas, &io()).err(),
        Some(BackendError::Unavailable)
    );
    assert!(matches!(
        ledger.adopt(ext, &spec(2), 10, &quotas, &io()),
        Ok(Adoption::Repeat(_))
    ));
    assert_eq!(
        ledger
            .fence(ext, &NamespaceId([7; 32]), &quotas, &io())
            .err(),
        Some(BackendError::Unavailable)
    );
    drop(ledger);
    drop(serving);
}

#[test]
fn a_settlement_the_budget_refuses_still_stops_adoption() {
    use std::sync::atomic::AtomicBool;
    let spent = AtomicBool::new(true);
    let expired = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let cases: [(&str, IoBudget<'_>, BackendError); 2] = [
        (
            "cancelled",
            IoBudget {
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
                cancelled: &spent,
            },
            BackendError::Cancelled,
        ),
        (
            "deadline",
            IoBudget {
                deadline: expired,
                cancelled: &NEVER,
            },
            BackendError::Deadline,
        ),
    ];
    for (name, budget, expected) in cases {
        let guard = Root::new(name);
        let serving = Layout::open(&guard.0).unwrap().serve_lock().unwrap();
        let ledger = Ledger::open(&serving).unwrap();
        let spec = |n: u8| BeginSpec {
            intent: IntentId([n; 32]),
            key: BlobKey {
                namespace: NamespaceId([1; 32]),
                object: OpaqueKey([n; 32]),
            },
            payload_sha256: [0; 32],
            payload_bytes: 10,
            binding: vec![],
        };
        ledger
            .adopt("alpha", &spec(1), 10, &Quotas::contract(), &io())
            .unwrap();
        assert_eq!(
            ledger.settle_unknown("alpha", &IntentId([1; 32]), 3 * 4096, &budget),
            Err(expected)
        );
        let row = ledger
            .row("alpha", &IntentId([1; 32]), &io())
            .unwrap()
            .unwrap();
        assert_eq!(row.phase, Phase::Staging, "{name}: the row is untouched");
        assert_eq!(
            ledger
                .adopt("alpha", &spec(2), 10, &Quotas::contract(), &io())
                .err(),
            Some(BackendError::Unavailable),
            "{name}: the observation that could not be recorded stops adoption"
        );
    }
}
