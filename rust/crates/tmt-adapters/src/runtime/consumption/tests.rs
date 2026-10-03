use super::*;
use crate::test_support::TestDirectory;
use std::{fs, io::Write};

const CLAUDE: &str = include_str!("../fixtures/claude-usage-sequence.jsonl");
const CODEX: &str = include_str!("../fixtures/codex-token-count.jsonl");
const NOW: u64 = 1_790_000_000_000;

fn claude(root: &Path, path: &Path, previous: Option<&State>, now: u64) -> Option<State> {
    super::claude(
        root,
        path,
        previous,
        now,
        Instant::now() + std::time::Duration::from_secs(2),
    )
}

fn append(path: &Path, text: &str) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

fn claude_file() -> (TestDirectory, std::path::PathBuf, State) {
    let root = TestDirectory::new();
    let path = root.path.join("session.jsonl");
    fs::write(&path, format!("{}\n", CLAUDE.lines().next().unwrap())).unwrap();
    let state = claude(&root.path, &path, None, NOW).unwrap();
    (root, path, state)
}

#[test]
fn recorded_claude_groups_count_once_and_hook_replays_do_not_advance() {
    let (root, path, first) = claude_file();
    assert_eq!(
        first.value.counts(),
        Counts::default(),
        "first read is a baseline"
    );
    let records = CLAUDE
        .lines()
        .skip(1)
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    append(&path, &records);
    let next = claude(&root.path, &path, Some(&first), NOW + 100).unwrap();
    // Independently summed from the redacted fixture's 15 new message IDs;
    // cache-read is included in input, and not added to the total twice.
    assert_eq!(
        next.value.counts(),
        Counts {
            input: 5_415_987,
            output: 5_609,
            cached: 5_405_674,
        }
    );
    assert_eq!(next.value.sequence, 2);
    assert_eq!(next.value.epoch, first.value.epoch);
    assert!(next.value.complete && !next.value.gap);
    assert_eq!(
        claude(&root.path, &path, Some(&next), NOW + 200),
        Some(next)
    );
}

#[test]
fn a_content_block_group_straddling_reads_is_absorbed_exactly_once() {
    let (root, path, first) = claude_file();
    let duplicate = CLAUDE.lines().nth(1).unwrap();
    append(&path, &format!("{duplicate}\n"));
    let second = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
    let expected = claude_message(duplicate).unwrap().unwrap().counts;
    assert_eq!(second.value.counts(), expected);
    append(&path, &format!("{}\n", CLAUDE.lines().nth(2).unwrap()));
    let third = claude(&root.path, &path, Some(&second), NOW + 2).unwrap();
    assert_eq!(third.value.counts(), expected);
    assert_eq!(
        third.value.sequence, 3,
        "new source evidence, zero token delta"
    );
}

#[test]
fn unfinished_records_wait_for_the_newline_without_losing_or_counting_fragments() {
    let (root, path, first) = claude_file();
    let record = CLAUDE.lines().nth(1).unwrap();
    let split = record.len() / 2;
    append(&path, &record[..split]);
    let pending = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
    assert!(!pending.value.complete && !pending.value.gap);
    assert_eq!(pending.value.counts(), Counts::default());
    assert_eq!(
        pending.cursor.as_ref().unwrap().offset,
        first.cursor.as_ref().unwrap().offset
    );
    assert_eq!(
        claude(&root.path, &path, Some(&pending), NOW + 2),
        Some(pending.clone())
    );
    append(&path, &format!("{}\n", &record[split..]));
    let next = claude(&root.path, &path, Some(&pending), NOW + 3).unwrap();
    assert_eq!(
        next.value.counts(),
        claude_message(record).unwrap().unwrap().counts
    );
    assert!(next.value.complete && !next.value.gap);
}

