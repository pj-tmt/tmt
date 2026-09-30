use super::*;

#[test]
fn setup_has_one_provider_consent_surface_and_hook_dispatch_is_hidden() {
    assert_eq!(
        parsed(&["setup"]).invocation,
        Invocation::Setup {
            provider: None,
            remove: false,
            usage: UsageHook::Keep,
            yes: false,
        }
    );
    assert_eq!(
        parsed(&["setup", "claude", "--remove", "--yes", "--json"]).invocation,
        Invocation::Setup {
            provider: Some("claude".into()),
            remove: true,
            usage: UsageHook::Keep,
            yes: true,
        }
    );
    // Guided setup takes --yes for its one plan; --remove needs a driver.
    assert_eq!(
        parsed(&["setup", "--yes"]).invocation,
        Invocation::Setup {
            provider: None,
            remove: false,
            usage: UsageHook::Keep,
            yes: true,
        }
    );
    // The usage hook is an explicit opt-in or opt-out, never with --remove.
    for (argv, usage) in [
        (vec!["setup", "--usage"], UsageHook::Install),
        (
            vec!["setup", "codex", "--usage", "--yes"],
            UsageHook::Install,
        ),
        (vec!["setup", "claude", "--no-usage"], UsageHook::Remove),
    ] {
        assert!(matches!(
            parsed(&argv).invocation,
            Invocation::Setup { usage: parsed, .. } if parsed == usage
        ));
    }
    for argv in [
        vec!["setup", "--remove"],
        vec!["setup", "unknown"],
        vec!["setup", "claude", "--force"],
        vec!["setup", "--usage", "--no-usage"],
        vec!["setup", "claude", "--remove", "--usage"],
        vec!["setup", "claude", "--remove", "--no-usage"],
    ] {
        assert!(parse(&args(&argv)).is_err());
    }
    assert_eq!(
        parsed(&["__hook", "claude"]).invocation,
        Invocation::ProviderHook {
            provider: "claude".into(),
            worker: false
        }
    );
    assert_eq!(
        parsed(&["__hook", "codex"]).invocation,
        Invocation::ProviderHook {
            provider: "codex".into(),
            worker: false
        }
    );
    assert_eq!(
        parsed(&["setup", "codex", "--yes"]).invocation,
        Invocation::Setup {
            provider: Some("codex".into()),
            remove: false,
            usage: UsageHook::Keep,
            yes: true
        }
    );
    assert!(!crate::skill_reminder::eligible_for_drift(&parsed(&[
        "__hook", "claude"
    ])));
    let grammar = crate::grammar::grammar();
    assert!(grammar.find_subcommand("__hook").unwrap().is_hide_set());
}
