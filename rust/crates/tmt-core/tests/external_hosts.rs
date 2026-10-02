//! Registering external hosts is once per process, so it has a test binary
//! of its own: nothing else here sees a registered host.

use tmt_core::{
    host::{AlreadyRegistered, HostGrammar, HostKind, MAX_EXTERNAL_HOSTS, register_external_hosts},
    names::{validate_existing_name, validate_name},
};

fn grammar(name: &str, prefix: &str, target: Option<&str>) -> HostGrammar {
    HostGrammar::new(name, prefix, target).unwrap()
}

#[test]
fn approved_hosts_are_registered_once_and_parse_as_external() {
    let mut grammars = vec![
        grammar("fake", "fake-", Some("f{n}")),
        // Herdr was built in until #1082; its driver now declares its syntax.
        grammar("herdr", "term_", Some("w{n}:p{n}")),
        // Skipped: a built-in's name, then the same prefix as `fake`.
        grammar("tmux", "tm-", None),
        grammar("copy", "fake-", None),
    ];
    grammars.extend((0..20).map(|n| grammar(&format!("extra{n}"), &format!("x{n}-"), None)));
    let registered: Vec<String> = register_external_hosts(grammars)
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(registered.len(), MAX_EXTERNAL_HOSTS);
    assert_eq!(registered[..2], ["fake", "herdr"]);
    assert!(
        !registered
            .iter()
            .any(|name| name == "tmux" || name == "copy")
    );
    assert_eq!(registered.last().map(String::as_str), Some("extra13"));

    let fake = HostKind::parse("fake").unwrap();
    assert!(matches!(fake, HostKind::External(_)));
    assert_eq!(HostKind::of_pane_id("fake-1"), Some(fake));
    assert_eq!(HostKind::label("fake-1", Some("f3")), "f3");
    assert_eq!(HostKind::parse("tmux"), Some(HostKind::Tmux));
    // Past the limit: the name still reads, but no syntax is its own.
    let skipped = HostKind::parse("extra15").unwrap();
    assert!(!skipped.is_pane_id("x15-1"));
    assert_eq!(HostKind::all().count(), 1 + MAX_EXTERNAL_HOSTS);

    let herdr = HostKind::parse("herdr").unwrap();
    assert_eq!(HostKind::of_pane_id("term_65ca1161edc141"), Some(herdr));
    assert!(herdr.is_target("w1:p2") && !herdr.is_target("term_1"));
    assert_eq!(HostKind::label("term_1", Some("w1:p2")), "w1:p2");
    // As before #1082: no new identity takes such a name, an earlier one
    // keeps it.
    assert!(validate_name("w1:p2").is_err());
    assert_eq!(
        validate_existing_name("W1:P2").unwrap().canonical_name(),
        "w1:p2"
    );

    // Its targets are no new identity's name, but an earlier holder keeps one.
    assert!(validate_name("f1").is_err());
    assert!(validate_existing_name("f1").is_ok());

    // The hosts are fixed for the process: a second registration fails.
    assert_eq!(
        register_external_hosts(vec![grammar("late", "lt-", None)]),
        Err(AlreadyRegistered)
    );
    assert!(!HostKind::parse("late").unwrap().is_pane_id("lt-1"));
    assert!(HostKind::parse("fake").unwrap().is_pane_id("fake-1"));
}
