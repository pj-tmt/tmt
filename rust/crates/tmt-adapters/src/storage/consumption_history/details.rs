//! Additive evidence in the existing history buckets, never a second ledger.
use super::{Consumption, ModelUsage, StorageError, invalid};
use crate::runtime::consumption::MAX_MODELS;
use serde::{Deserialize, Serialize};
use tmt_core::{binding::session::HarnessId, limits::MAX_JS_SAFE_INTEGER};

/// The most drivers one bucket attributes; a wider aggregate omits `byDriver`.
pub(super) const MAX_DRIVERS: usize = 4;

/// One driver's share of a bucket's deltas, counted between two reads of that driver.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DriverUsage {
    pub driver: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
}

impl DriverUsage {
    fn valid(&self) -> bool {
        HarnessId::new(&self.driver).is_ok()
            && self.cached_input_tokens <= self.input_tokens
            && self
                .input_tokens
                .checked_add(self.output_tokens)
                .is_some_and(|total| total <= MAX_JS_SAFE_INTEGER)
    }
    fn merge(&mut self, other: &Self) -> Result<(), StorageError> {
        self.input_tokens = self
            .input_tokens
            .checked_add(other.input_tokens)
            .ok_or_else(invalid)?;
        self.output_tokens = self
            .output_tokens
            .checked_add(other.output_tokens)
            .ok_or_else(invalid)?;
        self.cached_input_tokens = self
            .cached_input_tokens
            .checked_add(other.cached_input_tokens)
            .ok_or_else(invalid)?;
        Ok(())
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Details {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    /// Its rows sum to at most the bucket's tokens; the rest is unattributed. An
    /// empty list says the same as none, so it is never written out.
    #[serde(default, skip_serializing_if = "unattributed")]
    pub by_model: Option<Vec<ModelUsage>>,
    /// Ascending by driver. Its rows sum to at most the bucket's tokens; the rest
    /// (rows counted before drivers were recorded) is unattributed, never a guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_driver: Option<Vec<DriverUsage>>,
}

fn unattributed(rows: &Option<Vec<ModelUsage>>) -> bool {
    rows.as_ref().is_none_or(Vec::is_empty)
}

impl Details {
    pub fn valid(&self, input: u64, output: u64, cached: u64) -> bool {
        self.cache_write_tokens.is_none_or(|n| n <= input)
            && self.by_model.as_ref().is_none_or(|rows| {
                rows.len() <= MAX_MODELS
                    && rows.iter().enumerate().all(|(index, row)| {
                        row.valid() && !rows[..index].iter().any(|old| old.model_id == row.model_id)
                    })
                    && rows
                        .iter()
                        .try_fold((0u64, 0u64, 0u64), |(a, b, c), row| {
                            Some((
                                a.checked_add(row.input_tokens)?,
                                b.checked_add(row.output_tokens)?,
                                c.checked_add(row.cached_input_tokens)?,
                            ))
                        })
                        .is_some_and(|(a, b, c)| a <= input && b <= output && c <= cached)
            })
            && self.by_driver.as_ref().is_none_or(|rows| {
                (1..=MAX_DRIVERS).contains(&rows.len())
                    && rows.iter().enumerate().all(|(index, row)| {
                        row.valid() && rows[..index].iter().all(|old| old.driver < row.driver)
                    })
                    && rows
                        .iter()
                        .try_fold((0u64, 0u64, 0u64), |(a, b, c), row| {
                            Some((
                                a.checked_add(row.input_tokens)?,
                                b.checked_add(row.output_tokens)?,
                                c.checked_add(row.cached_input_tokens)?,
                            ))
                        })
                        .is_some_and(|(a, b, c)| a <= input && b <= output && c <= cached)
            })
    }
    pub fn delta(old: &Consumption, next: &Consumption, driver: &str) -> Self {
        let input = next.input_tokens - old.input_tokens;
        let output = next.output_tokens - old.output_tokens;
        let cached = next.cached_input_tokens - old.cached_input_tokens;
        let cache_write_tokens = old
            .cache_write_tokens
            .zip(next.cache_write_tokens)
            .and_then(|(a, b)| b.checked_sub(a))
            .filter(|n| *n <= input);
        // `delta_by_model` groups the increment since the preceding sequence, so after
        // a skipped sequence it covers only part of the delta: the rest stays
        // unattributed. A repeated sequence counts nothing and must not be applied twice.
        let by_model = next.delta_by_model.clone().filter(|rows| {
            next.sequence != old.sequence
                && !rows.is_empty()
                && rows.iter().map(|row| row.input_tokens).sum::<u64>() <= input
                && rows.iter().map(|row| row.output_tokens).sum::<u64>() <= output
                && rows.iter().map(|row| row.cached_input_tokens).sum::<u64>() <= cached
        });
        // Counted between two reads of one driver, so the whole delta is its own. A
        // driver id that is not a plain id (never expected) is left unattributed.
        let by_driver = (input != 0 || output != 0)
            .then(|| DriverUsage {
                driver: driver.to_owned(),
                input_tokens: input,
                output_tokens: output,
                cached_input_tokens: cached,
            })
            .filter(DriverUsage::valid)
            .map(|row| vec![row]);
        Self {
            cache_write_tokens,
            by_model,
            by_driver,
        }
    }

    pub fn merge(&mut self, other: &Self) -> Result<(), StorageError> {
        self.cache_write_tokens = self
            .cache_write_tokens
            .zip(other.cache_write_tokens)
            .map(|(a, b)| a.checked_add(b).ok_or_else(invalid))
            .transpose()?;
        // A side without models only leaves its share unattributed; a fifth model
        // drops the split built so far.
        if let Some(incoming) = other.by_model.as_ref().filter(|rows| !rows.is_empty()) {
            let rows = self.by_model.get_or_insert_with(Vec::new);
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
        // Unlike models, a side without drivers only leaves its share unattributed.
        if let Some(incoming) = &other.by_driver {
            let rows = self.by_driver.get_or_insert_with(Vec::new);
            for row in incoming {
                match rows.binary_search_by(|old| old.driver.cmp(&row.driver)) {
                    Ok(index) => rows[index].merge(row)?,
                    Err(index) if rows.len() < MAX_DRIVERS => rows.insert(index, row.clone()),
                    Err(_) => {
                        self.by_driver = None;
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}
