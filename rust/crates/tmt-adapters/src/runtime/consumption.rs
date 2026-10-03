//! Driver-owned completed-request counters. Claude tracks appended records,
//! not context growth; Codex reports cumulative counters itself. The source
//! formats are unofficial, and evidence loss establishes a new baseline.
use super::transcript;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::Path,
    time::Instant,
};
use tmt_core::limits::is_valid_js_safe_integer;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Counts {
    input: u64,
    output: u64,
    cached: u64,
}

impl Counts {
    fn valid(self) -> bool {
        self.cached <= self.input
            && self
                .input
                .checked_add(self.output)
                .is_some_and(is_valid_js_safe_integer)
    }

    fn add(self, other: Self) -> Option<Self> {
        let next = Self {
            input: self.input.checked_add(other.input)?,
            output: self.output.checked_add(other.output)?,
            cached: self.cached.checked_add(other.cached)?,
        };
        next.valid().then_some(next)
    }
}

/// The public contract. Cached input is already part of input; sequence and
/// epoch are evidence boundaries, not request identities or generation times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Consumption {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub epoch: String,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub complete: bool,
    pub gap: bool,
}

impl Consumption {
    fn baseline(now: u64, gap: bool) -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            epoch: uuid::Uuid::new_v4().to_string(),
            sequence: 1,
            observed_at_ms: now,
            complete: !gap,
            gap,
        }
    }

    fn counts(&self) -> Counts {
        Counts {
            input: self.input_tokens,
            output: self.output_tokens,
            cached: self.cached_input_tokens,
        }
    }

    fn set_counts(&mut self, counts: Counts) {
        self.input_tokens = counts.input;
        self.output_tokens = counts.output;
        self.cached_input_tokens = counts.cached;
    }

    fn advance(&mut self, now: u64) -> Option<()> {
        if now < self.observed_at_ms {
            return None;
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .filter(|n| is_valid_js_safe_integer(*n))?;
        self.observed_at_ms = now;
        Some(())
    }

    fn valid(&self) -> bool {
        self.counts().valid()
            && uuid::Uuid::parse_str(&self.epoch).is_ok()
            && self.sequence > 0
            && is_valid_js_safe_integer(self.sequence)
            && self.observed_at_ms > 0
            && is_valid_js_safe_integer(self.observed_at_ms)
            && !(self.complete && self.gap)
    }

    pub fn document(&self) -> Value {
        json!(self)
    }
}

/// Compact opaque cursor data stays inside DriverState's existing 1 KiB cap.
/// Hashes retain equality, not provider IDs or paths. Only the last counted
/// message is retained: unexpected noncontiguous repeats may count again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceCursor {
    dev: u64,
    ino: u64,
    offset: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    counts: Option<Counts>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    discard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub value: Consumption,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<SourceCursor>,
}

impl State {
    pub fn read(value: &Value) -> Option<Self> {
        let state: Self = serde_json::from_value(value.clone()).ok()?;
        let hash = |hash: &str| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        (state.value.valid()
            && state.cursor.as_ref().is_none_or(|cursor| {
                cursor.last.as_deref().is_none_or(hash)
                    && cursor.request.as_deref().is_none_or(hash)
                    && cursor.counts.is_none_or(Counts::valid)
                    && cursor.last.is_some() == cursor.counts.is_some()
                    && (cursor.last.is_some() || cursor.request.is_none())
            }))
        .then_some(state)
    }

    pub fn document(&self) -> Value {
        json!(self)
    }
}

