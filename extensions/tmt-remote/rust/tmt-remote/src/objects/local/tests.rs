//! Crash windows, races and budgets at exact durable milestones of the real
//! `LocalFs` algorithms. A test observes a named milestone to halt (as a crash
//! would: the state is exactly what was synced), block, or coordinate; then it
//! reopens the same root and checks the durable ledger and tree. Nothing infers
//! an effect from a sleep, and none of this proves power-loss behavior.
use super::*;
use crate::{
    objects::{payload_charge, system_clock},
    state::Layout,
};
use rusqlite::Connection;
use std::{
    fs,
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

const EXT: &str = "alpha";
const DAY: u64 = 86_400_000;
const C: usize = limits::OBJECT_CHUNK_BYTES as usize;
static NEVER: AtomicBool = AtomicBool::new(false);

fn io() -> IoBudget<'static> {
    IoBudget {
        deadline: Instant::now() + Duration::from_secs(60),
        cancelled: &NEVER,
    }
}
fn bytes(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|i| (i as u8).wrapping_mul(29).wrapping_add(seed))
        .collect()
}
fn spec(intent: u8, namespace: u8, object: u8, payload: &[u8]) -> BeginSpec {
    BeginSpec {
        intent: IntentId([intent; 32]),
        key: BlobKey {
            namespace: NamespaceId([namespace; 32]),
            object: super::super::OpaqueKey([object; 32]),
        },
        payload_sha256: Sha256::digest(payload).into(),
        payload_bytes: payload.len() as u64,
        binding: vec![intent; 8],
    }
}
fn receipt_of(sp: &BeginSpec) -> Receipt {
    receipt(&Row {
        extension: EXT.into(),
        spec: sp.clone(),
        adopted_ms: 0,
        expires_ms: 0,
        phase: Phase::Committed,
        next_index: 0,
        received: 0,
        staged: false,
    })
}

