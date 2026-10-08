use super::*;

#[test]
fn focus_callback_is_hidden_and_does_not_enter_human_output_or_drift() {
    let argv = [
        "__focus-hook",
        "claude",
        "--launch",
        "{}",
        "--worker",
        "--work-budget-ms",
        "999",
    ];
    assert_eq!(
        parsed(&argv).invocation,
        Invocation::FocusHook {
            provider: "claude".into(),
            launch: "{}".into(),
            worker: true,
            work_budget_ms: Some(999)
        }
    );
    assert!(!crate::skill_reminder::eligible_for_drift(&parsed(&argv)));
    assert!(
        crate::grammar::grammar()
            .find_subcommand("__focus-hook")
            .unwrap()
            .is_hide_set()
    );
    assert!(parse(&args(&["__focus-hook", "claude"])).is_err());
    assert!(
        parse(&args(&[
            "__focus-hook",
            "claude",
            "--launch",
            "{}",
            "--activity-only"
        ]))
        .is_err()
    );
    assert!(matches!(
        parsed(&["__hook", "claude", "--activity-only"]).invocation,
        Invocation::ProviderHook {
            activity_only: true,
            ..
        }
    ));
}

#[test]
fn setup_has_one_provider_consent_surface_and_hook_dispatch_is_hidden() {
    assert_eq!(
        parsed(&["setup"]).invocation,
        Invocation::Setup {
            provider: None,
            status: false,
            remove: false,
            usage: UsageHook::Default,
            yes: false,
        }
    );
    assert_eq!(
        parsed(&["setup", "claude", "--remove", "--yes", "--json"]).invocation,
        Invocation::Setup {
            provider: Some("claude".into()),
            status: false,
            remove: true,
            usage: UsageHook::Default,
            yes: true,
        }
    );
    // Guided setup takes --yes for its one plan; --remove needs a driver.
    assert_eq!(
        parsed(&["setup", "--yes"]).invocation,
        Invocation::Setup {
            provider: None,
            status: false,
            remove: false,
            usage: UsageHook::Default,
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
            activity_only: false,
            provider: "claude".into(),
            worker: false,
            work_budget_ms: None
        }
    );
    assert_eq!(
        parsed(&["__hook", "codex"]).invocation,
        Invocation::ProviderHook {
            activity_only: false,
            provider: "codex".into(),
            worker: false,
            work_budget_ms: None
        }
    );
    assert_eq!(
        parsed(&["setup", "codex", "--yes"]).invocation,
        Invocation::Setup {
            provider: Some("codex".into()),
            status: false,
            remove: false,
            usage: UsageHook::Default,
            yes: true
        }
    );
    assert!(!crate::skill_reminder::eligible_for_drift(&parsed(&[
        "__hook", "claude"
    ])));
    let grammar = crate::grammar::grammar();
    assert!(grammar.find_subcommand("__hook").unwrap().is_hide_set());
}

#[test]
fn setup_status_is_read_only_and_conflicts_with_mutations() {
    assert!(matches!(
        parsed(&["setup", "codex", "--status", "--json"]).invocation,
        Invocation::Setup { status: true, .. }
    ));
    for flag in ["--yes", "--usage", "--no-usage", "--remove"] {
        assert!(parse(&args(&["setup", "claude", "--status", flag])).is_err());
    }
}

#[test]
fn private_hook_worker_budget_is_typed_bounded_and_requires_worker() {
    for budget in ["0", "1", "2000"] {
        assert_eq!(
            parsed(&["__hook", "codex", "--worker", "--work-budget-ms", budget]).invocation,
            Invocation::ProviderHook {
                activity_only: false,
                provider: "codex".into(),
                worker: true,
                work_budget_ms: Some(budget.parse().unwrap())
            }
        );
    }
    for budget in ["", "-1", "+1", "1.5", "1s", "2001", "18446744073709551616"] {
        assert!(
            parse(&args(&[
                "__hook",
                "codex",
                "--worker",
                "--work-budget-ms",
                budget
            ]))
            .is_err()
        );
    }
    assert!(parse(&args(&["__hook", "codex", "--work-budget-ms", "1"])).is_err());
    assert_eq!(
        parsed(&["__hook", "claude", "--worker"]).invocation,
        Invocation::ProviderHook {
            activity_only: false,
            provider: "claude".into(),
            worker: true,
            work_budget_ms: None
        }
    );
}