#[test]
fn source_shrink_replacement_and_scan_overflow_start_at_eof_with_a_gap() {
    for change in ["shrink", "replace", "overflow"] {
        let (root, path, first) = claude_file();
        match change {
            "shrink" => fs::write(&path, "").unwrap(),
            "replace" => {
                let replacement = path.with_extension("replacement");
                fs::write(&replacement, CLAUDE).unwrap();
                fs::rename(replacement, &path).unwrap();
            }
            _ => append(&path, &"x".repeat(transcript::TAIL_LIMIT as usize + 1)),
        }
        let gap = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
        assert_ne!(gap.value.epoch, first.value.epoch, "{change}");
        assert!(gap.value.gap && !gap.value.complete);
        assert_eq!(gap.value.counts(), Counts::default());
        assert_eq!(
            gap.cursor.as_ref().unwrap().offset,
            fs::metadata(&path).unwrap().len()
        );
        // Historical records are not replayed on an identical retry.
        assert_eq!(
            claude(&root.path, &path, Some(&gap), NOW + 2),
            Some(gap.clone())
        );
        append(&path, &format!("\n{}\n", CLAUDE.lines().nth(3).unwrap()));
        let recovered = claude(&root.path, &path, Some(&gap), NOW + 3).unwrap();
        assert_eq!(recovered.value.epoch, gap.value.epoch);
        assert!(recovered.value.complete && !recovered.value.gap);
    }
}

#[test]
fn malformed_or_divergent_main_records_gap_while_sidechains_are_skipped() {
    for change in [
        "sidechain",
        "synthetic",
        "divergent",
        "request",
        "negative",
        "malformed",
    ] {
        let (root, path, first) = claude_file();
        let mut record: Value = serde_json::from_str(CLAUDE.lines().next().unwrap()).unwrap();
        let skipped = match change {
            "sidechain" => {
                record["isSidechain"] = true.into();
                true
            }
            "synthetic" => {
                record["message"]["model"] = "<synthetic>".into();
                true
            }
            "divergent" => {
                record["message"]["usage"]["output_tokens"] = 9.into();
                false
            }
            "request" => {
                record["requestId"] = "different".into();
                false
            }
            "negative" => {
                record["message"]["usage"]["input_tokens"] = (-1).into();
                false
            }
            _ => false,
        };
        let line = if change == "malformed" {
            "{not json}".to_owned()
        } else {
            record.to_string()
        };
        append(&path, &format!("{line}\n"));
        let next = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
        assert_eq!(next.value.counts(), Counts::default());
        assert_eq!(next.value.gap, !skipped, "{change}");
    }
}

#[test]
fn the_approved_noncontiguous_repeat_limitation_is_explicit() {
    let (root, path, first) = claude_file();
    let second = CLAUDE.lines().nth(1).unwrap();
    let old = CLAUDE.lines().next().unwrap();
    append(&path, &format!("{second}\n{old}\n"));
    let next = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
    let expected = claude_message(second)
        .unwrap()
        .unwrap()
        .counts
        .add(claude_message(old).unwrap().unwrap().counts)
        .unwrap();
    assert_eq!(next.value.counts(), expected);
}

#[test]
fn recorded_codex_cumulative_totals_include_cache_and_reasoning_once() {
    let root = TestDirectory::new();
    let path = root.path.join("rollout.jsonl");
    fs::write(&path, CODEX).unwrap();
    let state = codex(&root.path, &path, None, NOW).unwrap();
    assert_eq!(
        state.value.counts(),
        Counts {
            input: 2_674_657_871,
            output: 6_910_968,
            cached: 2_624_251_008
        }
    );
    assert_eq!(
        state.value.input_tokens + state.value.output_tokens,
        2_681_568_839
    );
    assert_eq!(
        codex(&root.path, &path, Some(&state), NOW + 1),
        Some(state.clone())
    );
    let mut next: Value = serde_json::from_str(CODEX).unwrap();
    next["payload"]["info"]["total_token_usage"]["output_tokens"] = 6_910_978.into();
    next["payload"]["info"]["total_token_usage"]["total_tokens"] = 2_681_568_849u64.into();
    append(&path, &format!("{}\n", next));
    let updated = codex(&root.path, &path, Some(&state), NOW + 2).unwrap();
    assert_eq!(updated.value.output_tokens - state.value.output_tokens, 10);
    assert_eq!(updated.value.sequence, 2);
    assert_eq!(updated.value.epoch, state.value.epoch);
}

