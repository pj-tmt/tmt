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
fn probes<'a>(sends: &'a [(u64, Option<Capture>)]) -> Vec<Probe<'a>> {
    sends
        .iter()
        .map(|(at_ms, capture)| Probe {
            at_ms: *at_ms,
            capture: capture.as_ref(),
        })
        .collect()
}
fn at(places: &[Place], index: usize) -> (bool, bool, Vec<usize>) {
    let place = &places[index];
    (place.answered_below, place.hoisted, place.answers.clone())
}

#[test]
fn a_send_typed_during_the_turn_gets_the_answer_after_it_and_the_first_says_so() {
    // A is answered by a turn settled at 262; B was sent at 218 with no capture of its own.
    let sends = [
        (100, Some(answered(110, 262))),
        (218, Some(Capture::NoAnswer)),
    ];
    let places = place(&probes(&sends));
    assert_eq!(
        at(&places, 0),
        (true, true, vec![]),
        "A: answered below, not under itself"
    );
    assert_eq!(
        at(&places, 1),
        (false, false, vec![0]),
        "B carries A's answer"
    );
}

#[test]
fn three_sends_in_one_turn_share_the_answer_under_the_last() {
    let sends = [
        (100, Some(answered(110, 262))),
        (218, Some(Capture::NoAnswer)),
        (237, None),
    ];
    let places = place(&probes(&sends));
    assert_eq!(at(&places, 0), (true, true, vec![]));
    assert_eq!(at(&places, 1), (true, false, vec![]));
    assert_eq!(at(&places, 2), (false, false, vec![0]));
}

#[test]
fn a_send_with_its_own_capture_or_after_the_turn_is_not_part_of_the_group() {
    let own = [
        (100, Some(answered(110, 262))),
        (218, Some(answered(220, 300))),
    ];
    let places = place(&probes(&own));
    assert_eq!(
        at(&places, 0),
        (false, false, vec![]),
        "B has its own answer"
    );
    assert_eq!(at(&places, 1), (false, false, vec![]));
    let waiting = [
        (100, Some(answered(110, 262))),
        (218, Some(Capture::Pending)),
        (240, Some(Capture::Bound)),
    ];
    let places = place(&probes(&waiting));
    assert!(
        places.iter().all(|place| *place == Place::default()),
        "a turn in progress is its own"
    );
    let after = [(100, Some(answered(110, 262))), (262, None), (300, None)];
    let places = place(&probes(&after));
    assert!(
        places.iter().all(|place| *place == Place::default()),
        "sent at or after the turn settled"
    );
}

#[test]
fn a_lone_answered_send_and_plain_sends_are_untouched() {
    let sends = [
        (100, Some(answered(110, 262))),
        (400, None),
        (500, Some(Capture::Pending)),
    ];
    let places = place(&probes(&sends));
    assert!(places.iter().all(|place| *place == Place::default()));
    assert!(place(&[]).is_empty());
    // An answer without a window cannot group anything.
    let unbound = [
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
        ),
        (218, None),
    ];
    assert!(
        place(&probes(&unbound))
            .iter()
            .all(|place| *place == Place::default())
    );
}

#[test]
fn only_a_plain_delivered_label_gets_the_answered_below_note() {
    assert_eq!(
        answered_below("✓ delivered"),
        "✓ delivered · answered below"
    );
    assert_eq!(answered_below("◌ held · digest"), "◌ held · digest");
    assert_eq!(answered_below("⠹ working…"), "⠹ working…");
}