struct Env {
    root: PathBuf,
    serving: Serving,
    now: Arc<AtomicU64>,
}
impl Env {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = PathBuf::from(format!(
            "/tmp/t1850u-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        Self {
            root,
            serving,
            now: Arc::new(AtomicU64::new(1_000_000_000)),
        }
    }
    fn open(&self) -> LocalFs<'_> {
        let now = self.now.clone();
        let clock: Clock = Arc::new(move || now.load(Ordering::SeqCst));
        LocalFs::open(&self.serving, Quotas::contract(), clock, &io())
            .expect("the root settles and opens")
    }
    fn ledger(&self) -> Connection {
        Connection::open(self.root.join("remote/objects.db")).unwrap()
    }
    fn row(&self, intent: u8) -> (String, i64, i64, i64, i64) {
        self.ledger()
            .query_row(
                "SELECT phase,payload_charged,entry_held,staged,received FROM intents WHERE intent=?1",
                [vec![intent; 32]],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap()
    }
    fn blob(&self, namespace: u8, object: u8) -> PathBuf {
        self.root
            .join(EXT)
            .join("objects/blobs")
            .join(hex(&[namespace; 32]))
            .join(hex(&[object; 32]))
    }
    fn stage(&self, intent: u8) -> PathBuf {
        self.root
            .join(EXT)
            .join("objects/staging")
            .join(hex(&[intent; 32]))
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn backend<'a>(local: &'a LocalFs<'_>) -> LocalHandle<'a> {
    local.handle(ExtensionId::new(EXT).unwrap())
}
fn send_all(b: &impl ObjectBackend, sp: &BeginSpec, payload: &[u8]) {
    b.begin(sp, &io()).unwrap();
    for (index, part) in payload.chunks(C).enumerate() {
        b.append(sp.intent, index as u32, part, &io()).unwrap();
    }
}
fn halt(point: Milestone) -> Observer {
    Arc::new(move |reached| reached != point)
}
fn halt_once(point: Milestone) -> Observer {
    let fired = AtomicBool::new(false);
    Arc::new(move |reached| reached != point || fired.swap(true, Ordering::SeqCst))
}
/// Every entry under `path` with its mode, length and SHA-256.
fn snapshot(path: &Path) -> Vec<(PathBuf, u32, String)> {
    let mut out = Vec::new();
    let mut stack = vec![path.to_owned()];
    while let Some(next) = stack.pop() {
        let metadata = fs::symlink_metadata(&next).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        if metadata.is_dir() {
            for entry in fs::read_dir(&next).unwrap() {
                stack.push(entry.unwrap().path());
            }
            out.push((next, mode, "directory".into()));
        } else {
            let body = fs::read(&next).unwrap_or_default();
            out.push((
                next,
                mode,
                format!("{}:{}", body.len(), hex(&Sha256::digest(&body))),
            ));
        }
    }
    out.sort();
    out
}
fn plant(path: &Path, body: &[u8]) {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path.parent().unwrap())
        .unwrap();
    let mut file = fs::OpenOptions::new();
    file.write(true).create_new(true).mode(0o600);
    std::io::Write::write_all(&mut file.open(path).unwrap(), body).unwrap();
}

/// Commit a complete upload but stop at `point`; the process "dies" there.
fn commit_stopped_at(env: &Env, point: Milestone, payload: &[u8]) -> BeginSpec {
    let sp = spec(1, 1, 1, payload);
    let local = env.open();
    let b = backend(&local);
    send_all(&b, &sp, payload);
    local.observe(halt(point));
    assert_eq!(
        b.commit(sp.intent, &io()),
        Err(BackendError::Unavailable),
        "{point:?}"
    );
    sp
}
fn assert_settled_committed(env: &Env, sp: &BeginSpec, payload: &[u8]) {
    let local = env.open();
    let b = backend(&local);
    assert_eq!(
        b.status(sp.intent, &io()).unwrap().state,
        TransferState::Committed(receipt_of(sp))
    );
    assert_eq!(fs::read(env.blob(1, 1)).unwrap(), payload);
    assert_eq!(
        fs::metadata(env.blob(1, 1)).unwrap().nlink(),
        1,
        "one name, one charge"
    );
    assert!(!env.stage(1).exists(), "the staging name is gone");
    assert_eq!(env.row(1).0, "committed");
    assert_eq!((env.row(1).3), 0, "nothing left to clean");
    assert_eq!(b.commit(sp.intent, &io()), Ok(receipt_of(sp)));
    assert_eq!(
        b.begin(sp, &io()),
        Ok(BeginResult::Committed(receipt_of(sp)))
    );
    assert_eq!(b.usage(None, &io()).unwrap().entries, 1);
    assert_eq!(b.read(sp.key, 0, 8, &io()).unwrap().bytes, payload[..8]);
}

#[test]
fn a_crash_after_adoption_before_any_byte_keeps_the_charged_original() {
    let env = Env::new();
    let payload = bytes(C + 3, 1);
    let sp = spec(1, 1, 1, &payload);
    {
        let local = env.open();
        local.observe(halt(Milestone::Adopted));
        assert_eq!(
            backend(&local).begin(&sp, &io()),
            Err(BackendError::Unavailable),
            "the acknowledgment was lost"
        );
    }
    let (phase, charged, entry, staged, received) = env.row(1);
    assert_eq!(
        (phase.as_str(), entry, staged, received),
        ("staging", 1, 1, 0)
    );
    assert_eq!(charged as u64, payload_charge(payload.len() as u64));
    assert!(
        !env.root.join(EXT).exists(),
        "reservation precedes any file"
    );
    let local = env.open();
    let b = backend(&local);
    assert!(matches!(
        b.status(sp.intent, &io()).unwrap().state,
        TransferState::Pending(_)
    ));
    for (index, part) in payload.chunks(C).enumerate() {
        b.append(sp.intent, index as u32, part, &io()).unwrap();
    }
    assert_eq!(b.commit(sp.intent, &io()), Ok(receipt_of(&sp)));
}

#[test]
fn an_unacknowledged_tail_is_trimmed_before_readiness_and_the_part_resent() {
    let env = Env::new();
    let payload = bytes(2 * C + 7, 2);
    let sp = spec(1, 1, 1, &payload);
    {
        let local = env.open();
        let b = backend(&local);
        b.begin(&sp, &io()).unwrap();
        b.append(sp.intent, 0, &payload[..C], &io()).unwrap();
        local.observe(halt(Milestone::ChunkSynced));
        assert_eq!(
            b.append(sp.intent, 1, &payload[C..2 * C], &io()),
            Err(BackendError::Unavailable)
        );
    }
    assert_eq!(
        fs::metadata(env.stage(1)).unwrap().len(),
        2 * C as u64,
        "bytes synced"
    );
    assert_eq!(
        env.row(1).4,
        C as i64,
        "but only the first part was acknowledged"
    );
    let local = env.open();
    assert_eq!(
        fs::metadata(env.stage(1)).unwrap().len(),
        C as u64,
        "readiness cut the tail back to the checkpoint"
    );
    let b = backend(&local);
    assert_eq!(
        b.append(sp.intent, 1, &payload[C..2 * C], &io())
            .map(|p| p.received),
        Ok(2 * C as u64)
    );
    b.append(sp.intent, 2, &payload[2 * C..], &io()).unwrap();
    assert_eq!(b.commit(sp.intent, &io()), Ok(receipt_of(&sp)));
    assert_eq!(fs::read(env.blob(1, 1)).unwrap(), payload);
}

#[test]
fn a_part_acknowledgment_is_durable_before_it_is_returned() {
    let env = Env::new();
    let payload = bytes(C + 1, 3);
    let sp = spec(1, 1, 1, &payload);
    let local = env.open();
    let b = backend(&local);
    b.begin(&sp, &io()).unwrap();
    local.observe(halt(Milestone::ChunkAcked));
    assert_eq!(
        b.append(sp.intent, 0, &payload[..C], &io()),
        Err(BackendError::Unavailable)
    );
    assert_eq!(env.row(1).4, C as i64);
    local.observe(Arc::new(|_| true));
    assert_eq!(
        b.append(sp.intent, 0, &payload[..C], &io())
            .map(|p| (p.next_index, p.received)),
        Ok((1, C as u64)),
        "the lost acknowledgment is observed by repeating the same part"
    );
}

#[test]
fn every_commit_window_settles_the_same_original_before_readiness() {
    let payload = bytes(C + 11, 4);
    // Crash before the commit is adopted: nothing is possibly published.
    {
        let env = Env::new();
        let sp = commit_stopped_at(&env, Milestone::CommitVerified, &payload);
        assert_eq!(env.row(1).0, "staging");
        let local = env.open();
        let b = backend(&local);
        assert!(matches!(
            b.status(sp.intent, &io()).unwrap().state,
            TransferState::Pending(_)
        ));
        assert_eq!(b.commit(sp.intent, &io()), Ok(receipt_of(&sp)));
    }
    // Commit adopted: publication is possible from here, in each sub-window.
    for point in [
        Milestone::CommitAdopted,
        Milestone::Linked,
        Milestone::LinkSynced,
    ] {
        let env = Env::new();
        let sp = commit_stopped_at(&env, point, &payload);
        assert_eq!(env.row(1).0, "committing", "{point:?}");
        let linked = env.blob(1, 1).exists();
        assert_eq!(linked, point != Milestone::CommitAdopted, "{point:?}");
        {
            let local = env.open_unsettled_view();
            let b = backend(&local);
            let before = snapshot(&env.root);
            assert_eq!(
                b.status(sp.intent, &io()).unwrap().state,
                TransferState::Unknown,
                "{point:?}"
            );
            assert_eq!(
                b.stat(sp.key, &io()),
                Err(BackendError::Missing),
                "{point:?}"
            );
            assert_eq!(
                b.read(sp.key, 0, 4, &io()),
                Err(BackendError::Missing),
                "{point:?}"
            );
            assert_eq!(
                snapshot(&env.root),
                before,
                "{point:?}: status and reads never repair"
            );
        }
        assert_settled_committed(&env, &sp, &payload);
    }
    // Receipt durable, staging name not yet removed; and fully cleaned.
    {
        let env = Env::new();
        let sp = commit_stopped_at(&env, Milestone::Receipted, &payload);
        assert_eq!((env.row(1).0.as_str(), env.row(1).3), ("committed", 1));
        assert!(env.stage(1).exists());
        assert_eq!(fs::metadata(env.blob(1, 1)).unwrap().nlink(), 2);
        assert_settled_committed(&env, &sp, &payload);
    }
    {
        let env = Env::new();
        let sp = commit_stopped_at(&env, Milestone::StagingUnlinked, &payload);
        assert_eq!(env.row(1).3, 0);
        assert_settled_committed(&env, &sp, &payload);
    }
}
impl Env {
    /// A view of the half-finished root for read-only observation: the ledger is
    /// reopened without settling, by opening with a budget that never reconciles.
    fn open_unsettled_view(&self) -> LocalFs<'_> {
        let now = self.now.clone();
        let clock: Clock = Arc::new(move || now.load(Ordering::SeqCst));
        let ledger = Ledger::open(&self.serving).unwrap();
        LocalFs {
            _lease: &self.serving,
            inner: Inner {
                data_root: self.root.clone(),
                ledger,
                flight: Flight::default(),
                quotas: Quotas::contract(),
                clock,
                observer: Mutex::new(None),
            },
        }
    }
}

