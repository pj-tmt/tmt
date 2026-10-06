//! Reusable backend scenarios. An adapter implements [`Adapter`]; `conformance!`
//! turns every scenario into a test. Scenarios use only the public interface and
//! the documented charge formula (see the object-backends reference).
use sha2::{Digest, Sha256};
use std::{
    sync::{Barrier, atomic::AtomicBool},
    time::{Duration, Instant},
};
use tmt_remote::{
    limits,
    objects::{
        BackendError, BeginResult, BeginSpec, BlobKey, IntentId, IoBudget, Limit, NamespaceId,
        ObjectBackend, OpaqueKey, Progress, Quotas, Receipt, TransferState, Usage,
    },
};

pub const CHUNK: usize = limits::OBJECT_CHUNK_BYTES as usize;
pub const DAY: u64 = 24 * 60 * 60 * 1000;

/// A running adapter whose extension handles share one accounting.
pub trait Session: Sync {
    fn backend(&self, extension: &str) -> Box<dyn ObjectBackend + '_>;
    fn advance(&self, milliseconds: u64);
    /// Drop and reopen the adapter on its durable state.
    fn restart(&mut self);
    fn installation(&self) -> Usage;
}
pub trait Adapter {
    fn run(&self, quotas: Quotas, scenario: &mut dyn FnMut(&mut dyn Session));
}

static NEVER: AtomicBool = AtomicBool::new(false);
pub fn io() -> IoBudget<'static> {
    IoBudget {
        deadline: Instant::now() + Duration::from_secs(60),
        cancelled: &NEVER,
    }
}
pub fn bytes(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}
pub fn spec(intent: u8, namespace: u8, object: u8, payload: &[u8]) -> BeginSpec {
    BeginSpec {
        intent: IntentId([intent; 32]),
        key: key(namespace, object),
        payload_sha256: Sha256::digest(payload).into(),
        payload_bytes: payload.len() as u64,
        binding: vec![intent; 16],
    }
}
pub fn key(namespace: u8, object: u8) -> BlobKey {
    BlobKey {
        namespace: NamespaceId([namespace; 32]),
        object: OpaqueKey([object; 32]),
    }
}
pub fn receipt(spec: &BeginSpec) -> Receipt {
    Receipt {
        intent: spec.intent,
        key: spec.key,
        payload_sha256: spec.payload_sha256,
        payload_bytes: spec.payload_bytes,
        binding: spec.binding.clone(),
    }
}
pub fn pending(spec: &BeginSpec, parts: u32, received: u64) -> Progress {
    Progress {
        intent: spec.intent,
        next_index: parts,
        received,
    }
}
/// Payload bytes are charged rounded up to the physical block.
pub fn round(length: usize) -> u64 {
    (length as u64).div_ceil(limits::OBJECT_BLOCK_BYTES) * limits::OBJECT_BLOCK_BYTES
}
pub const RECORD: u64 = limits::OBJECT_RECORD_BYTES;
pub const FENCE: u64 = limits::OBJECT_FENCE_BYTES;
pub const BASE: u64 = limits::OBJECT_LEDGER_BASE_BYTES;
/// The fixed payload tree an extension with any namespace owns.
pub const TREE: u64 = limits::OBJECT_TREE_BASE_BYTES;
/// What one adoption adds when its namespace already has a fence row.
pub fn row_charge(length: usize) -> u64 {
    round(length) + RECORD
}

/// Begin, send canonical parts and commit through the interface alone.
pub fn upload(backend: &dyn ObjectBackend, spec: &BeginSpec, payload: &[u8]) -> Receipt {
    backend.begin(spec, &io()).unwrap();
    let chunk = backend.capabilities().chunk_bytes as usize;
    for (index, part) in payload.chunks(chunk).enumerate() {
        backend
            .append(spec.intent, index as u32, part, &io())
            .unwrap();
    }
    backend.commit(spec.intent, &io()).unwrap()
}
fn namespace_usage(backend: &dyn ObjectBackend, namespace: u8) -> Usage {
    backend
        .usage(Some(NamespaceId([namespace; 32])), &io())
        .unwrap()
}
fn contract() -> Quotas {
    Quotas::contract()
}

pub fn commit_publishes_exact_bytes(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(2 * CHUNK + 4464, 1);
        let sp = spec(1, 1, 1, &payload);
        assert_eq!(
            backend.begin(&sp, &io()),
            Ok(BeginResult::Pending(pending(&sp, 0, 0)))
        );
        assert_eq!(backend.stat(sp.key, &io()), Err(BackendError::Missing));
        assert_eq!(
            backend.read(sp.key, 0, 16, &io()),
            Err(BackendError::Missing)
        );
        for (index, part) in payload.chunks(CHUNK).enumerate() {
            let received = ((index + 1) * CHUNK).min(payload.len()) as u64;
            assert_eq!(
                backend.append(sp.intent, index as u32, part, &io()),
                Ok(pending(&sp, index as u32 + 1, received))
            );
            // Incomplete objects never become readable.
            assert_eq!(backend.stat(sp.key, &io()), Err(BackendError::Missing));
            assert_eq!(
                backend.read(sp.key, 0, 16, &io()),
                Err(BackendError::Missing)
            );
        }
        assert_eq!(backend.commit(sp.intent, &io()), Ok(receipt(&sp)));
        assert_eq!(backend.stat(sp.key, &io()), Ok(receipt(&sp)));
        let part = backend.read(sp.key, 0, 100, &io()).unwrap();
        assert_eq!((part.offset, part.total_bytes), (0, payload.len() as u64));
        assert_eq!(part.bytes, payload[..100]);
        let part = backend
            .read(sp.key, CHUNK as u64, CHUNK as u32, &io())
            .unwrap();
        assert_eq!(part.bytes, payload[CHUNK..2 * CHUNK]);
        let tail = backend
            .read(sp.key, 2 * CHUNK as u64, CHUNK as u32, &io())
            .unwrap();
        assert_eq!(
            tail.bytes,
            payload[2 * CHUNK..],
            "a read stops at the payload end"
        );
        assert_eq!(
            backend.status(sp.intent, &io()).unwrap().state,
            TransferState::Committed(receipt(&sp))
        );
        assert_eq!(
            backend.usage(None, &io()),
            Ok(Usage {
                charged_bytes: row_charge(payload.len()) + FENCE + TREE,
                entries: 1,
                active_uploads: 0,
                retained_identities: 1
            })
        );
    });
}