fn fingerprint(id: &str) -> Option<String> {
    if id.is_empty() || id.len() > 256 || id.trim().is_empty() || id.chars().any(char::is_control) {
        return None;
    }
    Some(
        Sha256::digest(id.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn optional_count(value: &Value, key: &str) -> Option<u64> {
    match value.get(key) {
        None => Some(0),
        Some(value) => value.as_u64(),
    }
}

struct Message {
    id: String,
    request: Option<String>,
    counts: Counts,
}

/// Ok(None) is a foreign, sidechain or synthetic record; Err means a main
/// assistant record cannot establish its consumption safely.
fn claude_message(line: &str) -> Result<Option<Message>, ()> {
    // Without an assistant value or any escapes this cannot be a main
    // record. Validate syntax without constructing its (often large) content:
    // malformed foreign records must still gap, just as candidate records do.
    if !line.contains("\"assistant\"")
        && !line.contains('\\')
        // RawValue does not enforce Value's recursion limit. A conservative
        // delimiter count keeps deeply nested evidence on the existing path.
        && line.bytes().filter(|b| matches!(b, b'{' | b'[')).take(128).count() < 128
    {
        serde_json::from_str::<&serde_json::value::RawValue>(line).map_err(|_| ())?;
        return Ok(None);
    }
    let entry: Value = serde_json::from_str(line).map_err(|_| ())?;
    if entry["type"] != "assistant"
        || entry["isSidechain"] == true
        || entry["message"]["model"] == "<synthetic>"
    {
        return Ok(None);
    }
    let message = &entry["message"];
    let usage = &message["usage"];
    let cached = optional_count(usage, "cache_read_input_tokens").ok_or(())?;
    let counts = Counts {
        input: usage["input_tokens"]
            .as_u64()
            .ok_or(())?
            .checked_add(cached)
            .ok_or(())?
            .checked_add(optional_count(usage, "cache_creation_input_tokens").ok_or(())?)
            .ok_or(())?,
        output: usage["output_tokens"].as_u64().ok_or(())?,
        cached,
    };
    let id = fingerprint(message["id"].as_str().ok_or(())?).ok_or(())?;
    let request = match entry.get("requestId") {
        None => None,
        Some(value) => Some(fingerprint(value.as_str().ok_or(())?).ok_or(())?),
    };
    if !counts.valid() {
        return Err(());
    }
    Ok(Some(Message {
        id,
        request,
        counts,
    }))
}

/// First observation baselines at EOF, without replaying historical requests.
/// A lost cursor also baselines at EOF, explicitly marked as a gap.
pub fn claude(
    root: &Path,
    path: &Path,
    previous: Option<&State>,
    now: u64,
    deadline: Instant,
) -> Option<State> {
    claude_until(root, path, previous, now, || Instant::now() >= deadline)
}

fn claude_until(
    root: &Path,
    path: &Path,
    previous: Option<&State>,
    now: u64,
    mut expired: impl FnMut() -> bool,
) -> Option<State> {
    if now == 0 || !is_valid_js_safe_integer(now) {
        return None;
    }
    let mut file = transcript::open(root, path)?;
    let metadata = file.metadata().ok()?;
    let end = metadata.len();
    let boundary = if end == 0 {
        true
    } else {
        file.seek(SeekFrom::Start(end - 1)).ok()?;
        let mut byte = [0];
        file.read_exact(&mut byte).ok()?;
        byte[0] == b'\n'
    };
    let baseline = |gap| State {
        value: Consumption::baseline(now, gap),
        cursor: Some(SourceCursor {
            dev: metadata.dev(),
            ino: metadata.ino(),
            offset: end,
            last: None,
            request: None,
            counts: None,
            discard: !boundary,
        }),
    };
    let Some(previous) = previous else {
        // Absorb the last historical group if later content-block records
        // for it are appended. IDs absent from legacy fixtures mean no counter.
        let message =
            transcript::latest_at(&mut file, end, |line| claude_message(line).ok().flatten())?;
        let mut next = baseline(false);
        next.value.complete = boundary;
        let cursor = next.cursor.as_mut()?;
        cursor.last = Some(message.id);
        cursor.request = message.request;
        cursor.counts = Some(message.counts);
        return Some(next);
    };
    if now < previous.value.observed_at_ms {
        return None;
    }
    let cursor = previous.cursor.as_ref()?;
    if cursor.dev != metadata.dev() || cursor.ino != metadata.ino() || end < cursor.offset {
        return Some(baseline(true));
    }
    if end == cursor.offset {
        return Some(previous.clone());
    }
    file.seek(SeekFrom::Start(cursor.offset)).ok()?;
    let mut reader = BufReader::with_capacity(8192, file.take(end - cursor.offset));
    // A record (including its newline) retains the old one-MiB bound, but
    // the appended range can contain arbitrarily many records until deadline.
    let mut raw = Vec::with_capacity(transcript::TAIL_LIMIT as usize);
    let mut next = previous.clone();
    let mut consumed = 0u64;
    let mut read = 0u64;
    let complete = loop {
        if consumed == end - cursor.offset {
            break true;
        }
        if expired() {
            break false;
        }
        let available = reader.fill_buf().ok()?;
        if expired() {
            break false;
        }
        if available.is_empty() {
            if read != end - cursor.offset {
                return None;
            }
            break false;
        }
        let size = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if raw.len() + size > transcript::TAIL_LIMIT as usize {
            return Some(baseline(true));
        }
        raw.extend_from_slice(&available[..size]);
        reader.consume(size);
        read += size as u64;
        if raw.last() != Some(&b'\n') {
            continue;
        }
        if next.cursor.as_ref()?.discard {
            consumed = read;
            next.cursor.as_mut()?.discard = false;
            raw.clear();
            continue;
        }
        let Ok(line) = std::str::from_utf8(&raw) else {
            return Some(baseline(true));
        };
        if line.trim().is_empty() {
            consumed = read;
            raw.clear();
            continue;
        }
        if expired() {
            break false;
        }
        let parsed = claude_message(line);
        raw.clear();
        // Checkpoint only after validation, even if parsing crossed the
        // deadline. The next iteration checks time before reading more.
        let message = match parsed {
            Ok(Some(message)) => message,
            Ok(None) => {
                consumed = read;
                continue;
            }
            Err(()) => return Some(baseline(true)),
        };
        consumed = read;
        let cursor = next.cursor.as_mut()?;
        if cursor.last.as_ref() == Some(&message.id) {
            if cursor.request != message.request || cursor.counts != Some(message.counts) {
                return Some(baseline(true));
            }
            continue;
        }
        let Some(counts) = next.value.counts().add(message.counts) else {
            return Some(baseline(true));
        };
        next.value.set_counts(counts);
        cursor.last = Some(message.id);
        cursor.request = message.request;
        cursor.counts = Some(message.counts);
    };
    if consumed == 0 && complete == previous.value.complete {
        return Some(previous.clone());
    }
    next.cursor.as_mut()?.offset += consumed;
    next.value.complete = complete;
    next.value.gap = false;
    next.value.advance(now)?;
    Some(next)
}

fn codex_counts(entry: &Value) -> Option<Counts> {
    let usage = &entry["payload"]["info"]["total_token_usage"];
    let counts = Counts {
        input: usage["input_tokens"].as_u64()?,
        output: usage["output_tokens"].as_u64()?,
        cached: usage["cached_input_tokens"].as_u64()?,
    };
    // The provider's total includes output (including reasoning) once.
    (counts.valid()
        && usage["total_tokens"].as_u64()? == counts.input.checked_add(counts.output)?)
    .then_some(counts)
}

pub fn codex(root: &Path, path: &Path, previous: Option<&State>, now: u64) -> Option<State> {
    if now == 0 || !is_valid_js_safe_integer(now) {
        return None;
    }
    let mut file = transcript::open(root, path)?;
    let metadata = file.metadata().ok()?;
    // A recognized but invalid newest event stops the search. Older valid
    // counters must not masquerade as a fresh accepted observation.
    let (counts, complete) = transcript::latest_complete_at(&mut file, metadata.len(), |line| {
        let entry: Value = match serde_json::from_str(line) {
            Ok(entry) => entry,
            Err(_) => return Some(None),
        };
        (entry["type"] == "event_msg" && entry["payload"]["type"] == "token_count")
            .then(|| codex_counts(&entry))
    })?;
    let counts = counts?;
    let cursor = SourceCursor {
        dev: metadata.dev(),
        ino: metadata.ino(),
        offset: metadata.len(),
        last: None,
        request: None,
        counts: None,
        discard: false,
    };
    let Some(previous) = previous else {
        let mut value = Consumption::baseline(now, false);
        value.set_counts(counts);
        value.complete = complete;
        return Some(State {
            value,
            cursor: Some(cursor),
        });
    };
    if now < previous.value.observed_at_ms {
        return None;
    }
    let old = previous.value.counts();
    let old_cursor = previous.cursor.as_ref()?;
    let replaced = cursor.dev != old_cursor.dev
        || cursor.ino != old_cursor.ino
        || cursor.offset < old_cursor.offset;
    if counts == old && cursor == *old_cursor {
        return Some(previous.clone());
    }
    let decreased = replaced
        || counts.input < old.input
        || counts.output < old.output
        || counts.cached < old.cached;
    let mut value = if decreased {
        Consumption::baseline(now, true)
    } else {
        previous.value.clone()
    };
    value.set_counts(counts);
    if !decreased {
        value.gap = false;
        value.complete = complete;
        value.advance(now)?;
    }
    Some(State {
        value,
        cursor: Some(cursor),
    })
}

#[cfg(test)]
mod tests;