#[test]
fn a_repeated_commit_continues_the_same_publication_without_a_restart() {
    let env = Env::new();
    let payload = bytes(C + 2, 5);
    let sp = spec(1, 1, 1, &payload);
    let local = env.open();
    let b = backend(&local);
    send_all(&b, &sp, &payload);
    local.observe(halt_once(Milestone::LinkSynced));
    assert_eq!(b.commit(sp.intent, &io()), Err(BackendError::Unavailable));
    assert_eq!(
        b.status(sp.intent, &io()).unwrap().state,
        TransferState::Unknown
    );
    assert_eq!(
        b.commit(sp.intent, &io()),
        Ok(receipt_of(&sp)),
        "same original, no replacement"
    );
    assert_eq!(fs::read(env.blob(1, 1)).unwrap(), payload);
    assert_eq!(b.usage(None, &io()).unwrap().retained_identities, 1);
}

#[test]
fn a_corrupt_destination_closes_the_original_as_unknown_and_is_never_overwritten() {
    let payload = bytes(C + 6, 6);
    let mut wrong = payload.clone();
    wrong[3] ^= 0xff;
    // Found at restart.
    {
        let env = Env::new();
        let sp = commit_stopped_at(&env, Milestone::CommitAdopted, &payload);
        plant(&env.blob(1, 1), &wrong);
        let local = env.open();
        let b = backend(&local);
        assert_eq!(env.row(1).0, "unknown");
        assert_eq!(
            b.status(sp.intent, &io()).unwrap().state,
            TransferState::Unknown
        );
        assert_eq!(fs::read(env.blob(1, 1)).unwrap(), wrong, "never replaced");
        assert_eq!(
            fs::read(env.stage(1)).unwrap(),
            payload,
            "the staged bytes are kept"
        );
        assert_eq!(b.stat(sp.key, &io()), Err(BackendError::Missing));
        assert_eq!(b.commit(sp.intent, &io()), Err(BackendError::Unavailable));
        assert_eq!(b.discard(sp.intent, &io()), Err(BackendError::Conflict));
        let usage = b.usage(None, &io()).unwrap();
        assert_eq!(usage.entries, 1, "unknown stays charged");
        assert_eq!(
            usage.charged_bytes,
            2 * payload_charge(payload.len() as u64)
                + limits::OBJECT_RECORD_BYTES
                + limits::OBJECT_FENCE_BYTES
                + limits::OBJECT_TREE_BASE_BYTES,
            "two distinct bodies are retained, so both are charged"
        );
        // Unrelated work still proceeds.
        let other = spec(2, 2, 2, b"ok");
        send_all(&b, &other, b"ok");
        assert!(b.commit(other.intent, &io()).is_ok());
        drop(b);
        drop(local);
        assert_eq!(env.row(1).0, "unknown", "stable across restarts");
    }
    // A real create-only race: a conflicting file appears between adoption and link.
    {
        let env = Env::new();
        let sp = spec(1, 1, 1, &payload);
        let local = env.open();
        let b = backend(&local);
        send_all(&b, &sp, &payload);
        let (path, body) = (env.blob(1, 1), wrong.clone());
        local.observe(Arc::new(move |point| {
            if point == Milestone::CommitAdopted {
                plant(&path, &body);
            }
            true
        }));
        assert_eq!(b.commit(sp.intent, &io()), Err(BackendError::Unavailable));
        assert_eq!(env.row(1).0, "unknown");
        assert_eq!(fs::read(env.blob(1, 1)).unwrap(), wrong);
    }
    // Identical bytes in another inode are a second body, not a recovery.
    {
        let env = Env::new();
        let sp = commit_stopped_at(&env, Milestone::CommitAdopted, &payload);
        plant(&env.blob(1, 1), &payload);
        let local = env.open();
        assert_eq!(env.row(1).0, "unknown");
        assert_eq!(
            env.row(1).1,
            2 * payload_charge(payload.len() as u64) as i64
        );
        assert_eq!(
            backend(&local).status(sp.intent, &io()).unwrap().state,
            TransferState::Unknown
        );
        assert_eq!(fs::read(env.stage(1)).unwrap(), payload);
    }
    // The staging name and the destination as one inode is the interrupted link.
    {
        let env = Env::new();
        let sp = commit_stopped_at(&env, Milestone::CommitAdopted, &payload);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(env.blob(1, 1).parent().unwrap())
            .unwrap();
        fs::hard_link(env.stage(1), env.blob(1, 1)).unwrap();
        assert_settled_committed(&env, &sp, &payload);
        assert_eq!(env.row(1).1, payload_charge(payload.len() as u64) as i64);
    }
    // Neither bytes nor staging survived a link: damage, not absence.
    {
        let env = Env::new();
        let sp = commit_stopped_at(&env, Milestone::CommitAdopted, &payload);
        fs::remove_file(env.stage(1)).unwrap();
        let local = env.open();
        assert_eq!(env.row(1).0, "unknown");
        assert_eq!(
            env.row(1).1,
            payload_charge(payload.len() as u64) as i64,
            "an observation of nothing never lowers the reservation"
        );
        assert_eq!(
            backend(&local).status(sp.intent, &io()).unwrap().state,
            TransferState::Unknown
        );
    }
}