pub fn empty_payload_commits_without_parts(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let sp = spec(1, 1, 1, b"");
        assert_eq!(
            backend.begin(&sp, &io()),
            Ok(BeginResult::Pending(pending(&sp, 0, 0)))
        );
        assert_eq!(
            namespace_usage(&*backend, 1),
            Usage {
                charged_bytes: RECORD + FENCE,
                entries: 1,
                active_uploads: 1,
                retained_identities: 1
            },
            "an empty payload still reserves an entry and its metadata"
        );
        assert_eq!(backend.commit(sp.intent, &io()), Ok(receipt(&sp)));
        let part = backend.read(sp.key, 0, 1, &io()).unwrap();
        assert_eq!((part.total_bytes, part.bytes.len()), (0, 0));
        assert_eq!(
            backend.read(sp.key, 1, 1, &io()),
            Err(BackendError::Invalid)
        );
    });
}

pub fn lost_acknowledgments_observe_the_original(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(CHUNK + 10, 2);
        let sp = spec(1, 1, 1, &payload);
        let first = backend.begin(&sp, &io()).unwrap();
        let usage = backend.usage(None, &io()).unwrap();
        assert_eq!(
            backend.begin(&sp, &io()),
            Ok(first),
            "a repeated begin observes"
        );
        assert_eq!(
            backend.usage(None, &io()),
            Ok(usage),
            "and adopts nothing new"
        );
        let acknowledged = backend
            .append(sp.intent, 0, &payload[..CHUNK], &io())
            .unwrap();
        assert_eq!(
            backend.append(sp.intent, 0, &payload[..CHUNK], &io()),
            Ok(acknowledged)
        );
        backend
            .append(sp.intent, 1, &payload[CHUNK..], &io())
            .unwrap();
        let committed = backend.commit(sp.intent, &io()).unwrap();
        assert_eq!(backend.commit(sp.intent, &io()), Ok(committed.clone()));
        assert_eq!(
            backend.begin(&sp, &io()),
            Ok(BeginResult::Committed(committed.clone()))
        );
        assert_eq!(
            backend.usage(None, &io()).unwrap().entries,
            1,
            "repeats never reserve again"
        );
        let transfer = backend.status(sp.intent, &io()).unwrap();
        assert_eq!(transfer.state, TransferState::Committed(committed));
        let original = transfer.original.unwrap();
        assert_eq!(original.spec, sp);
        assert!(original.expires_at_ms > original.adopted_at_ms);
    });
}

pub fn changed_input_conflicts_and_identity_is_per_extension(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let alpha = s.backend("alpha");
        let beta = s.backend("beta");
        let payload = bytes(100, 3);
        let sp = spec(1, 1, 1, &payload);
        alpha.begin(&sp, &io()).unwrap();
        let changed = [
            BeginSpec {
                key: key(2, 1),
                ..sp.clone()
            },
            BeginSpec {
                key: key(1, 2),
                ..sp.clone()
            },
            BeginSpec {
                payload_sha256: [9; 32],
                ..sp.clone()
            },
            BeginSpec {
                payload_bytes: 101,
                ..sp.clone()
            },
            BeginSpec {
                binding: vec![0; 3],
                ..sp.clone()
            },
            BeginSpec {
                binding: vec![],
                ..sp.clone()
            },
        ];
        for other in changed {
            assert_eq!(
                alpha.begin(&other, &io()),
                Err(BackendError::Conflict),
                "{other:?}"
            );
        }
        // The same original ID under another extension is a different identity.
        assert_eq!(
            beta.status(sp.intent, &io()).unwrap().state,
            TransferState::NotObserved
        );
        assert_eq!(
            beta.begin(&sp, &io()),
            Ok(BeginResult::Pending(pending(&sp, 0, 0)))
        );
        assert_eq!(beta.usage(None, &io()).unwrap().entries, 1);
        assert_eq!(alpha.usage(None, &io()).unwrap().entries, 1);
        // A different original may not claim a live key.
        assert_eq!(
            alpha.begin(&spec(2, 1, 1, &payload), &io()),
            Err(BackendError::Conflict)
        );
        assert_eq!(
            alpha.status(IntentId([7; 32]), &io()).unwrap().state,
            TransferState::NotObserved,
            "absence is not proof of no effect, only an unobserved original"
        );
    });
}