#[test]
fn codex_partial_lines_wait_and_invalid_newest_events_do_not_reuse_old_totals() {
    let root = TestDirectory::new();
    let path = root.path.join("rollout.jsonl");
    fs::write(&path, CODEX).unwrap();
    let first = codex(&root.path, &path, None, NOW).unwrap();
    let mut event: Value = serde_json::from_str(CODEX).unwrap();
    event["payload"]["info"]["total_token_usage"]["output_tokens"] = 6_910_978.into();
    event["payload"]["info"]["total_token_usage"]["total_tokens"] = 2_681_568_849u64.into();
    append(&path, &event.to_string());
    let pending = codex(&root.path, &path, Some(&first), NOW + 1).unwrap();
    assert_eq!(pending.value.counts(), first.value.counts());
    assert!(!pending.value.complete && !pending.value.gap);
    assert_eq!(
        codex(&root.path, &path, Some(&pending), NOW + 2),
        Some(pending.clone())
    );
    append(&path, "\n");
    let next = codex(&root.path, &path, Some(&pending), NOW + 3).unwrap();
    assert!(next.value.complete && !next.value.gap);
    assert_eq!(next.value.output_tokens - first.value.output_tokens, 10);
    event["payload"]["info"]["total_token_usage"]["total_tokens"] = 1.into();
    append(&path, &format!("{}\n", event));
    assert!(codex(&root.path, &path, Some(&next), NOW + 4).is_none());
    fs::write(&path, format!("{CODEX}{{broken}}\n")).unwrap();
    assert!(codex(&root.path, &path, Some(&next), NOW + 5).is_none());
    fs::write(&path, [CODEX.as_bytes(), &[0xff, b'\n']].concat()).unwrap();
    assert!(codex(&root.path, &path, Some(&next), NOW + 6).is_none());
}

#[test]
fn codex_regression_and_file_replacement_are_new_baselines_not_negative_rates() {
    for replacement in [false, true] {
        let root = TestDirectory::new();
        let path = root.path.join("rollout.jsonl");
        fs::write(&path, CODEX).unwrap();
        let first = codex(&root.path, &path, None, NOW).unwrap();
        if replacement {
            let new = root.path.join("new.jsonl");
            fs::write(&new, CODEX).unwrap();
            fs::rename(new, &path).unwrap();
        } else {
            let mut event: Value = serde_json::from_str(CODEX).unwrap();
            event["payload"]["info"]["total_token_usage"] = json!({"input_tokens": 10, "output_tokens": 2, "cached_input_tokens": 5, "total_tokens": 12});
            append(&path, &format!("{}\n", event));
        }
        let next = codex(&root.path, &path, Some(&first), NOW + 1).unwrap();
        assert_ne!(next.value.epoch, first.value.epoch);
        assert!(next.value.gap && !next.value.complete);
    }
}

#[test]
fn invalid_totals_and_clock_rollback_are_unavailable() {
    let root = TestDirectory::new();
    let path = root.path.join("rollout.jsonl");
    fs::write(&path, CODEX).unwrap();
    let first = codex(&root.path, &path, None, NOW).unwrap();
    assert!(codex(&root.path, &path, Some(&first), NOW - 1).is_none());
    for usage in [
        json!({"input_tokens": 1, "output_tokens": 1, "cached_input_tokens": 2, "total_tokens": 2}),
        json!({"input_tokens": 1, "output_tokens": 1, "cached_input_tokens": 0, "total_tokens": 3}),
        json!({"input_tokens": tmt_core::limits::MAX_JS_SAFE_INTEGER, "output_tokens": 1, "cached_input_tokens": 0, "total_tokens": tmt_core::limits::MAX_JS_SAFE_INTEGER + 1}),
    ] {
        let mut event: Value = serde_json::from_str(CODEX).unwrap();
        event["payload"]["info"]["total_token_usage"] = usage;
        fs::write(&path, event.to_string()).unwrap();
        assert!(codex(&root.path, &path, None, NOW).is_none());
    }
    let (root, path, first) = claude_file();
    assert!(claude(&root.path, &path, Some(&first), NOW - 1).is_none());
}

