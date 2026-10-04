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
    let wire: WireBatch = serde_json::from_slice(&input).map_err(|_| DecodeFault::InvalidInput)?;
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
    let baseline = binary(&wire.baseline, BASELINE_BYTES)?;
    let updates: Vec<_> = wire
        .updates
        .iter()
        .map(|v| binary(v, UPDATE_BYTES))
        .collect::<Result<_, _>>()?;
    if updates.iter().map(Vec::len).sum::<usize>() > UPDATE_BYTES {
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
    let before = project(&doc, wire.namespace)?;
    // Edit only the admitted structs. The delta never reattributes foreign content.
    let merged = if let Some(source) = &wire.source {
        if wire.namespace != Namespace::Content || source.len() > BASELINE_BYTES {
            return Err(DecodeFault::InvalidInput);
        }
        let old = before["html"].as_str().ok_or(DecodeFault::Rejected)?;
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
        tx.encode_state_as_update_v1(&vector)
    } else {
        // Merge the author's updates, never encode the shared document.
        yrs::merge_updates_v1(updates.iter().map(Vec::as_slice))
            .map_err(|_| DecodeFault::Rejected)?
    };
    let projection = project(&doc, wire.namespace)?;
    if merged.len() > UPDATE_BYTES {
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
    let producing = matches!(wire.action, BaselineAction::Produce {});
    let (update, expected_source, expected_commitment) = match wire.action {
        BaselineAction::Produce {} => {
            let source = binary(&wire.source, BASELINE_BYTES)?;
            validate_view(&source, &wire.title, &digest)?;
            let source_text =
                std::str::from_utf8(&source).map_err(|_| DecodeFault::InvalidInput)?;
            (
                fresh_baseline(
                    Doc::new(),
                    source_text,
                    &wire.title,
                    wire.publisher_agent.as_deref(),
                ),
                Some(source),
                None,
            )
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
        memory_limit: memory_limit(),
        pid: std::process::id(),
    })
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