pub fn parts_are_canonical_ordered_and_exact(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(2 * CHUNK + 5, 4);
        let sp = spec(1, 1, 1, &payload);
        backend.begin(&sp, &io()).unwrap();
        let first = &payload[..CHUNK];
        assert_eq!(
            backend.append(sp.intent, 1, &payload[CHUNK..2 * CHUNK], &io()),
            Err(BackendError::Conflict),
            "out of order"
        );
        for noncanonical in [
            &first[..100],
            &payload[..CHUNK + 1],
            &first[..CHUNK - 1],
            &[][..],
        ] {
            assert_eq!(
                backend.append(sp.intent, 0, noncanonical, &io()),
                Err(BackendError::Invalid),
                "{} bytes",
                noncanonical.len()
            );
        }
        assert_eq!(
            backend.status(sp.intent, &io()).unwrap().state,
            TransferState::Pending(pending(&sp, 0, 0)),
            "rejected parts have no effect"
        );
        backend.append(sp.intent, 0, first, &io()).unwrap();
        let mut altered = first.to_vec();
        altered[0] ^= 1;
        assert_eq!(
            backend.append(sp.intent, 0, &altered, &io()),
            Err(BackendError::Conflict),
            "a repeat must equal the acknowledged bytes"
        );
        assert_eq!(
            backend.commit(sp.intent, &io()),
            Err(BackendError::Conflict),
            "an incomplete payload cannot commit"
        );
        backend
            .append(sp.intent, 1, &payload[CHUNK..2 * CHUNK], &io())
            .unwrap();
        backend
            .append(sp.intent, 2, &payload[2 * CHUNK..], &io())
            .unwrap();
        assert_eq!(
            backend.append(sp.intent, 3, &[1], &io()),
            Err(BackendError::Conflict),
            "no part beyond the declared length"
        );
        assert_eq!(backend.commit(sp.intent, &io()), Ok(receipt(&sp)));
        assert_eq!(
            backend.append(sp.intent, 0, first, &io()),
            Err(BackendError::Conflict),
            "a committed original takes no more parts"
        );
    });
}

pub fn digest_mismatch_never_publishes(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(CHUNK, 5);
        let mut sp = spec(1, 1, 1, &payload);
        sp.payload_sha256 = [0; 32];
        backend.begin(&sp, &io()).unwrap();
        backend.append(sp.intent, 0, &payload, &io()).unwrap();
        assert_eq!(backend.commit(sp.intent, &io()), Err(BackendError::Invalid));
        assert_eq!(backend.stat(sp.key, &io()), Err(BackendError::Missing));
        assert!(matches!(
            backend.status(sp.intent, &io()).unwrap().state,
            TransferState::Pending(_)
        ));
        assert_eq!(backend.discard(sp.intent, &io()), Ok(()));
    });
}

pub fn reads_are_bounded_and_exact(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(1000, 6);
        let sp = spec(1, 1, 1, &payload);
        upload(&*backend, &sp, &payload);
        assert_eq!(
            backend.read(sp.key, 0, 0, &io()),
            Err(BackendError::Invalid)
        );
        assert_eq!(
            backend.read(sp.key, 0, limits::OBJECT_CHUNK_BYTES + 1, &io()),
            Err(BackendError::Invalid)
        );
        assert_eq!(
            backend.read(sp.key, 1000, 1, &io()),
            Err(BackendError::Invalid)
        );
        assert_eq!(
            backend.read(sp.key, 5000, 1, &io()),
            Err(BackendError::Invalid)
        );
        assert_eq!(
            backend.read(sp.key, 990, 100, &io()).unwrap().bytes,
            payload[990..]
        );
        assert_eq!(
            backend.read(key(1, 9), 0, 1, &io()),
            Err(BackendError::Missing),
            "an unknown key"
        );
        assert_eq!(
            s.backend("beta").read(sp.key, 0, 1, &io()),
            Err(BackendError::Missing),
            "another extension cannot name this object"
        );
    });
}

pub fn expiry_closes_staging_for_good(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(CHUNK + 1, 7);
        let sp = spec(1, 1, 1, &payload);
        backend.begin(&sp, &io()).unwrap();
        backend
            .append(sp.intent, 0, &payload[..CHUNK], &io())
            .unwrap();
        s.advance(DAY - 1);
        assert!(matches!(
            backend.status(sp.intent, &io()).unwrap().state,
            TransferState::Pending(_)
        ));
        s.advance(1);
        let transfer = backend.status(sp.intent, &io()).unwrap();
        assert_eq!(transfer.state, TransferState::Expired);
        assert_eq!(
            transfer.original.unwrap().spec,
            sp,
            "the original stays retained"
        );
        assert_eq!(
            backend.append(sp.intent, 1, &payload[CHUNK..], &io()),
            Err(BackendError::Conflict)
        );
        assert_eq!(
            backend.commit(sp.intent, &io()),
            Err(BackendError::Conflict)
        );
        assert_eq!(
            backend.begin(&sp, &io()),
            Ok(BeginResult::Terminal(TransferState::Expired)),
            "an expired original never becomes executable again"
        );
        assert_eq!(
            namespace_usage(&*backend, 1),
            Usage {
                charged_bytes: RECORD + FENCE,
                entries: 0,
                active_uploads: 0,
                retained_identities: 1
            },
            "payload and entry are released, the retained identity stays charged"
        );
        // A new original may claim the freed key; the old one stays dead.
        let again = spec(2, 1, 1, &payload);
        assert!(matches!(
            backend.begin(&again, &io()),
            Ok(BeginResult::Pending(_))
        ));
        assert_eq!(
            backend.begin(&sp, &io()),
            Ok(BeginResult::Terminal(TransferState::Expired))
        );
    });
}

