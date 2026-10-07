//! The ledger as a pure state machine: one accepted conversation, then one refusal
//! per way a frame can be out of order, stale, duplicated or mismatched. A refused
//! frame changes nothing, and both roles reach the same verdicts.
use super::super::tests::{
    admit, admit_read, answer, big_read_result, done, origin_state, read_request, request, transfer,
};
use super::*;
use crate::Frame;

/// A step: the frame and whether the extension is the side sending it.
type Step = (Frame, bool);
fn from_extension(frame: Frame) -> Step {
    (frame, true)
}
fn from_remote(frame: Frame) -> Step {
    (frame, false)
}

/// Run `steps` as `role`; the verdict is the first refusal, with its step index.
fn run(role: Role, caps: Caps, budget: Option<Budget>, steps: &[Step]) -> Option<(usize, Reason)> {
    let mut ledger = Ledger::new(role, caps, budget);
    for (index, (frame, extension_sends)) in steps.iter().enumerate() {
        let sending = (role == Role::Extension) == *extension_sends;
        if let Err(reason) = ledger.apply(frame, sending) {
            return Some((index, reason));
        }
    }
    None
}
fn wide() -> Caps {
    Caps {
        requests: 8,
        callbacks: 8,
    }
}
/// A full conversation: a request, two callbacks one after the other, the result.
fn conversation() -> Vec<Step> {
    let t = transfer(1);
    vec![
        from_extension(request(1, t)),
        from_remote(origin_state()),
        from_remote(admit(1, 1, t)),
        from_extension(answer(1, 1)),
        from_remote(admit(2, 1, t)),
        from_extension(answer(2, 1)),
        from_remote(done(1, t)),
    ]
}

#[test]
fn the_accepted_conversation_passes_in_both_roles() {
    for role in [Role::Extension, Role::Remote] {
        assert_eq!(run(role, wide(), None, &conversation()), None, "{role:?}");
    }
}

#[test]
fn every_out_of_order_stale_or_mismatched_frame_is_refused_in_both_roles() {
    let (t1, t2) = (transfer(1), transfer(2));
    #[rustfmt::skip]
    let rows: Vec<(&str, Vec<Step>, Reason)> = vec![
        ("repeated request id", vec![from_extension(request(1, t1)), from_extension(request(1, t1))], Reason::Order),
        ("lower request id", vec![from_extension(request(5, t1)), from_extension(request(3, t2))], Reason::Order),
        ("request id after the largest", vec![from_extension(request(u64::MAX, t1)), from_extension(request(u64::MAX, t2))], Reason::Order),
        ("callback for no request", vec![from_remote(admit(1, 9, t1))], Reason::Unknown),
        ("callback for another operation", vec![from_extension(request(1, t1)), from_remote(admit_read(1, 1))], Reason::Mismatch),
        ("callback for another transfer", vec![from_extension(request(1, t1)), from_remote(admit(1, 1, t2))], Reason::Mismatch),
        ("second callback before the first is answered", vec![from_extension(request(1, t1)), from_remote(admit(1, 1, t1)), from_remote(admit(2, 1, t1))], Reason::Busy),
        ("callback id not above the last", vec![from_extension(request(1, t1)), from_remote(admit(5, 1, t1)), from_extension(answer(5, 1)), from_remote(admit(3, 1, t1))], Reason::Order),
        ("callback id reused", vec![from_extension(request(1, t1)), from_remote(admit(1, 1, t1)), from_extension(answer(1, 1)), from_remote(admit(1, 1, t1))], Reason::Order),
        ("answer to no callback", vec![from_extension(request(1, t1)), from_extension(answer(1, 1))], Reason::Unknown),
        ("answer naming another request", vec![from_extension(request(1, t1)), from_extension(request(2, t2)), from_remote(admit(1, 1, t1)), from_extension(answer(1, 2))], Reason::Mismatch),
        ("answer given twice", vec![from_extension(request(1, t1)), from_remote(admit(1, 1, t1)), from_extension(answer(1, 1)), from_extension(answer(1, 1))], Reason::Unknown),
        ("result for no request", vec![from_remote(done(1, t1))], Reason::Unknown),
        ("result given twice", vec![from_extension(request(1, t1)), from_remote(done(1, t1)), from_remote(done(1, t1))], Reason::Unknown),
        ("result of another operation", vec![from_extension(request(1, t1)), from_remote(big_read_result(1))], Reason::Mismatch),
        ("result naming another transfer", vec![from_extension(request(1, t1)), from_remote(done(1, t2))], Reason::Mismatch),
        ("result while a callback is outstanding", vec![from_extension(request(1, t1)), from_remote(admit(1, 1, t1)), from_remote(done(1, t1))], Reason::Busy),
        ("answer after the request ended", vec![from_extension(request(1, t1)), from_remote(done(1, t1)), from_extension(answer(1, 1))], Reason::Unknown),
        ("result sent by the extension", vec![from_extension(request(1, t1)), from_extension(done(1, t1))], Reason::Direction),
        ("request sent by Remote", vec![from_remote(request(1, t1))], Reason::Direction),
        ("callback sent by the extension", vec![from_extension(request(1, t1)), from_extension(admit(1, 1, t1))], Reason::Direction),
        ("answer sent by Remote", vec![from_extension(request(1, t1)), from_remote(admit(1, 1, t1)), from_remote(answer(1, 1))], Reason::Direction),
        ("lifecycle sent by the extension", vec![from_extension(origin_state())], Reason::Direction),
    ];
    // The accepted twin of every row: the same frames in an order that is allowed.
    assert_eq!(run(Role::Extension, wide(), None, &conversation()), None);
    for (name, steps, reason) in rows {
        for role in [Role::Extension, Role::Remote] {
            let last = steps.len() - 1;
            assert_eq!(
                run(role, wide(), None, &steps),
                Some((last, reason)),
                "{name} as {role:?}"
            );
        }
    }
}

