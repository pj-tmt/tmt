//! Quota Details shows source timestamps as local clock times, never as raw epoch milliseconds.
use super::*;
use crate::board::limits::{Provider, Reading, Window};

const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;
/// 2026-10-26T14:05:00Z, a Monday.
const NOW: u64 = 1_793_023_500_000;
const NBSP: &str = "\u{a0}";

fn at(zone: &str) -> jiff::tz::TimeZone {
    jiff::tz::TimeZone::get(zone).unwrap()
}

fn held(zone: &str, observed_ago: u64) -> Vec<String> {
    let standings = vec![Standing {
        current: None,
        rates: Default::default(),
        provider: Provider::named("claude"),
        latest: Some(Reading {
            observed_at_ms: NOW - observed_ago,
            weekly: Window {
                left: 44.0,
                resets_at_ms: NOW + 70 * HOUR,
            },
            short: None,
        }),
        burn: None,
        trend: None,
    }];
    held_lines(Some(&standings), NOW, true, &at(zone))
}

fn observed(zone: &str, observed_ago: u64) -> String {
    held(zone, observed_ago)
        .into_iter()
        .find(|line| line.starts_with("Observed: "))
        .expect("an Observed line")
}

#[test]
fn an_observation_today_shows_its_clock_time_in_the_local_zone() {
    // 13:05Z is 22:05 in Tokyo, the same local day as now (23:05 there).
    assert!(observed("Asia/Tokyo", HOUR).starts_with("Observed: 22:05 · "));
    assert!(
        observed("UTC", HOUR).starts_with("Observed: 13:05 · "),
        "the same instant reads differently in another zone"
    );
}

#[test]
fn the_day_boundary_follows_the_zone_and_not_utc() {
    // 23:05Z on the 25th is the previous day in UTC but 08:05 on the 26th in Tokyo.
    assert!(
        observed("UTC", 15 * HOUR).starts_with(&format!("Observed: Sun{NBSP}10-25{NBSP}23:05 · "))
    );
    assert!(observed("Asia/Tokyo", 15 * HOUR).starts_with("Observed: 08:05 · "));
}

#[test]
fn an_older_day_shows_weekday_and_date_and_another_year_its_full_date() {
    assert!(
        observed("UTC", 3 * DAY).starts_with(&format!("Observed: Fri{NBSP}10-23{NBSP}14:05 · "))
    );
    assert!(observed("UTC", 400 * DAY).starts_with(&format!("Observed: 2025-09-21{NBSP}14:05 · ")));
}

#[test]
fn a_daylight_saving_change_uses_the_zone_offset_at_that_instant() {
    // 2026-03-08T06:59Z is 01:59 EST; 07:30Z is already 03:30 EDT, the same local day.
    let zone = at("America/New_York");
    assert_eq!(stamp(&zone, 1_772_953_140_000, 1_772_955_000_000), "01:59");
    assert_eq!(stamp(&zone, 1_772_955_000_000, 1_772_955_000_000), "03:30");
}

#[test]
fn an_unrepresentable_timestamp_is_a_dash_not_a_number() {
    assert_eq!(stamp(&at("UTC"), u64::MAX, NOW), "—");
}

#[test]
fn no_details_line_keeps_a_raw_epoch_millisecond_number() {
    for zone in ["UTC", "Asia/Tokyo", "America/Los_Angeles"] {
        for line in held(zone, 2 * HOUR) {
            let digits = line.chars().filter(char::is_ascii_digit).count();
            assert!(!line.contains(" ms") && digits < 10, "{zone}: {line}");
        }
    }
}

#[test]
fn a_time_is_one_unbreakable_word_so_a_narrow_details_wraps_before_it() {
    let line = observed("UTC", 3 * DAY);
    for width in [16, 20, 30] {
        let wrapped = crate::board::notes::wrap(&line, width);
        assert!(
            wrapped
                .iter()
                .any(|piece| piece.contains(&format!("Fri{NBSP}10-23{NBSP}14:05"))),
            "{width}: {wrapped:?}"
        );
    }
}

#[test]
fn a_width_below_one_time_still_loses_none_of_it() {
    let line = observed("UTC", 3 * DAY);
    let wrapped = crate::board::notes::wrap(&line, 8).concat();
    assert_eq!(wrapped.replace(' ', ""), line.replace(' ', ""));
}
