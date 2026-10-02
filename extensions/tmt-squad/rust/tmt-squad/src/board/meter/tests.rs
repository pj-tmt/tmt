use super::super::rate::tests::input;
use super::*;

#[test]
fn cubic_frames_settle_exactly_and_retarget_from_displayed_digits() {
    let now = Instant::now();
    let mut meter = Meter::new(TokenRate::default(), &input(100), now);
    assert_eq!(meter.digits().as_deref(), None);
    meter.sample(Ok(&input(200)), now + Duration::from_secs(10));
    let start = now + Duration::from_secs(10);
    assert!(!meter.tick(start + Duration::from_millis(249)));
    assert!(meter.tick(start + Duration::from_millis(250)));
    let expected = 15.0 * (1.0 - (1.0 - 250.0_f64 / 600.0).powi(3));
    assert!((meter.displayed - expected).abs() < 1e-10);
    let displayed = meter.displayed;
    meter.sample(Ok(&input(300)), start + Duration::from_millis(300));
    assert_eq!(meter.animation.as_ref().unwrap().from, displayed);
    assert_eq!(meter.displayed, displayed, "retarget has no jump");
    let end = start + Duration::from_millis(900);
    meter.tick(end);
    assert_eq!(meter.displayed, meter.reading.unwrap().rate);
    assert!(meter.animation.is_none());
    assert!(meter.wait(end).is_none());
    assert!(
        !meter.tick(end + Duration::from_secs(100)),
        "no idle animation"
    );
}

#[test]
fn reduced_motion_and_equal_rates_are_immediate_and_idle() {
    let now = Instant::now();
    let settings = TokenRate {
        reduced_motion: true,
        ..Default::default()
    };
    let mut meter = Meter::new(settings, &input(100), now);
    meter.sample(Ok(&input(200)), now + Duration::from_secs(10));
    assert_eq!(meter.digits().as_deref(), Some("15"));
    assert!(meter.wait(now).is_none());
    meter.sample(Ok(&input(300)), now + Duration::from_secs(20));
    assert_eq!(meter.digits().as_deref(), Some("15"));
    assert!(meter.wait(now).is_none());
    assert_eq!(meter.sparkline().chars().count(), 8);
}

#[test]
fn windows_switch_without_animation_and_sampling_skips_five_seconds() {
    let now = Instant::now();
    let mut meter = Meter::new(TokenRate::default(), &input(100), now);
    meter.sample(Ok(&input(200)), now + Duration::from_secs(10));
    meter.select(TokenWindow::Five, now + Duration::from_secs(10));
    assert_eq!(meter.digits().as_deref(), Some("30"));
    assert!(meter.animation.is_none());
    assert_eq!(meter.label().as_deref(), Some("5s"));
    meter.select(TokenWindow::Hour, now + Duration::from_secs(10));
    assert_eq!(meter.digits().as_deref(), Some("15"));
    assert_eq!(meter.label().as_deref(), Some("10s"));
    assert_eq!(
        TokenWindow::Hour.next(Duration::from_secs(10)),
        TokenWindow::Minute
    );
    assert_eq!(
        TokenWindow::Five.available(Duration::from_secs(6)),
        TokenWindow::Minute
    );
}

#[test]
fn layout_keeps_partial_and_nondefault_labels_and_steps_aside() {
    let now = Instant::now();
    let mut meter = Meter::new(
        TokenRate {
            enabled: true,
            ..Default::default()
        },
        &input(100),
        now,
    );
    assert!(meter.layout(100).is_none());
    meter.sample(Ok(&input(100)), now + Duration::from_secs(10));
    let full = meter.layout(100).unwrap();
    assert!(full.spark);
    assert_eq!(full.label.as_deref(), Some("10s"));
    let compact = meter.layout(full.width - 1).unwrap();
    assert!(!compact.spark);
    assert!(compact.label.is_some());
    let short = meter.layout(16).unwrap();
    assert_eq!(short.unit, "/s");
    assert!(short.label.is_some());
    assert!(meter.layout(15).is_none());
    meter.sample(Ok(&input(100)), now + Duration::from_secs(60));
    assert!(meter.full_default());
    let short = meter.layout(12).unwrap();
    assert_eq!(short.unit, "/s");
    assert!(short.label.is_none());
    meter.select(TokenWindow::Hour, now + Duration::from_secs(60));
    assert!(!meter.full_default());
    assert!(meter.layout(12).is_none());
}

#[test]
fn returning_tab_expires_short_window_and_retains_long_gap_history() {
    let now = Instant::now();
    let mut meter = Meter::new(
        TokenRate {
            enabled: true,
            ..Default::default()
        },
        &input(100),
        now,
    );
    meter.sample(Ok(&input(200)), now + Duration::from_secs(10));
    meter.suspend(now + Duration::from_secs(11));
    meter.resume(TokenWindow::Minute, now + Duration::from_secs(80));
    assert_eq!(meter.digits(), None);
    meter.select(TokenWindow::Hour, now + Duration::from_secs(80));
    assert!(meter.digits().unwrap().starts_with('≥'));
    meter.sample(Ok(&input(1_000)), now + Duration::from_secs(85));
    assert_eq!(
        meter.reading.unwrap().rate,
        150.0 / 85.0,
        "cached tab cannot bridge its unobserved interval"
    );
}
