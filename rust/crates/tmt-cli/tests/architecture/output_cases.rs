//! Adversarial and positive examples for the #436 output guard.

use super::{
    output::{Exact, violations},
    source::Source,
};

fn syntax(package: &str, file: &str, text: &str) -> Source {
    Source {
        package: package.into(),
        file: file.into(),
        syntax: syn::parse_file(text)
            .unwrap_or_else(|error| panic!("fixture {package}/{file} must parse: {error}")),
    }
}

fn check(sources: &[Source], exact: &[Exact], migrating: &[(&str, &str)]) -> Vec<String> {
    violations(sources, exact, migrating)
}

#[test]
fn print_macros_and_raw_standard_streams_bypass_the_style_layer() {
    let text = r#"
        use std::io::{self, stderr, Write};
        fn list() { println!("x"); eprint!("y"); }
        fn show() { let _ = io::stdout().lock(); }
        fn warn() { let _ = std::io::stderr(); }
        fn wrapped() { let _ = writeln!(io::stdout(), "z"); }
    "#;
    assert_eq!(
        check(&[syntax("tmt-cli", "list_command.rs", text)], &[], &[]),
        [
            "tmt-cli/list_command.rs: use io::stderr in <module> bypasses the style layer",
            "tmt-cli/list_command.rs: eprint! in list bypasses the style layer",
            "tmt-cli/list_command.rs: println! in list bypasses the style layer",
            "tmt-cli/list_command.rs: io::stdout in show bypasses the style layer",
            "tmt-cli/list_command.rs: io::stderr in warn bypasses the style layer",
            "tmt-cli/list_command.rs: io::stdout in wrapped bypasses the style layer",
        ]
    );
}

#[test]
fn escape_sequences_are_caught_in_literals_and_macro_arguments() {
    let text = r#"
        const RED: &str = "\x1b[31m";
        fn bold() -> String { format!("\u{1b}[1m{}", 1) }
        fn bytes() -> &'static [u8] { b"\x1b[0m" }
        fn one() -> char { '\u{1b}' }
        #[cfg(test)]
        mod tests { fn fixture() -> &'static str { "\x1b[2J" } }
    "#;
    assert_eq!(
        check(&[syntax("tmt-ops", "status.rs", text)], &[], &[]),
        [
            "tmt-ops/status.rs: an escape sequence literal in <module> bypasses the style layer",
            "tmt-ops/status.rs: an escape sequence literal in bold bypasses the style layer",
            "tmt-ops/status.rs: an escape sequence literal in bytes bypasses the style layer",
            "tmt-ops/status.rs: an escape sequence literal in one bypasses the style layer",
        ]
    );
}

#[test]
fn only_the_cli_crates_are_guarded() {
    let text = r#"fn render() { println!("x"); let _ = std::io::stdout(); }"#;
    for package in [
        "tmt-cli-style",
        "tmt-command-output",
        "tmt-adapters",
        "tmt-office",
    ] {
        assert_eq!(
            check(&[syntax(package, "lib.rs", text)], &[], &[]),
            [] as [String; 0]
        );
    }
    for package in ["tmt-cli", "tmt-office-command", "tmt-ops"] {
        assert_eq!(check(&[syntax(package, "lib.rs", text)], &[], &[]).len(), 2);
    }
}

#[test]
fn an_exact_body_exempts_only_its_own_function() {
    let exact = [Exact {
        package: "tmt-cli",
        file: "response_command.rs",
        function: "result",
        reason: "the stored response body, byte for byte",
    }];
    let clean = r#"fn result() { let _ = std::io::stdout(); }"#;
    assert_eq!(
        check(
            &[syntax("tmt-cli", "response_command.rs", clean)],
            &exact,
            &[]
        ),
        [] as [String; 0]
    );
    let stray = r#"
        fn result() { let _ = std::io::stdout(); }
        fn reply() { println!("sent"); }
    "#;
    assert_eq!(
        check(
            &[syntax("tmt-cli", "response_command.rs", stray)],
            &exact,
            &[]
        ),
        ["tmt-cli/response_command.rs: println! in reply bypasses the style layer"]
    );
}

#[test]
fn stale_or_unexplained_exemptions_fail() {
    let exact = [Exact {
        package: "tmt-cli",
        file: "response_command.rs",
        function: "result",
        reason: " ",
    }];
    assert_eq!(
        check(
            &[syntax("tmt-cli", "response_command.rs", "fn result() {}")],
            &exact,
            &[]
        ),
        [
            "tmt-cli/response_command.rs: exact-body exemption for result needs a reason",
            "tmt-cli/response_command.rs: exact-body exemption for result matches no output",
        ]
    );
}

#[test]
fn the_migration_list_holds_exactly_the_files_still_bypassing() {
    let bypassing = syntax(
        "tmt-cli",
        "room_command.rs",
        r#"fn run() { println!("x"); }"#,
    );
    let migrated = syntax("tmt-cli", "notes_command.rs", "fn run() {}");
    let listed = [
        ("tmt-cli", "room_command.rs"),
        ("tmt-cli", "notes_command.rs"),
    ];
    assert_eq!(
        check(&[bypassing, migrated], &[], &listed),
        [
            "tmt-cli/notes_command.rs: output goes through the style layer; remove it from the migration list"
        ]
    );
}
