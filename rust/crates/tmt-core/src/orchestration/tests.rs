use super::*;

const DAY_MS: u64 = 86_400_000;
const MONDAY: i32 = 2_461_585;
struct UtcCalendar;
impl OrchCalendar for UtcCalendar {
    fn day_at(&self, ms: u64) -> Result<i32, OrchError> {
        Ok(2_440_588 + (ms / DAY_MS) as i32)
    }
    fn boundary(&self, day: i32, minute: u16, _: bool) -> Result<u64, OrchError> {
        Ok((day - 2_440_588) as u64 * DAY_MS + u64::from(minute) * 60_000)
    }
}
fn at(day: i32, minute: u16) -> u64 {
    UtcCalendar.boundary(day, minute, false).unwrap()
}
fn plan(id: &str, priority: i32, fields: OrchFields) -> OrchPlan {
    OrchPlan {
        id: id.into(),
        name: id.into(),
        revision: 1,
        priority,
        enabled: true,
        targets: vec![OrchTarget::All],
        schedule: OrchSchedule::Always,
        fields,
        members: BTreeMap::new(),
    }
}
fn world(plans: Vec<OrchPlan>) -> OrchSnapshot {
    OrchSnapshot {
        identities: vec![OrchIdentity {
            id: "seat".into(),
            name: "night-1".into(),
        }],
        plans,
        ..Default::default()
    }
}
#[test]
fn higher_awake_restores_receiving_and_routine_but_preserves_sending_and_limit() {
    let lower = plan(
        "quiet",
        1,
        OrchFields {
            state: Some(OrchState::Sleep),
            sending: Some(OrchSending::Leads),
            talk_limit: Some(Some(20)),
            ..Default::default()
        },
    );
    let higher = plan(
        "shift",
        2,
        OrchFields {
            state: Some(OrchState::Awake),
            ..Default::default()
        },
    );
    let value = effective(
        &world(vec![higher, lower]),
        "seat",
        at(MONDAY, 600),
        &UtcCalendar,
    )
    .unwrap();
    assert_eq!(value.receiving.value, OrchReceiving::Direct);
    assert_eq!(value.routine.value, OrchRoutine::Run);
    assert_eq!(value.receiving.source.plan_id.as_deref(), Some("shift"));
    assert!(value.receiving.source.defaulted);
    assert_eq!(value.sending.value, OrchSending::Leads);
    assert_eq!(value.talk_limit.value, Some(20));
    assert_eq!(value.next_change_at_ms, None);
}
#[test]
fn fields_and_member_state_expand_in_their_plan_then_higher_plan_wins() {
    let mut lower = plan(
        "low",
        1,
        OrchFields {
            state: Some(OrchState::Sleep),
            ..Default::default()
        },
    );
    lower.members.insert(
        "seat".into(),
        OrchFields {
            state: Some(OrchState::Focus),
            receiving: Some(OrchReceiving::Auto),
            ..Default::default()
        },
    );
    let higher = plan(
        "high",
        2,
        OrchFields {
            routine: Some(OrchRoutine::Run),
            talk_limit: Some(None),
            ..Default::default()
        },
    );
    let value = effective(
        &world(vec![lower, higher]),
        "seat",
        at(MONDAY, 600),
        &UtcCalendar,
    )
    .unwrap();
    assert_eq!(value.state.value, OrchState::Focus);
    assert_eq!(value.receiving.value, OrchReceiving::Auto);
    assert_eq!(value.receiving.source.kind, OrchSourceKind::Member);
    assert!(!value.receiving.source.defaulted);
    assert_eq!(value.routine.value, OrchRoutine::Run);
    assert_eq!(value.talk_limit.source.plan_id.as_deref(), Some("high"));
    assert_eq!(value.talk_limit.value, None);
}
#[test]
fn personal_focus_is_half_open_and_leaves_sending_unchanged() {
    let mut snapshot = world(vec![plan(
        "awake",
        1,
        OrchFields {
            sending: Some(OrchSending::Paused),
            ..Default::default()
        },
    )]);
    snapshot.focus.push(OrchFocus {
        identity_id: "seat".into(),
        revision: 4,
        start_ms: at(MONDAY, 600),
        end_ms: at(MONDAY, 660),
    });
    let focus = effective(&snapshot, "seat", at(MONDAY, 600), &UtcCalendar).unwrap();
    assert_eq!(focus.receiving.value, OrchReceiving::Urgent);
    assert_eq!(focus.receiving.source.kind, OrchSourceKind::Focus);
    assert_eq!(focus.sending.value, OrchSending::Paused);
    assert_eq!(focus.next_change_at_ms, Some(at(MONDAY, 660)));
    assert_eq!(
        effective(&snapshot, "seat", at(MONDAY, 660), &UtcCalendar)
            .unwrap()
            .state
            .value,
        OrchState::Awake
    );
}
#[test]
fn overnight_biweekly_belongs_to_starting_weekday_and_anchor_phase() {
    let schedule = OrchSchedule::Weekly {
        anchor_day: MONDAY,
        interval_weeks: 2,
        weekdays: vec![1],
        start_minute: 1320,
        end_minute: 360,
        all_day: false,
    };
    let mut p = plan(
        "night",
        1,
        OrchFields {
            state: Some(OrchState::Low),
            ..Default::default()
        },
    );
    p.schedule = schedule;
    let snapshot = world(vec![p]);
    for (day, minute, expected) in [
        (MONDAY, 1319, false),
        (MONDAY, 1320, true),
        (MONDAY + 1, 359, true),
        (MONDAY + 1, 360, false),
        (MONDAY + 7, 1320, false),
        (MONDAY + 14, 1320, true),
        (MONDAY - 14, 1320, true),
    ] {
        assert_eq!(
            !effective(&snapshot, "seat", at(day, minute), &UtcCalendar)
                .unwrap()
                .active_plans
                .is_empty(),
            expected
        );
    }
}
#[test]
fn current_names_select_patterns_without_changing_identity_or_inheriting_a_successor() {
    let mut p = plan(
        "night",
        1,
        OrchFields {
            state: Some(OrchState::Sleep),
            ..Default::default()
        },
    );
    p.targets = vec![OrchTarget::Pattern("night-?".into())];
    let mut snapshot = world(vec![p]);
    assert_eq!(
        effective(&snapshot, "seat", at(MONDAY, 0), &UtcCalendar)
            .unwrap()
            .state
            .value,
        OrchState::Sleep
    );
    snapshot.identities[0].name = "core-1".into();
    assert_eq!(
        effective(&snapshot, "seat", at(MONDAY, 0), &UtcCalendar)
            .unwrap()
            .state
            .value,
        OrchState::Awake
    );
    assert_eq!(
        effective(&snapshot, "successor", at(MONDAY, 0), &UtcCalendar),
        Err(OrchError::NotFound)
    );
    assert!(pattern_matches("夜-*", "夜-一"));
    assert!(!pattern_matches("night-?", "night-12"));
}
#[test]
fn timeline_uses_exact_boundaries_and_disabled_plans_have_no_effect() {
    let mut p = plan("once", 1, OrchFields::default());
    p.schedule = OrchSchedule::Once {
        start_ms: at(MONDAY, 60),
        end_ms: at(MONDAY, 90),
    };
    let mut disabled = plan(
        "off",
        2,
        OrchFields {
            state: Some(OrchState::Sleep),
            ..Default::default()
        },
    );
    disabled.enabled = false;
    let snapshot = world(vec![p, disabled]);
    let timeline = orch_timeline(
        &snapshot,
        OrchSlot {
            start_ms: at(MONDAY, 0),
            end_ms: at(MONDAY, 120),
        },
        &UtcCalendar,
    )
    .unwrap();
    assert_eq!(timeline.len(), 3);
    assert_eq!(
        timeline[1],
        OrchTimelineInterval {
            start_ms: at(MONDAY, 60),
            end_ms: at(MONDAY, 90),
            plan_ids: vec!["once".into()]
        }
    );
    assert!(timeline[0].plan_ids.is_empty());
    assert!(timeline[2].plan_ids.is_empty());
}
#[test]
fn low_defaults_run_and_invalid_repeat_refuses_instead_of_panicking() {
    let value = effective(
        &world(vec![plan(
            "low",
            1,
            OrchFields {
                state: Some(OrchState::Low),
                ..Default::default()
            },
        )]),
        "seat",
        at(MONDAY, 0),
        &UtcCalendar,
    )
    .unwrap();
    assert_eq!(value.receiving.value, OrchReceiving::Batch(1_800_000));
    assert_eq!(value.routine.value, OrchRoutine::Run);
    assert_eq!(
        slot_on(
            &OrchSchedule::Weekly {
                anchor_day: MONDAY,
                interval_weeks: 0,
                weekdays: vec![1],
                start_minute: 0,
                end_minute: 1,
                all_day: false
            },
            MONDAY,
            &UtcCalendar
        ),
        Err(OrchError::Invalid)
    );
}