pub fn completed_objects_do_not_expire(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(500, 8);
        let sp = spec(1, 1, 1, &payload);
        upload(&*backend, &sp, &payload);
        s.advance(400 * DAY);
        assert_eq!(backend.stat(sp.key, &io()), Ok(receipt(&sp)));
        assert_eq!(backend.read(sp.key, 0, 500, &io()).unwrap().bytes, payload);
        assert_eq!(
            backend.status(sp.intent, &io()).unwrap().state,
            TransferState::Committed(receipt(&sp))
        );
        assert_eq!(
            backend.begin(&sp, &io()),
            Ok(BeginResult::Committed(receipt(&sp)))
        );
    });
}

pub fn discard_closes_the_original(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(CHUNK + 1, 9);
        let sp = spec(1, 1, 1, &payload);
        assert_eq!(
            backend.discard(sp.intent, &io()),
            Err(BackendError::Missing)
        );
        backend.begin(&sp, &io()).unwrap();
        backend
            .append(sp.intent, 0, &payload[..CHUNK], &io())
            .unwrap();
        assert_eq!(backend.discard(sp.intent, &io()), Ok(()));
        assert_eq!(backend.discard(sp.intent, &io()), Ok(()), "idempotent");
        assert_eq!(
            backend.status(sp.intent, &io()).unwrap().state,
            TransferState::Discarded
        );
        assert_eq!(
            backend.begin(&sp, &io()),
            Ok(BeginResult::Terminal(TransferState::Discarded))
        );
        assert_eq!(
            backend.append(sp.intent, 1, &payload[CHUNK..], &io()),
            Err(BackendError::Conflict)
        );
        assert_eq!(
            backend.commit(sp.intent, &io()),
            Err(BackendError::Conflict)
        );
        assert_eq!(
            namespace_usage(&*backend, 1),
            Usage {
                charged_bytes: RECORD + FENCE,
                entries: 0,
                active_uploads: 0,
                retained_identities: 1
            }
        );
        let done = spec(2, 1, 2, &payload);
        upload(&*backend, &done, &payload);
        assert_eq!(
            backend.discard(done.intent, &io()),
            Err(BackendError::Conflict),
            "a committed original is not discardable"
        );
        assert_eq!(backend.stat(done.key, &io()), Ok(receipt(&done)));
    });
}

fn saturate(
    adapter: &dyn Adapter,
    quotas: Quotas,
    limit: Limit,
    first: &mut dyn FnMut(&dyn ObjectBackend),
    refused: &mut dyn FnMut(&dyn ObjectBackend) -> BeginSpec,
) {
    adapter.run(quotas, &mut |s| {
        let backend = s.backend("alpha");
        first(&*backend);
        let before = (backend.usage(None, &io()).unwrap(), s.installation());
        let spec = refused(&*backend);
        assert_eq!(
            backend.begin(&spec, &io()),
            Err(BackendError::Capacity(limit)),
            "{limit:?}"
        );
        assert_eq!(
            (backend.usage(None, &io()).unwrap(), s.installation()),
            before,
            "a refusal leaves no charge, entry or identity"
        );
        assert_eq!(
            backend.status(spec.intent, &io()).unwrap().state,
            TransferState::NotObserved
        );
    });
}
fn one(backend: &dyn ObjectBackend, intent: u8, namespace: u8, object: u8) {
    backend
        .begin(&spec(intent, namespace, object, b"x"), &io())
        .unwrap();
}