#[test]
fn a_failed_measurement_stops_adoption_through_every_live_handle_until_restart() {
    let env = Env::new();
    let payload = bytes(C + 3, 23);
    let mut wrong = payload.clone();
    wrong[0] ^= 1;
    let (sp, other) = (spec(1, 1, 1, &payload), spec(2, 2, 2, &payload));
    let local = env.open();
    let b = backend(&local);
    send_all(&b, &sp, &payload);
    send_all(&b, &other, &payload);
    b.commit(other.intent, &io()).unwrap();
    let pending = spec(3, 3, 3, &payload);
    b.begin(&pending, &io()).unwrap();
    // A conflicting destination appears and the physical measurement then fails.
    let (path, body) = (env.blob(1, 1), wrong.clone());
    local.observe(Arc::new(move |point| {
        if point == Milestone::CommitAdopted {
            plant(&path, &body);
        }
        point != Milestone::Measure
    }));
    assert_eq!(b.commit(sp.intent, &io()), Err(BackendError::Unavailable));
    assert_eq!(env.row(1).0, "committing", "unsettled, not guessed");
    // Every live handle of this ledger refuses new allocation...
    let fresh = spec(9, 9, 9, b"x");
    let beta = local.handle(ExtensionId::new("beta").unwrap());
    for handle in [&b, &beta] {
        assert_eq!(handle.begin(&fresh, &io()), Err(BackendError::Unavailable));
    }
    assert_eq!(
        b.remove_namespace(NamespaceId([8; 32]), &io()),
        Err(BackendError::Unavailable),
        "a new tombstone is an allocation too"
    );
    // ...while existing originals stay observable and settled work stays readable.
    assert_eq!(
        b.status(sp.intent, &io()).unwrap().state,
        TransferState::Unknown
    );
    assert!(matches!(
        b.begin(&pending, &io()),
        Ok(BeginResult::Pending(_))
    ));
    assert_eq!(b.stat(other.key, &io()), Ok(receipt_of(&other)));
    assert_eq!(b.discard(pending.intent, &io()), Ok(()));
    assert_eq!(
        fs::read(env.blob(1, 1)).unwrap(),
        wrong,
        "nothing overwritten"
    );
    drop(b);
    drop(local);
    // A restart measures again, settles the original with both bodies charged, and
    // only then admits work (within the limits the measured charge leaves).
    let local = env.open();
    assert_eq!(env.row(1).0, "unknown");
    assert_eq!(
        env.row(1).1,
        2 * payload_charge(payload.len() as u64) as i64
    );
    assert!(backend(&local).begin(&fresh, &io()).is_ok());
}

#[test]
fn an_unrepresentable_aggregate_is_unavailable_live_and_refuses_readiness_after_restart() {
    let env = Env::new();
    let payload = bytes(64, 24);
    let (first, second) = (spec(1, 1, 1, &payload), spec(2, 2, 2, &payload));
    {
        let local = env.open();
        let b = backend(&local);
        for sp in [&first, &second] {
            send_all(&b, sp, &payload);
            b.commit(sp.intent, &io()).unwrap();
        }
        // Out-of-band fixture corruption: two charges whose sum no integer can hold.
        env.ledger()
            .execute(
                "UPDATE intents SET phase='unknown',payload_charged=9223372036854775807",
                [],
            )
            .unwrap();
        assert_eq!(b.usage(None, &io()), Err(BackendError::Unavailable));
        assert_eq!(
            b.begin(&spec(9, 9, 9, b"x"), &io()),
            Err(BackendError::Unavailable),
            "no allocation is admitted against accounting that cannot be represented"
        );
        assert_eq!(
            b.status(first.intent, &io()).unwrap().state,
            TransferState::Unknown,
            "observation of the original is still answered"
        );
    }
    match LocalFs::open(&env.serving, Quotas::contract(), system_clock(), &io()) {
        Ok(_) => panic!("readiness must refuse an unrepresentable aggregate"),
        Err(error) => assert_eq!(error.code, "REMOTE_OBJECTS_UNAVAILABLE"),
    }
}

