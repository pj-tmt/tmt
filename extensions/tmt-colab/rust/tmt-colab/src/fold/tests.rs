//! Pure private arithmetic and one-stream gzip admission controls.
use super::*;
use flate2::{Compression, write::GzEncoder};
use std::io::Write;

#[test]
fn batch_delta_admission_accepts_exact_limits_and_rejects_count_bytes_and_overflow() {
    let page = "10000000-0000-4000-8000-000000000001";
    let mut input = MaterializationInput {
        baseline: Vec::new(),
        updates: Vec::new(),
        own_updates: BTreeMap::new(),
        signing_keys: BTreeMap::new(),
        owner_provenance: BTreeMap::new(),
        local_writer: String::new(),
        tally: crate::page::UsageCounts {
            updates: crate::decoder::WRITE_TAIL_UPDATES - 1,
            tail_bytes: crate::decoder::WRITE_TAIL_BYTES - 1,
            ..Default::default()
        },
    };
    assert!(input.admit_deltas(page, 1, 1).is_ok());
    for (count, bytes) in [(2, 1), (1, 2)] {
        let error = input.admit_deltas(page, count, bytes).unwrap_err();
        assert!(matches!(error.downcast_ref::<OwnerFault>(),
            Some(OwnerFault::PageCapacity(capacity)) if capacity.edit));
    }
    input.tally.updates += 1;
    input.tally.tail_bytes += 1;
    assert!(input.admit_deltas(page, 0, 0).is_ok());
    assert!(input.admit_deltas(page, 1, 0).is_err());
    assert!(input.admit_deltas(page, 0, 1).is_err());
    for (count, bytes) in [(usize::MAX, 0), (0, usize::MAX)] {
        let error = input.admit_deltas(page, count, bytes).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<OwnerFault>(),
            Some(OwnerFault::Capacity)
        ));
    }
}

#[test]
fn usage_counts_the_tail_and_the_checkpoints_apart_and_names_other_devices_changes() {
    let mut tally = crate::page::UsageCounts::default();
    tally.add(false, true, 100).unwrap();
    tally.add(false, false, 50).unwrap();
    tally.add(false, false, 25).unwrap();
    tally.add(true, true, 4000).unwrap();
    tally.add(true, false, 1000).unwrap();
    let usage = crate::page::PageUsage::new(tally, 5175, None);
    assert_eq!(
        (
            usage.updates,
            usage.tail_bytes,
            usage.checkpoint_bytes,
            usage.other_device_updates
        ),
        (3, 175, 5000, 2),
        "a checkpoint is never a change after a checkpoint, whoever wrote it"
    );
    assert_eq!(usage.update_limit, crate::decoder::WRITE_TAIL_UPDATES);
    assert_eq!(usage.tail_bytes_limit, crate::decoder::WRITE_TAIL_BYTES);
    let mut full = crate::page::UsageCounts::default();
    full.add(false, true, usize::MAX).unwrap();
    assert!(
        full.add(false, true, 1).is_err(),
        "bytes overflow is a capacity error"
    );
    full.tail_bytes = 0;
    full.updates = usize::MAX;
    assert!(
        full.add(false, true, 1).is_err(),
        "count overflow is a capacity error"
    );
}

