//! An instant as the clock text of a zone, shared by every board surface that shows one.

/// An instant in `zone`: the time today, the weekday and date within this year, and the full
/// date in another year, so an old date is never read as a recent one.
pub(in crate::board) fn time_in(
    at_ms: i64,
    now_ms: i64,
    zone: &jiff::tz::TimeZone,
) -> Option<String> {
    let at = jiff::Timestamp::from_millisecond(at_ms)
        .ok()?
        .to_zoned(zone.clone());
    let now = jiff::Timestamp::from_millisecond(now_ms)
        .ok()?
        .to_zoned(zone.clone());
    Some(
        at.strftime(if at.date() == now.date() {
            "%H:%M"
        } else if at.year() == now.year() {
            "%a %m-%d %H:%M"
        } else {
            "%Y-%m-%d %H:%M"
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::time_in;

    /// 2026-10-04T14:30:00Z.
    const NOW: i64 = 1_791_124_200_000;
    const HOUR: i64 = 3_600_000;

    #[test]
    fn today_shows_the_time_another_day_this_year_adds_weekday_and_date() {
        let tokyo = jiff::tz::TimeZone::get("Asia/Tokyo").unwrap();
        assert_eq!(time_in(NOW, NOW, &tokyo).as_deref(), Some("23:30"));
        assert_eq!(
            time_in(NOW + HOUR, NOW, &tokyo).as_deref(),
            Some("Mon 10-05 00:30")
        );
        assert_eq!(
            time_in(NOW + HOUR, NOW, &jiff::tz::TimeZone::UTC).as_deref(),
            Some("15:30")
        );
    }

    #[test]
    fn another_year_shows_the_full_date() {
        let tokyo = jiff::tz::TimeZone::get("Asia/Tokyo").unwrap();
        let year = 366 * 24 * HOUR;
        assert_eq!(
            time_in(NOW + year, NOW, &tokyo).as_deref(),
            Some("2027-10-05 23:30")
        );
        assert_eq!(
            time_in(NOW - year, NOW, &tokyo).as_deref(),
            Some("2025-10-03 23:30")
        );
    }

    #[test]
    fn the_year_is_the_zones_not_utcs() {
        // 2026-12-31T20:00Z is already 2027-01-01 05:00 in Tokyo.
        let tokyo = jiff::tz::TimeZone::get("Asia/Tokyo").unwrap();
        let new_year_eve = 1_798_747_200_000;
        assert_eq!(
            time_in(new_year_eve, NOW, &tokyo).as_deref(),
            Some("2027-01-01 05:00")
        );
        assert_eq!(
            time_in(new_year_eve, NOW, &jiff::tz::TimeZone::UTC).as_deref(),
            Some("Thu 12-31 20:00")
        );
    }

    #[test]
    fn an_unrepresentable_instant_is_none() {
        assert_eq!(time_in(i64::MAX, NOW, &jiff::tz::TimeZone::UTC), None);
    }
}