pub fn active_intents_are_capped_and_freed_by_discard(adapter: &dyn Adapter) {
    let quotas = Quotas {
        active_intents: 2,
        ..contract()
    };
    saturate(
        adapter,
        quotas,
        Limit::ActiveIntents,
        &mut |b| {
            one(b, 1, 1, 1);
            one(b, 2, 1, 2);
        },
        &mut |_| spec(3, 1, 3, b"x"),
    );
    adapter.run(quotas, &mut |s| {
        let backend = s.backend("alpha");
        one(&*backend, 1, 1, 1);
        one(&*backend, 2, 1, 2);
        backend.discard(IntentId([1; 32]), &io()).unwrap();
        one(&*backend, 3, 1, 3);
    });
}
pub fn retained_identities_are_never_evicted(adapter: &dyn Adapter) {
    let discarded = |b: &dyn ObjectBackend, intent: u8, object: u8| {
        one(b, intent, 1, object);
        b.discard(IntentId([intent; 32]), &io()).unwrap();
    };
    saturate(
        adapter,
        Quotas {
            retained_extension: 2,
            ..contract()
        },
        Limit::RetainedExtension,
        &mut |b| {
            discarded(b, 1, 1);
            discarded(b, 2, 2);
        },
        &mut |_| spec(3, 1, 3, b"x"),
    );
    adapter.run(
        Quotas {
            retained_extension: 2,
            ..contract()
        },
        &mut |s| {
            let (alpha, beta) = (s.backend("alpha"), s.backend("beta"));
            discarded(&*alpha, 1, 1);
            discarded(&*alpha, 2, 2);
            one(&*beta, 3, 1, 3);
            assert_eq!(
                alpha.begin(&spec(1, 1, 1, b"x"), &io()),
                Ok(BeginResult::Terminal(TransferState::Discarded)),
                "a full extension still answers its retained originals"
            );
        },
    );
    saturate(
        adapter,
        Quotas {
            retained_installation: 2,
            ..contract()
        },
        Limit::RetainedInstallation,
        &mut |b| {
            discarded(b, 1, 1);
            discarded(b, 2, 2);
        },
        &mut |_| spec(3, 1, 3, b"x"),
    );
}
pub fn entries_are_reserved_before_bytes_at_every_level(adapter: &dyn Adapter) {
    saturate(
        adapter,
        Quotas {
            namespace_entries: 2,
            ..contract()
        },
        Limit::NamespaceEntries,
        &mut |b| {
            one(b, 1, 1, 1);
            one(b, 2, 1, 2);
        },
        &mut |_| spec(3, 1, 3, b"x"),
    );
    saturate(
        adapter,
        Quotas {
            extension_entries: 2,
            ..contract()
        },
        Limit::ExtensionEntries,
        &mut |b| {
            one(b, 1, 1, 1);
            one(b, 2, 2, 2);
        },
        &mut |_| spec(3, 3, 3, b"x"),
    );
    adapter.run(
        Quotas {
            installation_entries: 2,
            ..contract()
        },
        &mut |s| {
            // The second slot belongs to another extension; the first is full for both.
            one(&*s.backend("alpha"), 1, 1, 1);
            one(&*s.backend("beta"), 2, 1, 1);
            assert_eq!(
                s.backend("beta").begin(&spec(3, 1, 3, b"x"), &io()),
                Err(BackendError::Capacity(Limit::InstallationEntries))
            );
            s.backend("alpha")
                .discard(IntentId([1; 32]), &io())
                .unwrap();
            one(&*s.backend("beta"), 3, 1, 3);
        },
    );
}
pub fn bytes_saturate_exactly_at_every_level(adapter: &dyn Adapter) {
    let one_object = round(1) + RECORD + FENCE;
    // The first adoption fits exactly; the limit minus one byte refuses it.
    for (limit, quotas) in [
        (
            Limit::NamespaceBytes,
            Quotas {
                namespace_bytes: one_object - 1,
                ..contract()
            },
        ),
        (
            Limit::ExtensionBytes,
            Quotas {
                extension_bytes: one_object + TREE - 1,
                ..contract()
            },
        ),
        (
            Limit::InstallationBytes,
            Quotas {
                installation_bytes: BASE + one_object + TREE - 1,
                ..contract()
            },
        ),
    ] {
        adapter.run(quotas, &mut |s| {
            let backend = s.backend("alpha");
            assert_eq!(
                backend.begin(&spec(1, 1, 1, b"x"), &io()),
                Err(BackendError::Capacity(limit))
            );
            assert_eq!(backend.usage(None, &io()).unwrap(), Usage::default());
        });
    }
    saturate(
        adapter,
        Quotas {
            namespace_bytes: one_object,
            ..contract()
        },
        Limit::NamespaceBytes,
        &mut |b| one(b, 1, 1, 1),
        &mut |_| spec(2, 1, 2, b"x"),
    );
    saturate(
        adapter,
        Quotas {
            extension_bytes: one_object + TREE,
            ..contract()
        },
        Limit::ExtensionBytes,
        &mut |b| one(b, 1, 1, 1),
        &mut |_| spec(2, 2, 2, b"x"),
    );
    saturate(
        adapter,
        Quotas {
            installation_bytes: BASE + one_object + TREE,
            ..contract()
        },
        Limit::InstallationBytes,
        &mut |b| one(b, 1, 1, 1),
        &mut |_| spec(2, 2, 2, b"x"),
    );
    // Another extension's use counts against the installation but not this extension.
    adapter.run(
        Quotas {
            installation_bytes: BASE + 2 * (one_object + TREE),
            ..contract()
        },
        &mut |s| {
            one(&*s.backend("alpha"), 1, 1, 1);
            one(&*s.backend("beta"), 2, 1, 1);
            assert_eq!(
                s.backend("alpha").begin(&spec(3, 2, 3, b"x"), &io()),
                Err(BackendError::Capacity(Limit::InstallationBytes))
            );
            assert_eq!(
                s.installation().charged_bytes,
                BASE + 2 * (one_object + TREE)
            );
        },
    );
    // A discard releases payload and entry, never the retained identity.
    adapter.run(
        Quotas {
            namespace_bytes: one_object,
            ..contract()
        },
        &mut |s| {
            let backend = s.backend("alpha");
            one(&*backend, 1, 1, 1);
            backend.discard(IntentId([1; 32]), &io()).unwrap();
            assert_eq!(namespace_usage(&*backend, 1).charged_bytes, RECORD + FENCE);
            assert_eq!(
                backend.begin(&spec(2, 1, 2, b"x"), &io()),
                Err(BackendError::Capacity(Limit::NamespaceBytes)),
                "the retained record still holds its share of the namespace"
            );
        },
    );
}
pub fn many_tiny_and_empty_objects_are_charged_each(adapter: &dyn Adapter) {
    let count = 200;
    // Room for the empty objects and exactly one more one-byte object.
    let quotas = Quotas {
        namespace_bytes: count * RECORD + FENCE + round(1) + RECORD,
        namespace_entries: count as u32 + 1,
        active_intents: 1000,
        ..contract()
    };
    adapter.run(quotas, &mut |s| {
        let backend = s.backend("alpha");
        for n in 0..count as u16 {
            let id = |fill: u8| {
                let mut id = [fill; 32];
                id[..2].copy_from_slice(&n.to_be_bytes());
                id
            };
            let sp = BeginSpec {
                intent: IntentId(id(1)),
                key: BlobKey {
                    namespace: NamespaceId([1; 32]),
                    object: OpaqueKey(id(2)),
                },
                payload_sha256: Sha256::digest(b"").into(),
                payload_bytes: 0,
                // The largest binding: a retained record is charged in full regardless.
                binding: vec![7; limits::OBJECT_BINDING_BYTES],
            };
            backend.begin(&sp, &io()).unwrap();
            backend.commit(sp.intent, &io()).unwrap();
        }
        assert_eq!(
            namespace_usage(&*backend, 1),
            Usage {
                charged_bytes: count * RECORD + FENCE,
                entries: count as u32,
                active_uploads: 0,
                retained_identities: count as u32
            },
            "each empty object holds an entry and a full metadata record"
        );
        // One more tiny object fits exactly; the next is refused before any allocation.
        one(&*backend, 250, 1, 250);
        assert_eq!(
            backend.begin(&spec(251, 1, 251, b"x"), &io()),
            Err(BackendError::Capacity(Limit::NamespaceEntries))
        );
    });
}

