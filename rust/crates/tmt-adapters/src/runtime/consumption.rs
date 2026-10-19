//! Driver-owned completed-request counters. Claude tracks appended records,
//! not context growth; Codex reports cumulative counters itself. The source
//! formats are unofficial, and evidence loss establishes a new baseline.
use super::transcript;
mod attribution;
mod credits;
pub mod family;
mod rate_limits;
pub use attribution::ModelUsage;
pub(crate) use attribution::valid_service_tier;
pub use credits::Credits;
pub use rate_limits::{RateLimits, Window as RateWindow};
pub(crate) const MAX_MODELS: usize = 4;
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
pub struct Counts {
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

    /// What `self` has beyond `other`, or None when any counter is lower.
    fn sub(self, other: Self) -> Option<Self> {
        let next = Self {
            input: self.input.checked_sub(other.input)?,
            output: self.output.checked_sub(other.output)?,
            cached: self.cached.checked_sub(other.cached)?,
        };
        next.valid().then_some(next)
    }

    /// What `self` gained over an earlier reading of the same cumulative
    /// counters: each counter's rise, with cached input kept inside input.
    fn gained(self, earlier: Self) -> Self {
        let input = self.input.saturating_sub(earlier.input);
        Self {
            input,
            output: self.output.saturating_sub(earlier.output),
            cached: self.cached.saturating_sub(earlier.cached).min(input),
        }
    }