#[test]
fn cursor_data_is_private_and_consumption_documents_are_validated() {
    let (_, _, state) = claude_file();
    let public = state.value.document();
    assert_eq!(public.as_object().unwrap().len(), 8);
    assert!(public.get("cursor").is_none());
    assert_eq!(State::read(&state.document()), Some(state.clone()));
    for pointer in ["/value/sequence", "/value/observedAtMs"] {
        let mut bad = state.document();
        *bad.pointer_mut(pointer).unwrap() = 0.into();
        assert!(State::read(&bad).is_none());
    }
    let mut bad = state.document();
    bad["cursor"]["last"] = "not a hash".into();
    assert!(State::read(&bad).is_none());
}

#[test]
fn consumption_round_trips_with_activity_and_never_expands_the_one_kib_cap() {
    use crate::runtime::driver_state;
    use tmt_core::{
        binding::session::{
            DriverState, ProviderSessionId,
            activity::{ActivityPhase, Event},
        },
        endpoint::ProcessIncarnation,
    };
    for wide in [false, true] {
        let (_, _, mut consumption) = claude_file();
        let width = if wide { 128 } else { 1 };
        let session = ProviderSessionId::new(&"s".repeat(width)).unwrap();
        let process = ProcessIncarnation::new(42, &"p".repeat(width)).unwrap();
        let event = Event {
            phase: ActivityPhase::Working,
            turn: Some(ProviderSessionId::new(&"t".repeat(width)).unwrap()),
        };
        let usage = driver_state::Usage::new(100, None, NOW).unwrap();
        let old = driver_state::after_start(Some(&"m".repeat(width)), Some(usage), None).unwrap();
        let active =
            driver_state::after_activity(Some(&old), &session, &process, &event, NOW).unwrap();
        assert!(!consumption.value.gap && consumption.value.complete);
        let next =
            driver_state::after_observation(Some(usage), Some(consumption.clone()), Some(&active))
                .unwrap();
        consumption.value.gap = true;
        consumption.value.complete = false;
        assert!(next.document().len() <= DriverState::MAXIMUM_BYTES);
        assert_eq!(driver_state::state_usage(&next), Some(usage));
        assert_eq!(
            driver_state::state_activity(&next),
            driver_state::state_activity(&active)
        );
        if wide {
            assert!(
                driver_state::state_consumption(&next).is_none(),
                "optional counters cannot crowd out activity"
            );
            assert_eq!(next, active);
        } else {
            assert_eq!(next.version(), driver_state::CONSUMPTION_STATE_VERSION);
            assert_eq!(active.version(), driver_state::ACTIVITY_STATE_VERSION);
            assert!(
                consumption.value.gap && !consumption.value.complete,
                "legacy v3 starts a new measurement with a gap"
            );
            assert_eq!(driver_state::state_consumption(&next), Some(consumption));
            let stop = Event {
                phase: ActivityPhase::Idle,
                turn: event.turn.clone(),
            };
            let ended =
                driver_state::after_activity(Some(&next), &session, &process, &stop, NOW + 1)
                    .unwrap();
            assert_eq!(
                driver_state::state_consumption(&ended),
                driver_state::state_consumption(&next)
            );
            let restarted = driver_state::after_start(None, None, Some(&ended)).unwrap();
            assert!(driver_state::state_consumption(&restarted).is_none());
            assert_eq!(restarted.version(), driver_state::MODEL_STATE_VERSION);
        }
    }
}

