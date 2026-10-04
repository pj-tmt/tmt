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
    let expected = 150.0 * (1.0 - (1.0 - 250.0_f64 / 600.0).powi(3));
    assert!((meter.displayed - expected).abs() < 1e-10);
    let displayed = meter.displayed;
    meter.sample(Ok(&input(300)), start + Duration::from_millis(300));
    assert_eq!(meter.animation.as_ref().unwrap().from, displayed);
    assert_eq!(meter.displayed, displayed, "retarget has no jump");
    let end = start + Duration::from_millis(900);
    meter.tick(end);
    assert_eq!(meter.displayed, meter.reading.unwrap().tokens as f64);
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
    assert_eq!(meter.digits().as_deref(), Some("~150"));
    assert!(meter.wait(now).is_none());
    meter.sample(Ok(&input(200)), now + Duration::from_secs(20));
    assert_eq!(meter.digits().as_deref(), Some("~150"));
    assert!(meter.wait(now).is_none());
    assert_eq!(meter.sparkline().chars().count(), 8);
}

#[test]
fn windows_switch_totals_without_animation() {
    let now = Instant::now();
    let mut meter = Meter::new(TokenRate::default(), &input(100), now);
    meter.sample(Ok(&input(200)), now + Duration::from_secs(10));
    meter.select(TokenWindow::FIVE_MINUTES, now + Duration::from_secs(10));
    assert_eq!(meter.digits().as_deref(), Some("~150"));
    assert!(meter.animation.is_none());
    assert_eq!(meter.label().as_deref(), Some("5m"));
    meter.select(TokenWindow::HOUR, now + Duration::from_secs(10));
    assert_eq!(meter.digits().as_deref(), Some("~150"));
    assert_eq!(meter.label().as_deref(), Some("1h"));
    assert_eq!(
        TokenWindow::HOUR.next(TokenWindow::DEFAULTS),
        TokenWindow::MINUTE
    );
    assert_eq!(
        TokenWindow::MINUTE.available(TokenWindow::DEFAULTS),
        TokenWindow::MINUTE
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
    assert!(
        meter.layout(100).is_none(),
        "a baseline has no covered interval"
    );
    meter.sample(Ok(&input(100)), now + Duration::from_secs(10));
    let full = meter.layout(100).unwrap();
    assert!(full.spark);
    assert_eq!(full.label.as_deref(), Some("1m"));
    let compact = meter.layout(full.width - 1).unwrap();
    assert!(!compact.spark);
    assert!(compact.label.is_some());
    let short = meter.layout(10).unwrap();
    assert_eq!(short.unit, "");
    assert!(short.label.is_some());
    assert!(meter.layout(9).is_none());
    meter.sample(Ok(&input(100)), now + Duration::from_secs(60));
    let short = meter.layout(10).unwrap();
    assert_eq!(short.unit, "");
    assert!(short.label.is_some());
    meter.select(TokenWindow::HOUR, now + Duration::from_secs(60));
    assert!(meter.layout(9).is_none());
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
    meter.resume(TokenWindow::MINUTE, now + Duration::from_secs(80));
    assert_eq!(meter.digits(), None);
    meter.select(TokenWindow::HOUR, now + Duration::from_secs(80));
    assert!(meter.digits().unwrap().starts_with('~'));
    meter.sample(Ok(&input(1_000)), now + Duration::from_secs(85));
    assert_eq!(
        meter.reading.unwrap().tokens as f64,
        150.0,
        "cached tab cannot bridge its unobserved interval"
    );
}

#[test]
fn history_opens_immediately_and_home_live_extension_matches_named_totals() {
    use super::super::rate::tests::{fixture_seeds, historical_input, history_fixture};
    let fixture = history_fixture();
    let seeds = fixture_seeds(&fixture);
    let input = historical_input(&fixture, true);
    let now = Instant::now();
    let mut named = Meter::new(TokenRate::default(), &input, now);
    named.origin_ms = 22_500;
    named.seed(&input, &seeds, now, true);
    assert_eq!(named.digits().as_deref(), Some("~30"));
    assert!(named.animation.is_none());
    let template = input.joined(&std::collections::BTreeMap::new());
    let mut home = Meter::new(TokenRate::default(), &template, now);
    home.origin_ms = 22_500;
    home.seed(&template, &seeds, now, false);
    assert_eq!(home.total(2, now).unwrap().tokens, 21);
    home.sample(Ok(&input), now + Duration::from_secs(5));
    assert_eq!(
        home.total(2, now + Duration::from_secs(5)).unwrap().tokens,
        named.total(2, now).unwrap().tokens
    );
    named.select(TokenWindow::FIVE_MINUTES, now);
    assert_eq!(named.label().as_deref(), Some("5m"));
    assert_eq!(named.digits().as_deref(), Some("~30"));
}