#[test]
fn staged_bytes_lost_below_the_checkpoint_are_unknown_not_guessed() {
    let env = Env::new();
    let payload = bytes(2 * C, 7);
    let sp = spec(1, 1, 1, &payload);
    {
        let local = env.open();
        let b = backend(&local);
        b.begin(&sp, &io()).unwrap();
        b.append(sp.intent, 0, &payload[..C], &io()).unwrap();
    }
    fs::remove_file(env.stage(1)).unwrap();
    let local = env.open();
    let b = backend(&local);
    assert_eq!(env.row(1).0, "unknown");
    assert_eq!(
        b.status(sp.intent, &io()).unwrap().state,
        TransferState::Unknown
    );
    assert_eq!(
        b.append(sp.intent, 1, &payload[C..], &io()),
        Err(BackendError::Conflict)
    );
}

#[test]
fn discard_and_expiry_release_charges_only_after_the_files_are_confirmed_gone() {
    let payload = bytes(C + 1, 8);
    for (point, expire) in [
        (Milestone::DiscardAdopted, false),
        (Milestone::CloseUnlinked, false),
        (Milestone::CloseUnlinked, true),
    ] {
        let env = Env::new();
        let sp = spec(1, 1, 1, &payload);
        {
            let local = env.open();
            let b = backend(&local);
            send_all(&b, &sp, &payload[..]);
            local.observe(halt(point));
            let result = if expire {
                env.now.fetch_add(DAY + 1, Ordering::SeqCst);
                b.begin(&sp, &io()).map(|_| ())
            } else {
                b.discard(sp.intent, &io())
            };
            assert_eq!(result, Err(BackendError::Unavailable), "{point:?}");
        }
        let (phase, charged, entry, staged, _) = env.row(1);
        assert_eq!(
            phase,
            if expire { "expiring" } else { "discarding" },
            "{point:?}"
        );
        assert_eq!(
            (entry, staged),
            (1, 1),
            "still charged: nothing confirmed yet"
        );
        assert!(charged > 0);
        assert_eq!(
            env.stage(1).exists(),
            point == Milestone::DiscardAdopted,
            "{point:?}"
        );
        let local = env.open();
        let b = backend(&local);
        let (phase, charged, entry, staged, _) = env.row(1);
        assert_eq!(phase, if expire { "expired" } else { "discarded" });
        assert_eq!(
            (charged, entry, staged),
            (0, 0, 0),
            "released after confirmed cleanup"
        );
        assert!(!env.stage(1).exists());
        assert_eq!(
            b.status(sp.intent, &io()).unwrap().state,
            if expire {
                TransferState::Expired
            } else {
                TransferState::Discarded
            }
        );
        assert_eq!(b.usage(None, &io()).unwrap().retained_identities, 1);
    }
}

#[test]
fn a_deadline_never_downgrades_a_publication_and_aged_staging_closes_at_readiness() {
    let payload = bytes(C + 1, 9);
    // Committing at the deadline stays committed.
    {
        let env = Env::new();
        let sp = commit_stopped_at(&env, Milestone::CommitAdopted, &payload);
        env.now.fetch_add(10 * DAY, Ordering::SeqCst);
        assert_settled_committed(&env, &sp, &payload);
    }
    // Incomplete staging past its deadline closes when the root is next opened.
    {
        let env = Env::new();
        let sp = spec(1, 1, 1, &payload);
        {
            let local = env.open();
            send_all(&backend(&local), &sp, &payload[..]);
        }
        env.now.fetch_add(DAY, Ordering::SeqCst);
        let local = env.open();
        assert_eq!(env.row(1).0, "expired");
        assert!(!env.stage(1).exists());
        assert_eq!(
            backend(&local).status(sp.intent, &io()).unwrap().state,
            TransferState::Expired
        );
    }
}

#[test]
fn every_removal_window_resumes_to_a_closed_namespace_with_retained_originals() {
    let payload = bytes(C + 1, 10);
    for point in [
        Milestone::Fenced,
        Milestone::RemovalMarked,
        Milestone::RemovalUnlinked,
    ] {
        let env = Env::new();
        let (done, open) = (spec(1, 1, 1, &payload), spec(2, 1, 2, &payload));
        {
            let local = env.open();
            let b = backend(&local);
            send_all(&b, &done, &payload[..]);
            b.commit(done.intent, &io()).unwrap();
            send_all(&b, &open, &payload[..]);
            local.observe(halt(point));
            assert_eq!(
                b.remove_namespace(NamespaceId([1; 32]), &io()),
                Err(BackendError::Unavailable),
                "{point:?}"
            );
            // Closed from the moment the fence is durable, before any file is gone.
            local.observe(Arc::new(|_| true));
            assert_eq!(
                b.stat(done.key, &io()),
                Err(BackendError::Missing),
                "{point:?}"
            );
            assert_eq!(
                b.begin(&spec(3, 1, 3, b"x"), &io()),
                Err(BackendError::Conflict)
            );
            assert_eq!(
                b.append(open.intent, 1, &payload[C..], &io()),
                Err(BackendError::Conflict)
            );
        }
        assert_eq!(
            env.blob(1, 1).exists(),
            point != Milestone::RemovalUnlinked,
            "{point:?}"
        );
        assert!(
            env.row(1).2 == 1 || point == Milestone::RemovalUnlinked,
            "{point:?}: still charged"
        );
        let local = env.open();
        let b = backend(&local);
        assert!(!env.blob(1, 1).exists() && !env.stage(2).exists());
        assert!(
            !env.blob(1, 1).parent().unwrap().exists(),
            "the namespace directory is removed"
        );
        for intent in [1, 2] {
            assert_eq!(env.row(intent).0, "deleted");
            assert_eq!((env.row(intent).1, env.row(intent).2), (0, 0));
            assert_eq!(
                b.status(IntentId([intent; 32]), &io()).unwrap().state,
                TransferState::Unavailable
            );
        }
        assert_eq!(
            b.usage(None, &io()).unwrap().charged_bytes,
            2 * limits::OBJECT_RECORD_BYTES
                + limits::OBJECT_FENCE_BYTES
                + limits::OBJECT_TREE_BASE_BYTES
        );
        assert_eq!(
            b.begin(&spec(3, 1, 3, b"x"), &io()),
            Err(BackendError::Conflict)
        );
    }
}

