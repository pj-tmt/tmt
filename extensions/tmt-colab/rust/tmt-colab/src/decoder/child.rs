//! All production yrs decoding lives here, reached only by the private child entry.
use super::*;
use crate::edit_scale::content_hunks;
use std::io::{Read, Write};
use yrs::{
    Any, Doc, GetString, Map, MapRef, Out, ReadTxn, Root, StateVector, Text, TextRef, Transact,
    TransactionMut, Update, updates::decoder::Decode,
};

pub(super) fn run() -> std::process::ExitCode {
    if !initialize() {
        return std::process::ExitCode::FAILURE;
    }
    match execute() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => {
            diagnostic("decoder rejected");
            std::process::ExitCode::FAILURE
        }
    }
}
fn initialize() -> bool {
    // Linux enforces RLIMIT_AS. macOS accepts it without enforcing it, so report
    // unavailable rather than claiming memory containment or refusing to run.
    #[cfg(target_os = "linux")]
    {
        use nix::sys::resource::{Resource, getrlimit, setrlimit};
        if setrlimit(Resource::RLIMIT_AS, MEMORY_BYTES, MEMORY_BYTES).is_err()
            || getrlimit(Resource::RLIMIT_AS).ok() != Some((MEMORY_BYTES, MEMORY_BYTES))
        {
            diagnostic("decoder memory limit failed");
            return false;
        }
    }
    if memory_limit() == MemoryLimit::Unavailable {
        diagnostic("memory limit unavailable");
    }
    std::panic::set_hook(Box::new(|_| diagnostic("decoder panic")));
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    #[cfg(test)]
    Memory,
    Input,
    Json,
    Binary,
    Decode,
    Apply,
    OwnValidation,
    Project,
    Merge,
    Edit,
    Generation,
    Bounds,
    Replay,
    Prefix,
    ReplyBuild,
    ReplySerialize,
    ReplyWrite,
}
impl Stage {
    #[cfg(test)]
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Input => "input",
            Self::Json => "json",
            Self::Binary => "binary",
            Self::Decode => "decode",
            Self::Apply => "apply",
            Self::OwnValidation => "own",
            Self::Project => "project",
            Self::Merge => "merge",
            Self::Edit => "edit",
            Self::Generation => "generation",
            Self::Bounds => "bounds",
            Self::Replay => "replay",
            Self::Prefix => "prefix",
            Self::ReplyBuild => "reply-build",
            Self::ReplySerialize => "reply-json",
            Self::ReplyWrite => "reply-write",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CheckpointBoundary {
    Enter,
    Leave,
}
type Checkpoint<'a> = dyn FnMut(Stage, CheckpointBoundary, usize) + 'a;
fn noop(_: Stage, _: CheckpointBoundary, _: usize) {}

#[cfg(test)]
pub(super) fn observed_request(
    reader: impl Read,
    command: Option<&str>,
    writer: &mut impl Write,
    checkpoint: &mut Checkpoint<'_>,
) -> Result<(), DecodeFault> {
    checkpoint(Stage::Memory, CheckpointBoundary::Enter, 0);
    if !initialize() {
        return Err(DecodeFault::InvalidInput);
    }
    checkpoint(Stage::Memory, CheckpointBoundary::Leave, 0);
    // Baseline composition remains production-only; this helper observes the two reached paths.
    if command == Some("baseline") {
        return Err(DecodeFault::InvalidInput);
    }
    execute_with(reader, command, writer, checkpoint)
}
#[cfg(test)]
pub(super) fn request_for_test(
    reader: impl Read,
    command: Option<&str>,
    writer: &mut impl Write,
    checkpoint: &mut Checkpoint<'_>,
) -> Result<(), DecodeFault> {
    execute_with(reader, command, writer, checkpoint)
}
fn execute() -> Result<(), DecodeFault> {
    execute_with(
        std::io::stdin(),
        std::env::args().nth(2).as_deref(),
        &mut ProductionReply,
        &mut noop,
    )
}
fn execute_with(
    reader: impl Read,
    command: Option<&str>,
    writer: &mut impl Write,
    checkpoint: &mut Checkpoint<'_>,
) -> Result<(), DecodeFault> {
    checkpoint(Stage::Input, CheckpointBoundary::Enter, 0);
    let mut input = Vec::new();
    reader
        .take((STREAM_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| DecodeFault::InvalidInput)?;
    if input.len() > STREAM_BYTES {
        return Err(DecodeFault::InvalidInput);
    }
    checkpoint(Stage::Input, CheckpointBoundary::Leave, input.len());
    if command == Some("baseline") {
        return baseline(&input);
    }
    if command == Some("prepare-content") {
        return prepare_content(&input, writer, checkpoint);
    }
    checkpoint(Stage::Json, CheckpointBoundary::Enter, 0);
    let wire: WireBatch<BorrowedWireText<'_>> =
        serde_json::from_slice(&input).map_err(|_| DecodeFault::InvalidInput)?;
    checkpoint(Stage::Json, CheckpointBoundary::Leave, 0);
    if wire.version != 1
        || wire.updates.len() > UPDATES
        || (wire.records.is_some() && wire.namespace != Namespace::Own)
        || (wire.rebuild
            && (!wire.merge_only || wire.namespace != Namespace::Content || wire.records.is_some()))
    {
        return Err(DecodeFault::InvalidInput);
    }
    if let Some(records) = &wire.records {
        validate_own_records(records)?;
    }
    checkpoint(Stage::Binary, CheckpointBoundary::Enter, 0);
    let baseline = binary(&wire.baseline, STATE_BYTES)?;
    if wire.rebuild && !baseline.is_empty() {
        return Err(DecodeFault::InvalidInput);
    }
    let updates: Vec<_> = wire
        .updates
        .iter()
        .map(|v| binary(v, STATE_BYTES))
        .collect::<Result<_, _>>()?;
    if baseline.len() + updates.iter().map(Vec::len).sum::<usize>() > STATE_BYTES {
        return Err(DecodeFault::InvalidInput);
    }
    checkpoint(Stage::Binary, CheckpointBoundary::Leave, updates.len());
    let doc = Doc::new();
    let names: &[&str] = match wire.namespace {
        Namespace::Content => &["html", "meta"],
        Namespace::Own => &["threads", "messages", "intents", "replies"],
    };
    for name in names {
        if *name == "html" {
            doc.get_or_insert_text(*name);
        } else {
            doc.get_or_insert_map(*name);
        }
    }
    let mut discussion: std::collections::BTreeMap<(String, String), Value> =
        std::collections::BTreeMap::new();
    for (index, bytes) in std::iter::once(&baseline)
        .filter(|v| !v.is_empty())
        .chain(updates.iter())
        .enumerate()
    {
        checkpoint(Stage::Decode, CheckpointBoundary::Enter, index);
        let update = Update::decode_v1(bytes).map_err(|_| DecodeFault::Rejected)?;
        checkpoint(Stage::Decode, CheckpointBoundary::Leave, index + 1);
        checkpoint(Stage::Apply, CheckpointBoundary::Enter, index);
        doc.transact_mut()
            .apply_update(update)
            .map_err(|_| DecodeFault::Rejected)?;
        checkpoint(Stage::Apply, CheckpointBoundary::Leave, index + 1);
        if wire.namespace == Namespace::Own {
            checkpoint(Stage::OwnValidation, CheckpointBoundary::Enter, index);
            let next = protected_own_records(&doc)?;
            if discussion.iter().any(|((root, key), value)| {
                next.get(&(root.clone(), key.clone())) != Some(value)
                    && !next
                        .get(&(root.clone(), key.clone()))
                        .is_some_and(|next| crate::edit_lock::advances(value, next, root, key))
            }) {
                return Err(DecodeFault::Rejected);
            }
            discussion = next;
            checkpoint(Stage::OwnValidation, CheckpointBoundary::Leave, index + 1);
        }
    }
    if wire.merge_only {
        checkpoint(Stage::Merge, CheckpointBoundary::Enter, 0);
        let incomplete = {
            let txn = doc.transact();
            txn.store().pending_update().is_some() || txn.store().pending_ds().is_some()
        };
        let merged = if wire.rebuild && !incomplete {
            rebuild_content(&doc)?
        } else if wire.namespace == Namespace::Own && !incomplete {
            // Complete own documents reclaim replaced cells; pending own steps retain raw bytes.
            project(&doc, Namespace::Own)?;
            doc.transact()
                .encode_state_as_update_v1(&StateVector::default())
        } else {
            // Content streams and incomplete own steps retain all raw structs/delete sets.
            yrs::merge_updates_v1(updates.iter().map(Vec::as_slice))
                .map_err(|_| DecodeFault::Rejected)?
        };
        if merged.len() > STATE_BYTES {
            return Err(DecodeFault::Rejected);
        }
        checkpoint(Stage::Merge, CheckpointBoundary::Leave, 0);
        checkpoint(Stage::ReplyBuild, CheckpointBoundary::Enter, 0);
        let reply = WireResult {
            version: 1,
            namespace: wire.namespace,
            input_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(&input)),
            merged: EncodedBytes(&merged),
            projection: Value::Null,
            memory_limit: memory_limit(),
            pid: std::process::id(),
        };
        checkpoint(Stage::ReplyBuild, CheckpointBoundary::Leave, 0);
        return write_reply_with(&reply, writer, checkpoint);
    }
    checkpoint(Stage::Project, CheckpointBoundary::Enter, 0);
    let before = project(&doc, wire.namespace)?;
    checkpoint(Stage::Project, CheckpointBoundary::Leave, 0);
    // Edit only the admitted structs. The delta never reattributes foreign content.
    let (merged, projection) = if let Some(records) = &wire.records {
        checkpoint(Stage::Edit, CheckpointBoundary::Enter, 0);
        let vector = doc.transact().state_vector();
        let mut tx = doc.transact_mut();
        for record in records {
            if let Some(existing) = before
                .get(&record.root)
                .and_then(|root| root.get(&record.key))
            {
                if existing == &record.value {
                    continue;
                }
                if !crate::edit_lock::advances(existing, &record.value, &record.root, &record.key) {
                    return Err(DecodeFault::Rejected);
                }
            }
            let map = Root::<MapRef>::new(record.root.as_str())
                .get(&tx)
                .ok_or(DecodeFault::Rejected)?;
            map.insert(
                &mut tx,
                record.key.as_str(),
                Any::from_json(&record.value.to_string()).map_err(|_| DecodeFault::Rejected)?,
            );
        }
        let merged = tx.encode_state_as_update_v1(&vector);
        drop(tx);
        checkpoint(Stage::Edit, CheckpointBoundary::Leave, 0);
        checkpoint(Stage::Project, CheckpointBoundary::Enter, 1);
        let projection = project(&doc, wire.namespace)?;
        checkpoint(Stage::Project, CheckpointBoundary::Leave, 1);
        (merged, projection)
    } else {
        // Merge the author's updates, never encode the shared document.
        checkpoint(Stage::Merge, CheckpointBoundary::Enter, 0);
        let merged = yrs::merge_updates_v1(updates.iter().map(Vec::as_slice))
            .map_err(|_| DecodeFault::Rejected)?;
        checkpoint(Stage::Merge, CheckpointBoundary::Leave, 0);
        (merged, before)
    };
    // A prepared edit is one update; a read's merged tail may be the whole state.
    if merged.len()
        > if wire.records.is_some() {
            UPDATE_BYTES
        } else {
            STATE_BYTES
        }
    {
        return Err(DecodeFault::Rejected);
    }
    checkpoint(Stage::ReplyBuild, CheckpointBoundary::Enter, 0);
    let reply = WireResult {
        version: 1,
        namespace: wire.namespace,
        input_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(&input)),
        merged: EncodedBytes(&merged),
        projection,
        memory_limit: memory_limit(),
        pid: std::process::id(),
    };
    checkpoint(Stage::ReplyBuild, CheckpointBoundary::Leave, 0);
    write_reply_with(&reply, writer, checkpoint)
}
fn content_document(
    baseline: &[u8],
    updates: &[Vec<u8>],
    checkpoint: &mut Checkpoint<'_>,
) -> Result<Doc, DecodeFault> {
    let doc = Doc::new();
    doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    for (index, bytes) in std::iter::once(baseline)
        .filter(|v| !v.is_empty())
        .chain(updates.iter().map(Vec::as_slice))
        .enumerate()
    {
        let mut tx = doc.transact_mut();
        checkpoint(Stage::Decode, CheckpointBoundary::Enter, index);
        let update = Update::decode_v1(bytes).map_err(|_| DecodeFault::Rejected)?;
        checkpoint(Stage::Decode, CheckpointBoundary::Leave, index + 1);
        checkpoint(Stage::Apply, CheckpointBoundary::Enter, index);
        tx.apply_update(update).map_err(|_| DecodeFault::Rejected)?;
        drop(tx);
        checkpoint(Stage::Apply, CheckpointBoundary::Leave, index + 1);
    }
    Ok(doc)
}
#[cfg(test)]
fn replay_content(
    baseline: &[u8],
    admitted: &[Vec<u8>],
    updates: &[Vec<u8>],
    expected: &Value,
) -> Result<(), DecodeFault> {
    replay_content_with(baseline, admitted, updates, expected, &mut noop)
}
fn replay_content_with(
    baseline: &[u8],
    admitted: &[Vec<u8>],
    updates: &[Vec<u8>],
    expected: &Value,
    checkpoint: &mut Checkpoint<'_>,
) -> Result<(), DecodeFault> {
    checkpoint(Stage::Replay, CheckpointBoundary::Enter, 0);
    let replay = content_document(baseline, admitted, checkpoint)?;
    for (index, bytes) in updates.iter().enumerate() {
        let mut tx = replay.transact_mut();
        checkpoint(Stage::Decode, CheckpointBoundary::Enter, index);
        let update = Update::decode_v1(bytes).map_err(|_| DecodeFault::Rejected)?;
        checkpoint(Stage::Decode, CheckpointBoundary::Leave, index + 1);
        checkpoint(Stage::Apply, CheckpointBoundary::Enter, index);
        tx.apply_update(update).map_err(|_| DecodeFault::Rejected)?;
        drop(tx);
        checkpoint(Stage::Apply, CheckpointBoundary::Leave, index + 1);
        checkpoint(Stage::Prefix, CheckpointBoundary::Enter, index);
        project(&replay, Namespace::Content)?; // Reject unresolved causal prefixes, not only the final state.
        checkpoint(Stage::Prefix, CheckpointBoundary::Leave, index + 1);
    }
    checkpoint(Stage::Project, CheckpointBoundary::Enter, updates.len());
    if project(&replay, Namespace::Content)? != *expected {
        return Err(DecodeFault::Rejected);
    }
    checkpoint(Stage::Project, CheckpointBoundary::Leave, updates.len());
    checkpoint(Stage::Replay, CheckpointBoundary::Leave, updates.len());
    Ok(())
}
fn prepare_content(
    input: &[u8],
    writer: &mut impl Write,
    checkpoint: &mut Checkpoint<'_>,
) -> Result<(), DecodeFault> {
    checkpoint(Stage::Json, CheckpointBoundary::Enter, 0);
    let wire: WireContentPreparation<BorrowedWireText<'_>, Value, BorrowedWireText<'_>> =
        serde_json::from_slice(input).map_err(|_| DecodeFault::InvalidInput)?;
    checkpoint(Stage::Json, CheckpointBoundary::Leave, 0);
    if wire.version != 1
        || wire.source.len() > BASELINE_BYTES
        || wire.updates.len() > UPDATES
        || wire
            .publisher_agent
            .as_deref()
            .is_some_and(|v| !valid_publisher_agent(v))
    {
        return Err(DecodeFault::InvalidInput);
    }
    checkpoint(Stage::Binary, CheckpointBoundary::Enter, 0);
    let baseline = binary(&wire.baseline, STATE_BYTES)?;
    let admitted = wire
        .updates
        .iter()
        .map(|v| binary(v, STATE_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    if baseline.len() + admitted.iter().map(Vec::len).sum::<usize>() > STATE_BYTES {
        return Err(DecodeFault::InvalidInput);
    }
    checkpoint(Stage::Binary, CheckpointBoundary::Leave, admitted.len());
    let doc = content_document(&baseline, &admitted, checkpoint)?;
    checkpoint(Stage::Project, CheckpointBoundary::Enter, 0);
    let before = project(&doc, Namespace::Content)?;
    checkpoint(Stage::Project, CheckpointBoundary::Leave, 0);
    if before != wire.expected_base {
        return Err(DecodeFault::Rejected);
    }
    let edit = ContentEdit {
        source: &wire.source,
        publisher_agent: wire.publisher_agent.as_deref(),
        layout: wire.layout.as_ref(),
        attachments: wire.attachments.as_ref(),
    };
    if wire.layout.as_ref().is_some_and(|layout| !layout.valid()) {
        return Err(DecodeFault::InvalidInput);
    }
    if let Some(change) = edit.attachments {
        // The parent already checked this; the isolated child never trusts its caller's reading.
        change
            .validate(&tmt_colab_model::crypto::digest(wire.source.as_bytes()))
            .map_err(|_| DecodeFault::InvalidInput)?;
    }
    let expected = if let Some(structured) = &wire.structured {
        if !wire.source.is_empty()
            || wire.publisher_agent.is_some()
            || wire.layout.is_some()
            || wire.attachments.is_some()
        {
            return Err(DecodeFault::InvalidInput);
        }
        edited_structured_projection(&before, structured)?
    } else {
        edited_projection(&before, edit)?
    };
    checkpoint(Stage::Generation, CheckpointBoundary::Enter, 0);
    let mut updates = Vec::new();
    if expected != before {
        if let Some(structured) = &wire.structured {
            let key = structured.kind.key();
            let meta = doc.get_or_insert_map("meta");
            let mut tx = doc.transact_mut();
            match expected["meta"].get(key) {
                Some(value) => {
                    meta.insert(
                        &mut tx,
                        key,
                        Any::from_json(&value.to_string()).map_err(|_| DecodeFault::Rejected)?,
                    );
                }
                // A cleared value (a template mark) leaves the key absent.
                None => {
                    meta.remove(&mut tx, key);
                }
            }
            updates.push(tx.encode_update_v1());
        } else {
            let old = before["html"].as_str().ok_or(DecodeFault::Rejected)?;
            let html = doc.get_or_insert_text("html");
            let meta = doc.get_or_insert_map("meta");
            // One step per changed region, last to first so that an edit never moves the offset
            // of a region before it. A region's new text is cut into steps of at most
            // CREATE_CHUNK_BYTES, the first of which removes the old text.
            let mut steps: Vec<(usize, usize, &str)> = Vec::new();
            for hunk in content_hunks(old, &wire.source).iter().rev() {
                let (mut at, mut remove, mut rest) = (hunk.start, hunk.remove, hunk.insert);
                loop {
                    let mut cut = rest.len().min(CREATE_CHUNK_BYTES);
                    while !rest.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    let (piece, tail) = rest.split_at(cut);
                    steps.push((at, remove, piece));
                    (at, remove, rest) = (at + piece.len(), 0, tail);
                    if rest.is_empty() {
                        break;
                    }
                }
            }
            // Steps share an update until their new text would pass the chunk size. A change that
            // touches only metadata is one update with no step.
            let mut groups: Vec<Vec<(usize, usize, &str)>> = vec![Vec::new()];
            let mut inserted = 0;
            for step in steps {
                if inserted + step.2.len() > CREATE_CHUNK_BYTES
                    && !groups[groups.len() - 1].is_empty()
                {
                    groups.push(Vec::new());
                    inserted = 0;
                }
                inserted += step.2.len();
                let last = groups.len() - 1;
                groups[last].push(step);
            }
            for (index, group) in groups.iter().enumerate() {
                let mut tx = doc.transact_mut();
                if index == 0 {
                    if before["meta"]["publisherAgent"].as_str() != edit.publisher_agent {
                        if let Some(agent) = edit.publisher_agent {
                            meta.insert(&mut tx, "publisherAgent", agent);
                        } else {
                            meta.remove(&mut tx, "publisherAgent");
                        }
                    }
                    for key in ["layout", "layoutReason"] {
                        if before["meta"].get(key) != expected["meta"].get(key) {
                            if let Some(value) = expected["meta"].get(key).and_then(Value::as_str) {
                                meta.insert(&mut tx, key, value);
                            } else {
                                meta.remove(&mut tx, key);
                            }
                        }
                    }
                    if before["meta"]["attachments"] != expected["meta"]["attachments"] {
                        if let Some(list) = expected["meta"].get("attachments") {
                            meta.insert(
                                &mut tx,
                                "attachments",
                                Any::from_json(&list.to_string())
                                    .map_err(|_| DecodeFault::Rejected)?,
                            );
                        } else {
                            meta.remove(&mut tx, "attachments");
                        }
                    }
                }
                for &(at, remove, piece) in group {
                    if remove > 0 {
                        html.remove_range(&mut tx, at as u32, remove as u32);
                    }
                    if !piece.is_empty() {
                        html.insert(&mut tx, at as u32, piece);
                    }
                }
                updates.push(tx.encode_update_v1());
            }
        }
        if updates.len() > WRITE_TAIL_UPDATES
            || updates.iter().any(|v| v.len() > UPDATE_BYTES)
            || updates.iter().map(Vec::len).sum::<usize>() > WRITE_TAIL_BYTES
        {
            return Err(DecodeFault::Rejected);
        }
    }
    checkpoint(Stage::Generation, CheckpointBoundary::Leave, updates.len());
    checkpoint(Stage::Project, CheckpointBoundary::Enter, 1);
    let projection = project(&doc, Namespace::Content)?;
    checkpoint(Stage::Project, CheckpointBoundary::Leave, 1);
    checkpoint(Stage::Bounds, CheckpointBoundary::Enter, 0);
    if projection != expected
        || doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
            .len()
            > STATE_BYTES
        || serde_json::to_vec(&projection)
            .map_err(|_| DecodeFault::Rejected)?
            .len()
            > STATE_BYTES
    {
        return Err(DecodeFault::Rejected);
    }
    checkpoint(Stage::Bounds, CheckpointBoundary::Leave, 0);
    replay_content_with(&baseline, &admitted, &updates, &expected, checkpoint)?;
    checkpoint(Stage::ReplyBuild, CheckpointBoundary::Enter, 0);
    let reply = WirePreparedContent {
        version: 1,
        input_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(input)),
        batch: if updates.is_empty() {
            WireContentBatch::Noop
        } else {
            WireContentBatch::Updates {
                updates: updates.iter().map(|v| EncodedBytes(v)).collect(),
            }
        },
        projection,
        memory_limit: memory_limit(),
        pid: std::process::id(),
    };
    checkpoint(Stage::ReplyBuild, CheckpointBoundary::Leave, 0);
    write_reply_with(&reply, writer, checkpoint)
}
// Capture only materialized typed records, allowing existing checkpoint steps
// with pending dependencies. Records cannot disappear; only a typed lock cell may advance.
fn protected_own_records(
    doc: &Doc,
) -> Result<std::collections::BTreeMap<(String, String), Value>, DecodeFault> {
    let txn = doc.transact();
    let mut records = std::collections::BTreeMap::new();
    for root in ["threads", "intents", "messages"] {
        let map = Root::<MapRef>::new(root)
            .get(&txn)
            .ok_or(DecodeFault::Rejected)?;
        for (key, value) in map.iter(&txn) {
            if let Out::Any(Any::Map(fields)) = value
                && matches!(fields.get("kind"), Some(Any::String(kind)) if matches!(kind.as_ref(), "thread" | "comment" | "thread-status" | "thread-notification" | "proposal-decision" | "needs-you-handled" | "attachment-publication" | "edit-request" | "edit-result" | "edit-lock"))
            {
                records.insert(
                    (root.into(), key.into()),
                    serde_json::to_value(Any::Map(fields)).map_err(|_| DecodeFault::Rejected)?,
                );
            }
        }
    }
    Ok(records)
}

struct ProductionReply;
impl Write for ProductionReply {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        tmt_cli_style::stream::stdout(true).write(bytes)
    }
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        tmt_cli_style::stream::stdout(true).write_all(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn write_reply(reply: &impl Serialize) -> Result<(), DecodeFault> {
    write_reply_with(reply, &mut ProductionReply, &mut noop)
}
fn write_reply_with(
    reply: &impl Serialize,
    writer: &mut impl Write,
    checkpoint: &mut Checkpoint<'_>,
) -> Result<(), DecodeFault> {
    checkpoint(Stage::ReplySerialize, CheckpointBoundary::Enter, 0);
    let output = serde_json::to_vec(reply).map_err(|_| DecodeFault::InvalidOutput)?;
    if output.len() > STREAM_BYTES {
        return Err(DecodeFault::InvalidOutput);
    }
    checkpoint(
        Stage::ReplySerialize,
        CheckpointBoundary::Leave,
        output.len(),
    );
    checkpoint(Stage::ReplyWrite, CheckpointBoundary::Enter, 0);
    writer
        .write_all(&output)
        .map_err(|_| DecodeFault::InvalidOutput)?;
    checkpoint(Stage::ReplyWrite, CheckpointBoundary::Leave, output.len());
    Ok(())
}

fn diagnostic(message: &str) {
    let _ = writeln!(tmt_cli_style::stream::stderr(), "{message}");
}

fn rebuild_content(doc: &Doc) -> Result<Vec<u8>, DecodeFault> {
    let before = project(doc, Namespace::Content)?;
    let txn = doc.transact();
    let vector = txn.state_vector();
    let encoded = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    let candidate = Doc::new();
    candidate.get_or_insert_text("html");
    candidate.get_or_insert_map("meta");
    candidate
        .transact_mut()
        .apply_update(Update::decode_v1(&encoded).map_err(|_| DecodeFault::Rejected)?)
        .map_err(|_| DecodeFault::Rejected)?;
    if project(&candidate, Namespace::Content)? != before {
        return Err(DecodeFault::Rejected);
    }
    let txn = candidate.transact();
    if txn.state_vector() != vector {
        return Err(DecodeFault::Rejected);
    }
    let replayed = txn.encode_state_as_update_v1(&StateVector::default());
    equivalent_update(&encoded, &replayed)?;
    Ok(encoded)
}

mod comparison {
    use super::{Any, DecodeFault, Update};
    use yrs::updates::decoder::Decode;
    /// Comparison-only encoder: yrs 0.28.0 sorts update structs but writes Any maps in hash order.
    /// Sort nested map keys while preserving every encoded struct, field and delete-set entry. These
    /// bytes are never published. Update::PartialEq alone is insufficient: it compares Item IDs only.
    struct ComparisonEncoder(yrs::updates::encoder::EncoderV1);
    impl yrs::encoding::write::Write for ComparisonEncoder {
        fn write_all(&mut self, bytes: &[u8]) {
            yrs::encoding::write::Write::write_all(&mut self.0, bytes);
        }
    }
    impl yrs::updates::encoder::Encoder for ComparisonEncoder {
        fn to_vec(self) -> Vec<u8> {
            yrs::updates::encoder::Encoder::to_vec(self.0)
        }
        fn reset_ds_cur_val(&mut self) {
            self.0.reset_ds_cur_val();
        }
        fn write_ds_clock(&mut self, clock: u32) {
            self.0.write_ds_clock(clock);
        }
        fn write_ds_len(&mut self, len: u32) {
            self.0.write_ds_len(len);
        }
        fn write_left_id(&mut self, id: &yrs::ID) {
            self.0.write_left_id(id);
        }
        fn write_right_id(&mut self, id: &yrs::ID) {
            self.0.write_right_id(id);
        }
        fn write_client(&mut self, client: yrs::ClientID) {
            self.0.write_client(client);
        }
        fn write_info(&mut self, info: u8) {
            self.0.write_info(info);
        }
        fn write_parent_info(&mut self, is_key: bool) {
            self.0.write_parent_info(is_key);
        }
        fn write_type_ref(&mut self, info: u8) {
            self.0.write_type_ref(info);
        }
        fn write_len(&mut self, len: u32) {
            self.0.write_len(len);
        }
        fn write_any(&mut self, any: &Any) {
            comparison_any(&mut self.0, any);
        }
        fn write_json(&mut self, any: &Any) {
            // Preserve EncoderV1's JSON encoding. Its map order can still differ, causing a
            // conservative false rejection; a rejected rebuild publishes nothing.
            self.0.write_json(any);
        }
        fn write_key(&mut self, key: &str) {
            self.0.write_key(key);
        }
    }
    fn comparison_any(encoder: &mut yrs::updates::encoder::EncoderV1, value: &Any) {
        use yrs::encoding::write::Write as YWrite;
        match value {
            Any::Map(map) => {
                encoder.write_u8(118);
                encoder.write_var(map.len() as u64);
                let mut entries = map.iter().collect::<Vec<_>>();
                entries.sort_unstable_by_key(|(key, _)| *key);
                for (key, value) in entries {
                    encoder.write_string(key);
                    comparison_any(encoder, value);
                }
            }
            Any::Array(items) => {
                encoder.write_u8(117);
                encoder.write_var(items.len() as u64);
                for value in items.iter() {
                    comparison_any(encoder, value);
                }
            }
            other => other.encode(encoder),
        }
    }
    pub(super) fn equivalent_update(expected: &[u8], actual: &[u8]) -> Result<(), DecodeFault> {
        use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
        let normalized = |bytes: &[u8]| -> Result<Vec<u8>, DecodeFault> {
            let update = Update::decode_v1(bytes).map_err(|_| DecodeFault::Rejected)?;
            let mut encoder = ComparisonEncoder(EncoderV1::new());
            update.encode(&mut encoder);
            Ok(encoder.to_vec())
        };
        if normalized(expected)? != normalized(actual)? {
            return Err(DecodeFault::Rejected);
        }
        Ok(())
    }
}
use comparison::equivalent_update;

fn project(doc: &Doc, namespace: Namespace) -> Result<Value, DecodeFault> {
    let names: &[&str] = match namespace {
        Namespace::Content => &["html", "meta"],
        Namespace::Own => &["threads", "messages", "intents", "replies"],
    };
    let txn = doc.transact();
    if txn.store().pending_update().is_some()
        || txn.store().pending_ds().is_some()
        || txn.root_refs().any(|(name, _)| !names.contains(&name))
    {
        return Err(DecodeFault::Rejected);
    }
    let mut roots = serde_json::Map::new();
    for name in names {
        // Named roots do not carry a wire-level type declaration. Inspect both
        // list and map aspects to reject mixed-type mutations of declared roots.
        let map = Root::<MapRef>::new(*name)
            .get(&txn)
            .ok_or(DecodeFault::Rejected)?;
        let text = Root::<TextRef>::new(*name)
            .get(&txn)
            .ok_or(DecodeFault::Rejected)?;
        if *name == "html" {
            if map.len(&txn) != 0
                || text.diff(&txn, |_| ()).iter().any(|d| {
                    d.attributes.is_some() || !matches!(&d.insert, Out::Any(Any::String(_)))
                })
            {
                return Err(DecodeFault::Rejected);
            }
            let source = text.get_string(&txn);
            if text.len(&txn) as usize != source.len() {
                return Err(DecodeFault::Rejected);
            }
            roots.insert((*name).into(), Value::String(source));
        } else {
            if text.len(&txn) != 0 {
                return Err(DecodeFault::Rejected);
            }
            let mut values = serde_json::Map::new();
            for (key, value) in map.iter(&txn) {
                let Out::Any(value) = value else {
                    return Err(DecodeFault::Rejected);
                };
                values.insert(
                    key.into(),
                    serde_json::to_value(value).map_err(|_| DecodeFault::Rejected)?,
                );
            }
            roots.insert((*name).into(), Value::Object(values));
        }
    }
    let projection = Value::Object(roots);
    validate_projection(namespace, &projection)?;
    Ok(projection)
}
fn baseline(input: &[u8]) -> Result<(), DecodeFault> {
    let wire: WireBaseline =
        serde_json::from_slice(input).map_err(|_| DecodeFault::InvalidInput)?;
    if wire.version != 1
        || wire
            .page_state
            .as_ref()
            .is_some_and(|v| !valid_page_state(v))
        || wire
            .creator
            .as_ref()
            .is_some_and(|v| crate::question_registry::validate_creator(v).is_err())
        || wire
            .questions
            .as_ref()
            .is_some_and(|v| crate::question_registry::validate(v).is_err())
        || wire
            .template
            .as_ref()
            .is_some_and(|v| crate::template_marks::validate_template(v).is_err())
        || wire
            .from_template
            .as_ref()
            .is_some_and(|v| crate::template_marks::validate_from(v).is_err())
        || wire.creation_recipient.as_ref().is_some_and(|v| !v.valid())
        || wire
            .original_author
            .as_deref()
            .is_some_and(|v| !valid_publisher_agent(v))
        || wire
            .publisher_agent
            .as_deref()
            .is_some_and(|v| !valid_publisher_agent(v))
    {
        return Err(DecodeFault::InvalidInput);
    }
    let digest = binary(&wire.source_digest, 32)?;
    if digest.len() != 32 || wire.title.len() > BASELINE_TITLE_BYTES {
        return Err(DecodeFault::InvalidInput);
    }
    let producing = matches!(wire.action, BaselineAction::Produce { .. });
    let expected_shell = match &wire.action {
        BaselineAction::Produce { shell, .. } => shell.clone(),
        _ => None,
    };
    let mut chunks = Vec::new();
    let (update, expected_source, expected_commitment) = match wire.action {
        BaselineAction::Produce { chunk_bytes, shell } => {
            let source = binary(&wire.source, BASELINE_BYTES)?;
            validate_view(&source, &wire.title, &digest)?;
            let source_text =
                std::str::from_utf8(&source).map_err(|_| DecodeFault::InvalidInput)?;
            let creation = BaselineMeta {
                creator: wire.creator.as_ref(),
                questions: wire.questions.as_ref(),
                template: wire.template.as_ref(),
                from_template: wire.from_template.as_ref(),
                publisher_agent: wire.publisher_agent.as_deref(),
                original_author: wire.original_author.as_deref(),
                creation_recipient: wire.creation_recipient.as_ref(),
                layout: wire.layout.as_ref(),
                attachments: wire.attachments.as_ref(),
                work: wire.work.as_ref(),
                page_state: wire.page_state.as_ref(),
            };
            if creation.layout.is_some_and(|layout| !layout.valid()) {
                return Err(DecodeFault::InvalidInput);
            }
            // Work rides on an ordinary page's single baseline update, never on a shell or on the
            // chunked creation of a new page.
            if wire.page_state.is_some() && (shell.is_some() || chunk_bytes.is_some()) {
                return Err(DecodeFault::InvalidInput);
            }
            if let Some(work) = &wire.work {
                crate::work::validate(work).map_err(|_| DecodeFault::InvalidInput)?;
                if shell.is_some() || chunk_bytes.is_some() {
                    return Err(DecodeFault::InvalidInput);
                }
            }
            let update = if let Some(shell) = shell {
                crate::shell::validate(&shell).map_err(|_| DecodeFault::InvalidInput)?;
                if !source.is_empty() || wire.attachments.is_some() || wire.layout.is_some() {
                    return Err(DecodeFault::InvalidInput);
                }
                let doc = Doc::new();
                let initial = fresh_baseline(Doc::new(), source_text, &wire.title, &creation);
                doc.get_or_insert_text("html");
                let meta = doc.get_or_insert_map("meta");
                let mut tx = doc.transact_mut();
                tx.apply_update(Update::decode_v1(&initial).map_err(|_| DecodeFault::Rejected)?)
                    .map_err(|_| DecodeFault::Rejected)?;
                meta.insert(&mut tx, "pageType", "shell");
                meta.insert(
                    &mut tx,
                    "shell",
                    Any::from_json(&shell.to_string()).map_err(|_| DecodeFault::Rejected)?,
                );
                let update = tx.encode_state_as_update_v1(&StateVector::default());
                if chunk_bytes.is_some() {
                    if update.len() > UPDATE_BYTES {
                        return Err(DecodeFault::Rejected);
                    }
                    chunks.push(update.clone());
                }
                update
            } else {
                match chunk_bytes {
                    Some(size) if (1024..=UPDATE_BYTES - 1024).contains(&size) => {
                        let doc = Doc::new();
                        chunks = chunked_baseline(&doc, source_text, &wire.title, &creation, size);
                        doc.transact()
                            .encode_state_as_update_v1(&StateVector::default())
                    }
                    Some(_) => return Err(DecodeFault::InvalidInput),
                    None => fresh_baseline(Doc::new(), source_text, &wire.title, &creation),
                }
            };
            (update, Some(source), None)
        }
        BaselineAction::Verify { update, commitment } => {
            // Verification transmits the update once. Materialized source must
            // match the authenticated fold's digest before commitment admission.
            if !wire.source.is_empty() {
                return Err(DecodeFault::InvalidInput);
            }
            (
                binary(&update, BASELINE_UPDATE_BYTES)?,
                None,
                Some(binary(&commitment, 32)?),
            )
        }
    };
    if update.len() > BASELINE_UPDATE_BYTES {
        return Err(DecodeFault::Rejected);
    }
    // Decode into a second fresh document. Exact text, title, roots, types and
    // absence of pending dependencies must agree before returning any result.
    let doc = Doc::new();
    doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    doc.transact_mut()
        .apply_update(Update::decode_v1(&update).map_err(|_| DecodeFault::Rejected)?)
        .map_err(|_| DecodeFault::Rejected)?;
    let projection = project(&doc, Namespace::Content)?;
    if producing && projection["meta"].get("shell") != expected_shell.as_ref() {
        return Err(DecodeFault::Rejected);
    }
    if producing && projection["meta"].get("pageState") != wire.page_state.as_ref() {
        return Err(DecodeFault::Rejected);
    }
    if producing && projection["meta"].get("work") != wire.work.as_ref() {
        return Err(DecodeFault::Rejected);
    }
    if producing
        && (projection["meta"].get("creator") != wire.creator.as_ref()
            || projection["meta"].get("questions") != wire.questions.as_ref()
            || projection["meta"].get("template") != wire.template.as_ref()
            || projection["meta"].get("fromTemplate") != wire.from_template.as_ref())
    {
        return Err(DecodeFault::Rejected);
    }
    let source_text = projection["html"].as_str().ok_or(DecodeFault::Rejected)?;
    validate_view(source_text.as_bytes(), &wire.title, &digest)?;
    if projection["meta"]["title"].as_str() != Some(wire.title.as_str())
        || (producing
            && (projection["meta"]["publisherAgent"].as_str() != wire.publisher_agent.as_deref()
                || projection["meta"]["originalAuthor"].as_str()
                    != wire.original_author.as_deref()))
        || (producing
            && (projection["meta"].get("layout").and_then(Value::as_str)
                != wire.layout.as_ref().map(|v| v.kind.as_str())
                || projection["meta"]
                    .get("layoutReason")
                    .and_then(Value::as_str)
                    != wire.layout.as_ref().and_then(|v| v.reason.as_deref())))
        || (producing
            && projection["meta"].get("creationRecipient")
                != wire
                    .creation_recipient
                    .as_ref()
                    .map(|v| serde_json::to_value(v).expect("typed recipient"))
                    .as_ref())
        || expected_source.is_some_and(|source| source != source_text.as_bytes())
        || (producing
            && projection["meta"].get("attachments")
                != wire
                    .attachments
                    .as_ref()
                    .map(|v| serde_json::to_value(v).expect("typed attachments"))
                    .as_ref())
    {
        return Err(DecodeFault::Rejected);
    }
    if !chunks.is_empty() {
        // The ordered updates alone must rebuild the same page.
        let replay = Doc::new();
        replay.get_or_insert_text("html");
        replay.get_or_insert_map("meta");
        for chunk in &chunks {
            if chunk.len() > UPDATE_BYTES {
                return Err(DecodeFault::Rejected);
            }
            replay
                .transact_mut()
                .apply_update(Update::decode_v1(chunk).map_err(|_| DecodeFault::Rejected)?)
                .map_err(|_| DecodeFault::Rejected)?;
        }
        if project(&replay, Namespace::Content)? != projection {
            return Err(DecodeFault::Rejected);
        }
    }
    let commitment = baseline_commitment(source_text.as_bytes(), &update)?;
    if expected_commitment.is_some_and(|expected| expected.as_slice() != commitment) {
        return Err(DecodeFault::Rejected);
    }
    write_reply(&WireBaselineResult {
        version: 1,
        input_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(input)),
        source_digest: URL_SAFE_NO_PAD.encode(digest),
        commitment: URL_SAFE_NO_PAD.encode(commitment),
        update: URL_SAFE_NO_PAD.encode(update),
        chunks: chunks
            .iter()
            .map(|chunk| URL_SAFE_NO_PAD.encode(chunk))
            .collect(),
        memory_limit: memory_limit(),
        pid: std::process::id(),
    })
}
/// The page as ordered updates, each inserting at most `size` bytes of text, so that none exceeds
/// the update limit. The first also sets the title and the publisher label.
fn chunked_baseline(
    doc: &Doc,
    source: &str,
    title: &str,
    creation: &BaselineMeta,
    size: usize,
) -> Vec<Vec<u8>> {
    let html = doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    let mut updates = Vec::new();
    let mut rest = source;
    let mut first = true;
    while first || !rest.is_empty() {
        let mut cut = rest.len().min(size);
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let (piece, tail) = rest.split_at(cut);
        let mut txn = doc.transact_mut();
        let end = html.len(&txn);
        html.insert(&mut txn, end, piece);
        if first {
            meta.insert(&mut txn, "title", title);
            creation.insert(&meta, &mut txn);
        }
        updates.push(txn.encode_update_v1());
        rest = tail;
        first = false;
    }
    updates
}
/// The optional metadata a creation baseline carries beside its title.
struct BaselineMeta<'a> {
    creator: Option<&'a Value>,
    questions: Option<&'a Value>,
    template: Option<&'a Value>,
    from_template: Option<&'a Value>,
    publisher_agent: Option<&'a str>,
    original_author: Option<&'a str>,
    creation_recipient: Option<&'a CreationRecipient>,
    layout: Option<&'a crate::decoder::PageLayout>,
    attachments: Option<&'a tmt_colab_model::attachment::DocumentAttachments>,
    work: Option<&'a Value>,
    page_state: Option<&'a Value>,
}
impl BaselineMeta<'_> {
    fn insert(&self, meta: &MapRef, txn: &mut TransactionMut) {
        for (key, value) in [
            ("creator", self.creator),
            ("questions", self.questions),
            ("template", self.template),
            ("fromTemplate", self.from_template),
            ("pageState", self.page_state),
        ] {
            if let Some(value) = value {
                meta.insert(
                    txn,
                    key,
                    Any::from_json(&value.to_string()).expect("validated metadata"),
                );
            }
        }
        if let Some(agent) = self.publisher_agent {
            meta.insert(txn, "publisherAgent", agent);
        }
        if let Some(author) = self.original_author {
            meta.insert(txn, "originalAuthor", author);
        }
        if let Some(recipient) = self.creation_recipient {
            meta.insert(txn, "creationRecipient", recipient_any(recipient));
        }
        if let Some(layout) = self.layout {
            meta.insert(txn, "layout", layout.kind.as_str());
            if let Some(reason) = &layout.reason {
                meta.insert(txn, "layoutReason", reason.as_str());
            }
        }
        if let Some(attachments) = self.attachments {
            meta.insert(
                txn,
                "attachments",
                Any::from_json(&serde_json::to_string(attachments).expect("typed attachments"))
                    .expect("inert attachments"),
            );
        }
        if let Some(work) = self.work {
            meta.insert(
                txn,
                "work",
                Any::from_json(&work.to_string()).expect("inert work"),
            );
        }
    }
}
fn fresh_baseline(doc: Doc, source: &str, title: &str, creation: &BaselineMeta) -> Vec<u8> {
    let html = doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    let mut txn = doc.transact_mut();
    html.insert(&mut txn, 0, source);
    meta.insert(&mut txn, "title", title);
    creation.insert(&meta, &mut txn);
    txn.encode_state_as_update_v1(&StateVector::default())
}
fn recipient_any(recipient: &CreationRecipient) -> Any {
    Any::Map(std::sync::Arc::new(std::collections::HashMap::from([
        (
            "machineId".to_owned(),
            Any::String(recipient.machine_id.clone().into()),
        ),
        (
            "agentId".to_owned(),
            Any::String(recipient.agent_id.clone().into()),
        ),
    ])))
}

