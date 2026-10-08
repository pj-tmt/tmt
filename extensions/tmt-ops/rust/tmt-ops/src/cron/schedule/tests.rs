use super::*;

fn ms(text: &str) -> i64 {
    text.parse::<Timestamp>().unwrap().as_millisecond()
}
fn cron(text: &str, zone: &str) -> Schedule {
    Schedule::parse(ScheduleInput::Cron(text), zone, 0).unwrap()
}

#[test]
fn five_field_table_covers_names_lists_ranges_steps_and_day_alternatives() {
    for (pattern, after, next) in [
        (
            "*/15 * * * *",
            "2026-10-03T00:00:00Z",
            "2026-10-03T00:15:00Z",
        ),
        (
            "5,35 8-10/2 * * *",
            "2026-10-03T08:05:00Z",
            "2026-10-03T08:35:00Z",
        ),
        (
            "0 9 * JAN-MAR MON-FRI",
            "2026-03-31T09:00:00Z",
            "2027-01-01T09:00:00Z",
        ),
        (
            "0 9 13 * fri",
            "2026-10-08T09:00:00Z",
            "2026-10-09T09:00:00Z",
        ),
        (
            "0 9 13 * fri",
            "2026-10-12T09:00:00Z",
            "2026-10-13T09:00:00Z",
        ),
        ("0 0 * * 7", "2026-10-03T00:00:00Z", "2026-10-04T00:00:00Z"),
        ("0 0 * * 0", "2026-10-03T00:00:00Z", "2026-10-04T00:00:00Z"),
        (
            "0 0 29 feb *",
            "2025-03-01T00:00:00Z",
            "2028-02-29T00:00:00Z",
        ),
        (
            "0 0 */2 * mon",
            "2026-10-04T00:00:00Z",
            "2026-10-05T00:00:00Z",
        ),
        (
            "*/100 * * * *",
            "2026-10-03T00:00:00Z",
            "2026-10-03T01:00:00Z",
        ),
    ] {
        assert_eq!(
            cron(pattern, "UTC").next_after(ms(after)).unwrap(),
            Some(ms(next)),
            "{pattern}"
        );
    }
    assert_eq!(
        cron("0 0 31 feb *", "UTC")
            .next_after(ms("2026-01-01T00:00:00Z"))
            .unwrap(),
        None
    );
}

#[test]
fn malformed_and_nonstandard_patterns_are_rejected_without_panics() {
    for text in [
        "",
        "@daily",
        "0 0 * *",
        "0 0 0 * * *",
        "60 * * * *",
        "* 24 * * *",
        "0 0 0 * *",
        "0 0 * 13 *",
        "0 0 * * 8",
        "0 0 * * MON#2",
        "0 0 L * *",
        "0 0 ? * mon",
        "*/0 * * * *",
        "0/2 * * * *",
        "1-0 * * * *",
        "0, * * * *",
        "0 0 * JAX *",
        "*/-1 * * * *",
        "0 0 * * 四",
        "0 0 * * ,mon",
        "0 0 * * 1--2",
    ] {
        assert!(
            Schedule::parse(ScheduleInput::Cron(text), "UTC", 0).is_err(),
            "{text}"
        );
    }
    for value in [
        "",
        "0s",
        "-1m",
        "1.5h",
        "1",
        "1ms",
        "1é",
        "9日",
        "9223372036854775807d",
    ] {
        assert!(
            Schedule::parse(
                ScheduleInput::Every {
                    duration: value,
                    from: None
                },
                "UTC",
                0
            )
            .is_err(),
            "{value}"
        );
    }
    for value in ["9:00", "24:00", "09:60", "09:0x", "００:00"] {
        assert!(
            Schedule::parse(
                ScheduleInput::At {
                    time: value,
                    on: None
                },
                "UTC",
                0
            )
            .is_err(),
            "{value}"
        );
    }
    assert!(Schedule::parse(ScheduleInput::Cron("* * * * *"), "Mars/Nowhere", 0).is_err());
}

