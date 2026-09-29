use semver::Version;

use super::*;

fn version(value: &str) -> Version {
    Version::parse(value).expect("test version must be valid")
}

#[test]
fn upgrade_intent_is_resolved_before_discovery() {
    let current = installed("5.0.0-alpha.2", Channel::Alpha, Some("5.0.0-alpha.2"));
    assert_eq!(
        select_upgrade(&current, None, None, false),
        Ok(UpgradeSelection::Pinned)
    );
    assert_eq!(
        select_upgrade(&current, Some(Channel::Stable), None, false),
        Err(VersionError::Pinned)
    );
    assert_eq!(
        select_upgrade(&current, None, Some("5.0.0-alpha.3"), true),
        Err(VersionError::InvalidSelection)
    );
    assert_eq!(
        select_upgrade(&current, None, Some("5.0.0-alpha.1"), false),
        Err(VersionError::Downgrade)
    );
    assert_eq!(
        select_upgrade(&current, None, Some("invalid"), false),
        Err(VersionError::InvalidSelection)
    );
    assert_eq!(
        select_upgrade(&current, None, Some("5.0.0-alpha.3"), false),
        Ok(UpgradeSelection::Fetch {
            channel: Channel::Alpha,
            exact: Some(version("5.0.0-alpha.3")),
            pin: PinAction::PinCandidate
        })
    );
    assert_eq!(
        select_upgrade(&current, None, None, true),
        Ok(UpgradeSelection::Fetch {
            channel: Channel::Alpha,
            exact: None,
            pin: PinAction::Clear
        })
    );
}

#[test]
fn latest_selection_is_channel_scoped_semantic_and_unambiguous() {
    let versions = ["5.0.0-alpha.2", "5.0.0-alpha.10", "5.0.0", "6.0.0-beta.1"].map(version);
    assert_eq!(
        latest_in_channel(&versions, Channel::Alpha).unwrap(),
        Some(&versions[1])
    );
    assert_eq!(
        latest_in_channel(&versions, Channel::Stable).unwrap(),
        Some(&versions[2])
    );
    assert_eq!(latest_in_channel(&[], Channel::Stable).unwrap(), None);
    assert_eq!(
        latest_in_channel(
            &[version("5.0.0+one"), version("5.0.0+two")],
            Channel::Stable
        ),
        Err(VersionError::EqualPrecedenceChange)
    );
}

fn installed(value: &str, channel: Channel, pinned: Option<&str>) -> InstalledVersion {
    InstalledVersion {
        version: version(value),
        channel,
        pinned_version: pinned.map(version),
    }
}

#[test]
fn first_install_and_explicit_pin_advancement_preserve_exact_state() {
    let candidate = version("5.0.0-alpha.10");
    for pin in [
        PinAction::Preserve,
        PinAction::Clear,
        PinAction::PinCandidate,
    ] {
        let plan = plan_version(None, &candidate, Channel::Alpha, pin).unwrap();
        assert!(plan.changed);
        assert_eq!(plan.state.version, candidate);
        assert_eq!(plan.state.channel, Channel::Alpha);
        assert_eq!(
            plan.state.pinned_version,
            (pin == PinAction::PinCandidate).then(|| candidate.clone())
        );
    }
    let current = installed("5.0.0-alpha.2", Channel::Alpha, Some("5.0.0-alpha.2"));
    for pin in [PinAction::Clear, PinAction::PinCandidate] {
        let plan = plan_version(Some(&current), &candidate, Channel::Alpha, pin).unwrap();
        assert!(plan.changed);
        assert_eq!(plan.state.version, candidate);
        assert_eq!(
            plan.state.pinned_version,
            (pin == PinAction::PinCandidate).then(|| candidate.clone())
        );
    }
}

