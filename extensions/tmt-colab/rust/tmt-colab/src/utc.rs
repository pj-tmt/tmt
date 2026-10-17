//! Calendar time in UTC for the text surfaces that print one (the conversation export and the
//! `home summary` rows). Each formats its own way from the parts.

/// An epoch-millisecond time split into UTC calendar parts (proleptic Gregorian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcParts {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: u64,
    pub minute: u64,
    pub second: u64,
}

/// The UTC calendar parts of `ms` milliseconds since 1970-01-01T00:00:00Z (civil-from-days).
pub fn utc_parts(ms: u64) -> UtcParts {
    let days = (ms / 86_400_000) as i64;
    let seconds = (ms / 1000) % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    UtcParts {
        year: year_of_era + era * 400 + i64::from(month <= 2),
        month,
        day,
        hour: seconds / 3600,
        minute: (seconds / 60) % 60,
        second: seconds % 60,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> (i64, i64, i64, u64, u64, u64) {
        let p = utc_parts(ms);
        (p.year, p.month, p.day, p.hour, p.minute, p.second)
    }

    #[test]
    fn calendar_parts_follow_the_proleptic_gregorian_calendar() {
        assert_eq!(at(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(at(1_792_229_400_000), (2026, 10, 17, 9, 30, 0));
        // Leap day of a century year divisible by 400, then the last second before March.
        assert_eq!(at(951_782_400_000 + 59 * 60_000), (2000, 2, 29, 0, 59, 0));
        assert_eq!(at(951_868_799_000), (2000, 2, 29, 23, 59, 59));
        assert_eq!(at(951_868_800_000), (2000, 3, 1, 0, 0, 0));
        // A century year not divisible by 400 has no leap day.
        assert_eq!(at(4_107_542_399_000), (2100, 2, 28, 23, 59, 59));
        assert_eq!(at(4_107_542_400_000), (2100, 3, 1, 0, 0, 0));
    }
}