#[test]
fn legacy_missing_ids_are_context_only_and_the_scanner_shares_path_safety() {
    let root = TestDirectory::new();
    let outside = TestDirectory::new();
    let path = root.path.join("legacy.jsonl");
    fs::write(
        &path,
        include_str!("../fixtures/claude-assistant-usage.jsonl"),
    )
    .unwrap();
    assert!(claude(&root.path, &path, None, NOW).is_none());
    let foreign = outside.path.join("other.jsonl");
    fs::write(&foreign, CLAUDE).unwrap();
    assert!(claude(&root.path, &foreign, None, NOW).is_none());
    let link = root.path.join("link.jsonl");
    std::os::unix::fs::symlink(&foreign, &link).unwrap();
    assert!(claude(&root.path, &link, None, NOW).is_none());
    let fifo = root.path.join("fifo.jsonl");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
    assert!(claude(&root.path, &fifo, None, NOW).is_none());
}

#[test]
fn bounded_appends_deduplicate_across_buffer_boundaries() {
    let (root, path, first) = claude_file();
    let foreign = format!(
        "{{\"type\":\"user\",\"text\":\"{}\"}}\n",
        "x".repeat(13_000)
    );
    for _ in 0..60 {
        append(&path, &foreign);
    }
    let mut record: Value = serde_json::from_str(CLAUDE.lines().nth(1).unwrap()).unwrap();
    record["message"]["content"] = "x".repeat(9_000).into();
    append(&path, &format!("{record}\n{record}\n"));
    let next = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
    assert_eq!(
        next.value.counts(),
        claude_message(&record.to_string()).unwrap().unwrap().counts
    );
    assert_eq!(next.value.epoch, first.value.epoch);
    assert!(next.value.complete && !next.value.gap);
    assert_eq!(
        next.cursor.unwrap().offset,
        fs::metadata(path).unwrap().len()
    );
}

#[test]
fn prefilter_preserves_escaped_keys_missing_usage_and_malformed_foreign_evidence() {
    let record = CLAUDE.lines().nth(1).unwrap();
    for line in [
        record.replace("\"usage\"", "\"\\u0075sage\""),
        record.replace("assistant", "\\u0061ssistant"),
        "{\"type\":\"user\",\"text\":\"large tool output\"}".into(),
        "{\"type\":\"assistant\",\"message\":{}}".into(),
        "{\"type\":\"user\", broken}".into(),
        format!("{}0{}", "[".repeat(128), "]".repeat(128)),
    ] {
        // The old full-Value path is the independent acceptance oracle.
        let reference = serde_json::from_str::<Value>(&line);
        let valid = reference
            .as_ref()
            .is_ok_and(|v| v["type"] != "assistant" || v["message"]["usage"].is_object());
        let (root, path, first) = claude_file();
        append(&path, &format!("{line}\n"));
        let next = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
        assert_eq!(next.value.gap, !valid, "{line}");
        if valid && reference.unwrap()["type"] == "assistant" {
            assert_eq!(
                next.value.counts(),
                claude_message(record).unwrap().unwrap().counts
            );
        }
    }
}

#[test]
fn deadline_before_validation_keeps_the_cursor_and_can_retry_without_new_bytes() {
    for expire_at in [0, 1, 2] {
        let (root, path, first) = claude_file();
        let record = CLAUDE.lines().nth(1).unwrap();
        append(&path, &format!("{record}\n"));
        let mut checks = 0;
        let pending = claude_until(&root.path, &path, Some(&first), NOW + 1, || {
            let expired = checks >= expire_at;
            checks += 1;
            expired
        })
        .unwrap();
        assert!(!pending.value.gap && !pending.value.complete);
        assert_eq!(pending.value.epoch, first.value.epoch);
        assert_eq!(pending.value.counts(), first.value.counts());
        assert_eq!(pending.cursor, first.cursor, "buffered is not validated");
        assert_eq!(pending.value.sequence, first.value.sequence + 1);
        assert_eq!(
            claude_until(&root.path, &path, Some(&pending), NOW + 2, || true),
            Some(pending.clone())
        );
        let complete = claude(&root.path, &path, Some(&pending), NOW + 3).unwrap();
        assert!(complete.value.complete && !complete.value.gap);
        assert_eq!(complete.value.epoch, first.value.epoch);
        assert_eq!(
            complete.value.counts(),
            claude_message(record).unwrap().unwrap().counts
        );
    }
}

