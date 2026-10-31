use super::*;
const ID: &str = "10000000-0000-4000-8000-000000000001";
const PLAN: &str = "20000000-0000-4000-8000-000000000001";
fn snapshot() -> OrchSnapshot {
    OrchSnapshot {
        identities: vec![OrchIdentity {
            id: ID.into(),
            name: "seat-1".into(),
        }],
        ..Default::default()
    }
}
fn plan() -> OrchPlan {
    OrchPlan {
        id: PLAN.into(),
        name: "Quiet".into(),
        revision: 1,
        priority: 10,
        enabled: true,
        targets: vec![OrchTarget::All],
        schedule: OrchSchedule::Always,
        fields: OrchFields {
            state: Some(OrchState::Sleep),
            ..Default::default()
        },
        members: BTreeMap::new(),
    }
}

#[test]
fn plan_changes_are_candidates_and_revisions_advance_once_only_for_changes() {
    let original = snapshot();
    let added = changed_snapshot(&original, &OrchMutation::AddPlan(plan())).unwrap();
    assert!(original.plans.is_empty());
    assert_eq!(original.revision, 0);
    assert_eq!(added.revision, 1);
    let same = changed_snapshot(
        &added,
        &OrchMutation::SetPlan {
            expected_revision: 1,
            plan: plan(),
        },
    )
    .unwrap();
    assert_eq!(same, added);
    let mut edited = plan();
    edited.enabled = false;
    let set = changed_snapshot(
        &added,
        &OrchMutation::SetPlan {
            expected_revision: 1,
            plan: edited,
        },
    )
    .unwrap();
    assert_eq!(set.revision, 2);
    assert_eq!(set.plans[0].revision, 2);
    assert!(!set.plans[0].enabled);
    let removed = changed_snapshot(
        &set,
        &OrchMutation::RemovePlan {
            id: PLAN.into(),
            expected_revision: 2,
        },
    )
    .unwrap();
    assert_eq!(removed.revision, 3);
    assert!(removed.plans.is_empty());
}
#[test]
fn conflicts_never_mutate_the_captured_snapshot() {
    let added = changed_snapshot(&snapshot(), &OrchMutation::AddPlan(plan())).unwrap();
    let before = added.clone();
    assert_eq!(
        changed_snapshot(&added, &OrchMutation::AddPlan(plan())),
        Err(OrchError::Conflict)
    );
    let mut collision = plan();
    collision.id = "20000000-0000-4000-8000-000000000002".into();
    assert_eq!(
        changed_snapshot(&added, &OrchMutation::AddPlan(collision.clone())),
        Err(OrchError::Conflict)
    );
    collision.name = "Other".into();
    assert_eq!(
        changed_snapshot(&added, &OrchMutation::AddPlan(collision)),
        Err(OrchError::Conflict)
    );
    assert_eq!(
        changed_snapshot(
            &added,
            &OrchMutation::RemovePlan {
                id: PLAN.into(),
                expected_revision: 2
            }
        ),
        Err(OrchError::Conflict)
    );
    assert_eq!(added, before);
}
#[test]
fn focus_is_bounded_and_clear_is_explicit_without_rewriting_plans() {
    let original = snapshot();
    let intent = OrchMutation::SetFocus {
        identity_id: ID.into(),
        start_ms: 1_000,
        end_ms: 61_000,
    };
    let focused = changed_snapshot(&original, &intent).unwrap();
    assert_eq!(focused.focus[0].revision, 1);
    assert_eq!(focused.revision, 1);
    assert_eq!(changed_snapshot(&focused, &intent).unwrap(), focused);
    let cleared = changed_snapshot(
        &focused,
        &OrchMutation::ClearFocus {
            identity_id: ID.into(),
        },
    )
    .unwrap();
    assert!(cleared.focus.is_empty());
    assert_eq!(cleared.revision, 2);
    assert_eq!(
        changed_snapshot(
            &cleared,
            &OrchMutation::ClearFocus {
                identity_id: ID.into()
            }
        )
        .unwrap(),
        cleared
    );
    for (identity, start, end) in [
        (ID, 0, 1),
        (ID, 1, 1),
        (ID, 2, 1),
        (ID, 1, 30 * 86_400_000 + 2),
        ("missing", 1, 2),
    ] {
        assert!(
            changed_snapshot(
                &original,
                &OrchMutation::SetFocus {
                    identity_id: identity.into(),
                    start_ms: start,
                    end_ms: end
                }
            )
            .is_err()
        );
    }
    assert_eq!(original, snapshot());
}
#[test]
fn revision_overflow_is_refused_before_publication() {
    let mut full = snapshot();
    full.revision = MAX_JS_SAFE_INTEGER;
    assert_eq!(
        changed_snapshot(&full, &OrchMutation::AddPlan(plan())),
        Err(OrchError::Limit)
    );
    assert!(full.plans.is_empty());
}

#[test]
fn leads_and_route_grants_are_candidates_that_refuse_unknown_members() {
    const OTHER: &str = "10000000-0000-4000-8000-000000000002";
    let mut original = snapshot();
    original.identities.push(OrchIdentity {
        id: OTHER.into(),
        name: "seat-2".into(),
    });
    let leads = OrchMutation::SetLeads {
        identity_ids: vec![OTHER.into(), ID.into()],
    };
    let led = changed_snapshot(&original, &leads).unwrap();
    assert_eq!(led.leads, [ID, OTHER], "canonical order");
    assert_eq!(led.revision, 1);
    assert_eq!(changed_snapshot(&led, &leads).unwrap().revision, 1, "no-op");
    for (ids, error) in [
        (vec![ID.into(), ID.into()], OrchError::Invalid),
        (vec!["missing".into()], OrchError::MemberNotFound),
    ] {
        assert_eq!(
            changed_snapshot(&original, &OrchMutation::SetLeads { identity_ids: ids }),
            Err(error)
        );
    }

    let grant = |until_ms| sending::OrchRouteGrant {
        from_id: ID.into(),
        to_id: OTHER.into(),
        until_ms,
        granted_by: None,
    };
    let allowed = changed_snapshot(
        &original,
        &OrchMutation::AllowRoute {
            grant: grant(500),
            now_ms: 100,
        },
    )
    .unwrap();
    assert_eq!(allowed.route_grants, [grant(500)]);
    // A later allow replaces the pair's grant; an ended one is pruned.
    let replaced = changed_snapshot(
        &allowed,
        &OrchMutation::AllowRoute {
            grant: grant(900),
            now_ms: 600,
        },
    )
    .unwrap();
    assert_eq!(replaced.route_grants, [grant(900)]);
    for (until_ms, now_ms) in [(100, 100), (100 + sending::ORCH_MAX_GRANT_MS + 1, 100)] {
        assert_eq!(
            changed_snapshot(
                &original,
                &OrchMutation::AllowRoute {
                    grant: grant(until_ms),
                    now_ms
                }
            ),
            Err(OrchError::Invalid)
        );
    }
    let mut to_self = grant(500);
    to_self.to_id = ID.into();
    assert_eq!(
        changed_snapshot(
            &original,
            &OrchMutation::AllowRoute {
                grant: to_self,
                now_ms: 100
            }
        ),
        Err(OrchError::Invalid)
    );
}