#[cfg(test)]
mod baseline_tests {
    use super::*;
    #[test]
    fn fresh_baseline_matches_independent_update_v1_vectors() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../contracts/vectors/baseline-v1.json"
        ))
        .unwrap();
        for vector in vectors.as_array().unwrap() {
            let source = vector["source"].as_str().unwrap();
            let title = vector["title"].as_str().unwrap();
            let recipient = vector
                .get("creationRecipient")
                .map(|v| serde_json::from_value::<CreationRecipient>(v.clone()).unwrap());
            let layout = vector["layout"]
                .as_str()
                .map(|kind| crate::decoder::PageLayout {
                    kind: crate::decoder::LayoutKind::parse(kind).unwrap(),
                    reason: vector["layoutReason"].as_str().map(str::to_owned),
                });
            let update = fresh_baseline(
                Doc::with_client_id(1159),
                source,
                title,
                &BaselineMeta {
                    page_state: None,
                    creator: None,
                    questions: None,
                    template: None,
                    from_template: None,
                    publisher_agent: vector["publisherAgent"].as_str(),
                    original_author: vector["originalAuthor"].as_str(),
                    creation_recipient: recipient.as_ref(),
                    layout: layout.as_ref(),
                    attachments: None,
                    work: None,
                },
            );
            let encoded = URL_SAFE_NO_PAD.encode(&update);
            let expected_commitment = if encoded == vector["update"] {
                &vector["commitment"]
            } else {
                assert_eq!(encoded, vector["alternateUpdate"]);
                &vector["alternateCommitment"]
            };
            assert_eq!(
                URL_SAFE_NO_PAD.encode(Sha256::digest(source.as_bytes())),
                vector["sourceDigest"]
            );
            assert_eq!(
                URL_SAFE_NO_PAD.encode(baseline_commitment(source.as_bytes(), &update).unwrap()),
                *expected_commitment
            );
        }
    }
}