#[test]
fn a_removal_fences_a_pending_publication_so_it_cannot_complete_later() {
    let env = Env::new();
    let payload = bytes(C + 1, 11);
    let sp = spec(1, 1, 1, &payload);
    let local = env.open();
    let b = backend(&local);
    send_all(&b, &sp, &payload);
    local.observe(halt_once(Milestone::CommitAdopted));
    assert_eq!(b.commit(sp.intent, &io()), Err(BackendError::Unavailable));
    assert_eq!(env.row(1).0, "committing");
    assert_eq!(b.remove_namespace(NamespaceId([1; 32]), &io()), Ok(()));
    assert_eq!(env.row(1).0, "deleted");
    assert_eq!(b.commit(sp.intent, &io()), Err(BackendError::Conflict));
    assert!(
        !env.blob(1, 1).exists(),
        "the fenced publication never landed"
    );
    assert!(!env.stage(1).exists());
}

/// Every wait in a paused-worker fixture is bounded by this.
const BOUND: Duration = Duration::from_secs(20);

/// Releases a paused worker when dropped, including while a failed assertion unwinds.
struct ReleaseOnDrop<'a>(&'a dyn Fn());
impl Drop for ReleaseOnDrop<'_> {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// Run `commit` on a worker that pauses at `CommitAdopted` and call `during` while
/// it is paused. The release is owned by a drop guard, so a failing assertion in
/// `during` still releases and joins the worker, and every wait is bounded: the
/// fixture fails instead of hanging.
fn while_commit_paused<R>(
    local: &LocalFs<'_>,
    b: &LocalHandle<'_>,
    sp: &BeginSpec,
    during: impl FnOnce(&dyn Fn()) -> R,
) -> (BackendResult<Receipt>, R) {
    let (reached_tx, reached_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (reached_tx, release_rx) = (Mutex::new(reached_tx), Mutex::new(release_rx));
    local.observe(Arc::new(move |point| {
        if point == Milestone::CommitAdopted {
            let _ = reached_tx.lock().unwrap().send(());
            let _ = release_rx.lock().unwrap().recv_timeout(BOUND);
        }
        true
    }));
    let release = move || {
        let _ = release_tx.send(());
    };
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| b.commit(sp.intent, &io()));
        let _outer = ReleaseOnDrop(&release);
        reached_rx
            .recv_timeout(BOUND)
            .expect("the commit reached its paused point");
        let during = during(&release);
        release();
        (worker.join().unwrap(), during)
    })
}

/// Poll the durable namespace fence; readiness, not time.
fn wait_for_fence(env: &Env) {
    let deadline = Instant::now() + BOUND;
    loop {
        let state: Option<String> = env
            .ledger()
            .query_row("SELECT state FROM namespaces", [], |r| r.get(0))
            .ok();
        if state.as_deref() == Some("removing") {
            return;
        }
        assert!(Instant::now() < deadline, "the fence never became durable");
        std::thread::yield_now();
    }
}

#[test]
fn removal_waits_for_the_commit_it_fenced_and_refuses_everything_after_the_fence() {
    let env = Env::new();
    let payload = bytes(C + 1, 12);
    let sp = spec(1, 1, 1, &payload);
    let local = env.open();
    let b = backend(&local);
    send_all(&b, &sp, &payload);
    let removed = AtomicBool::new(false);
    let (commit, removal) = while_commit_paused(&local, &b, &sp, |release| {
        std::thread::scope(|inner| {
            // Declared inside the scope: on a failed assertion it releases the paused
            // commit before the scope joins the removal that is waiting for it.
            let _inner = ReleaseOnDrop(release);
            let removal = inner.spawn(|| {
                let result = b.remove_namespace(NamespaceId([1; 32]), &io());
                removed.store(true, Ordering::SeqCst);
                result
            });
            wait_for_fence(&env);
            assert!(
                !removed.load(Ordering::SeqCst),
                "removal cannot pass its drain"
            );
            assert_eq!(
                env.row(1).0,
                "committing",
                "the adopted commit is not torn down"
            );
            assert_eq!(
                b.begin(&spec(2, 1, 2, b"x"), &io()),
                Err(BackendError::Conflict)
            );
            assert_eq!(b.stat(sp.key, &io()), Err(BackendError::Missing));
            release();
            removal.join().unwrap()
        })
    });
    assert_eq!(commit, Ok(receipt_of(&sp)));
    assert_eq!(removal, Ok(()));
    assert_eq!(env.row(1).0, "deleted");
    assert!(!env.blob(1, 1).exists() && !env.stage(1).exists());
    assert_eq!(
        b.status(sp.intent, &io()).unwrap().state,
        TransferState::Unavailable
    );
}

