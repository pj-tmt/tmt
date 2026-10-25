use jiff::{
    Timestamp,
    civil::{Date, DateTime},
    tz::{AmbiguousOffset, TimeZone},
};
use tmt_core::orchestration::{OrchCalendar, OrchError, OrchSlot};

pub struct OrchZone {
    zone: TimeZone,
    name: String,
}
impl OrchZone {
    pub fn load(name: Option<&str>) -> Result<Self, OrchError> {
        let zone = match name {
            Some(name) => TimeZone::get(name),
            None => TimeZone::try_system(),
        }
        .map_err(|_| OrchError::TimeZone)?;
        let name = zone.iana_name().ok_or(OrchError::TimeZone)?.to_owned();
        Ok(Self { zone, name })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn format_ms(&self, at_ms: u64) -> Result<String, OrchError> {
        let t = Timestamp::from_millisecond(i64::try_from(at_ms).map_err(|_| OrchError::Invalid)?)
            .map_err(|_| OrchError::Invalid)?;
        Ok(t.to_zoned(self.zone.clone()).to_string())
    }
    pub fn parse_at(&self, text: &str) -> Result<u64, OrchError> {
        let timestamp = if text.len() == 16 && text.as_bytes()[10] == b' ' {
            let local: DateTime = text
                .replace(' ', "T")
                .parse()
                .map_err(|_| OrchError::Invalid)?;
            self.zone
                .to_ambiguous_timestamp(local)
                .unambiguous()
                .map_err(|_| OrchError::Invalid)?
        } else {
            text.parse::<Timestamp>().map_err(|_| OrchError::Invalid)?
        };
        millis(timestamp)
    }
    pub fn range(&self, at_ms: u64, days: u8) -> Result<OrchSlot, OrchError> {
        if !matches!(days, 1 | 7 | 30) {
            return Err(OrchError::Invalid);
        }
        let day = self.day_at(at_ms)?;
        Ok(OrchSlot {
            start_ms: self.boundary(day, 0, false)?,
            end_ms: self.boundary(day + i32::from(days), 0, true)?,
        })
    }
}
fn millis(timestamp: Timestamp) -> Result<u64, OrchError> {
    u64::try_from(timestamp.as_millisecond())
        .ok()
        .filter(|v| *v > 0 && *v <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
        .ok_or(OrchError::Invalid)
}
fn julian(date: Date) -> Result<i32, OrchError> {
    time::Date::from_calendar_date(
        i32::from(date.year()),
        time::Month::try_from(date.month() as u8).map_err(|_| OrchError::Invalid)?,
        date.day() as u8,
    )
    .map(|d| d.to_julian_day())
    .map_err(|_| OrchError::Invalid)
}
fn civil(day: i32) -> Result<Date, OrchError> {
    let d = time::Date::from_julian_day(day).map_err(|_| OrchError::Invalid)?;
    Date::new(
        d.year().try_into().map_err(|_| OrchError::Invalid)?,
        d.month() as i8,
        d.day() as i8,
    )
    .map_err(|_| OrchError::Invalid)
}
pub(super) fn parse_date(text: &str) -> Result<i32, OrchError> {
    if text.len() != 10 {
        return Err(OrchError::Invalid);
    }
    julian(text.parse::<Date>().map_err(|_| OrchError::Invalid)?)
}
pub(super) fn date_text(day: i32) -> Result<String, OrchError> {
    civil(day).map(|d| d.to_string())
}
impl OrchCalendar for OrchZone {
    fn day_at(&self, at_ms: u64) -> Result<i32, OrchError> {
        let t = Timestamp::from_millisecond(i64::try_from(at_ms).map_err(|_| OrchError::Invalid)?)
            .map_err(|_| OrchError::Invalid)?;
        julian(t.to_zoned(self.zone.clone()).date())
    }
    fn boundary(&self, day: i32, minute: u16, end: bool) -> Result<u64, OrchError> {
        if minute >= 1440 {
            return Err(OrchError::Invalid);
        }
        let local = civil(day)?.at((minute / 60) as i8, (minute % 60) as i8, 0, 0);
        let ambiguous = self.zone.to_ambiguous_timestamp(local);
        let timestamp = match ambiguous.offset() {
            AmbiguousOffset::Gap { .. } => {
                let earlier = ambiguous.earlier().map_err(|_| OrchError::TimeZone)?;
                // The transition itself is the first valid instant after the gap,
                // including non-hour and skipped-date transitions.
                self.zone
                    .following(earlier)
                    .next()
                    .ok_or(OrchError::TimeZone)?
                    .timestamp()
            }
            AmbiguousOffset::Fold { .. } if end => {
                ambiguous.later().map_err(|_| OrchError::TimeZone)?
            }
            _ => ambiguous.earlier().map_err(|_| OrchError::TimeZone)?,
        };
        millis(timestamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gap_clamps_and_fold_encloses_both_occurrences() {
        let z = OrchZone::load(Some("America/New_York")).unwrap();
        let gap = parse_date("2026-03-08").unwrap();
        assert_eq!(
            z.boundary(gap, 150, false).unwrap(),
            z.parse_at("2026-03-08T07:00:00Z").unwrap()
        );
        let fold = parse_date("2026-11-01").unwrap();
        assert_eq!(
            z.boundary(fold, 90, false).unwrap(),
            z.parse_at("2026-11-01T05:30:00Z").unwrap()
        );
        assert_eq!(
            z.boundary(fold, 90, true).unwrap(),
            z.parse_at("2026-11-01T06:30:00Z").unwrap()
        );
        assert!(z.parse_at("2026-03-08 02:30").is_err());
        assert!(z.parse_at("2026-11-01 01:30").is_err());
    }
    #[test]
    fn local_days_and_non_hour_gap_are_not_fixed_durations() {
        let z = OrchZone::load(Some("Australia/Lord_Howe")).unwrap();
        let d = parse_date("2026-10-04").unwrap();
        assert_eq!(
            z.boundary(d, 135, false).unwrap(),
            z.parse_at("2026-10-03T15:30:00Z").unwrap()
        );
        let range = z
            .range(z.parse_at("2026-10-04T00:00:00+10:30").unwrap(), 1)
            .unwrap();
        assert_eq!(range.end_ms - range.start_ms, 23 * 3_600_000 + 1_800_000);
        assert!(OrchZone::load(Some("not/a-zone")).is_err());
    }
}