#[test]
fn local_days_and_half_hour_dst_gaps_are_not_utc_calendar_days() {
    let weekdays = Schedule::parse(
        ScheduleInput::At {
            time: "09:00",
            on: Some("weekdays"),
        },
        "Asia/Tokyo",
        0,
    )
    .unwrap();
    assert_eq!(
        weekdays.next_after(ms("2026-10-02T00:00:00Z")).unwrap(),
        Some(ms("2026-10-05T00:00:00Z"))
    );
    let selected = Schedule::parse(
        ScheduleInput::At {
            time: "10:00",
            on: Some("mon,thu"),
        },
        "Asia/Kolkata",
        0,
    )
    .unwrap();
    assert_eq!(
        selected.next_after(ms("2026-10-05T04:30:00Z")).unwrap(),
        Some(ms("2026-10-08T04:30:00Z"))
    );
    let gap = cron("15 2 * * *", "Australia/Lord_Howe");
    assert_eq!(
        gap.next_after(ms("2026-10-02T15:45:00Z")).unwrap(),
        Some(ms("2026-10-04T15:15:00Z")),
        "October 4's 02:15 does not exist"
    );
}

#[test]
fn fixed_times_skip_spring_gaps_and_choose_only_the_first_fall_occurrence() {
    let spring = cron("30 2 * * *", "America/New_York");
    assert_eq!(
        spring.next_after(ms("2026-03-07T07:30:00Z")).unwrap(),
        Some(ms("2026-03-09T06:30:00Z"))
    );
    let fall = cron("30 1 * * *", "America/New_York");
    assert_eq!(
        fall.next_after(ms("2026-11-01T04:00:00Z")).unwrap(),
        Some(ms("2026-11-01T05:30:00Z"))
    );
    assert_eq!(
        fall.next_after(ms("2026-11-01T05:30:00Z")).unwrap(),
        Some(ms("2026-11-02T06:30:00Z"))
    );
    assert_eq!(
        fall.next_after(ms("2026-11-01T06:00:00Z")).unwrap(),
        Some(ms("2026-11-02T06:30:00Z"))
    );
}

#[test]
fn elapsed_intervals_keep_one_anchor_across_dst_and_skip_past_slots() {
    let schedule = Schedule::parse(
        ScheduleInput::Every {
            duration: "3h",
            from: Some("00:00"),
        },
        "America/New_York",
        ms("2026-03-08T05:00:00Z"),
    )
    .unwrap();
    assert_eq!(
        schedule.next_after(ms("2026-03-08T05:00:00Z")).unwrap(),
        Some(ms("2026-03-08T08:00:00Z"))
    );
    assert_eq!(
        schedule.next_after(ms("2026-03-08T12:34:56Z")).unwrap(),
        Some(ms("2026-03-08T14:00:00Z"))
    );
    let from_gap = Schedule::parse(
        ScheduleInput::Every {
            duration: "1h",
            from: Some("02:30"),
        },
        "America/New_York",
        ms("2026-03-08T05:00:00Z"),
    )
    .unwrap();
    assert_eq!(
        from_gap.next_after(ms("2026-03-08T05:00:00Z")).unwrap(),
        Some(ms("2026-03-09T06:30:00Z"))
    );
    let every = Schedule::parse(
        ScheduleInput::Every {
            duration: "1h",
            from: None,
        },
        "America/New_York",
        ms("2026-11-01T05:30:00Z"),
    )
    .unwrap();
    assert_eq!(
        every.next_after(ms("2026-11-01T05:30:00Z")).unwrap(),
        Some(ms("2026-11-01T06:30:00Z"))
    );
}

#[test]
fn persisted_schedules_preserve_zone_anchor_and_readable_form() {
    for input in [
        ScheduleInput::Every {
            duration: "30m",
            from: Some("09:00"),
        },
        ScheduleInput::At {
            time: "09:00",
            on: Some("weekdays"),
        },
        ScheduleInput::Cron("0 */3 * * *"),
    ] {
        let schedule = Schedule::parse(input, "Asia/Tokyo", ms("2026-10-03T08:12:00Z")).unwrap();
        let restored = Schedule::from_document(&schedule.document()).unwrap();
        assert_eq!(schedule, restored);
        assert_eq!(
            schedule.next_after(ms("2026-10-04T08:12:00Z")).unwrap(),
            restored.next_after(ms("2026-10-04T08:12:00Z")).unwrap()
        );
        assert!(schedule.readable().ends_with("Asia/Tokyo"));
    }
}
