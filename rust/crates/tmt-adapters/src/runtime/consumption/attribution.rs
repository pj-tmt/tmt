//! Optional billing evidence. Missing attribution never invalidates legacy totals.
use super::{Counts, MAX_MODELS, State, codex_counts, fingerprint, transcript};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsage {
    pub model_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
}

impl ModelUsage {
    pub(crate) fn valid(&self) -> bool {
        valid_model(&self.model_id)
            && Counts {
                input: self.input_tokens,
                output: self.output_tokens,
                cached: self.cached_input_tokens,
            }
            .valid()
            && self
                .cache_write_tokens
                .is_none_or(|n| n <= self.input_tokens)
    }
    pub(crate) fn merge(&mut self, other: &Self) -> Option<()> {
        self.input_tokens = self.input_tokens.checked_add(other.input_tokens)?;
        self.output_tokens = self.output_tokens.checked_add(other.output_tokens)?;
        self.cached_input_tokens = self
            .cached_input_tokens
            .checked_add(other.cached_input_tokens)?;
        self.cache_write_tokens = self
            .cache_write_tokens
            .zip(other.cache_write_tokens)
            .and_then(|(a, b)| a.checked_add(b));
        Some(())
    }
}

pub(super) use crate::runtime::driver_state::valid_model;

pub(super) fn model(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|model| valid_model(model))
        .map(str::to_owned)
}

pub(super) fn valid(rows: &[ModelUsage], total: Counts) -> bool {
    if rows.len() > MAX_MODELS {
        return false;
    }
    let mut sum = Counts::default();
    for (index, row) in rows.iter().enumerate() {
        let counts = Counts {
            input: row.input_tokens,
            output: row.output_tokens,
            cached: row.cached_input_tokens,
        };
        if !valid_model(&row.model_id)
            || rows[..index].iter().any(|old| old.model_id == row.model_id)
            || !counts.valid()
            || row.cache_write_tokens.is_some_and(|n| n > counts.input)
        {
            return false;
        }
        let Some(next) = sum.add(counts) else {
            return false;
        };
        sum = next;
    }
    sum.input <= total.input && sum.output <= total.output && sum.cached <= total.cached
}

pub(super) fn add(
    rows: &mut Option<Vec<ModelUsage>>,
    model: Option<String>,
    counts: Counts,
    write: Option<u64>,
) {
    let Some(model) = model else {
        *rows = None;
        return;
    };
    let Some(entries) = rows.as_mut() else {
        return;
    };
    if let Some(row) = entries.iter_mut().find(|row| row.model_id == model) {
        let incoming = ModelUsage {
            model_id: model,
            input_tokens: counts.input,
            output_tokens: counts.output,
            cached_input_tokens: counts.cached,
            cache_write_tokens: write,
        };
        if row.merge(&incoming).is_none() {
            *rows = None;
        }
    } else if entries.len() < MAX_MODELS {
        entries.push(ModelUsage {
            model_id: model,
            input_tokens: counts.input,
            output_tokens: counts.output,
            cached_input_tokens: counts.cached,
            cache_write_tokens: write,
        });
    } else {
        *rows = None;
    }
}

type CodexEvidence = (Option<u64>, Option<String>, Option<Vec<ModelUsage>>);

/// TurnContextItem.model is the effective turn model. Rate-limit model aliases
/// and last_token_usage are not substitutes for its cumulative counter delta.
pub(super) fn codex(
    file: &mut std::fs::File,
    end: u64,
    previous: Option<&State>,
) -> Option<CodexEvidence> {
    let tail = transcript::read_tail(file, end)?;
    let start = end - tail.len() as u64;
    let mut offset = start;
    let mut model_id = None;
    let mut context_turn = None;
    let mut newest_model = None;
    let mut write = None;
    let mut counts = previous.map(|state| state.value.counts());
    let mut old_write = previous.and_then(|state| state.value.cache_write_tokens);
    let mut rows = previous
        .filter(|state| {
            state
                .cursor
                .as_ref()
                .is_some_and(|cursor| cursor.offset >= start)
        })
        .map(|_| Vec::new());
    for raw in tail.split_inclusive(|byte| *byte == b'\n') {
        offset += raw.len() as u64;
        if raw.last() != Some(&b'\n') {
            break;
        }
        let entry: Value = serde_json::from_slice(raw).ok()?;
        if entry["type"] == "turn_context" {
            model_id = model(&entry["payload"]["model"]);
            context_turn = entry["payload"]["turn_id"].as_str().and_then(fingerprint);
        } else if entry["type"] == "event_msg"
            && matches!(
                entry["payload"]["type"].as_str(),
                Some("task_started" | "turn_started")
            )
        {
            // The marker may follow a context for this exact turn, or precede
            // its new context. Never inherit an older turn's model when that
            // new context is absent. Provider IDs remain transient hashes.
            let turn = entry["payload"]["turn_id"].as_str().and_then(fingerprint);
            if turn.is_none() || turn != context_turn {
                model_id = None;
                context_turn = None;
            }
        } else if entry["type"] == "event_msg" && entry["payload"]["type"] == "token_count" {
            let next = codex_counts(&entry)?;
            write = entry["payload"]["info"]["total_token_usage"]["cache_write_input_tokens"]
                .as_u64()
                .filter(|n| *n <= next.input);
            newest_model = model_id.clone();
            if previous.is_some_and(|state| {
                offset > state.cursor.as_ref().map_or(end, |cursor| cursor.offset)
            }) {
                if let Some(old) = counts {
                    let delta = next
                        .input
                        .checked_sub(old.input)
                        .zip(next.output.checked_sub(old.output))
                        .zip(next.cached.checked_sub(old.cached));
                    if let Some(((input, output), cached)) =
                        delta.filter(|((input, _), cached)| cached <= input)
                    {
                        if input != 0 || output != 0 {
                            add(
                                &mut rows,
                                model_id.clone(),
                                Counts {
                                    input,
                                    output,
                                    cached,
                                },
                                write
                                    .zip(old_write)
                                    .and_then(|(a, b)| a.checked_sub(b))
                                    .filter(|n| *n <= input),
                            );
                        }
                    } else {
                        rows = None;
                    }
                } else {
                    rows = None;
                }
                counts = Some(next);
                old_write = write;
            }
        }
    }
    Some((write, newest_model, rows))
}