#[test]
fn usage_percent_is_the_fullest_of_the_tail_limits_and_the_state_budget_and_passes_100_for_a_reader()
 {
    use crate::{
        decoder::*,
        page::{PageUsage, UsageCounts},
    };
    let percent = |updates, tail_bytes, state, gzip| {
        PageUsage::new(
            UsageCounts {
                updates,
                tail_bytes,
                ..Default::default()
            },
            state,
            gzip,
        )
        .percent
    };
    assert_eq!(percent(0, 0, 0, None), 0);
    assert_eq!(percent(100, 0, 0, None), 50);
    assert_eq!(percent(0, WRITE_TAIL_BYTES / 4 * 3, 0, None), 75);
    assert_eq!(percent(40, WRITE_TAIL_BYTES / 2, 0, None), 50);
    assert_eq!(
        percent(199, 0, 0, None),
        99,
        "one change short of the limit never shows 100"
    );
    assert_eq!(percent(200, 0, 0, None), 100);
    assert_eq!(percent(1000, 0, 0, None), 500);
    assert_eq!(
        percent(usize::MAX, 0, 0, None),
        usize::MAX / WRITE_TAIL_UPDATES
    );
    // An unmeasured state bounds the gzipped size from above; a measured one replaces it.
    let budget = PAGE_BUDGET_GZIP_BYTES;
    assert_eq!(percent(0, 0, budget / 4, None), 25);
    assert_eq!(percent(0, 0, budget * 2, Some(budget / 5 * 4)), 80);
    assert_eq!(
        percent(150, 0, budget, Some(budget / 2)),
        75,
        "the changes are the fuller limit"
    );
    assert_eq!(percent(10, 0, budget * 3, Some(budget)), 100);
}

#[test]
fn usage_names_the_limit_it_is_nearest_to_and_a_tie_goes_to_the_changes_first() {
    use crate::{
        decoder::*,
        page::{Limiting, PageUsage, UsageCounts},
    };
    let limiting = |updates, tail_bytes, state, gzip| {
        PageUsage::new(
            UsageCounts {
                updates,
                tail_bytes,
                ..Default::default()
            },
            state,
            gzip,
        )
        .limiting
    };
    assert_eq!(limiting(0, 0, 0, None), Limiting::Changes);
    assert_eq!(limiting(100, 0, 0, None), Limiting::Changes);
    assert_eq!(
        limiting(10, WRITE_TAIL_BYTES / 2, 0, None),
        Limiting::RecentData
    );
    assert_eq!(
        limiting(
            10,
            0,
            PAGE_BUDGET_GZIP_BYTES * 2,
            Some(PAGE_BUDGET_GZIP_BYTES / 2)
        ),
        Limiting::StoredSize
    );
    assert_eq!(
        limiting(0, 0, PAGE_BUDGET_GZIP_BYTES / 2, None),
        Limiting::StoredSize
    );
    // 50% of the changes and 50% of the bytes and 50% of the budget: the changes name it.
    assert_eq!(
        limiting(
            100,
            WRITE_TAIL_BYTES / 2,
            0,
            Some(PAGE_BUDGET_GZIP_BYTES / 2)
        ),
        Limiting::Changes
    );
    assert_eq!(
        limiting(0, WRITE_TAIL_BYTES / 2, 0, Some(PAGE_BUDGET_GZIP_BYTES / 2)),
        Limiting::RecentData
    );
}

#[test]
fn usage_gzips_the_state_only_from_a_quarter_mebibyte_and_counts_every_part_of_it() {
    let input = |baseline: usize, updates: Vec<Vec<u8>>, own: Vec<Vec<u8>>| MaterializationInput {
        baseline: vec![7; baseline],
        updates,
        own_updates: BTreeMap::from([("device".to_owned(), own)]),
        signing_keys: BTreeMap::new(),
        owner_provenance: BTreeMap::new(),
        local_writer: String::new(),
        tally: Default::default(),
    };
    let small = input(1000, vec![vec![1; 1000]], vec![vec![2; 1000]])
        .usage(true)
        .unwrap();
    assert_eq!((small.state_bytes, small.gzip_bytes), (3000, None));
    let edge = GZIP_MEASURED_FROM;
    let below = input(0, vec![vec![1; edge - 1]], vec![])
        .usage(true)
        .unwrap();
    assert_eq!(below.gzip_bytes, None);
    // Repetitive bytes compress to almost nothing, and the baseline, the content updates and the
    // own updates all count toward the one stream.
    let big = input(edge, vec![vec![1; edge]], vec![vec![2; edge]]);
    let counted = big.usage(false).unwrap();
    assert_eq!(
        (counted.state_bytes, counted.gzip_bytes),
        (3 * edge, None),
        "a caller that only needs the counts never compresses"
    );
    let large = big.usage(true).unwrap();
    assert_eq!(large.state_bytes, 3 * edge);
    let gzip = large.gzip_bytes.expect("measured from a quarter mebibyte");
    assert!(0 < gzip && gzip < edge / 100, "{gzip}");
    assert_eq!(large.percent, gzip * 100 / large.gzip_limit);
}