    /// No counter is lower than `other`'s.
    fn covers(self, other: Self) -> bool {
        self.input >= other.input && self.output >= other.output && self.cached >= other.cached
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Normalized increments since the preceding sequence, grouped by the
    /// model actually recorded for each accepted request/turn, not launch model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_by_model: Option<Vec<ModelUsage>>,
    /// The account's rate-limit windows on the newest accepted source line.
    /// Absent when the driver has no source or the line carries none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<RateLimits>,
    /// The Codex credit balance on the newest accepted source line. Absent when
    /// the line carries none or any field is invalid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<Credits>,
    /// The provider's service tier id (`default`, `priority`) from the newest
    /// settings event in the bounded tail. Absent means unknown, never `default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
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
            cache_write_tokens: None,
            model_id: None,
            delta_by_model: None,
            rate_limits: None,
            credits: None,
            service_tier: None,
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

    pub(crate) fn valid(&self) -> bool {
        self.counts().valid()
            && self
                .cache_write_tokens
                .is_none_or(|n| n <= self.input_tokens)
            && self
                .model_id
                .as_deref()
                .is_none_or(attribution::valid_model)
            && self
                .delta_by_model
                .as_ref()
                .is_none_or(|rows| attribution::valid(rows, self.counts()))
            && self.rate_limits.as_ref().is_none_or(RateLimits::valid)
            && self.credits.as_ref().is_none_or(Credits::valid)
            && self
                .service_tier
                .as_deref()
                .is_none_or(attribution::valid_service_tier)
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

/// What the sessions a Codex session started have contributed to `value`'s
/// counters (see [`family`]), carried unchanged by reads that do not look at them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Carry {
    input: u64,
    output: u64,
    cached: u64,
    /// The family was still being discovered or followed when this was taken.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    incomplete: bool,
}

impl Carry {
    fn counts(self) -> Counts {
        Counts {
            input: self.input,
            output: self.output,
            cached: self.cached,
        }
    }
    /// Nothing contributed and nothing missing is the absent form.
    fn of(counts: Counts, incomplete: bool) -> Option<Self> {
        (counts != Counts::default() || incomplete).then_some(Self {
            input: counts.input,
            output: counts.output,
            cached: counts.cached,
            incomplete,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub value: Consumption,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<SourceCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    family: Option<Carry>,
}

impl State {
    pub fn read(value: &Value) -> Option<Self> {
        let mut value = value.clone();
        if let Some(rows) = value
            .pointer_mut("/value/deltaByModel")
            .and_then(Value::as_array_mut)
        {
            // The opaque cursor stores tuples to stay under its existing cap;
            // public projections always use named fields.
            for row in rows {
                if let Value::Array(tuple) = row {
                    if tuple.len() != 5 {
                        return None;
                    }
                    *row = json!({"modelId":tuple[0],"inputTokens":tuple[1],"outputTokens":tuple[2],"cachedInputTokens":tuple[3],"cacheWriteTokens":tuple[4]});
                }
            }
        }
        let state: Self = serde_json::from_value(value).ok()?;
        let hash = |hash: &str| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        (state.value.valid()
            && state.family.is_none_or(|carry| {
                carry.counts().valid() && state.value.counts().covers(carry.counts())
            })
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
        let mut value = json!(self);
        if let Some(rows) = self.value.delta_by_model.as_ref() {
            value["value"]["deltaByModel"] = json!(
                rows.iter()
                    .map(|row| json!([
                        row.model_id,
                        row.input_tokens,
                        row.output_tokens,
                        row.cached_input_tokens,
                        row.cache_write_tokens
                    ]))
                    .collect::<Vec<_>>()
            );
        }
        value
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
    cache_write: Option<u64>,
    model: Option<String>,
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
        cache_write: usage["cache_creation_input_tokens"]
            .as_u64()
            .filter(|n| *n <= counts.input),
        model: attribution::model(&message["model"]),
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
        family: None,
    };
    let Some(previous) = previous else {
        if end == 0 {
            let mut next = baseline(false);
            next.value.cache_write_tokens = Some(0);
            next.value.delta_by_model = Some(Vec::new());
            return Some(next);
        }
        // Absorb the last historical group if later content-block records
        // for it are appended. IDs absent from legacy fixtures mean no counter.
        let message =
            transcript::latest_at(&mut file, end, |line| claude_message(line).ok().flatten())?;
        let mut next = baseline(false);
        next.value.complete = boundary;
        next.value.cache_write_tokens = Some(0);
        next.value.model_id = message.model;
        next.value.delta_by_model = Some(Vec::new());
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
    if cursor.dev != metadata.dev()
        || cursor.ino != metadata.ino()
        || end < cursor.offset
        || end - cursor.offset > transcript::TAIL_LIMIT
    {
        return Some(baseline(true));
    }
    if end == cursor.offset {
        return Some(previous.clone());
    }
    file.seek(SeekFrom::Start(cursor.offset)).ok()?;
    let mut reader = BufReader::with_capacity(8192, file.take(end - cursor.offset));
    // Only the bounded appended tail is eligible. Backlog outside that tail
    // establishes a gap above, without replaying the old prefix.
    let mut raw = Vec::with_capacity(transcript::TAIL_LIMIT as usize);
    let mut next = previous.clone();
    next.value.delta_by_model = Some(Vec::new());
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
        next.value.cache_write_tokens = next
            .value
            .cache_write_tokens
            .zip(message.cache_write)
            .and_then(|(a, b)| a.checked_add(b));
        next.value.model_id = message.model.clone();
        attribution::add(
            &mut next.value.delta_by_model,
            message.model,
            message.counts,
            message.cache_write,
        );
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
    codex_with(root, path, previous, None, now)
}

/// The session's own rollout, plus what the sessions it started contributed
/// (`family`, from [`family::advance`]). Without a `family` the previous
/// contribution is carried unchanged, so a read that does not look at the
/// started sessions never moves the totals back.
pub fn codex_with(
    root: &Path,
    path: &Path,
    previous: Option<&State>,
    family: Option<&family::Contribution>,
    now: u64,
) -> Option<State> {
    if now == 0 || !is_valid_js_safe_integer(now) {
        return None;
    }
    let mut file = transcript::open(root, path)?;
    let metadata = file.metadata().ok()?;
    // A recognized but invalid newest event stops the search. Older valid
    // counters must not masquerade as a fresh accepted observation.
    let (latest, complete) = transcript::latest_complete_at(&mut file, metadata.len(), |line| {
        let entry: Value = match serde_json::from_str(line) {
            Ok(entry) => entry,
            Err(_) => return Some(None),
        };
        (entry["type"] == "event_msg" && entry["payload"]["type"] == "token_count").then(|| {
            // Invalid limits only omit themselves; they never invalidate counters.
            codex_counts(&entry)
                .map(|counts| (counts, rate_limits::codex(&entry), credits::codex(&entry)))
        })
    })?;
    let (counts, rate_limits, credits) = latest?;
    let carried = previous.and_then(|state| state.family);
    let (contribution, family_incomplete) = match family {
        Some(family) => (family.counts, !family.complete),
        None => carried.map_or((Counts::default(), false), |carry| {
            (carry.counts(), carry.incomplete)
        }),
    };
    let total = counts.add(contribution)?;
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
        value.set_counts(total);
        value.rate_limits = rate_limits;
        value.credits = credits;
        value.complete = complete && !family_incomplete;
        if let Some(details) = attribution::codex(&mut file, metadata.len(), None) {
            value.cache_write_tokens = details.0;
            value.model_id = details.1;
            value.service_tier = details.3;
        }
        return Some(State {
            value,
            cursor: Some(cursor),
            family: Carry::of(contribution, family_incomplete),
        });
    };
    if now < previous.value.observed_at_ms {
        return None;
    }
    let carried_counts = carried.map_or(Counts::default(), Carry::counts);
    let carried_incomplete = carried.is_some_and(|carry| carry.incomplete);
    // The session's own counters, without what its family added to the total.
    let own = previous.value.counts().sub(carried_counts);
    let old_cursor = previous.cursor.as_ref()?;
    let replaced = cursor.dev != old_cursor.dev
        || cursor.ino != old_cursor.ino
        || cursor.offset < old_cursor.offset;
    if own == Some(counts)
        && cursor == *old_cursor
        && contribution == carried_counts
        && family_incomplete == carried_incomplete
    {
        return Some(previous.clone());
    }
    let own_before = State {
        value: Consumption {
            input_tokens: own.map_or(0, |own| own.input),
            output_tokens: own.map_or(0, |own| own.output),
            cached_input_tokens: own.map_or(0, |own| own.cached),
            ..previous.value.clone()
        },
        cursor: previous.cursor.clone(),
        family: None,
    };
    let details =
        own.and_then(|_| attribution::codex(&mut file, metadata.len(), Some(&own_before)));
    // A family that lost ground is lost evidence, like a counter that did.
    let decreased = replaced
        || own.is_none_or(|own| !counts.covers(own))
        || !contribution.covers(carried_counts);
    let mut value = if decreased {
        Consumption::baseline(now, true)
    } else {
        previous.value.clone()
    };
    value.set_counts(total);
    value.rate_limits = rate_limits;
    value.credits = credits;
    value.cache_write_tokens = details.as_ref().and_then(|details| details.0);
    value.model_id = details.as_ref().and_then(|details| details.1.clone());
    value.service_tier = details.as_ref().and_then(|details| details.3.clone());
    value.delta_by_model = if decreased {
        None
    } else {
        family::merge_rows(
            details.and_then(|details| details.2),
            family.and_then(|family| family.rows.clone()),
        )
    };
    if !decreased {
        value.gap = false;
        value.complete = complete && !family_incomplete;
        value.advance(now)?;
    }
    Some(State {
        value,
        cursor: Some(cursor),
        family: Carry::of(contribution, family_incomplete),
    })
}

#[cfg(test)]
mod tests;
