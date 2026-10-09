use super::*;
fn reading(tokens: u128, partial: bool) -> Option<Reading> {
    Some(Reading {
        tokens,
        partial,
        span: 60_000,
    })
}
fn frame(
    state: &mut Counters,
    key: &str,
    value: Option<Reading>,
    reduced: bool,
    visible: bool,
    now: Instant,
) -> String {
    state.begin();
    let text = state.digits(key.into(), value, reduced, now);
    if visible {
        state.visible(
            key,
            Rect::new(4, 7, 7, 1),
            String::new(),
            Align::Right,
            true,
        );
    }
    state.finish(None, false);
    text
}

#[test]
fn home_digits_increase_decrease_retarget_and_settle_without_changing_readings() {
    let now = Instant::now();
    let mut state = Counters::default();
    let raw = reading(100, false);
    assert_eq!(
        frame(&mut state, "uuid:room:1m", raw, false, true, now),
        "100"
    );
    assert_eq!(
        state.wait(now),
        None,
        "first usable evidence never counts from zero"
    );
    let start = now + Duration::from_secs(1);
    assert_eq!(
        frame(
            &mut state,
            "uuid:room:1m",
            reading(400, false),
            false,
            true,
            start
        ),
        "100"
    );
    assert!(!state.tick(start + Duration::from_millis(249)));
    assert!(state.tick(start + Duration::from_millis(250)));
    assert_eq!(
        frame(
            &mut state,
            "uuid:room:1m",
            reading(400, false),
            false,
            true,
            start + Duration::from_millis(250)
        ),
        "340"
    );
    // A decreasing target starts at the last displayed value, not either raw target.
    assert_eq!(
        frame(
            &mut state,
            "uuid:room:1m",
            reading(200, true),
            false,
            true,
            start + Duration::from_millis(300)
        ),
        "~340"
    );
    assert!(state.tick(start + Duration::from_millis(550)));
    assert_eq!(
        frame(
            &mut state,
            "uuid:room:1m",
            reading(200, true),
            false,
            true,
            start + Duration::from_millis(550)
        ),
        "~228"
    );
    assert!(state.tick(start + Duration::from_millis(900)));
    assert_eq!(
        frame(
            &mut state,
            "uuid:room:1m",
            reading(200, true),
            false,
            true,
            start + Duration::from_millis(900)
        ),
        "~200"
    );
    assert_eq!(state.wait(start + Duration::from_millis(900)), None);
    assert!(
        !state.tick(now + Duration::from_secs(100)),
        "settled has zero animation wakeups"
    );
    assert_eq!(raw.unwrap().tokens, 100);
}

#[test]
fn unavailable_zero_partial_and_reduced_motion_keep_their_evidence_meaning() {
    let now = Instant::now();
    let mut state = Counters::default();
    for (value, text) in [
        (None, "–"),
        (reading(0, false), "0"),
        (None, "–"),
        (reading(12, true), "~12"),
    ] {
        assert_eq!(frame(&mut state, "owner", value, false, true, now), text);
        assert_eq!(state.wait(now), None);
    }
    assert_eq!(
        frame(&mut state, "owner", reading(999, false), true, true, now),
        "999"
    );
    assert_eq!(state.wait(now), None);
}

#[test]
fn owner_window_reentry_hidden_and_covered_fields_do_not_leak_or_wake() {
    let now = Instant::now();
    let mut state = Counters::default();
    frame(
        &mut state,
        "uuid-a:room-a:1m",
        reading(100, false),
        false,
        true,
        now,
    );
    frame(
        &mut state,
        "uuid-a:room-a:1m",
        reading(900, false),
        false,
        true,
        now,
    );
    assert!(state.wait(now).is_some());
    assert_eq!(
        frame(
            &mut state,
            "uuid-b:room-b:5m",
            reading(600, false),
            false,
            true,
            now
        ),
        "600"
    );
    assert_eq!(state.wait(now), None);
    assert_eq!(
        state.fields.len(),
        1,
        "replaced owner/window drops old digits"
    );
    frame(
        &mut state,
        "uuid-b:room-b:5m",
        reading(900, false),
        false,
        false,
        now,
    );
    assert_eq!(
        state.wait(now),
        None,
        "offscreen field stops at the current target"
    );
    assert_eq!(
        frame(
            &mut state,
            "uuid-b:room-b:5m",
            reading(1200, false),
            false,
            true,
            now
        ),
        "1k"
    );
    frame(
        &mut state,
        "uuid-b:room-b:5m",
        reading(1800, false),
        false,
        true,
        now,
    );
    state.begin();
    state.digits("uuid-b:room-b:5m".into(), reading(1800, false), false, now);
    state.visible(
        "uuid-b:room-b:5m",
        Rect::new(4, 7, 7, 1),
        String::new(),
        Align::Right,
        true,
    );
    state.finish(Some(Rect::new(0, 6, 80, 4)), false);
    assert_eq!(state.wait(now), None, "opaque composer covers the field");
    state.clear();
    assert_eq!(
        frame(
            &mut state,
            "uuid-b:room-b:5m",
            reading(1800, false),
            false,
            true,
            now
        ),
        "2k"
    );
    assert_eq!(
        state.wait(now),
        None,
        "cached re-entry settles without fresh evidence"
    );
}

#[test]
fn wakeups_are_bounded_and_only_formatted_visible_digits_dirty_the_frame() {
    let now = Instant::now();
    let mut state = Counters::default();
    frame(&mut state, "owner", reading(100, false), false, true, now);
    frame(&mut state, "owner", reading(400, false), false, true, now);
    let mut time = now;
    let mut wakes = 0;
    while let Some(wait) = state.wait(time) {
        time += wait;
        state.tick(time);
        wakes += 1;
        assert!(wakes <= 3);
    }
    assert_eq!(wakes, 3);
    assert_eq!(
        frame(&mut state, "owner", reading(400, false), false, true, time),
        "400"
    );
    state.begin();
    state.digits("owner".into(), reading(800, false), false, time);
    state.visible(
        "owner",
        Rect::new(4, 7, 1, 1),
        "1h:".into(),
        Align::Right,
        true,
    );
    state.finish(None, false);
    assert!(
        !state.tick(time + Duration::from_millis(250)),
        "clipped digits cannot dirty the frame"
    );
}

#[test]
fn a_zero_height_header_cannot_keep_hidden_animation_alive() {
    let now = Instant::now();
    let mut state = Counters::default();
    frame(&mut state, "header", reading(100, false), false, true, now);
    state.begin();
    state.digits("header".into(), reading(900, false), false, now);
    state.visible(
        "header",
        Rect::new(4, 0, 7, 1),
        String::new(),
        Align::Left,
        false,
    );
    state.place_header(Rect::new(0, 2, 80, 0));
    state.finish(None, false);
    assert_eq!(state.wait(now), None);
    assert!(!state.tick(now + Duration::from_secs(1)));
}