#[test]
fn deadline_progress_catches_up_across_stops_without_recounting_a_message_group() {
    let (root, path, first) = claude_file();
    let records: Vec<_> = CLAUDE.lines().skip(1).take(3).collect();
    for record in &records {
        append(&path, &format!("{record}\n"));
    }
    let group = claude_message(records[0]).unwrap().unwrap();
    assert_eq!(group.id, claude_message(records[1]).unwrap().unwrap().id);
    let last = claude_message(records[2]).unwrap().unwrap();
    assert_ne!(group.id, last.id);
    let totals = [
        group.counts,
        group.counts,
        group.counts.add(last.counts).unwrap(),
    ];
    let mut state = first.clone();
    let mut offset = first.cursor.as_ref().unwrap().offset;
    for (index, record) in records.iter().enumerate() {
        // Permit one short record's read and validation, then expire before
        // the next record. Completed captured EOF needs no further work.
        let mut checks = 0;
        let next = claude_until(
            &root.path,
            &path,
            Some(&state),
            NOW + index as u64 + 1,
            || {
                checks += 1;
                checks > 3
            },
        )
        .unwrap();
        offset += record.len() as u64 + 1;
        assert_eq!(next.cursor.as_ref().unwrap().offset, offset);
        assert_eq!(next.value.counts(), totals[index]);
        assert_eq!(next.value.epoch, first.value.epoch);
        assert_eq!(next.value.sequence, state.value.sequence + 1);
        assert!(!next.value.gap);
        assert_eq!(next.value.complete, index == 2);
        state = next;
    }
    assert_eq!(offset, fs::metadata(&path).unwrap().len());
    assert_eq!(
        claude(&root.path, &path, Some(&state), NOW + 4),
        Some(state)
    );
}

#[test]
fn streaming_keeps_the_captured_descriptor_and_end() {
    let (root, path, first) = claude_file();
    let record = CLAUDE.lines().nth(1).unwrap();
    append(&path, &format!("{record}\n"));
    let end = fs::metadata(&path).unwrap().len();
    let mut changed = false;
    let next = claude_until(&root.path, &path, Some(&first), NOW + 1, || {
        if !changed {
            append(&path, CLAUDE);
            let replacement = path.with_extension("replacement");
            fs::write(&replacement, CLAUDE).unwrap();
            fs::rename(replacement, &path).unwrap();
            changed = true;
        }
        false
    })
    .unwrap();
    assert_eq!(next.cursor.unwrap().offset, end);
    assert_eq!(
        next.value.counts(),
        claude_message(record).unwrap().unwrap().counts
    );
    assert!(next.value.complete && !next.value.gap);
}

