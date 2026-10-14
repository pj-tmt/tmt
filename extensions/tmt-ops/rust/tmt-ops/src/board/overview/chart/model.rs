//! Bounded display model for the worker's interval series. No history acquisition.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub each_minutes: u8,
    pub range_minutes: u8,
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
    pub const EACH: [u8; 4] = [5, 15, 30, 60];
    pub const RANGE: [u8; 3] = [15, 30, 60];

    pub fn validate(self) -> Result<(), &'static str> {
        if !Self::EACH.contains(&self.each_minutes) || !Self::RANGE.contains(&self.range_minutes) {
            Err("Unsupported interval or range")
        } else if self.each_minutes > self.range_minutes {
            Err("Interval is longer than the range")
        } else {
            Ok(())
        }
    }

    pub fn bars(self) -> Result<usize, &'static str> {
        self.validate()?;
        Ok(usize::from(self.range_minutes / self.each_minutes))
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
    pub coverage: Coverage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Series {
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
            match (bucket.tokens, bucket.coverage) {
                (None, Coverage::Unknown) | (Some(_), Coverage::Complete | Coverage::Unknown) => (),
                (Some(_), Coverage::Partial { covered_ms }) if covered_ms <= interval as u64 => (),
                _ => return Err("Token reading and source coverage disagree"),
            }
        }
        Ok(Self { selection, buckets })
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
                coverage: Coverage::Complete,
            })
            .collect()
    }

    #[test]
    fn every_pair_is_explicit_and_invalid_change_is_inert() {
        let mut valid = 0;
        for each_minutes in Selection::EACH {
            for range_minutes in Selection::RANGE {
                let selection = Selection {
                    each_minutes,
                    range_minutes,
                };
                if each_minutes <= range_minutes {
                    valid += 1;
                    assert!(Series::new(selection, buckets(selection)).is_ok());
                } else {
                    let mut current = Selection::default();
                    assert_eq!(
                        current.change(selection),
                        Err("Interval is longer than the range")
                    );
                    assert_eq!(current, Selection::default());
                }
            }
        }
        assert_eq!(valid, 9);
        assert_eq!(Selection::default().bars(), Ok(12));
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