#[test]
fn channel_names_and_version_families_are_explicit() {
    assert_eq!(Channel::ALL, [Channel::Stable, Channel::Alpha]);
    assert_eq!(Channel::Stable.as_str(), "stable");
    assert_eq!(Channel::Alpha.as_str(), "alpha");
    assert_eq!(Channel::parse("STABLE"), Some(Channel::Stable));
    assert_eq!(Channel::parse("Alpha"), Some(Channel::Alpha));
    assert_eq!(Channel::parse("beta"), None);
    assert_eq!(Channel::parse(" stable"), None);

    assert!(Channel::Stable.accepts(&version("1.2.3")));
    assert!(!Channel::Stable.accepts(&version("1.2.3-alpha.1")));
    assert!(!Channel::Stable.accepts(&version("1.2.3-beta.1")));
    assert!(Channel::Alpha.accepts(&version("1.2.3-alpha.1")));
    assert!(Channel::Alpha.accepts(&version("1.2.3-alpha.10")));
    assert!(!Channel::Alpha.accepts(&version("1.2.3")));
    assert!(!Channel::Alpha.accepts(&version("1.2.3-beta.1")));

    assert_eq!(
        plan_version(
            None,
            &version("1.2.3-alpha.1"),
            Channel::Stable,
            PinAction::Preserve,
        ),
        Err(VersionError::WrongChannel)
    );
    assert_eq!(
        plan_version(None, &version("1.2.3"), Channel::Alpha, PinAction::Preserve,),
        Err(VersionError::WrongChannel)
    );
}

#[test]
fn alpha_prerelease_numeric_order_is_semantic() {
    let current = installed("5.0.0-alpha.2", Channel::Alpha, None);
    let candidate = version("5.0.0-alpha.10");

    assert_eq!(
        candidate.cmp_precedence(&current.version),
        Ordering::Greater
    );
    assert_eq!(
        plan_version(
            Some(&current),
            &candidate,
            Channel::Alpha,
            PinAction::Preserve,
        ),
        Ok(VersionPlan {
            state: installed("5.0.0-alpha.10", Channel::Alpha, None),
            changed: true,
        })
    );
}

#[test]
fn downgrade_is_rejected_even_when_pin_action_is_explicit() {
    let current = installed("1.2.0", Channel::Stable, None);
    let candidate = version("1.1.9");

    for pin in [PinAction::PinCandidate, PinAction::Clear] {
        assert_eq!(
            plan_version(Some(&current), &candidate, Channel::Stable, pin),
            Err(VersionError::Downgrade),
            "pin action {pin:?} must not permit a downgrade",
        );
    }
}

#[test]
fn preserved_pin_rejects_a_different_candidate() {
    let current = installed("5.0.0-alpha.2", Channel::Alpha, Some("5.0.0-alpha.2"));
    let candidate = version("5.0.0-alpha.3");

    assert_eq!(
        plan_version(
            Some(&current),
            &candidate,
            Channel::Alpha,
            PinAction::Preserve,
        ),
        Err(VersionError::Pinned)
    );
}

#[test]
fn same_version_pin_changes_metadata_but_preserve_is_a_noop() {
    let candidate = version("1.2.3");
    let unpinned = installed("1.2.3", Channel::Stable, None);
    let pinned = installed("1.2.3", Channel::Stable, Some("1.2.3"));

    assert_eq!(
        plan_version(
            Some(&unpinned),
            &candidate,
            Channel::Stable,
            PinAction::PinCandidate,
        ),
        Ok(VersionPlan {
            state: pinned.clone(),
            changed: true,
        })
    );
    assert_eq!(
        plan_version(
            Some(&unpinned),
            &candidate,
            Channel::Stable,
            PinAction::Preserve,
        ),
        Ok(VersionPlan {
            state: unpinned.clone(),
            changed: false,
        })
    );
    assert_eq!(
        plan_version(Some(&pinned), &candidate, Channel::Stable, PinAction::Clear,),
        Ok(VersionPlan {
            state: unpinned,
            changed: true,
        })
    );
    assert_eq!(
        plan_version(
            Some(&pinned),
            &candidate,
            Channel::Stable,
            PinAction::Preserve,
        ),
        Ok(VersionPlan {
            state: pinned,
            changed: false,
        })
    );
}

