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
    (
        &["office", "prop", "remove"],
        &["office", "prop", "rm"],
        &["--local", "sha256:pack", "--if-revision", "1"],
    ),
    (
        &["office", "prop", "list"],
        &["office", "prop", "ls"],
        &["--local", "--limit", "3", "--cursor", "next"],
    ),
    (
        &["office", "avatar", "remove"],
        &["office", "avatar", "rm"],
        &["--local", "sha256:avatar", "--if-revision", "2"],
    ),
    (
        &["office", "avatar", "list"],
        &["office", "avatar", "ls"],
        &["--local", "--limit", "4", "--cursor", "next"],
    ),
    (
        &["office", "board", "list"],
        &["office", "board", "ls"],
        &["--room", "reviewers", "--limit", "5"],
    ),
    (
        &["office", "board", "delete"],
        &["office", "board", "rm"],
        &[
            "entry",
            "--owner",
            "--if-revision",
            "3",
            "--operation-id",
            "operation",
        ],
    ),
    (&["office", "uninstall"], &["office", "rm"], &["--yes"]),
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
fn office_aliases_preserve_direct_and_embedded_dispatch() {
    for &(old, primary, operands) in CASES
        .iter()
        .filter(|(old, _, _)| old.first() == Some(&"office"))
    {
        let direct = |path: &[&str]| {
            tmt_office_command::parser::parse_public(&args(
                &[&path[1..], operands, &["--json", "--prefix", "/isolated"]].concat(),
            ))
            .unwrap()
        };
        let expected = direct(old);
        assert_eq!(direct(primary), expected);
        let embedded = parsed(&[primary, operands, &["--json", "--prefix", "/isolated"]].concat());
        assert_eq!(
            embedded.invocation,
            Invocation::Office {
                prefix: expected.prefix,
                operation: expected.operation
            }
        );
        assert!(embedded.mode.json);
    }
}
