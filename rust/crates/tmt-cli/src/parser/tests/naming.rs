use super::*;

// Each changed public spelling reaches the same typed dispatch request, with
// identical payload/options and JSON mode. Existing operation tests own effects.
const CASES: &[(&[&str], &[&str], &[&str])] = &[
    (&["list"], &["ls"], &[]),
    (&["rename"], &["mv"], &["worker", "reviewer"]),
    (&["room", "list"], &["room", "ls"], &[]),
    (&["room", "retire"], &["room", "rm"], &["reviewers"]),
    (
        &["extension", "uninstall"],
        &["extension", "rm"],
        &["squad", "--yes", "--prefix", "/isolated"],
    ),
    (&["extension", "list"], &["extension", "ls"], &[]),
    (
        &["extension", "hooks", "list"],
        &["extension", "hooks", "ls"],
        &[],
    ),
    (&["config", "clear"], &["config", "rm"], &["timeout"]),
    (&["preamble", "clear"], &["preamble", "rm"], &["worker"]),
    (&["x", "list"], &["x", "ls"], &[]),
    (
        &["identity", "rename"],
        &["identity", "mv"],
        &["worker", "reviewer"],
    ),
    (&["identity", "list"], &["identity", "ls"], &[]),
    (
        &["identity", "status", "clear"],
        &["identity", "status", "rm"],
        &["--identity", "worker"],
    ),
    (
        &["identity", "meta", "get"],
        &["identity", "meta", "show"],
        &["team", "--identity", "worker"],
    ),
    (
        &["identity", "meta", "list"],
        &["identity", "meta", "ls"],
        &[],
    ),
    (
        &["role", "clear"],
        &["role", "rm"],
        &["--identity", "worker"],
    ),
    (&["remove"], &["rm"], &["worker", "--force"]),
];

#[test]
fn concise_commands_and_long_aliases_preserve_dispatch_and_errors() {
    for &(old, primary, operands) in CASES {
        for json in [false, true] {
            let suffix: &[&str] = if json { &["--json"] } else { &[] };
            let words = |path: &[&str]| args(&[path, operands, suffix].concat());
            let expected = parse(&words(old)).unwrap();
            assert!(!matches!(expected.invocation, Invocation::Extension { .. }));
            assert_eq!(
                parse(&words(primary)).unwrap(),
                expected,
                "{old:?} -> {primary:?}"
            );
            let bad = |path: &[&str]| [words(path), vec![OsString::from("--absent")]].concat();
            let old_error = parse(&bad(old)).unwrap_err();
            let new_error = parse(&bad(primary)).unwrap_err();
            assert_eq!(old_error.code, new_error.code);
            assert_eq!(old_error.mode, new_error.mode);
            assert_eq!(new_error.code, "USAGE_ERROR");
        }
    }
}

#[test]
fn alias_help_always_uses_the_primary_spelling() {
    for &(old, primary, _) in CASES {
        let help = |path: &[&str]| {
            let path = path
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>();
            let command = crate::grammar::help_command(&path).unwrap();
            tmt_cli_style::help_text(&command, tmt_cli_style::Terminal::PLAIN)
        };
        let text = help(primary);
        assert_eq!(help(old), text, "{old:?}");
        assert!(text.contains(&format!("Usage: tmt {}", primary.join(" "))));
        assert!(!text.contains(&format!("tmt {}", old.join(" "))));
    }
}

#[test]
fn mv_is_reserved_before_external_dispatch_and_uninstall_remains_distinct() {
    for words in [
        vec!["mv", "worker", "reviewer"],
        vec!["--json", "mv", "worker", "reviewer"],
        vec!["help", "mv"],
        vec!["mv", "--help"],
        vec!["rename", "worker", "reviewer"],
    ] {
        assert!(
            crate::grammar::extensions::candidate(&crate::grammar::grammar(), &args(&words))
                .is_none()
        );
        assert!(!matches!(
            parsed(&words).invocation,
            Invocation::Extension { .. }
        ));
    }
    assert_eq!(
        parsed(&["mv", "worker", "reviewer"]).invocation,
        Invocation::Rename {
            old: "worker".into(),
            new: "reviewer".into()
        }
    );
    assert!(crate::grammar::extensions::reserved(&crate::grammar::grammar()).contains("mv"));
    assert!(
        crate::grammar::grammar()
            .find_subcommand("uninstall")
            .is_some()
    );
    assert_eq!(
        parsed(&["uninstall", "--yes"]).invocation,
        Invocation::Uninstall {
            purge: false,
            yes: true,
            prefix: None
        }
    );
}

#[test]
fn discover_and_guarded_add_use_one_binder_and_validate_selectors_as_usage_errors() {
    use crate::invocation::Invocation;
    let digest = "a".repeat(64);
    let parsed = super::parsed(&["discover", "--host", "herdr", "--json"]);
    assert!(matches!(parsed.invocation, Invocation::Discover { .. }));
    let parsed = super::parsed(&[
        "add", "%3", "worker", "--expect", &digest, "--save", "--socket", "/tmp/a",
    ]);
    assert!(matches!(
        parsed.invocation,
        Invocation::GuardedBind {
            save: true,
            identity: None,
            ..
        }
    ));
    let uuid = "123e4567-e89b-42d3-a456-426614174000";
    let parsed = super::parsed(&["add", "%3", "--identity", uuid, "--expect", &digest]);
    assert!(matches!(
        parsed.invocation,
        Invocation::GuardedBind {
            name: None,
            identity: Some(_),
            ..
        }
    ));
    for args in [
        vec!["discover", "scan"],
        vec!["discover", "adopt"],
        vec![
            "add",
            "%3",
            "worker",
            "--identity",
            uuid,
            "--expect",
            &digest,
        ],
        vec!["add", "%3", "--identity", uuid],
        vec!["add", "%3", "worker", "--expect", "bad"],
        vec!["add", "window:1", "worker", "--expect", &digest],
        vec!["add", "%3", "--identity", "worker", "--expect", &digest],
    ] {
        let error = parse_error(&args);
        assert_eq!(error.code, "USAGE_ERROR");
    }
}
