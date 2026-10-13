//! Adaptive polling promises cadence, never push latency; time is injected.
use tmt_remote::native_sync::PollCadence;
#[test]
fn activity_is_live_for_two_minutes_then_decays_to_idle() {
    let mut cadence = PollCadence::default();
    assert_eq!(cadence.delay_ms(0, |_| 0), 60_000);
    cadence.activity(10_000);
    for (age, delay) in [
        (0, 5_000),
        (120_000, 5_000),
        (120_001, 10_000),
        (150_001, 20_000),
        (180_001, 40_000),
        (210_001, 60_000),
    ] {
        assert_eq!(cadence.delay_ms(10_000 + age, |_| 0), delay);
    }
    cadence.activity(1_000_000);
    assert_eq!(cadence.delay_ms(1_000_000, |_| 0), 5_000);
}
#[test]
fn jitter_never_polls_faster_and_failure_backoff_is_capped() {
    let mut cadence = PollCadence::default();
    cadence.activity(0);
    assert_eq!(cadence.delay_ms(0, |_| u64::MAX), 5_500);
    for _ in 0..20 {
        cadence.failed();
    }
    assert_eq!(cadence.delay_ms(0, |_| u64::MAX), 900_000);
    cadence.succeeded();
    assert_eq!(cadence.delay_ms(0, |_| 0), 5_000);
}

#[test]
fn an_older_publication_cannot_erase_a_newer_received_change() {
    let mut cadence = PollCadence::default();
    cadence.activity(0);
    cadence.activity(120_000);
    // Subscription sampling also observes the shared last-publication time.
    cadence.activity(0);
    assert_eq!(cadence.delay_ms(240_000, |_| 0), 5_000);
    assert_eq!(cadence.delay_ms(240_001, |_| 0), 10_000);
}
