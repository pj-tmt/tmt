use super::*;
use crate::native_install::{
    Channel, InstalledVersion, PinAction, PrVersionContext, UpgradeSelection, latest_in_channel,
    plan_candidate_version, plan_version, select_upgrade,
};

fn installed(version: &str, channel: Channel) -> InstalledVersion {
    InstalledVersion {
        version: version.parse().unwrap(),
        channel,
        pinned_version: None,
    }
}

fn identity(pr: &str, head: char, run: u64) -> PrCandidateIdentity {
    PrCandidateIdentity::new(
        PrNumber::parse(pr).unwrap(),
        head.to_string().repeat(40),
        run,
    )
    .unwrap()
}

#[test]
fn pr_identifiers_have_one_bounded_canonical_spelling() {
    for value in ["1", "234", "2147483647"] {
        assert_eq!(PrNumber::parse(value).unwrap().to_string(), value);
    }
    for value in [
        "",
        "0",
        "01",
        "+1",
        "-1",
        " 1",
        "1 ",
        "１",
        "2147483648",
        "4294967296",
    ] {
        assert_eq!(PrNumber::parse(value), None, "{value:?}");
    }
}

#[test]
fn channel_intent_round_trips_without_reinterpreting_a_pr_as_published_tags() {
    for value in ["pr1", "pr234", "pr2147483647"] {
        let channel = Channel::parse(value).unwrap();
        assert_eq!(channel.as_str(), value);
        assert_eq!(Channel::parse(&channel.as_str()), Some(channel));
        assert_eq!(
            latest_in_channel(&[], channel),
            Err(VersionError::InvalidSelection)
        );
    }
    for value in [
        "pr",
        "pr0",
        "pr01",
        "pr-1",
        "pr+1",
        "pr 1",
        "pr１",
        "pr2147483648",
        "PR1",
    ] {
        assert_eq!(Channel::parse(value), None, "{value:?}");
    }
}

#[test]
fn candidate_identity_requires_exact_source_and_positive_run() {
    let pr = PrNumber::parse("234").unwrap();
    assert!(PrCandidateIdentity::new(pr, "a".repeat(40), 1).is_some());
    for (sha, run) in [
        ("a".repeat(40), 0),
        ("a".repeat(39), 1),
        ("A".repeat(40), 1),
        ("g".repeat(40), 1),
    ] {
        assert!(PrCandidateIdentity::new(pr, sha, run).is_none());
    }
}

#[test]
fn newer_runs_order_rebases_and_repeats_without_semver_invention() {
    let current = identity("234", 'a', 10);
    assert_eq!(current.successor_changed(&current), Ok(false));
    assert_eq!(
        current.successor_changed(&identity("234", 'a', 11)),
        Ok(true)
    );
    assert_eq!(
        current.successor_changed(&identity("234", 'b', 11)),
        Ok(true)
    );
    for next in [
        identity("234", 'b', 10),
        identity("234", 'b', 9),
        identity("234", 'a', 9),
    ] {
        assert_eq!(
            current.successor_changed(&next),
            Err(VersionError::StalePrCandidate)
        );
    }
    assert_eq!(
        current.successor_changed(&identity("235", 'b', 11)),
        Err(VersionError::WrongChannel)
    );
}

#[test]
fn unknown_schema_evidence_refuses_even_with_consent() {
    for opted_in in [false, true] {
        for evidence in [
            (None, Some(48), Some(47)),
            (Some(48), None, Some(47)),
            (Some(48), Some(48), None),
        ] {
            assert_eq!(
                admit_schema(evidence.0, evidence.1, evidence.2, opted_in),
                Err(SchemaError::Unknown)
            );
        }
    }
}

#[test]
fn ahead_schema_needs_opt_in_and_retains_the_exact_admission_values() {
    assert_eq!(
        admit_schema(Some(49), Some(48), Some(48), false),
        Err(SchemaError::AheadOfAlpha)
    );
    let admitted = admit_schema(Some(49), Some(48), Some(48), true).unwrap();
    assert_eq!(
        admitted,
        SchemaAdmission {
            candidate: 49,
            latest_alpha: 48,
            local: 48,
            opted_in: true
        }
    );
    assert!(admitted.ahead_of_alpha());
    assert!(
        !admit_schema(Some(48), Some(48), Some(47), false)
            .unwrap()
            .ahead_of_alpha()
    );
}

#[test]
fn consent_never_authorizes_data_downgrade_and_alpha_return_waits_for_compatibility() {
    for opted_in in [false, true] {
        assert_eq!(
            admit_schema(Some(48), Some(48), Some(49), opted_in),
            Err(SchemaError::DataDowngrade)
        );
    }
    assert!(admit_schema(Some(49), Some(49), Some(49), false).is_ok());
}

#[test]
fn ordinary_later_upgrade_preserves_pr_intent_and_exact_versions_still_pin() {
    let channel = Channel::parse("pr234").unwrap();
    let current = installed("5.0.0-alpha.1", channel);
    assert_eq!(
        select_upgrade(&current, None, None, false),
        Ok(UpgradeSelection::Fetch {
            channel,
            exact: None,
            pin: PinAction::Preserve,
        })
    );
    assert_eq!(
        select_upgrade(&current, None, Some("5.0.0-alpha.2"), false),
        Ok(UpgradeSelection::Fetch {
            channel,
            exact: Some("5.0.0-alpha.2".parse().unwrap()),
            pin: PinAction::PinCandidate,
        })
    );
    let mut pinned = current.clone();
    pinned.pinned_version = Some(pinned.version.clone());
    assert_eq!(
        select_upgrade(&pinned, None, None, false),
        Ok(UpgradeSelection::Pinned)
    );
    assert_eq!(
        select_upgrade(&pinned, Some(Channel::Alpha), None, false),
        Err(VersionError::Pinned)
    );
}

