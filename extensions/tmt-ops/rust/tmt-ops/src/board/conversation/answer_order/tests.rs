use super::*;

fn answered(bound: u64, settled: u64) -> Capture {
    Capture::Answered {
        text: "answer".into(),
        truncated: false,
        provider: "claude".into(),
        ended_ms: settled,
        bound_ms: Some(bound),
        settled_ms: Some(settled),
    }
}
fn probes<'a>(sends: &'a [(u64, Option<Capture>, bool)]) -> Vec<Probe<'a>> {
    sends
        .iter()
        .map(|(at_ms, capture, direct)| Probe {
            at_ms: *at_ms,
            capture: capture.as_ref(),
            direct: *direct,
        })
        .collect()
}
fn roles(sends: &[(u64, Option<Capture>, bool)]) -> Vec<Role> {
    place(&probes(sends))
        .iter()
        .map(|place| place.role)
        .collect()
}

#[test]
fn sends_typed_while_the_turn_is_open_form_one_block_before_any_answer() {
    let sends = [
        (100, Some(Capture::Bound), true),
        (218, Some(Capture::Pending), true),
        (237, Some(Capture::Pending), true),
    ];
    assert_eq!(roles(&sends), [Role::First, Role::Member, Role::Last]);
    let places = place(&probes(&sends));
    assert!(places.iter().all(|place| place.grouped));
    assert!(places[2].answers.is_empty(), "no answer yet");
    assert!(places[0].header() && !places[1].header() && !places[2].header());
    assert!(!places[0].status() && !places[1].status() && places[2].status());
}

#[test]
fn a_just_sent_direct_send_groups_before_its_capture_is_read() {
    let sends = [(100, None, true), (110, None, true)];
    assert_eq!(roles(&sends), [Role::First, Role::Last]);
}

#[test]
fn the_answered_turn_belongs_to_the_block_and_paints_under_its_last_send() {
    let sends = [
        (100, Some(answered(110, 262)), true),
        (218, Some(Capture::NoAnswer), true),
        (237, None, true),
    ];
    let places = place(&probes(&sends));
    assert_eq!(
        places.iter().map(|place| place.role).collect::<Vec<_>>(),
        [Role::First, Role::Member, Role::Last]
    );
    assert_eq!(
        places[2].answers,
        [0],
        "the first send's answer follows the last"
    );
    assert!(places[0].answers.is_empty() && places[1].answers.is_empty());
}

#[test]
fn a_send_after_the_turn_settled_or_with_its_own_turn_starts_a_new_block() {
    let after = [
        (100, Some(answered(110, 262)), true),
        (262, None, true),
        (300, None, true),
    ];
    assert_eq!(
        roles(&after),
        [Role::Single, Role::First, Role::Last],
        "the send at the settle time opens the next block, the next one joins it"
    );
    let own_turn = [
        (100, Some(answered(110, 262)), true),
        (218, Some(answered(220, 300)), true),
    ];
    assert_eq!(roles(&own_turn), [Role::Single, Role::Single]);
    let bound = [
        (100, Some(answered(110, 262)), true),
        (218, Some(Capture::Bound), true),
    ];
    assert_eq!(roles(&bound), [Role::Single, Role::Single]);
}

#[test]
fn a_send_that_is_not_direct_or_failed_never_joins_and_ends_the_block() {
    let sends = [
        (100, Some(Capture::Bound), true),
        (150, None, false),
        (200, Some(Capture::Pending), true),
    ];
    assert_eq!(
        roles(&sends),
        [Role::Single, Role::Single, Role::Single],
        "the failed send in the middle splits the run"
    );
    let first_not_direct = [(100, None, false), (150, None, true), (160, None, true)];
    assert_eq!(
        roles(&first_not_direct),
        [Role::Single, Role::First, Role::Last]
    );
}

#[test]
fn a_turn_without_a_window_or_with_no_answer_groups_nothing() {
    let no_window = [
        (
            100,
            Some(Capture::Answered {
                text: "a".into(),
                truncated: false,
                provider: "claude".into(),
                ended_ms: 262,
                bound_ms: None,
                settled_ms: None,
            }),
            true,
        ),
        (218, None, true),
    ];
    assert_eq!(roles(&no_window), [Role::Single, Role::Single]);
    let no_answer = [(100, Some(Capture::NoAnswer), true), (110, None, true)];
    assert_eq!(roles(&no_answer), [Role::Single, Role::Single]);
    assert!(place(&[]).is_empty());
}

#[test]
fn the_block_status_is_the_newest_line_that_is_not_a_plain_delivery() {
    assert_eq!(
        block_status(["✓ delivered", "⠹ working…", "✓ delivered"].into_iter()),
        "⠹ working…"
    );
    assert_eq!(
        block_status(["✓ delivered", "✓ delivered"].into_iter()),
        "✓ delivered"
    );
    assert_eq!(
        block_status(["◌ held · digest", "⠹ working…"].into_iter()),
        "⠹ working…",
        "the newest wins among states that are not plain deliveries"
    );
    assert_eq!(block_status(std::iter::empty()), "");
}

#[test]
fn only_a_body_of_several_paragraphs_asks_for_a_blank_line_between_bodies() {
    assert!(!spaced("one line", "another line"));
    assert!(!spaced("two\nlines", "a third\n"));
    assert!(spaced("first paragraph\n\nsecond paragraph", "short"));
    assert!(spaced("short", "first\n\nsecond"));
}
