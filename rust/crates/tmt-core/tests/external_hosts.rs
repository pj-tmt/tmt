//! Registering external hosts is once per process, so it has a test binary
//! of its own: nothing else here sees a registered host.

use tmt_core::{
    host::{AlreadyRegistered, HostGrammar, HostKind, MAX_EXTERNAL_HOSTS, register_external_hosts},
    identity::{Identity, IdentityReader, Lifetime, addresses_pane, find_by_name},
    names::{is_pane_target, names_pane_target, validate_existing_name, validate_name},
};

fn grammar(name: &str, prefix: &str, target: Option<&str>) -> HostGrammar {
    HostGrammar::new(name, prefix, target).unwrap()
}

/// Storage that holds at most one identity.
struct Identities(Option<Identity>);

impl IdentityReader for Identities {
    type Error = std::convert::Infallible;

    fn find_identity(&self, canonical_name: &str) -> Result<Option<Identity>, Self::Error> {
        Ok(self
            .0
            .clone()
            .filter(|identity| identity.canonical_name == canonical_name))
    }

    fn list_identities(&self) -> Result<Vec<Identity>, Self::Error> {
        Ok(self.0.clone().into_iter().collect())
    }
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

    // Herdr numbers with digits and uppercase letters. A target is read with
    // its case kept: lowercase letters make a name, on any host.
    assert!(herdr.is_target("w1:p2E") && !herdr.is_target("w1:p2e"));
    assert!(is_pane_target("w1:p2E") && is_pane_target(" ｗ１:ｐ２Ｅ "));
    assert!(validate_name("w1:p2E").is_err());
    for name in ["w1:p2e", "W1:P2E", "web:prod", "sol", "fred"] {
        assert!(!is_pane_target(name), "{name}");
    }
    for name in ["w1:p2e", "web:prod", "sol", "fred"] {
        assert!(validate_name(name).is_ok(), "{name}");
    }
    // A name is found by its key, so one whose key is a target is refused
    // although its own text is not one.
    assert!(!is_pane_target("W1:P2") && names_pane_target("W1:P2"));
    assert!(validate_name("W1:P2").is_err());
    // tmux's syntax does not depend on case.
    assert!(is_pane_target("Work:1.2") && validate_name("Work:1.2").is_err());
    // An identity that already holds the key of a lettered target keeps
    // answering for it; without one the text addresses the pane.
    assert!(addresses_pane(&Identities(None), "w1:p2E").unwrap());
    let holder = Identities(Some(Identity {
        id: "identity-1".into(),
        name: "w1:p2e".into(),
        canonical_name: "w1:p2e".into(),
        lifetime: Lifetime::Saved,
        created_at: "2026-09-13T00:00:00Z".into(),
        updated_at: "2026-09-13T00:00:00Z".into(),
    }));
    assert!(!addresses_pane(&holder, "w1:p2E").unwrap());
    let found = find_by_name(&holder, "w1:p2E").unwrap();
    assert_eq!(
        found.map(|identity| identity.id).as_deref(),
        Some("identity-1")
    );
    assert_eq!(
        validate_existing_name("w1:p2E").unwrap().canonical_name(),
        "w1:p2e"
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