#[test]
fn missing_or_mismatched_pr_evidence_cannot_authorize_a_managed_install() {
    let channel = Channel::parse("pr234").unwrap();
    let version = "5.0.0-alpha.1".parse().unwrap();
    assert_eq!(
        plan_version(None, &version, channel, PinAction::Preserve),
        Err(VersionError::MissingPrEvidence)
    );
    let wrong = identity("235", 'a', 10);
    assert_eq!(
        plan_candidate_version(
            None,
            &version,
            channel,
            PinAction::Preserve,
            PrVersionContext {
                candidate: Some(&wrong),
                explicit_channel: true,
                ..Default::default()
            }
        ),
        Err(VersionError::MissingPrEvidence)
    );
    let candidate = identity("234", 'a', 10);
    assert_eq!(
        plan_candidate_version(
            None,
            &version,
            channel,
            PinAction::Preserve,
            PrVersionContext {
                candidate: Some(&candidate),
                ..Default::default()
            }
        ),
        Err(VersionError::ExplicitChannelRequired)
    );
}

#[test]
fn same_archived_version_may_switch_only_an_explicitly_named_different_channel() {
    let channel = Channel::parse("pr234").unwrap();
    let current = installed("5.0.0-alpha.1", Channel::Alpha);
    let candidate = identity("234", 'a', 10);
    let evidence = PrVersionContext {
        candidate: Some(&candidate),
        ..Default::default()
    };
    assert_eq!(
        plan_candidate_version(
            Some(&current),
            &current.version,
            channel,
            PinAction::Preserve,
            evidence
        ),
        Err(VersionError::ExplicitChannelRequired)
    );
    let plan = plan_candidate_version(
        Some(&current),
        &current.version,
        channel,
        PinAction::Preserve,
        PrVersionContext {
            explicit_channel: true,
            ..evidence
        },
    )
    .unwrap();
    assert!(plan.changed);
    assert_eq!(plan.state.version, current.version);
    assert_eq!(plan.state.channel, channel);
    assert!(
        plan_candidate_version(
            Some(&plan.state),
            &current.version,
            Channel::Alpha,
            PinAction::Preserve,
            PrVersionContext {
                current: Some(&candidate),
                explicit_channel: true,
                ..Default::default()
            }
        )
        .unwrap()
        .changed
    );
}

#[test]
fn a_newer_run_cannot_infer_same_version_refresh_or_change_an_immutable_identity() {
    let channel = Channel::parse("pr234").unwrap();
    let current = installed("5.0.0-alpha.1", channel);
    let previous = identity("234", 'a', 10);
    let next = identity("234", 'b', 11);
    for explicit_channel in [false, true] {
        assert_eq!(
            plan_candidate_version(
                Some(&current),
                &current.version,
                channel,
                PinAction::Preserve,
                PrVersionContext {
                    current: Some(&previous),
                    candidate: Some(&next),
                    explicit_channel
                }
            ),
            Err(VersionError::SameVersionPrCandidate)
        );
    }
    assert_eq!(
        plan_candidate_version(
            Some(&current),
            &"5.0.0-alpha.2".parse().unwrap(),
            channel,
            PinAction::Preserve,
            PrVersionContext {
                current: Some(&previous),
                candidate: Some(&previous),
                explicit_channel: false
            }
        ),
        Err(VersionError::InvalidSelection)
    );
    assert!(
        !plan_candidate_version(
            Some(&current),
            &current.version,
            channel,
            PinAction::Preserve,
            PrVersionContext {
                current: Some(&previous),
                candidate: Some(&previous),
                explicit_channel: false
            }
        )
        .unwrap()
        .changed
    );
}

#[test]
fn newer_version_and_run_advance_without_weakening_downgrade_or_build_metadata_guards() {
    let channel = Channel::parse("pr234").unwrap();
    let current = installed("5.0.0-alpha.2", channel);
    let previous = identity("234", 'a', 10);
    let next = identity("234", 'b', 11);
    let evidence = PrVersionContext {
        current: Some(&previous),
        candidate: Some(&next),
        explicit_channel: false,
    };
    assert!(
        plan_candidate_version(
            Some(&current),
            &"5.0.0-alpha.3".parse().unwrap(),
            channel,
            PinAction::Preserve,
            evidence
        )
        .unwrap()
        .changed
    );
    assert_eq!(
        plan_candidate_version(
            Some(&current),
            &"5.0.0-alpha.1".parse().unwrap(),
            channel,
            PinAction::Clear,
            evidence
        ),
        Err(VersionError::Downgrade)
    );
    assert_eq!(
        plan_candidate_version(
            Some(&current),
            &"5.0.0-alpha.2+different".parse().unwrap(),
            channel,
            PinAction::Preserve,
            evidence
        ),
        Err(VersionError::SameVersionPrCandidate)
    );
    let alpha = installed("5.0.0-alpha.2+one", Channel::Alpha);
    assert_eq!(
        plan_candidate_version(
            Some(&alpha),
            &"5.0.0-alpha.2+two".parse().unwrap(),
            channel,
            PinAction::Preserve,
            PrVersionContext {
                candidate: Some(&next),
                explicit_channel: true,
                ..Default::default()
            }
        ),
        Err(VersionError::EqualPrecedenceChange)
    );
}
