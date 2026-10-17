//! Bounded display model for the worker's interval series. No history acquisition.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub each_minutes: u32,
    pub range_minutes: u32,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            each_minutes: 5,
            range_minutes: 60,
        }
    }
}

impl Selection {
    pub const EACH: [u32; 4] = [5, 15, 30, 60];
    pub const RANGE: [u32; 5] = [60, 360, 1_440, 10_080, 43_200];

    pub fn each_choices(range_minutes: u32) -> &'static [u32] {
        match range_minutes {
            60 => &Self::EACH,
            360 => &[15, 30, 60],
            1_440 => &[60, 120],
            10_080 => &[360, 720, 1_440],
            43_200 => &[1_440, 2_880],
            _ => &[],
        }
    }

    /// A range transition visibly selects its first interval when the old one
    /// is not offered there. Direct invalid selections still leave state intact.
    pub fn with_range(self, range_minutes: u32) -> Result<Self, &'static str> {
        let choices = Self::each_choices(range_minutes);
        let first = *choices.first().ok_or("Unsupported interval or range")?;
        Ok(Self {
            each_minutes: if choices.contains(&self.each_minutes) {
                self.each_minutes
            } else {
                first
            },
            range_minutes,
        })
    }

    pub fn validate(self) -> Result<(), &'static str> {
        if !Self::RANGE.contains(&self.range_minutes) {
            Err("Unsupported interval or range")
        } else if self.each_minutes > self.range_minutes {
            Err("Interval is longer than the range")
        } else if !Self::each_choices(self.range_minutes).contains(&self.each_minutes) {
            Err("Unsupported interval or range")
        } else {
            Ok(())
        }
    }

    pub fn bars(self) -> Result<usize, &'static str> {
        self.validate()?;
        Ok((self.range_minutes / self.each_minutes) as usize)
    }

    /// Refusal preserves both controls. It never silently changes aggregation.
    pub fn change(&mut self, next: Self) -> Result<(), &'static str> {
        next.validate()?;
        *self = next;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    Complete,
    /// Source-backed duration only; a sample/evidence count is not a duration.
    Partial {
        covered_ms: u64,
    },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bucket {
    pub from_ms: i64,
    pub to_ms: i64,
    pub tokens: Option<u64>,
    pub providers: std::collections::BTreeMap<String, u64>,
    pub coverage: Coverage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Series {
    pub drivers: std::collections::BTreeSet<String>,
    pub totals: Option<(u128, std::collections::BTreeMap<String, u128>)>,
    pub selection: Selection,
    pub buckets: Vec<Bucket>,
}

impl Series {
    /// Admit exactly the selected interval grid. The worker remains the single
    /// projection owner; this model does not rebucket samples or manufacture gaps.
    pub fn new(selection: Selection, buckets: Vec<Bucket>) -> Result<Self, &'static str> {
        if buckets.len() != selection.bars()? {
            return Err("Interval count does not match the range");
        }
        let interval = i64::from(selection.each_minutes) * 60_000;
        for (index, bucket) in buckets.iter().enumerate() {
            if bucket.to_ms.checked_sub(bucket.from_ms) != Some(interval) {
                return Err("Interval duration does not match the selection");
            }
            if index > 0 && buckets[index - 1].to_ms != bucket.from_ms {
                return Err("Interval grid is not contiguous");
            }
            if bucket
                .providers
                .values()
                .map(|n| u128::from(*n))
                .sum::<u128>()
                > u128::from(bucket.tokens.unwrap_or(0))
            {
                return Err("Provider portions exceed the observed token total");
            }
            match (bucket.tokens, bucket.coverage) {
                (None, Coverage::Unknown) | (Some(_), Coverage::Complete | Coverage::Unknown) => (),
                (Some(_), Coverage::Partial { covered_ms }) if covered_ms <= interval as u64 => (),
                _ => return Err("Token reading and source coverage disagree"),
            }
        }
        let drivers = buckets
            .iter()
            .flat_map(|bucket| bucket.providers.keys().cloned())
            .collect();
        let mut amounts = std::collections::BTreeMap::new();
        let mut total = None;
        for bucket in &buckets {
            if let Some(tokens) = bucket.tokens {
                *total.get_or_insert(0u128) += u128::from(tokens);
            }
            for (driver, tokens) in &bucket.providers {
                *amounts.entry(driver.clone()).or_insert(0u128) += u128::from(*tokens);
            }
        }
        let totals = total.map(|tokens| (tokens, amounts));
        Ok(Self {
            drivers,
            totals,
            selection,
            buckets,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buckets(selection: Selection) -> Vec<Bucket> {
        let interval = i64::from(selection.each_minutes) * 60_000;
        (0..selection.bars().unwrap())
            .map(|index| Bucket {
                from_ms: index as i64 * interval,
                to_ms: (index as i64 + 1) * interval,
                tokens: Some(index as u64),
                providers: Default::default(),
                coverage: Coverage::Complete,
            })
            .collect()
    }

    #[test]
    fn every_pair_is_explicit_and_invalid_change_is_inert() {
        let mut valid = 0;
        for range_minutes in Selection::RANGE {
            for &each_minutes in Selection::each_choices(range_minutes) {
                let selection = Selection {
                    each_minutes,
                    range_minutes,
                };
                valid += 1;
                assert!(Series::new(selection, buckets(selection)).is_ok());
            }
        }
        assert_eq!(valid, 14);
        assert_eq!(Selection::default().bars(), Ok(12));
        let mut current = Selection::default();
        assert_eq!(
            current.change(Selection {
                each_minutes: 120,
                range_minutes: 60
            }),
            Err("Interval is longer than the range")
        );
        assert_eq!(current, Selection::default());
        assert_eq!(
            current.change(Selection {
                each_minutes: 5,
                range_minutes: 360
            }),
            Err("Unsupported interval or range")
        );
        assert_eq!(current, Selection::default());
    }

    #[test]
    fn range_transition_keeps_a_compatible_interval_or_selects_its_visible_default() {
        let mut selection = Selection::default();
        for (range, each, bars) in [
            (360, 15, 24),
            (1_440, 60, 24),
            (10_080, 360, 28),
            (43_200, 1_440, 30),
            (60, 5, 12),
        ] {
            selection = selection.with_range(range).unwrap();
            assert_eq!(selection.each_minutes, each);
            assert_eq!(selection.bars(), Ok(bars));
        }
        let hourly = Selection {
            each_minutes: 60,
            range_minutes: 60,
        };
        assert_eq!(hourly.with_range(360).unwrap().each_minutes, 60);
        assert!(hourly.with_range(15).is_err());
    }

    #[test]
    fn exact_values_survive_and_missing_is_not_zero() {
        let selection = Selection::default();
        let mut values = buckets(selection);
        values[0].tokens = None;
        values[0].coverage = Coverage::Unknown;
        values[1].tokens = Some(0);
        values[1].coverage = Coverage::Partial {
            covered_ms: 120_000,
        };
        let admitted = Series::new(selection, values.clone()).unwrap();
        assert_eq!(admitted.buckets, values);
        values[1].coverage = Coverage::Partial {
            covered_ms: 300_001,
        };
        assert!(Series::new(selection, values).is_err());
    }

    #[test]
    fn rejects_extra_gaps_bad_duration_and_unsupported_controls() {
        let selection = Selection::default();
        let mut values = buckets(selection);
        values.pop();
        assert!(Series::new(selection, values).is_err());
        let mut values = buckets(selection);
        values[1].from_ms += 1;
        assert!(Series::new(selection, values).is_err());
        let mut values = buckets(selection);
        values[1].from_ms += 1;
        values[1].to_ms += 1;
        assert!(Series::new(selection, values).is_err());
        assert!(
            Selection {
                each_minutes: 10,
                range_minutes: 60
            }
            .validate()
            .is_err()
        );
    }
}