#[test]
#[ignore = "manual synthetic scan timing; run under /usr/bin/time -l for peak RSS"]
fn streaming_scan_measurement() {
    let mib: usize = std::env::var("TMT_SCAN_MIB").unwrap().parse().unwrap();
    let mix = std::env::var("TMT_SCAN_MIX").unwrap();
    let (root, path, first) = claude_file();
    if mib == 0 {
        println!("baseline: fixture and initial cursor only");
        return;
    }
    let candidate = format!("{}\n", CLAUDE.lines().nth(1).unwrap());
    let foreign = format!(
        "{{\"type\":\"user\",\"text\":\"{}\"}}\n",
        "x".repeat(if mix == "long" { 900_000 } else { 8_000 })
    );
    let line = if mix == "usage" { &candidate } else { &foreign };
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    let mut remaining = mib * 1024 * 1024 - candidate.len();
    while remaining >= line.len() {
        file.write_all(line.as_bytes()).unwrap();
        remaining -= line.len();
    }
    file.write_all(" ".repeat(remaining.saturating_sub(1)).as_bytes())
        .unwrap();
    if remaining > 0 {
        file.write_all(b"\n").unwrap();
    }
    file.write_all(candidate.as_bytes()).unwrap();
    drop(file);
    let start = Instant::now();
    let next = super::claude(
        &root.path,
        &path,
        Some(&first),
        NOW + 1,
        start + std::time::Duration::from_secs(1),
    )
    .unwrap();
    println!(
        "mib={mib} mix={mix} scan_us={} gap={} complete={}",
        start.elapsed().as_micros(),
        next.value.gap,
        next.value.complete
    );
    let offset = next.cursor.unwrap().offset;
    let end = fs::metadata(path).unwrap().len();
    if mib > 1 {
        assert!(next.value.gap && !next.value.complete);
        assert_ne!(next.value.epoch, first.value.epoch);
        assert_eq!(offset, end);
        return;
    }
    assert!(!next.value.gap);
    assert_eq!(next.value.epoch, first.value.epoch);
    assert!(offset <= end);
    if next.value.complete {
        assert_eq!(offset, end);
        assert_eq!(
            next.value.counts(),
            claude_message(&candidate).unwrap().unwrap().counts
        );
    } else {
        assert!(offset < end, "deadline retains a checkpoint before EOF");
    }
}

#[test]
fn a_record_at_the_cap_counts_but_one_extra_byte_starts_a_gap() {
    for extra in [0, 1] {
        let (root, path, first) = claude_file();
        let record = CLAUDE.lines().nth(1).unwrap();
        let padding = " ".repeat(transcript::TAIL_LIMIT as usize - record.len() - 1 + extra);
        append(&path, &format!("{record}{padding}\n"));
        let next = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
        assert_eq!(next.value.gap, extra == 1);
        assert_eq!(next.value.complete, extra == 0);
        assert_eq!(next.value.epoch == first.value.epoch, extra == 0);
        assert_eq!(
            next.cursor.unwrap().offset,
            fs::metadata(path).unwrap().len()
        );
        assert_eq!(
            next.value.counts(),
            if extra == 0 {
                claude_message(record).unwrap().unwrap().counts
            } else {
                Counts::default()
            }
        );
    }
}

#[test]
fn backlog_outside_tail_rebaselines_at_eof_without_catchup() {
    let (root, path, first) = claude_file();
    append(
        &path,
        &format!("{}\n", " ".repeat(transcript::TAIL_LIMIT as usize)),
    );
    append(&path, CLAUDE);
    let next = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
    assert_ne!(next.value.epoch, first.value.epoch);
    assert!(next.value.gap && !next.value.complete);
    assert_eq!(next.value.input_tokens, 0);
    assert_eq!(
        next.cursor.as_ref().unwrap().offset,
        fs::metadata(path).unwrap().len()
    );
}

#[test]
fn empty_source_baseline_counts_first_append_before_stop() {
    let root = TestDirectory::new();
    let path = root.path.join("empty.jsonl");
    fs::write(&path, "").unwrap();
    let first = claude(&root.path, &path, None, NOW).unwrap();
    assert_eq!(first.value.input_tokens, 0);
    append(&path, CLAUDE.lines().nth(1).unwrap());
    append(&path, "\n");
    let next = claude(&root.path, &path, Some(&first), NOW + 1).unwrap();
    assert!(next.value.input_tokens > 0 && next.value.output_tokens > 0);
    let stop = claude(&root.path, &path, Some(&next), NOW + 2).unwrap();
    assert_eq!(stop, next);
}
