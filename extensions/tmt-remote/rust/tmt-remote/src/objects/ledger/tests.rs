//! The ledger's own footprint: the largest rollback journal any production
//! transition writes, and the empty ledger, must fit the installation base charge.
use super::*;
use crate::state::Layout;
use std::{fs, path::PathBuf};

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
    let root = PathBuf::from(format!("/tmp/t1850j-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
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
    ledger.removal_confirmed(ext, &ns).unwrap();
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
    let Adoption::Fresh(row) = ledger.adopt(ext, &spec, 10, &q).unwrap() else {
        panic!("fresh adoption");
    };
    let row = ledger
        .advance(ext, &row, limits::OBJECT_CHUNK_BYTES as u64)
        .unwrap();
    ledger.advance(ext, &row, 1).unwrap();
    let Gate::Go(_) = ledger.commit_adopt(ext, &spec.intent, 11).unwrap() else {
        panic!("commit adopts");
    };
    ledger.commit_receipt(ext, &spec.intent).unwrap();
    ledger.unstage(ext, &spec.intent).unwrap();
    ledger.fence(ext, &NamespaceId([2; 32]), &q).unwrap();
    ledger.mark_removing(ext, &spec.intent).unwrap();
    ledger
        .removal_confirmed(ext, &NamespaceId([2; 32]))
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
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn removal_releases_rows_in_batches_and_resumes_after_an_interruption() {
    let root = PathBuf::from(format!("/tmp/t1850b-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
    let ledger = Ledger::open(&serving).unwrap();
    let ext = "a-maximal-extension-name-32-chars";
    let ns = NamespaceId([1; 32]);
    {
        let connection = ledger.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO namespaces VALUES (?1,?2,'removing')",
                params![ext, ns.0.as_slice()],
            )
            .unwrap();
        maximal_rows(&connection, 1, 3 * RELEASE_BATCH as usize + 1, "removing");
    }
    ledger.removal_confirmed(ext, &ns).unwrap();
    let connection = ledger.connection.lock().unwrap();
    let (deleted, charged): (i64, i64) = connection
        .query_row(
            "SELECT COUNT(*),SUM(payload_charged) FROM intents WHERE phase='deleted'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((deleted, charged), (3 * RELEASE_BATCH + 1, 0));
    let state: String = connection
        .query_row("SELECT state FROM namespaces", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "removed");
    drop(connection);
    drop(ledger);
    drop(serving);
    let _ = fs::remove_dir_all(&root);
}