pub fn namespace_removal_closes_for_good(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let (alpha, beta) = (s.backend("alpha"), s.backend("beta"));
        let payload = bytes(CHUNK + 7, 10);
        let (committed, staged) = (spec(1, 1, 1, &payload), spec(2, 1, 2, &payload));
        let kept = spec(3, 2, 1, &payload);
        upload(&*alpha, &committed, &payload);
        alpha.begin(&staged, &io()).unwrap();
        alpha
            .append(staged.intent, 0, &payload[..CHUNK], &io())
            .unwrap();
        upload(&*alpha, &kept, &payload);
        let other = spec(1, 1, 1, &payload);
        upload(&*beta, &other, &payload);
        assert_eq!(alpha.remove_namespace(NamespaceId([1; 32]), &io()), Ok(()));
        assert_eq!(alpha.stat(committed.key, &io()), Err(BackendError::Missing));
        assert_eq!(
            alpha.read(committed.key, 0, 1, &io()),
            Err(BackendError::Missing)
        );
        for sp in [&committed, &staged] {
            let transfer = alpha.status(sp.intent, &io()).unwrap();
            assert_eq!(transfer.state, TransferState::Unavailable);
            assert_eq!(
                transfer.original.unwrap().spec,
                *sp,
                "the original binding is retained outside the removed payload"
            );
            assert_eq!(
                alpha.begin(sp, &io()),
                Ok(BeginResult::Terminal(TransferState::Unavailable)),
                "a removed original is never executable again"
            );
        }
        assert_eq!(
            alpha.begin(&spec(9, 1, 9, &payload), &io()),
            Err(BackendError::Conflict),
            "a removed namespace stays closed to new originals"
        );
        assert_eq!(
            alpha.append(staged.intent, 1, &payload[CHUNK..], &io()),
            Err(BackendError::Conflict)
        );
        assert_eq!(
            namespace_usage(&*alpha, 1),
            Usage {
                charged_bytes: 2 * RECORD + FENCE,
                entries: 0,
                active_uploads: 0,
                retained_identities: 2
            },
            "payloads and entries are released; tombstones stay charged"
        );
        assert_eq!(alpha.remove_namespace(NamespaceId([1; 32]), &io()), Ok(()));
        // Neighbours are untouched.
        assert_eq!(
            alpha.read(kept.key, 0, 4, &io()).unwrap().bytes,
            payload[..4]
        );
        assert_eq!(
            beta.read(other.key, 0, 4, &io()).unwrap().bytes,
            payload[..4]
        );
        // A namespace that never held anything is closed too.
        assert_eq!(alpha.remove_namespace(NamespaceId([5; 32]), &io()), Ok(()));
        assert_eq!(
            alpha.begin(&spec(8, 5, 8, &payload), &io()),
            Err(BackendError::Conflict)
        );
        assert_eq!(
            namespace_usage(&*alpha, 5),
            Usage {
                charged_bytes: FENCE,
                ..Usage::default()
            }
        );
    });
}

pub fn spent_budgets_stop_work_without_effect(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(50, 11);
        let sp = spec(1, 1, 1, &payload);
        let cancelled = AtomicBool::new(true);
        let stop = IoBudget {
            deadline: Instant::now() + Duration::from_secs(60),
            cancelled: &cancelled,
        };
        assert_eq!(backend.begin(&sp, &stop), Err(BackendError::Cancelled));
        let late = IoBudget {
            deadline: Instant::now(),
            cancelled: &NEVER,
        };
        assert_eq!(backend.begin(&sp, &late), Err(BackendError::Deadline));
        assert_eq!(backend.usage(None, &io()).unwrap(), Usage::default());
        assert_eq!(
            backend.status(sp.intent, &io()).unwrap().state,
            TransferState::NotObserved
        );
        upload(&*backend, &sp, &payload);
        assert_eq!(backend.stat(sp.key, &stop), Err(BackendError::Cancelled));
        assert_eq!(
            backend.read(sp.key, 0, 5, &late),
            Err(BackendError::Deadline)
        );
        assert_eq!(backend.usage(None, &late), Err(BackendError::Deadline));
        assert_eq!(
            backend.status(sp.intent, &late),
            Err(BackendError::Deadline)
        );
        assert_eq!(
            backend.commit(sp.intent, &stop),
            Err(BackendError::Cancelled)
        );
        assert_eq!(
            backend.stat(sp.key, &io()),
            Ok(receipt(&sp)),
            "state is unchanged"
        );
    });
}