#[test]
fn a_full_tail_offers_the_compaction_that_makes_room_and_other_refusals_do_not() {
    let page = "10000000-0000-4000-8000-000000000001";
    let input = |baseline: usize, updates: usize, tail_bytes: usize| MaterializationInput {
        baseline: vec![0; baseline],
        updates: Vec::new(),
        own_updates: BTreeMap::new(),
        signing_keys: BTreeMap::new(),
        owner_provenance: BTreeMap::new(),
        local_writer: String::new(),
        tally: crate::page::UsageCounts {
            updates,
            tail_bytes,
            ..Default::default()
        },
    };
    let message = |input: &MaterializationInput, count, bytes| {
        input
            .admit_deltas(page, count, bytes)
            .unwrap_err()
            .to_string()
    };
    let command = format!("tmt colab page compact {page}");
    let by_count = message(&input(0, crate::decoder::WRITE_TAIL_UPDATES, 0), 1, 0);
    let by_bytes = message(&input(0, 0, crate::decoder::WRITE_TAIL_BYTES), 0, 1);
    for message in [by_count, by_bytes] {
        assert!(message.contains(&command), "{message}");
        assert!(message.contains("Nothing was deleted."), "{message}");
        assert!(
            message.contains("If it is still full, export it"),
            "{message}"
        );
    }
    // A source past its own limit is not made smaller by combining changes.
    let content = message(&input(crate::decoder::BASELINE_BYTES + 1, 0, 0), 0, 0);
    assert!(content.contains("its content is"), "{content}");
    assert!(!content.contains("compact"), "{content}");
}

#[test]
fn checked_total_accepts_empty_and_exact_usize_max_and_rejects_overflow() {
    assert_eq!(checked_bytes([].into_iter()).unwrap(), 0);
    assert_eq!(
        checked_bytes([usize::MAX - 1, 1].into_iter()).unwrap(),
        usize::MAX
    );
    let error = checked_bytes([usize::MAX, 1].into_iter()).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<OwnerFault>(),
        Some(OwnerFault::Capacity)
    ));
}
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
#[test]
fn gzip_exact_budget_and_one_byte_over_use_one_stream() {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let noise = (0..5_000_001)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect::<Vec<_>>();
    // Establish the exact threshold with complete compressed output, independently of the counting sink.
    let mut low = 4_990_000usize;
    let mut high = 5_000_000usize;
    while low < high {
        let mid = (low + high) / 2;
        if gzip(&noise[..mid]).len() < 5_000_000 {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let exact = &noise[..low];
    assert_eq!(gzip(exact).len(), 5_000_000);
    let over = &noise[..low + 1];
    assert_eq!(gzip(over).len(), 5_000_001);
    assert_eq!(gzip_over_budget([exact].into_iter()).unwrap(), None);
    let split = low / 2;
    assert_eq!(
        gzip_over_budget([&exact[..split], &exact[split..]].into_iter()).unwrap(),
        None
    );
    assert_eq!(
        gzip_over_budget([&over[..split], &over[split..]].into_iter()).unwrap(),
        Some(5_000_001)
    );
}

#[test]
fn attachment_reference_revision_and_namespace_match_independent_shared_positions() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../contracts/vectors/attachment-v1.json"
    ))
    .unwrap();
    for vector in corpus["referenceRevisions"].as_array().unwrap() {
        let string = |field: &str| vector[field].as_str().unwrap();
        let hash: [u8; 32] = values::binary(string("headHash"), 32)
            .unwrap()
            .try_into()
            .unwrap();
        let head = statement::Head {
            revision: 7,
            hash,
            owner_member: statement::OwnerMember {
                id: string("writer").into(),
                signing_key: [0; 32],
                encryption_key: [0; 32],
            },
        };
        let cuts = vector["cuts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                let bytes = values::binary(c["cut"].as_str().unwrap(), 1024).unwrap();
                let cut = stream_cut::decode(&bytes).unwrap();
                Cut {
                    page: string("page").into(),
                    epoch: 1,
                    stream: cut.stream_id.into(),
                    namespace: cut.namespace.into(),
                    checkpoint_seq: values::decimal(cut.checkpoint_seq, true).unwrap(),
                    checkpoint_hash: cut.checkpoint_hash.copied(),
                    tail_seq: values::decimal(cut.tail_head_seq, true).unwrap(),
                    tail_hash: *cut.tail_head_hash,
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            crate::page::token(string("space"), string("page"), &head, 1, &cuts).unwrap(),
            string("token"),
            "{}: {:?}",
            string("name"),
            cuts.iter().map(Cut::payload).collect::<Vec<_>>()
        );
        let hex = crate::attachments::namespace(string("space"), string("page"))
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(hex, corpus["namespace"].as_str().unwrap());
    }
}

