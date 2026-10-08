//! Additive evidence in the existing history buckets, never a second ledger.
use super::{Consumption, ModelUsage, StorageError, invalid};
use crate::runtime::consumption::MAX_MODELS;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Details {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_model: Option<Vec<ModelUsage>>,
}

impl Details {
    pub fn valid(&self, input: u64, output: u64, cached: u64) -> bool {
        self.cache_write_tokens.is_none_or(|n| n <= input)
            && self.by_model.as_ref().is_none_or(|rows| {
                rows.len() <= MAX_MODELS
                    && rows.iter().enumerate().all(|(index, row)| {
                        row.valid() && !rows[..index].iter().any(|old| old.model_id == row.model_id)
                    })
                    && rows.iter().try_fold((0u64, 0u64, 0u64), |(a, b, c), row| {
                        Some((
                            a.checked_add(row.input_tokens)?,
                            b.checked_add(row.output_tokens)?,
                            c.checked_add(row.cached_input_tokens)?,
                        ))
                    }) == Some((input, output, cached))
            })
    }
    pub fn delta(old: &Consumption, next: &Consumption) -> Self {
        let input = next.input_tokens - old.input_tokens;
        let output = next.output_tokens - old.output_tokens;
        let cached = next.cached_input_tokens - old.cached_input_tokens;
        let cache_write_tokens = old
            .cache_write_tokens
            .zip(next.cache_write_tokens)
            .and_then(|(a, b)| b.checked_sub(a))
            .filter(|n| *n <= input);
        let by_model = if next.sequence == old.sequence {
            next.delta_by_model.as_ref().map(|_| Vec::new())
        } else {
            next.delta_by_model.clone().filter(|rows| {
                next.sequence == old.sequence + 1
                    && rows.iter().map(|row| row.input_tokens).sum::<u64>() == input
                    && rows.iter().map(|row| row.output_tokens).sum::<u64>() == output
                    && rows.iter().map(|row| row.cached_input_tokens).sum::<u64>() == cached
            })
        };
        Self {
            cache_write_tokens,
            by_model,
        }
    }

    pub fn merge(&mut self, other: &Self) -> Result<(), StorageError> {
        self.cache_write_tokens = self
            .cache_write_tokens
            .zip(other.cache_write_tokens)
            .map(|(a, b)| a.checked_add(b).ok_or_else(invalid))
            .transpose()?;
        match (&mut self.by_model, &other.by_model) {
            (Some(rows), Some(incoming)) => {
                for row in incoming {
                    if let Some(old) = rows.iter_mut().find(|old| old.model_id == row.model_id) {
                        old.merge(row).ok_or_else(invalid)?;
                    } else if rows.len() < MAX_MODELS {
                        rows.push(row.clone());
                    } else {
                        self.by_model = None;
                        break;
                    }
                }
            }
            _ => self.by_model = None,
        }
        Ok(())
    }
}
