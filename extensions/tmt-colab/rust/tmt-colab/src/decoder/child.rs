//! All production yrs decoding lives here, reached only by the private child entry.
use super::*;
use std::io::{Read, Write};
use yrs::{
    Any, Doc, GetString, Map, MapRef, Out, ReadTxn, Root, StateVector, Text, TextRef, Transact,
    Update, updates::decoder::Decode,
};

pub(super) fn run() -> std::process::ExitCode {
    // Linux enforces RLIMIT_AS. macOS accepts it without enforcing it, so report
    // unavailable rather than claiming memory containment or refusing to run.
    #[cfg(target_os = "linux")]
    {
        use nix::sys::resource::{Resource, getrlimit, setrlimit};
        if setrlimit(Resource::RLIMIT_AS, MEMORY_BYTES, MEMORY_BYTES).is_err()
            || getrlimit(Resource::RLIMIT_AS).ok() != Some((MEMORY_BYTES, MEMORY_BYTES))
        {
            diagnostic("decoder memory limit failed");
            return std::process::ExitCode::FAILURE;
        }
    }
    if memory_limit() == MemoryLimit::Unavailable {
        diagnostic("memory limit unavailable");
    }
    std::panic::set_hook(Box::new(|_| diagnostic("decoder panic")));
    match execute() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => {
            diagnostic("decoder rejected");
            std::process::ExitCode::FAILURE
        }
    }
}
fn execute() -> Result<(), DecodeFault> {
    let mut input = Vec::new();
    std::io::stdin()
        .take((STREAM_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| DecodeFault::InvalidInput)?;
    if input.len() > STREAM_BYTES {
        return Err(DecodeFault::InvalidInput);
    }
    if std::env::args().nth(2).as_deref() == Some("baseline") {
        return baseline(&input);
    }
    if std::env::args().nth(2).as_deref() == Some("prepare-content") {
        return prepare_content(&input);
    }
    let wire: WireBatch<BorrowedWireText<'_>> =
        serde_json::from_slice(&input).map_err(|_| DecodeFault::InvalidInput)?;
    if wire.version != 1
        || wire.updates.len() > UPDATES
        || wire
            .publisher_agent
            .as_deref()
            .is_some_and(|v| !valid_publisher_agent(v))
        || (wire.source.is_none() && wire.publisher_agent.is_some())
    {
        return Err(DecodeFault::InvalidInput);
    }
    let baseline = binary(&wire.baseline, STATE_BYTES)?;
    let updates: Vec<_> = wire
        .updates
        .iter()
        .map(|v| binary(v, STATE_BYTES))
        .collect::<Result<_, _>>()?;
    if baseline.len() + updates.iter().map(Vec::len).sum::<usize>() > STATE_BYTES {
        return Err(DecodeFault::InvalidInput);
    }
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
    let mut discussion = std::collections::BTreeMap::new();
    for bytes in std::iter::once(&baseline)
        .filter(|v| !v.is_empty())
        .chain(updates.iter())
    {
        let update = Update::decode_v1(bytes).map_err(|_| DecodeFault::Rejected)?;
        doc.transact_mut()
            .apply_update(update)
            .map_err(|_| DecodeFault::Rejected)?;
        if wire.namespace == Namespace::Own {
            let next = discussion_records(&doc)?;
            if discussion
                .iter()
                .any(|(key, value)| next.get(key) != Some(value))
            {
                return Err(DecodeFault::Rejected);
            }
            discussion = next;
        }
    }
    if wire.merge_only {
        if wire.source.is_some() {
            return Err(DecodeFault::InvalidInput);
        }
        let merged = yrs::merge_updates_v1(updates.iter().map(Vec::as_slice))
            .map_err(|_| DecodeFault::Rejected)?;
        if merged.len() > STATE_BYTES {
            return Err(DecodeFault::Rejected);
        }
        return write_reply(&WireResult {
            version: 1,
            namespace: wire.namespace,
            input_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(&input)),
            merged: URL_SAFE_NO_PAD.encode(merged),
            projection: Value::Null,
            memory_limit: memory_limit(),
            pid: std::process::id(),
        });
    }
    let before = project(&doc, wire.namespace)?;
    // Edit only the admitted structs. The delta never reattributes foreign content.
    let (merged, projection) = if let Some(source) = &wire.source {
        if wire.namespace != Namespace::Content || source.len() > BASELINE_BYTES {
            return Err(DecodeFault::InvalidInput);
        }
        let old = before["html"].as_str().ok_or(DecodeFault::Rejected)?;
        let (start, end) = content_diff(old, source);
        let vector = doc.transact().state_vector();
        let text = doc.get_or_insert_text("html");
        let meta = doc.get_or_insert_map("meta");
        let mut tx = doc.transact_mut();
        text.remove_range(
            &mut tx,
            start.try_into().map_err(|_| DecodeFault::Rejected)?,
            (old.len() - start - end)
                .try_into()
                .map_err(|_| DecodeFault::Rejected)?,
        );
        text.insert(
            &mut tx,
            start.try_into().map_err(|_| DecodeFault::Rejected)?,
            &source[start..source.len() - end],
        );
        if let Some(agent) = wire.publisher_agent.as_deref() {
            meta.insert(&mut tx, "publisherAgent", agent);
        } else {
            meta.remove(&mut tx, "publisherAgent");
        }
        let merged = tx.encode_state_as_update_v1(&vector);
        drop(tx);
        (merged, project(&doc, wire.namespace)?)
    } else {
        // Merge the author's updates, never encode the shared document.
        let merged = yrs::merge_updates_v1(updates.iter().map(Vec::as_slice))
            .map_err(|_| DecodeFault::Rejected)?;
        (merged, before)
    };
    // A prepared edit is one update; a read's merged tail may be the whole state.
    if merged.len()
        > if wire.source.is_some() {
            UPDATE_BYTES
        } else {
            STATE_BYTES
        }
    {
        return Err(DecodeFault::Rejected);
    }
    let reply = WireResult {
        version: 1,
        namespace: wire.namespace,
        input_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(&input)),
        merged: URL_SAFE_NO_PAD.encode(merged),
        projection,
        memory_limit: memory_limit(),
        pid: std::process::id(),
    };
    write_reply(&reply)
}
fn content_diff(old: &str, source: &str) -> (usize, usize) {
    let start = old
        .chars()
        .zip(source.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let end = old[start..]
        .chars()
        .rev()
        .zip(source[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    (start, end)
}
fn content_document(baseline: &[u8], updates: &[Vec<u8>]) -> Result<Doc, DecodeFault> {
    let doc = Doc::new();
    doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    for bytes in std::iter::once(baseline)
        .filter(|v| !v.is_empty())
        .chain(updates.iter().map(Vec::as_slice))
    {
        doc.transact_mut()
            .apply_update(Update::decode_v1(bytes).map_err(|_| DecodeFault::Rejected)?)
            .map_err(|_| DecodeFault::Rejected)?;
    }
    Ok(doc)
}
fn replay_content(
    baseline: &[u8],
    admitted: &[Vec<u8>],
    updates: &[Vec<u8>],
    expected: &Value,
) -> Result<(), DecodeFault> {
    let replay = content_document(baseline, admitted)?;
    for bytes in updates {
        replay
            .transact_mut()
            .apply_update(Update::decode_v1(bytes).map_err(|_| DecodeFault::Rejected)?)
            .map_err(|_| DecodeFault::Rejected)?;
        project(&replay, Namespace::Content)?; // Reject unresolved causal prefixes, not only the final state.
    }
    if project(&replay, Namespace::Content)? != *expected {
        return Err(DecodeFault::Rejected);
    }
    Ok(())
}
fn prepare_content(input: &[u8]) -> Result<(), DecodeFault> {
    let wire: WireContentPreparation<BorrowedWireText<'_>, Value, BorrowedWireText<'_>> =
        serde_json::from_slice(input).map_err(|_| DecodeFault::InvalidInput)?;
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
    let baseline = binary(&wire.baseline, STATE_BYTES)?;
    let admitted = wire
        .updates
        .iter()
        .map(|v| binary(v, STATE_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    if baseline.len() + admitted.iter().map(Vec::len).sum::<usize>() > STATE_BYTES {
        return Err(DecodeFault::InvalidInput);
    }
    let doc = content_document(&baseline, &admitted)?;
    let before = project(&doc, Namespace::Content)?;
    if before != wire.expected_base {
        return Err(DecodeFault::Rejected);
    }
    let edit = ContentEdit {
        source: &wire.source,
        publisher_agent: wire.publisher_agent.as_deref(),
    };
    let expected = edited_projection(&before, edit);
    let mut updates = Vec::new();
    if expected != before {
        let old = before["html"].as_str().ok_or(DecodeFault::Rejected)?;
        let (start, end) = content_diff(old, &wire.source);
        let mut rest = &wire.source[start..wire.source.len() - end];
        let html = doc.get_or_insert_text("html");
        let meta = doc.get_or_insert_map("meta");
        let mut first = true;
        let mut offset = start;
        while first || !rest.is_empty() {
            let mut cut = rest.len().min(CREATE_CHUNK_BYTES);
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            let (piece, tail) = rest.split_at(cut);
            let mut tx = doc.transact_mut();
            if first {
                let remove = old.len() - start - end;
                if remove > 0 {
                    html.remove_range(&mut tx, start as u32, remove as u32);
                }
                if before["meta"]["publisherAgent"].as_str() != edit.publisher_agent {
                    if let Some(agent) = edit.publisher_agent {
                        meta.insert(&mut tx, "publisherAgent", agent);
                    } else {
                        meta.remove(&mut tx, "publisherAgent");
                    }
                }
            }
            if !piece.is_empty() {
                html.insert(&mut tx, offset as u32, piece);
            }
            updates.push(tx.encode_update_v1());
            offset += piece.len();
            rest = tail;
            first = false;
        }
        if updates.len() > WRITE_TAIL_UPDATES
            || updates.iter().any(|v| v.len() > UPDATE_BYTES)
            || updates.iter().map(Vec::len).sum::<usize>() > WRITE_TAIL_BYTES
        {
            return Err(DecodeFault::Rejected);
        }
    }
    let projection = project(&doc, Namespace::Content)?;
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
    replay_content(&baseline, &admitted, &updates, &expected)?;
    write_reply(&WirePreparedContent {
        version: 1,
        input_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(input)),
        batch: if updates.is_empty() {
            WireContentBatch::Noop
        } else {
            WireContentBatch::Updates {
                updates: updates.iter().map(|v| URL_SAFE_NO_PAD.encode(v)).collect(),
            }
        },
        projection,
        memory_limit: memory_limit(),
        pid: std::process::id(),
    })
}
// Capture only materialized typed records, allowing existing checkpoint steps
// with pending dependencies. A later update cannot replace or remove a record.
fn discussion_records(
    doc: &Doc,
) -> Result<std::collections::BTreeMap<(String, String), Value>, DecodeFault> {
    let txn = doc.transact();
    let mut records = std::collections::BTreeMap::new();
    for root in ["threads", "messages"] {
        let map = Root::<MapRef>::new(root)
            .get(&txn)
            .ok_or(DecodeFault::Rejected)?;
        for (key, value) in map.iter(&txn) {
            if let Out::Any(Any::Map(fields)) = value
                && matches!(fields.get("kind"), Some(Any::String(kind)) if kind.as_ref() == "thread" || kind.as_ref() == "comment")
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

fn write_reply(reply: &impl Serialize) -> Result<(), DecodeFault> {
    let output = serde_json::to_vec(reply).map_err(|_| DecodeFault::InvalidOutput)?;
    if output.len() > STREAM_BYTES {
        return Err(DecodeFault::InvalidOutput);
    }
    tmt_cli_style::stream::stdout(true)
        .write_all(&output)
        .map_err(|_| DecodeFault::InvalidOutput)
}

fn diagnostic(message: &str) {
    let _ = writeln!(tmt_cli_style::stream::stderr(), "{message}");
}

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
    let mut chunks = Vec::new();
    let (update, expected_source, expected_commitment) = match wire.action {
        BaselineAction::Produce { chunk_bytes } => {
            let source = binary(&wire.source, BASELINE_BYTES)?;
            validate_view(&source, &wire.title, &digest)?;
            let source_text =
                std::str::from_utf8(&source).map_err(|_| DecodeFault::InvalidInput)?;
            let update = match chunk_bytes {
                Some(size) if (1024..=UPDATE_BYTES - 1024).contains(&size) => {
                    let doc = Doc::new();
                    chunks = chunked_baseline(
                        &doc,
                        source_text,
                        &wire.title,
                        wire.publisher_agent.as_deref(),
                        size,
                    );
                    doc.transact()
                        .encode_state_as_update_v1(&StateVector::default())
                }
                Some(_) => return Err(DecodeFault::InvalidInput),
                None => fresh_baseline(
                    Doc::new(),
                    source_text,
                    &wire.title,
                    wire.publisher_agent.as_deref(),
                ),
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
    let source_text = projection["html"].as_str().ok_or(DecodeFault::Rejected)?;
    validate_view(source_text.as_bytes(), &wire.title, &digest)?;
    if projection["meta"]["title"].as_str() != Some(wire.title.as_str())
        || (producing
            && projection["meta"]["publisherAgent"].as_str() != wire.publisher_agent.as_deref())
        || expected_source.is_some_and(|source| source != source_text.as_bytes())
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
    publisher_agent: Option<&str>,
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
            if let Some(agent) = publisher_agent {
                meta.insert(&mut txn, "publisherAgent", agent);
            }
        }
        updates.push(txn.encode_update_v1());
        rest = tail;
        first = false;
    }
    updates
}
fn fresh_baseline(doc: Doc, source: &str, title: &str, publisher_agent: Option<&str>) -> Vec<u8> {
    let html = doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    let mut txn = doc.transact_mut();
    html.insert(&mut txn, 0, source);
    meta.insert(&mut txn, "title", title);
    if let Some(agent) = publisher_agent {
        meta.insert(&mut txn, "publisherAgent", agent);
    }
    txn.encode_state_as_update_v1(&StateVector::default())
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
            let update = fresh_baseline(
                Doc::with_client_id(1159),
                source,
                title,
                vector["publisherAgent"].as_str(),
            );
            assert_eq!(URL_SAFE_NO_PAD.encode(&update), vector["update"]);
            assert_eq!(
                URL_SAFE_NO_PAD.encode(Sha256::digest(source.as_bytes())),
                vector["sourceDigest"]
            );
            assert_eq!(
                URL_SAFE_NO_PAD.encode(baseline_commitment(source.as_bytes(), &update).unwrap()),
                vector["commitment"]
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