#[cfg(test)]
mod content_preparation_tests {
    use super::*;
    #[test]
    fn causal_replay_rejects_unordered_unresolved_and_wrong_projection_with_a_positive_control() {
        let doc = Doc::with_client_id(123);
        let html = doc.get_or_insert_text("html");
        doc.get_or_insert_map("meta")
            .insert(&mut doc.transact_mut(), "title", "T");
        html.insert(&mut doc.transact_mut(), 0, "old");
        let baseline = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let mut first = doc.transact_mut();
        html.remove_range(&mut first, 0, 3);
        html.insert(&mut first, 0, "A");
        let a = first.encode_update_v1();
        drop(first);
        let mut second = doc.transact_mut();
        html.insert(&mut second, 1, "B");
        let b = second.encode_update_v1();
        drop(second);
        let expected = serde_json::json!({"html":"AB","meta":{"title":"T"}});
        replay_content(&baseline, &[], &[a.clone(), b.clone()], &expected).unwrap();
        assert!(replay_content(&baseline, &[], &[b.clone(), a.clone()], &expected).is_err());
        assert!(replay_content(&baseline, &[], &[b], &expected).is_err());
        assert!(replay_content(&baseline, &[], &[a], &expected).is_err());
    }

    #[test]
    fn reply_final_cap_precedes_output_and_serialization_failure_discards_partial_bytes() {
        struct Ascii(usize);
        impl std::fmt::Display for Ascii {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                const CHUNK: &str = concat!(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                );
                for _ in 0..self.0 / CHUNK.len() {
                    f.write_str(CHUNK)?;
                }
                f.write_str(&CHUNK[..self.0 % CHUNK.len()])
            }
        }
        impl Serialize for Ascii {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        #[derive(Default)]
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        // Only the unchanged production Vec holds the large serialized reply.
        // Quotes contribute two bytes; the oracle and destination allocate none.
        for (characters, accepted) in [(STREAM_BYTES - 2, true), (STREAM_BYTES - 1, false)] {
            let mut writer = Count::default();
            let mut events = Vec::new();
            let result = write_reply_with(&Ascii(characters), &mut writer, &mut |s, e, n| {
                events.push((s, e, n));
            });
            assert_eq!(result.is_ok(), accepted);
            assert_eq!(writer.0, if accepted { STREAM_BYTES } else { 0 });
            assert_eq!(
                events.last(),
                Some(&if accepted {
                    (Stage::ReplyWrite, CheckpointBoundary::Leave, STREAM_BYTES)
                } else {
                    (Stage::ReplySerialize, CheckpointBoundary::Enter, 0)
                })
            );
            if !accepted {
                assert!(matches!(result, Err(DecodeFault::InvalidOutput)));
            }
        }
        struct Broken;
        impl Serialize for Broken {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element("private partial reply")?;
                Err(serde::ser::Error::custom("serialization failed"))
            }
        }
        let mut writer = Count::default();
        let mut events = Vec::new();
        assert!(matches!(
            write_reply_with(&Broken, &mut writer, &mut |s, e, n| events.push((s, e, n))),
            Err(DecodeFault::InvalidOutput)
        ));
        assert_eq!(writer.0, 0);
        assert_eq!(
            events,
            [(Stage::ReplySerialize, CheckpointBoundary::Enter, 0)]
        );
    }

    #[test]
    fn reply_write_failure_preserves_original_error_and_has_no_completed_write_checkpoint() {
        struct Partial(usize);
        impl Write for Partial {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.0 == 0 {
                    self.0 = bytes.len().min(3);
                    Ok(self.0)
                } else {
                    Err(std::io::Error::other("private writer failure"))
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = Partial(0);
        let mut events = Vec::new();
        assert!(matches!(
            write_reply_with(&"abcdef", &mut writer, &mut |s, e, n| events
                .push((s, e, n))),
            Err(DecodeFault::InvalidOutput)
        ));
        assert_eq!(writer.0, 3);
        assert_eq!(
            events.last(),
            Some(&(Stage::ReplyWrite, CheckpointBoundary::Enter, 0))
        );
        assert!(
            !events
                .iter()
                .any(|v| v.0 == Stage::ReplyWrite && v.1 == CheckpointBoundary::Leave)
        );
    }
}

#[cfg(test)]
mod projection_reuse_tests {
    use super::*;

    #[test]
    fn unchanged_content_and_own_projection_matches_independent_values_after_merge() {
        for namespace in [Namespace::Content, Namespace::Own] {
            let doc = Doc::with_client_id(818);
            let expected = match namespace {
                Namespace::Content => {
                    doc.get_or_insert_text("html")
                        .insert(&mut doc.transact_mut(), 0, "old 🐈\n");
                    let meta = doc.get_or_insert_map("meta");
                    meta.insert(&mut doc.transact_mut(), "title", "T");
                    meta.insert(&mut doc.transact_mut(), "publisherAgent", "agent");
                    serde_json::json!({"html":"old 🐈\n","meta":{"title":"T","publisherAgent":"agent"}})
                }
                Namespace::Own => {
                    for name in ["threads", "messages", "intents", "replies"] {
                        doc.get_or_insert_map(name);
                    }
                    doc.get_or_insert_map("threads").insert(
                        &mut doc.transact_mut(),
                        "legacy",
                        "retained 🐈",
                    );
                    serde_json::json!({"threads":{"legacy":"retained 🐈"},"messages":{},"intents":{},"replies":{}})
                }
            };
            let original = doc
                .transact()
                .encode_state_as_update_v1(&StateVector::default());
            let admitted = project(&doc, namespace).unwrap();
            let merged = yrs::merge_updates_v1([original.as_slice()]).unwrap();
            assert_eq!(admitted, expected);
            assert_eq!(project(&doc, namespace).unwrap(), admitted);
            let reader = Doc::new();
            match namespace {
                Namespace::Content => {
                    reader.get_or_insert_text("html");
                    reader.get_or_insert_map("meta");
                }
                Namespace::Own => {
                    for name in ["threads", "messages", "intents", "replies"] {
                        reader.get_or_insert_map(name);
                    }
                }
            }
            reader
                .transact_mut()
                .apply_update(Update::decode_v1(&merged).unwrap())
                .unwrap();
            assert_eq!(project(&reader, namespace).unwrap(), admitted);
            assert_eq!(
                doc.transact()
                    .encode_state_as_update_v1(&StateVector::default()),
                original
            );
        }
    }

    #[test]
    fn replay_update_guard_accepts_a_different_any_map_key_order() {
        use std::{collections::HashMap, sync::Arc};
        use yrs::encoding::write::Write as YWrite;
        fn map_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
            let mut bytes = Vec::new();
            bytes.write_u8(118);
            bytes.write_var(entries.len() as u64);
            for (key, value) in entries {
                bytes.write_string(key);
                Any::from(*value).encode(&mut bytes);
            }
            bytes
        }
        let doc = Doc::with_client_id(54560);
        let mut fields = HashMap::new();
        fields.insert("a".to_owned(), Any::from("one"));
        fields.insert("b".to_owned(), Any::from("two"));
        doc.get_or_insert_map("meta").insert(
            &mut doc.transact_mut(),
            "info",
            Any::Map(Arc::new(fields)),
        );
        let first = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let ascending = map_bytes(&[("a", "one"), ("b", "two")]);
        let descending = map_bytes(&[("b", "two"), ("a", "one")]);
        let (position, replacement) =
            if let Some(pos) = first.windows(ascending.len()).position(|v| v == ascending) {
                (pos, &descending)
            } else {
                (
                    first
                        .windows(descending.len())
                        .position(|v| v == descending)
                        .expect("encoded map"),
                    &ascending,
                )
            };
        let mut second = first.clone();
        second[position..position + replacement.len()].copy_from_slice(replacement);
        assert_ne!(first, second);
        assert!(Update::decode_v1(&second).is_ok());
        assert!(equivalent_update(&first, &second).is_ok());
    }

    #[test]
    fn replay_update_guard_rejects_a_different_struct_identity() {
        let encode = |client| {
            let doc = Doc::with_client_id(client);
            doc.get_or_insert_text("html")
                .insert(&mut doc.transact_mut(), 0, "same");
            let txn = doc.transact();
            txn.encode_state_as_update_v1(&StateVector::default())
        };
        assert!(matches!(
            equivalent_update(&encode(54563), &encode(54564)),
            Err(DecodeFault::Rejected)
        ));
    }

    #[test]
    fn replay_update_guard_rejects_a_dropped_delete_set() {
        let mut options = yrs::Options::with_client_id(yrs::ClientID::new(54561));
        options.skip_gc = true;
        let doc = Doc::with_options(options);
        let html = doc.get_or_insert_text("html");
        html.insert(&mut doc.transact_mut(), 0, "abcd");
        let no_delete = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        html.remove_range(&mut doc.transact_mut(), 0, 4);
        let deleted = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let before = Update::decode_v1(&no_delete).unwrap();
        let after = Update::decode_v1(&deleted).unwrap();
        assert_eq!(before.state_vector(), after.state_vector());
        assert_eq!(before.insertions(true), after.insertions(true));
        assert_ne!(before.delete_set(), after.delete_set());
        assert!(equivalent_update(&deleted, &deleted).is_ok());
        assert!(matches!(
            equivalent_update(&deleted, &no_delete),
            Err(DecodeFault::Rejected)
        ));
    }

    #[test]
    fn replay_update_guard_rejects_changed_struct_at_the_same_clocks() {
        let encode = |text: &str| {
            let doc = Doc::with_client_id(54562);
            doc.get_or_insert_text("html")
                .insert(&mut doc.transact_mut(), 0, text);
            let txn = doc.transact();
            txn.encode_state_as_update_v1(&StateVector::default())
        };
        let expected = encode("abcd");
        let changed = encode("wxyz");
        let left = Update::decode_v1(&expected).unwrap();
        let right = Update::decode_v1(&changed).unwrap();
        assert_eq!(left.state_vector(), right.state_vector());
        assert_eq!(left.delete_set(), right.delete_set());
        assert!(matches!(
            equivalent_update(&expected, &changed),
            Err(DecodeFault::Rejected)
        ));
    }

    #[test]
    fn projection_rejects_foreign_and_mixed_roots_and_is_fresh_after_edit() {
        let doc = Doc::with_client_id(819);
        let html = doc.get_or_insert_text("html");
        html.insert(&mut doc.transact_mut(), 0, "old");
        doc.get_or_insert_map("meta")
            .insert(&mut doc.transact_mut(), "title", "T");
        let before = project(&doc, Namespace::Content).unwrap();
        html.insert(&mut doc.transact_mut(), 3, " new");
        let after = project(&doc, Namespace::Content).unwrap();
        assert_eq!(before["html"], "old");
        assert_eq!(after["html"], "old new");
        assert_eq!(before["meta"], after["meta"]);
        doc.get_or_insert_map("html")
            .insert(&mut doc.transact_mut(), "mixed", "bad");
        assert!(matches!(
            project(&doc, Namespace::Content),
            Err(DecodeFault::Rejected)
        ));
        let own = Doc::new();
        for name in ["threads", "messages", "intents", "replies"] {
            own.get_or_insert_map(name);
        }
        assert!(project(&own, Namespace::Own).is_ok());
        own.get_or_insert_map("foreign")
            .insert(&mut own.transact_mut(), "x", "bad");
        assert!(matches!(
            project(&own, Namespace::Own),
            Err(DecodeFault::Rejected)
        ));
    }
}