#[test]
fn sealed_historical_receipts_use_the_owner_bound_before_later_revocation() {
    use std::os::unix::fs::OpenOptionsExt;
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../contracts/vectors/authority-v1.json"
    ))
    .unwrap();
    struct SealedHistoryDirectory(std::path::PathBuf);
    impl Drop for SealedHistoryDirectory {
        fn drop(&mut self) {
            let result = std::fs::remove_dir_all(&self.0);
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
    let root = SealedHistoryDirectory(
        std::env::temp_dir().join(format!("tmt-colab-sealed-history-{}", std::process::id())),
    );
    std::fs::create_dir(&root.0).unwrap();
    let layout = crate::keyring::Layout::open(&root.0).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.0.join("colab/owner.key"))
        .unwrap();
    let seed = corpus["seed"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    file.write_all(&seed).unwrap();
    drop(file);
    let key = Keyring::read(&layout).unwrap();
    let page = corpus["page"].as_str().unwrap();
    for case in corpus["sealedHistoryCases"].as_array().unwrap() {
        let log = case["log"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| statement::Envelope::from_json(&serde_json::to_vec(v).unwrap()).unwrap())
            .collect::<Vec<_>>();
        let (states, payloads) = verify_log(&log, &key, page).unwrap();
        let mut snapshot = Snapshot {
            authority: states.last().unwrap().clone(),
            epoch: 1,
            cuts: Vec::new(),
            secret: corpus["epochKey"]
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
            devices: vec![Device {
                revoked: true,
                chain: serde_json::to_vec(&case["chain"]).unwrap(),
            }],
            states,
            payloads,
            baseline: None,
            objects: Vec::new(),
            anchors: BTreeMap::new(),
        };
        let stored = crate::store::owner::epoch::StoredObject {
            seq: values::decimal(case["seq"].as_str().unwrap(), false).unwrap(),
            hash: values::binary(case["envelopeHash"].as_str().unwrap(), 32)
                .unwrap()
                .try_into()
                .unwrap(),
            previous: [0; 32],
            checkpoint: case["kind"].as_str().unwrap() == "checkpoint",
            bytes: serde_json::to_vec(&case["receipt"]).unwrap(),
        };
        let cut = Cut {
            page: page.into(),
            epoch: 1,
            stream: page.into(),
            namespace: "own".into(),
            checkpoint_seq: 0,
            checkpoint_hash: None,
            tail_seq: stored.seq,
            tail_hash: stored.hash,
        };
        let opened = {
            snapshot.cuts.push(cut);
            snapshot.open_object(&key, page, 0, &stored)
        };
        assert_eq!(
            opened.is_ok(),
            case["admitted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Ok(opened) = opened {
            assert_eq!(
                opened.plaintext,
                values::binary(case["plaintext"].as_str().unwrap(), 128).unwrap()
            );
        }
    }
}