#[test]
fn invalid_current_channel_or_pin_state_fails_before_candidate_policy() {
    let invalid_channel = installed("5.0.0-alpha.1", Channel::Stable, None);
    assert_eq!(
        plan_version(
            Some(&invalid_channel),
            &version("5.0.0"),
            Channel::Stable,
            PinAction::Clear,
        ),
        Err(VersionError::InvalidCurrentState)
    );

    let invalid_pin = installed("5.0.0", Channel::Stable, Some("5.0.1"));
    assert_eq!(
        plan_version(
            Some(&invalid_pin),
            &version("5.1.0"),
            Channel::Stable,
            PinAction::Clear,
        ),
        Err(VersionError::InvalidCurrentState)
    );
}

#[test]
fn build_metadata_only_change_is_rejected_by_precedence() {
    let current = installed("1.2.3+build.1", Channel::Stable, None);
    let candidate = version("1.2.3+build.2");

    assert_eq!(
        candidate.cmp_precedence(&current.version),
        Ordering::Equal,
        "build metadata must not affect release precedence",
    );
    assert_eq!(
        plan_version(
            Some(&current),
            &candidate,
            Channel::Stable,
            PinAction::Preserve,
        ),
        Err(VersionError::EqualPrecedenceChange)
    );
}

#[test]
fn explicit_unpinned_alpha_to_stable_forward_transition_is_allowed() {
    let current = installed("5.0.0-alpha.2", Channel::Alpha, None);
    let candidate = version("5.0.0");

    assert_eq!(
        plan_version(
            Some(&current),
            &candidate,
            Channel::Stable,
            PinAction::Preserve,
        ),
        Ok(VersionPlan {
            state: installed("5.0.0", Channel::Stable, None),
            changed: true,
        })
    );
}

#[test]
fn squad_is_a_fixed_extension_product_with_two_command_links() {
    use super::Product;
    assert_eq!(
        Product::ALL,
        [Product::Cli, Product::Office, Product::Squad]
    );
    let squad = Product::parse("squad").unwrap();
    assert_eq!(
        (
            squad.tag_prefix(),
            squad.executable(),
            squad.package(),
            squad.namespace(),
            squad.links(),
        ),
        (
            "tmt-squad-v",
            "tmt-squad",
            "tmt-squad",
            "lib/tmt-squad",
            &["tmt-squad", "tmt-sq"][..],
        )
    );
    assert_eq!(squad.link_target(), "../lib/tmt-squad/current/tmt-squad");
    // Tag prefixes never overlap, so release selection stays per product.
    for product in Product::ALL {
        for other in Product::ALL.into_iter().filter(|other| *other != product) {
            assert!(
                !other.tag_prefix().starts_with(product.tag_prefix()),
                "{product:?} vs {other:?}"
            );
        }
    }
}

#[test]
fn the_cli_accepts_either_prerelease_flag_on_an_alpha_and_extensions_match_exactly() {
    use super::Product;
    let alpha = version("5.0.0-alpha.7");
    let stable = version("5.0.0");
    // The CLI's current policy (a normal release) and its flagged history.
    assert!(Product::Cli.accepts_prerelease_flag(&alpha, false));
    assert!(Product::Cli.accepts_prerelease_flag(&alpha, true));
    assert!(Product::Cli.accepts_prerelease_flag(&stable, false));
    assert!(!Product::Cli.accepts_prerelease_flag(&stable, true));
    for extension in [Product::Office, Product::Squad] {
        assert!(extension.accepts_prerelease_flag(&alpha, true));
        assert!(!extension.accepts_prerelease_flag(&alpha, false));
        assert!(extension.accepts_prerelease_flag(&stable, false));
        assert!(!extension.accepts_prerelease_flag(&stable, true));
    }
}