#[test]
fn a_refused_frame_changes_nothing() {
    let t = transfer(1);
    let mut steps = vec![
        from_extension(request(1, t)),
        from_extension(request(1, t)),
        from_remote(admit(1, 1, t)),
        from_remote(admit(2, 1, t)),
        from_extension(answer(1, 1)),
        from_remote(done(1, t)),
    ];
    // Remove the two refused frames and the rest is the same conversation.
    let verdict = run(Role::Extension, wide(), None, &steps[..2]);
    assert_eq!(verdict, Some((1, Reason::Order)));
    steps.remove(3);
    steps.remove(1);
    assert_eq!(run(Role::Extension, wide(), None, &steps), None);
}

#[test]
fn outstanding_entries_are_bounded_per_channel_and_per_installation() {
    let t = transfer(1);
    let caps = Caps {
        requests: 2,
        callbacks: 1,
    };
    let steps = vec![
        from_extension(request(1, t)),
        from_extension(request(2, t)),
        from_extension(request(3, t)),
    ];
    assert_eq!(
        run(Role::Extension, caps, None, &steps),
        Some((2, Reason::Capacity))
    );
    // A finished request frees its slot.
    let steps = vec![
        from_extension(request(1, t)),
        from_extension(request(2, t)),
        from_remote(done(1, t)),
        from_extension(request(3, t)),
    ];
    assert_eq!(run(Role::Extension, caps, None, &steps), None);
    // One callback slot: the second request's callback has to wait.
    let steps = vec![
        from_extension(request(1, t)),
        from_extension(request(2, t)),
        from_remote(admit(1, 1, t)),
        from_remote(admit(2, 2, t)),
    ];
    assert_eq!(
        run(Role::Remote, caps, None, &steps),
        Some((3, Reason::Capacity))
    );

    // Two channels of one installation share three request slots.
    let budget = Budget::new(3, 3);
    let mut first = Ledger::new(Role::Extension, wide(), Some(budget.clone()));
    let mut second = Ledger::new(Role::Extension, wide(), Some(budget));
    assert!(first.apply(&request(1, t), true).is_ok());
    assert!(first.apply(&request(2, t), true).is_ok());
    assert!(second.apply(&request(1, t), true).is_ok());
    assert_eq!(second.apply(&request(2, t), true), Err(Reason::Capacity));
    // Ending a request, or the whole channel, returns its slots.
    assert!(first.apply(&done(1, t), false).is_ok());
    assert!(second.apply(&request(2, t), true).is_ok());
    drop(first);
    assert!(second.apply(&request(3, t), true).is_ok());
    assert_eq!(second.apply(&request(4, t), true), Err(Reason::Capacity));
}

#[test]
fn lifecycle_frames_need_no_correlation_and_read_requests_name_no_transfer() {
    let steps = vec![
        from_remote(origin_state()),
        from_extension(read_request(1)),
        from_remote(admit_read(1, 1)),
        from_extension(answer(1, 1)),
        from_remote(big_read_result(1)),
    ];
    assert_eq!(run(Role::Extension, wide(), None, &steps), None);
}
