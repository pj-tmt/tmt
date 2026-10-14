//! Pure Gregorian dates shared by quota reset arithmetic and sign-in copy.
//! Days are relative to 1970-01-01; callers supply time, never this module.
pub(crate) struct CalendarDate {
    pub year: i64,
    pub month: i64,
    pub day: i64,
}
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
pub(crate) fn civil_from_days(days: i64) -> CalendarDate {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    CalendarDate {
        year: yoe + era * 400 + i64::from(month <= 2),
        month,
        day: doy - (153 * mp + 2) / 5 + 1,
    }
}
pub(crate) fn utc_date(milliseconds: u64) -> String {
    let date = civil_from_days((milliseconds / 86_400_000) as i64);
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_epoch_and_leap_day_dates_are_exact() {
        for (milliseconds, expected) in [
            (0, "1970-01-01"),
            (951_782_400_000, "2000-02-29"),
            (1_794_441_600_000, "2026-11-12"),
            (4_107_542_400_000, "2100-03-01"),
        ] {
            assert_eq!(utc_date(milliseconds), expected);
        }
    }
}