pub fn restart_preserves_originals_and_progress(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let payload = bytes(2 * CHUNK, 12);
        let (sp, done) = (spec(1, 1, 1, &payload), spec(2, 1, 2, &payload));
        {
            let backend = s.backend("alpha");
            backend.begin(&sp, &io()).unwrap();
            backend
                .append(sp.intent, 0, &payload[..CHUNK], &io())
                .unwrap();
            upload(&*backend, &done, &payload);
        }
        let usage = s.installation();
        s.restart();
        assert_eq!(s.installation(), usage);
        let backend = s.backend("alpha");
        assert_eq!(
            backend.status(sp.intent, &io()).unwrap().state,
            TransferState::Pending(pending(&sp, 1, CHUNK as u64)),
            "a restart never reissues the original"
        );
        assert_eq!(
            backend.begin(&sp, &io()),
            Ok(BeginResult::Pending(pending(&sp, 1, CHUNK as u64)))
        );
        backend
            .append(sp.intent, 1, &payload[CHUNK..], &io())
            .unwrap();
        assert_eq!(backend.commit(sp.intent, &io()), Ok(receipt(&sp)));
        assert_eq!(backend.stat(done.key, &io()), Ok(receipt(&done)));
        assert_eq!(
            backend.begin(&done, &io()),
            Ok(BeginResult::Committed(receipt(&done)))
        );
    });
}