#[test]
fn a_failing_assertion_while_the_commit_is_paused_releases_and_joins_every_worker() {
    let env = Env::new();
    let payload = bytes(C + 1, 22);
    let sp = spec(1, 1, 1, &payload);
    let local = env.open();
    let b = backend(&local);
    send_all(&b, &sp, &payload);
    let started = Instant::now();
    // The hardest case: a removal is blocked behind the paused commit when the
    // assertion fails, so the unwinding scope must release the commit to join it.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        while_commit_paused(&local, &b, &sp, |release| {
            std::thread::scope(|inner| {
                let _inner = ReleaseOnDrop(release);
                inner.spawn(|| b.remove_namespace(NamespaceId([1; 32]), &io()));
                wait_for_fence(&env);
                panic!("deliberately failing assertion");
            })
        })
    }));
    assert!(outcome.is_err(), "the failure is reported");
    assert!(
        started.elapsed() < BOUND,
        "the fixture failed within its bound ({:?})",
        started.elapsed()
    );
    // Both workers ran to completion before the unwind finished.
    assert_eq!(env.row(1).0, "deleted");
    assert!(!env.blob(1, 1).exists() && !env.stage(1).exists());
}

#[test]
fn a_fence_after_the_bytes_are_read_suppresses_disclosure() {
    let env = Env::new();
    let payload = bytes(64, 13);
    let sp = spec(1, 1, 1, &payload);
    let local = env.open();
    let b = backend(&local);
    send_all(&b, &sp, &payload);
    b.commit(sp.intent, &io()).unwrap();
    let ledger = env.root.join("remote/objects.db");
    local.observe(Arc::new(move |point| {
        if point == Milestone::ReadBytes {
            Connection::open(&ledger)
                .unwrap()
                .execute("UPDATE namespaces SET state='removing'", [])
                .unwrap();
        }
        true
    }));
    assert_eq!(
        b.read(sp.key, 0, 16, &io()),
        Err(BackendError::Missing),
        "the read finished after the namespace was fenced: nothing is returned"
    );
}

#[test]
fn a_spent_budget_leaves_the_effect_and_charge_and_the_state_is_settled_by_the_same_id() {
    let payload = bytes(C + 4, 14);
    let cases = [
        (Milestone::CommitVerified, "staging"),
        (Milestone::CommitAdopted, "committing"),
        (Milestone::LinkSynced, "committing"),
        (Milestone::Receipted, "committed"),
    ];
    for (point, phase) in cases {
        let env = Env::new();
        let sp = spec(1, 1, 1, &payload);
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let local = env.open();
            let b = backend(&local);
            send_all(&b, &sp, &payload);
            let flag = cancelled.clone();
            local.observe(Arc::new(move |reached| {
                if reached == point {
                    flag.store(true, Ordering::SeqCst);
                }
                true
            }));
            let budget = IoBudget {
                deadline: Instant::now() + Duration::from_secs(60),
                cancelled: &cancelled,
            };
            assert_eq!(
                b.commit(sp.intent, &budget),
                Err(BackendError::Cancelled),
                "{point:?}"
            );
        }
        assert_eq!(env.row(1).0, phase, "{point:?}: no rollback");
        assert_eq!(env.row(1).2, 1, "{point:?}: the entry stays charged");
        let local = env.open();
        let b = backend(&local);
        assert_eq!(b.commit(sp.intent, &io()), Ok(receipt_of(&sp)), "{point:?}");
        assert_eq!(fs::read(env.blob(1, 1)).unwrap(), payload);
        drop(local);
    }
}

#[test]
fn a_read_with_a_spent_budget_discloses_nothing() {
    let env = Env::new();
    let payload = bytes(64, 15);
    let sp = spec(1, 1, 1, &payload);
    let local = env.open();
    let b = backend(&local);
    send_all(&b, &sp, &payload);
    b.commit(sp.intent, &io()).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    local.observe(Arc::new(move |point| {
        if point == Milestone::ReadBytes {
            flag.store(true, Ordering::SeqCst);
        }
        true
    }));
    let budget = IoBudget {
        deadline: Instant::now() + Duration::from_secs(60),
        cancelled: &cancelled,
    };
    assert_eq!(b.read(sp.key, 0, 8, &budget), Err(BackendError::Cancelled));
}

/// Child half of the SIGKILL probe: commits until the link is made, then blocks
/// forever on a FIFO nobody writes, holding the serve lease.
#[test]
fn probe_child() {
    let Some(root) = std::env::var_os("TMT_OBJECTS_PROBE") else {
        return;
    };
    let root = PathBuf::from(root);
    let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
    let local = LocalFs::open(&serving, Quotas::contract(), system_clock(), &io()).unwrap();
    let b = backend(&local);
    let payload = bytes(C + 5, 16);
    let sp = spec(1, 1, 1, &payload);
    send_all(&b, &sp, &payload);
    local.observe(Arc::new(move |point| {
        if point == Milestone::Linked {
            let partial = root.join("ready.partial");
            fs::write(&partial, b"1").unwrap();
            fs::rename(&partial, root.join("ready")).unwrap();
            let mut gate = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(root.join("gate"))
                .unwrap();
            let mut byte = [0u8; 1];
            let _ = gate.read(&mut byte);
        }
        true
    }));
    let _ = b.commit(sp.intent, &io());
}

