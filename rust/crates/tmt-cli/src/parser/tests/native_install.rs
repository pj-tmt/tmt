use super::*;

#[test]
fn native_upgrade_alias_and_selection_share_one_typed_contract() {
    for command in ["upgrade", "update"] {
        let invocation = parsed(&[
            command,
            "--channel",
            "alpha",
            "--to",
            "5.0.0-alpha.3",
            "--json",
            "--yes",
        ]);
        assert!(invocation.mode.json);
        assert_eq!(
            invocation.invocation,
            Invocation::Upgrade {
                channel: Some(tmt_core::native_install::Channel::Alpha),
                exact: Some("5.0.0-alpha.3".into()),
                unpin: false,
                yes: true,
            }
        );
        assert_eq!(
            parsed(&[command, "--unpin"]).invocation,
            Invocation::Upgrade {
                channel: None,
                exact: None,
                unpin: true,
                yes: false
            }
        );
        assert_eq!(
            parse_error(&[command, "--to", "5.0.0", "--unpin", "--json"]).code,
            "USAGE_ERROR"
        );
        assert_eq!(
            parse_error(&[command, "--channel", "beta", "--json"]).code,
            "USAGE_ERROR"
        );
    }
}

#[test]
fn internal_native_install_requires_explicit_inputs_and_typed_pin_policy() {
    use tmt_core::native_install::{Channel, PinAction};
    let input = [
        "__native-install",
        "--archive",
        "archive.tar.gz",
        "--manifest",
        "manifest.json",
        "--prefix",
        "/prefix with spaces",
        "--channel",
        "alpha",
        "--json",
    ];
    let parsed = parsed(&input);
    assert!(parsed.mode.json);
    assert_eq!(
        parsed.invocation,
        Invocation::NativeInstall {
            product: tmt_core::native_install::Product::Cli,
            archive: "archive.tar.gz".into(),
            manifest: "manifest.json".into(),
            prefix: "/prefix with spaces".into(),
            channel: Channel::Alpha,
            pin: PinAction::Preserve,
        }
    );
    for pin in ["--pin", "--unpin"] {
        let mut args = input.to_vec();
        args.push(pin);
        let result = super::parse(&self::args(&args)).unwrap();
        assert!(
            matches!(result.invocation, Invocation::NativeInstall { pin: actual, .. }
            if actual == if pin == "--pin" { PinAction::PinCandidate } else { PinAction::Clear })
        );
    }
    for product in [
        tmt_core::native_install::Product::Office,
        tmt_core::native_install::Product::Remote,
        tmt_core::native_install::Product::Colab,
    ] {
        let mut selected = input.to_vec();
        selected.extend(["--product", product.as_str()]);
        assert!(
            matches!(super::parse(&self::args(&selected)).unwrap().invocation,
            Invocation::NativeInstall { product: actual, .. } if actual == product)
        );
    }
    let mut invalid_product = input.to_vec();
    invalid_product.extend(["--product", "third-party"]);
    assert_eq!(parse_error(&invalid_product).code, "USAGE_ERROR");
    let mut conflict = input.to_vec();
    conflict.extend(["--pin", "--unpin"]);
    assert_eq!(parse_error(&conflict).code, "USAGE_ERROR");
    assert_eq!(parse_error(&["__native-install"]).code, "USAGE_ERROR");
    let help = crate::grammar::public_grammar(&crate::grammar::grammar(), true);
    assert!(help.find_subcommand("__native-install").is_none());
}

#[test]
fn internal_extension_upgrade_requires_plan_or_consent() {
    assert_eq!(
        parsed(&["__native-upgrade-extensions", "--plan", "--json"]).invocation,
        Invocation::NativeUpgradeExtensions { plan: true }
    );
    assert_eq!(
        parsed(&["__native-upgrade-extensions", "--yes", "--json"]).invocation,
        Invocation::NativeUpgradeExtensions { plan: false }
    );
}

#[test]
fn internal_extension_upgrade_is_hidden_and_rejects_ambiguous_modes() {
    assert_eq!(
        parse_error(&["__native-upgrade-extensions"]).code,
        "USAGE_ERROR"
    );
    assert_eq!(
        parse_error(&["__native-upgrade-extensions", "--plan", "--yes"]).code,
        "USAGE_ERROR"
    );
    let help = crate::grammar::public_grammar(&crate::grammar::grammar(), true);
    assert!(
        help.find_subcommand("__native-upgrade-extensions")
            .is_none()
    );
}

#[test]
fn versioned_installer_handoff_is_disjoint_from_offline_arguments() {
    for probe in [false, true] {
        let mut args = vec!["__native-install", "--handoff-version", "1", "--json"];
        if probe {
            args.push("--probe");
        }
        assert_eq!(
            parsed(&args).invocation,
            Invocation::NativeInstallHandoff { probe }
        );
    }
    for args in [
        vec![
            "__native-install",
            "--handoff-version",
            "2",
            "--probe",
            "--json",
        ],
        vec!["__native-install", "--probe", "--json"],
        vec![
            "__native-install",
            "--handoff-version",
            "1",
            "--archive",
            "archive.tar.gz",
        ],
        vec!["__native-install", "--handoff-version", "1", "--pin"],
        vec![
            "__native-install",
            "--handoff-version",
            "1",
            "--product",
            "office",
        ],
    ] {
        assert_eq!(parse_error(&args).code, "USAGE_ERROR");
    }
}

#[test]
fn managed_skill_refresh_is_explicit() {
    for (args, managed) in [
        (vec!["__native-refresh-skills", "--json"], false),
        (vec!["__native-refresh-skills", "--managed", "--json"], true),
    ] {
        assert_eq!(
            parsed(&args).invocation,
            Invocation::NativeRefreshSkills { managed }
        );
    }
}