pub fn commit_and_discard_race_has_one_winner(adapter: &dyn Adapter) {
    adapter.run(
        Quotas {
            active_intents: 64,
            ..contract()
        },
        &mut |s| {
            let backend = s.backend("alpha");
            let payload = bytes(CHUNK + 3, 13);
            for round in 0..24u8 {
                let sp = spec(round + 1, 1, round + 1, &payload);
                backend.begin(&sp, &io()).unwrap();
                backend
                    .append(sp.intent, 0, &payload[..CHUNK], &io())
                    .unwrap();
                backend
                    .append(sp.intent, 1, &payload[CHUNK..], &io())
                    .unwrap();
                let barrier = Barrier::new(2);
                let (commit, discard) = std::thread::scope(|scope| {
                    let commit = scope.spawn(|| {
                        barrier.wait();
                        backend.commit(sp.intent, &io())
                    });
                    let discard = scope.spawn(|| {
                        barrier.wait();
                        backend.discard(sp.intent, &io())
                    });
                    (commit.join().unwrap(), discard.join().unwrap())
                });
                match (commit, discard) {
                    (Ok(committed), Err(BackendError::Conflict)) => {
                        assert_eq!(committed, receipt(&sp));
                        assert_eq!(backend.stat(sp.key, &io()), Ok(receipt(&sp)));
                        assert_eq!(
                            backend.status(sp.intent, &io()).unwrap().state,
                            TransferState::Committed(receipt(&sp))
                        );
                    }
                    (Err(BackendError::Conflict), Ok(())) => {
                        assert_eq!(backend.stat(sp.key, &io()), Err(BackendError::Missing));
                        assert_eq!(
                            backend.status(sp.intent, &io()).unwrap().state,
                            TransferState::Discarded
                        );
                    }
                    other => panic!("exactly one of commit and discard may win: {other:?}"),
                }
            }
            let usage = namespace_usage(&*backend, 1);
            assert_eq!(usage.active_uploads, 0);
            assert_eq!(usage.retained_identities, 24);
            let committed = usage.entries as u64;
            assert_eq!(
                usage.charged_bytes,
                committed * round_up_payload(payload.len()) + 24 * RECORD + FENCE,
                "only committed originals still hold payload"
            );
        },
    );
}
fn round_up_payload(length: usize) -> u64 {
    round(length)
}
pub fn commit_and_removal_serialize(adapter: &dyn Adapter) {
    adapter.run(
        Quotas {
            active_intents: 64,
            ..contract()
        },
        &mut |s| {
            let backend = s.backend("alpha");
            let payload = bytes(CHUNK + 3, 14);
            for round in 0..16u8 {
                let sp = spec(round + 1, round + 1, 1, &payload);
                backend.begin(&sp, &io()).unwrap();
                backend
                    .append(sp.intent, 0, &payload[..CHUNK], &io())
                    .unwrap();
                backend
                    .append(sp.intent, 1, &payload[CHUNK..], &io())
                    .unwrap();
                let barrier = Barrier::new(2);
                let (commit, removal) = std::thread::scope(|scope| {
                    let commit = scope.spawn(|| {
                        barrier.wait();
                        backend.commit(sp.intent, &io())
                    });
                    let removal = scope.spawn(|| {
                        barrier.wait();
                        backend.remove_namespace(sp.key.namespace, &io())
                    });
                    (commit.join().unwrap(), removal.join().unwrap())
                });
                assert_eq!(removal, Ok(()));
                assert!(
                    matches!(commit, Ok(_) | Err(BackendError::Conflict)),
                    "{commit:?}"
                );
                // Whoever won, the namespace is closed and nothing is disclosed or charged.
                assert_eq!(backend.stat(sp.key, &io()), Err(BackendError::Missing));
                assert_eq!(
                    backend.status(sp.intent, &io()).unwrap().state,
                    TransferState::Unavailable
                );
                let usage = namespace_usage(&*backend, round + 1);
                assert_eq!((usage.entries, usage.active_uploads), (0, 0));
                assert_eq!(usage.charged_bytes, RECORD + FENCE);
            }
        },
    );
}
pub fn concurrent_begins_admit_exactly_the_capacity(adapter: &dyn Adapter) {
    adapter.run(
        Quotas {
            namespace_entries: 5,
            active_intents: 100,
            ..contract()
        },
        &mut |s| {
            let backend = s.backend("alpha");
            let barrier = Barrier::new(16);
            let results: Vec<_> = std::thread::scope(|scope| {
                let workers: Vec<_> = (0..16u8)
                    .map(|n| {
                        let (backend, barrier) = (&backend, &barrier);
                        scope.spawn(move || {
                            barrier.wait();
                            backend.begin(&spec(n + 1, 1, n + 1, b"x"), &io())
                        })
                    })
                    .collect();
                workers.into_iter().map(|w| w.join().unwrap()).collect()
            });
            let admitted = results
                .iter()
                .filter(|r| matches!(r, Ok(BeginResult::Pending(_))))
                .count();
            assert_eq!(admitted, 5, "{results:?}");
            assert!(results.iter().all(|r| matches!(
                r,
                Ok(_) | Err(BackendError::Capacity(Limit::NamespaceEntries))
            )));
            assert_eq!(namespace_usage(&*backend, 1).entries, 5);
        },
    );
}
pub fn duplicate_commits_return_one_receipt(adapter: &dyn Adapter) {
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        let payload = bytes(CHUNK * 2, 15);
        let sp = spec(1, 1, 1, &payload);
        backend.begin(&sp, &io()).unwrap();
        backend
            .append(sp.intent, 0, &payload[..CHUNK], &io())
            .unwrap();
        backend
            .append(sp.intent, 1, &payload[CHUNK..], &io())
            .unwrap();
        let barrier = Barrier::new(6);
        let results: Vec<_> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..6)
                .map(|_| {
                    let (backend, barrier) = (&backend, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        backend.commit(sp.intent, &io())
                    })
                })
                .collect();
            workers.into_iter().map(|w| w.join().unwrap()).collect()
        });
        assert!(
            results.iter().all(|r| *r == Ok(receipt(&sp))),
            "{results:?}"
        );
        assert_eq!(namespace_usage(&*backend, 1).entries, 1);
    });
}
pub fn a_consumer_needs_only_the_interface(adapter: &dyn Adapter) {
    // The consumer below names no adapter; it is the whole of what a service may rely on.
    fn consumer(backend: &dyn ObjectBackend) -> Vec<u8> {
        let caps = backend.capabilities();
        assert!(caps.max_payload_bytes >= limits::OBJECT_PAYLOAD_BYTES);
        let payload = bytes(caps.chunk_bytes as usize + 17, 16);
        let sp = spec(1, 1, 1, &payload);
        let committed = upload(backend, &sp, &payload);
        assert_eq!(backend.stat(committed.key, &io()), Ok(committed.clone()));
        let mut out = Vec::new();
        let mut offset = 0;
        while offset < committed.payload_bytes {
            let part = backend
                .read(committed.key, offset, caps.chunk_bytes, &io())
                .unwrap();
            offset += part.bytes.len() as u64;
            out.extend(part.bytes);
        }
        assert_eq!(Sha256::digest(&out).as_slice(), committed.payload_sha256);
        out
    }
    adapter.run(contract(), &mut |s| {
        let backend = s.backend("alpha");
        assert_eq!(consumer(&*backend), bytes(CHUNK + 17, 16));
        assert!(!backend.id().is_empty());
    });
}

/// One test per scenario for an adapter expression.
#[macro_export]
macro_rules! conformance {
    ($adapter:expr) => {
        $crate::conformance!(@each $adapter;
            commit_publishes_exact_bytes
            empty_payload_commits_without_parts
            lost_acknowledgments_observe_the_original
            changed_input_conflicts_and_identity_is_per_extension
            parts_are_canonical_ordered_and_exact
            digest_mismatch_never_publishes
            reads_are_bounded_and_exact
            expiry_closes_staging_for_good
            completed_objects_do_not_expire
            discard_closes_the_original
            active_intents_are_capped_and_freed_by_discard
            retained_identities_are_never_evicted
            entries_are_reserved_before_bytes_at_every_level
            bytes_saturate_exactly_at_every_level
            many_tiny_and_empty_objects_are_charged_each
            namespace_removal_closes_for_good
            spent_budgets_stop_work_without_effect
            restart_preserves_originals_and_progress
            commit_and_discard_race_has_one_winner
            commit_and_removal_serialize
            concurrent_begins_admit_exactly_the_capacity
            duplicate_commits_return_one_receipt
            a_consumer_needs_only_the_interface
        );
    };
    (@each $adapter:expr; $($name:ident)+) => {
        $(
            #[test]
            fn $name() {
                $crate::conformance::$name(&$adapter);
            }
        )+
    };
}