#[test]
fn a_killed_process_leaves_a_published_file_without_a_receipt_that_the_next_lease_holder_settles() {
    use nix::{sys::stat::Mode, unistd::mkfifo};
    use std::{
        os::unix::process::ExitStatusExt,
        process::{Child, Command, Stdio},
    };
    struct Probe {
        child: Child,
        root: PathBuf,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    let root = PathBuf::from(format!("/tmp/t1850k-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    mkfifo(&root.join("gate"), Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "objects::local::tests::probe_child",
            "--nocapture",
        ])
        .env("TMT_OBJECTS_PROBE", &root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut probe = Probe {
        child,
        root: root.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while !root.join("ready").exists() {
        assert!(
            probe.child.try_wait().unwrap().is_none(),
            "the probe exited before reaching the milestone"
        );
        assert!(
            Instant::now() < deadline,
            "the probe never reached the milestone"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // The lease is held by the live child.
    assert!(matches!(
        Layout::open(&root).unwrap().serve_lock(),
        Err(error) if error.code == "REMOTE_ALREADY_SERVING"
    ));
    probe.child.kill().unwrap();
    let status = probe.child.wait().unwrap();
    assert_eq!(status.signal(), Some(9), "killed, not exited");
    // What the dead process left: the adopted commit and a linked file, no receipt.
    let key = root
        .join(EXT)
        .join("objects/blobs")
        .join(hex(&[1; 32]))
        .join(hex(&[1; 32]));
    let phase: String = Connection::open(root.join("remote/objects.db"))
        .unwrap()
        .query_row("SELECT phase FROM intents", [], |r| r.get(0))
        .unwrap();
    assert_eq!(phase, "committing");
    assert_eq!(fs::metadata(&key).unwrap().nlink(), 2);
    // The next lease holder settles the same original before readiness.
    let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
    let local = LocalFs::open(&serving, Quotas::contract(), system_clock(), &io()).unwrap();
    let payload = bytes(C + 5, 16);
    let sp = spec(1, 1, 1, &payload);
    assert_eq!(
        backend(&local).status(sp.intent, &io()).unwrap().state,
        TransferState::Committed(receipt_of(&sp))
    );
    assert_eq!(fs::read(&key).unwrap(), payload);
    assert_eq!(fs::metadata(&key).unwrap().nlink(), 1);
    assert_eq!(
        fs::read_dir(root.join(EXT).join("objects/staging"))
            .unwrap()
            .count(),
        0
    );
    drop(local);
    drop(serving);
}

#[test]
fn every_successful_return_rechecks_the_budget_before_disclosing() {
    let env = Env::new();
    let local = env.open();
    let b = backend(&local);
    let payload = bytes(C + 1, 21);
    let (done, open, gone) = (
        spec(1, 1, 1, &payload),
        spec(2, 2, 2, &payload),
        spec(3, 3, 3, &payload),
    );
    send_all(&b, &done, &payload);
    b.commit(done.intent, &io()).unwrap();
    b.begin(&open, &io()).unwrap();
    b.append(open.intent, 0, &payload[..C], &io()).unwrap();
    b.begin(&gone, &io()).unwrap();
    b.discard(gone.intent, &io()).unwrap();
    b.remove_namespace(NamespaceId([9; 32]), &io()).unwrap();
    // The budget is spent exactly as each operation is about to return its result.
    let cancelled = Arc::new(AtomicBool::new(false));
    let returns = Arc::new(AtomicUsize::new(0));
    let (flag, count) = (cancelled.clone(), returns.clone());
    local.observe(Arc::new(move |point| {
        if point == Milestone::Return {
            count.fetch_add(1, Ordering::SeqCst);
            flag.store(true, Ordering::SeqCst);
        }
        true
    }));
    type Op<'a> = Box<dyn Fn(&IoBudget<'_>) -> Result<(), BackendError> + 'a>;
    let ops: Vec<(&str, Op<'_>)> = vec![
        (
            "begin repeat, pending",
            Box::new(|io| b.begin(&open, io).map(drop)),
        ),
        (
            "begin repeat, committed",
            Box::new(|io| b.begin(&done, io).map(drop)),
        ),
        (
            "begin repeat, terminal",
            Box::new(|io| b.begin(&gone, io).map(drop)),
        ),
        (
            "append repeat of an acknowledged part",
            Box::new(|io| b.append(open.intent, 0, &payload[..C], io).map(drop)),
        ),
        (
            "commit of a committed original",
            Box::new(|io| b.commit(done.intent, io).map(drop)),
        ),
        (
            "status of an unknown original",
            Box::new(|io| b.status(IntentId([7; 32]), io).map(drop)),
        ),
        (
            "status of a committed original",
            Box::new(|io| b.status(done.intent, io).map(drop)),
        ),
        ("stat", Box::new(|io| b.stat(done.key, io).map(drop))),
        ("read", Box::new(|io| b.read(done.key, 0, 4, io).map(drop))),
        (
            "extension usage",
            Box::new(|io| b.usage(None, io).map(drop)),
        ),
        (
            "namespace usage",
            Box::new(|io| b.usage(Some(NamespaceId([1; 32])), io).map(drop)),
        ),
        (
            "installation usage",
            Box::new(|io| local.installation_usage(io).map(drop)),
        ),
        (
            "discard of a discarded original",
            Box::new(|io| b.discard(gone.intent, io)),
        ),
        (
            "removal of an already removed namespace",
            Box::new(|io| b.remove_namespace(NamespaceId([9; 32]), io)),
        ),
    ];
    for (name, op) in ops {
        cancelled.store(false, Ordering::SeqCst);
        returns.store(0, Ordering::SeqCst);
        let budget = IoBudget {
            deadline: Instant::now() + Duration::from_secs(60),
            cancelled: &cancelled,
        };
        assert_eq!(op(&budget), Err(BackendError::Cancelled), "{name}");
        assert_eq!(
            returns.load(Ordering::SeqCst),
            1,
            "{name}: reached its return exactly once"
        );
    }
}
