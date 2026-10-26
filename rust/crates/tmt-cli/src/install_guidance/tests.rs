use super::*;
use serde_json::json;

fn row(name: &str, current: bool) -> Value {
    json!({"name": name, "installed": true, "version": "0.1.0-alpha.1", "missing": false,
        "onPath": true, "shadowedBy": [], "dependencies": if name == "colab" { vec!["remote"] } else { vec![] }, "update": if current { "current" } else { "unknown" }})
}
fn actions(rows: &[Value], confirmed: &[Value]) -> Guidance {
    project_with_agent(rows, confirmed, Vec::new(), None, Some("codex"))
}
fn commands(guidance: &Guidance) -> Vec<String> {
    guidance
        .actions
        .iter()
        .map(|action| command(&action.argv))
        .collect()
}

#[test]
fn fresh_machine_has_one_discovery_and_one_install_per_missing_product() {
    let rows = crate::extension_install_command::setup::PRODUCTS
        .iter()
        .map(|p| json!({"name":p.as_str(), "installed":false,"missing":true,"shadowedBy":[]}))
        .collect::<Vec<_>>();
    let guidance = project_with_agent(&rows, &[], Vec::new(), None, None);
    assert_eq!(
        commands(&guidance),
        [
            "tmt setup",
            "tmt extension install ops --yes",
            "tmt extension install remote --yes",
            "tmt extension install colab --yes",
            "tmt extension install digest --yes"
        ]
    );
    assert_eq!(
        guidance
            .actions
            .iter()
            .filter(|a| a.kind == Kind::Install)
            .count(),
        4
    );
    assert!(guidance.missing("ops"));
    let document = guidance.document();
    assert_eq!(
        document["actions"][1],
        json!({"id":"install-ops","kind":"install","argv":["tmt","extension","install","ops","--yes"],"description":"Add the terminal workspace"})
    );
}

#[test]
fn known_current_commands_start_the_space_before_pairing_and_stay_within_five() {
    let rows = [
        row("ops", true),
        row("remote", true),
        row("colab", true),
        row("digest", true),
    ];
    let guidance = actions(&rows, &[]);
    assert_eq!(
        commands(&guidance),
        [
            "tmt run codex",
            "tmt ops ui",
            "tmt colab serve",
            "tmt remote pair",
            "tmt remote deploy firestore --help"
        ]
    );
    assert!(
        guidance
            .actions
            .iter()
            .any(|a| a.id == "remote-pair" && a.description.contains("user action"))
    );
}

#[test]
fn unknown_and_old_versions_offer_upgrade_and_help_without_claiming_nested_commands() {
    let rows = [
        row("ops", false),
        row("remote", false),
        row("colab", false),
        row("digest", false),
    ];
    let guidance = actions(&rows, &[]);
    assert_eq!(
        commands(&guidance),
        [
            "tmt run codex",
            "tmt upgrade",
            "tmt ops --help",
            "tmt remote --help",
            "tmt colab --help"
        ]
    );
    assert!(!guidance.missing("ops"));
    // A prior success for another version cannot authorize this installed version's syntax.
    assert!(
        !commands(&actions(
            &rows,
            &[json!({"product":"ops","status":"changed","version":"0.1.0-alpha.2"})]
        ))
        .contains(&"tmt ops ui".into())
    );
}

#[test]
fn post_apply_confirmation_uses_verified_version_and_excludes_failed_or_pinned_results() {
    for status in [
        "changed",
        "unchanged",
        "installed",
        "failed",
        "skippedPinned",
        "current",
        "consentRequired",
    ] {
        let rows = [row("ops", false)];
        let confirmed = [json!({"product":"ops","status":status,"version":"0.1.0-alpha.1"})];
        let result = commands(&actions(&rows, &confirmed));
        assert_eq!(
            result.contains(&"tmt ops ui".into()),
            matches!(status, "changed" | "unchanged" | "installed"),
            "{status}"
        );
    }
}

#[test]
fn damage_shadowing_incompatibility_and_missing_dependency_never_claim_readiness() {
    let mut damaged = row("ops", true);
    damaged["status"] = json!("repairRequired");
    let mut shadowed = row("remote", true);
    shadowed["shadowedBy"] = json!(["/other/tmt-remote"]);
    let colab = row("colab", true);
    let mut incompatible = row("digest", true);
    incompatible["cli"] = json!({"compatible":false});
    let result = commands(&actions(&[damaged, shadowed, colab, incompatible], &[]));
    assert!(!result.contains(&"tmt ops ui".into()));
    assert!(!result.contains(&"tmt remote pair".into()));
    assert!(!result.contains(&"tmt colab serve".into()));
    assert!(result.contains(&"tmt colab --help".into()));
}

#[test]
fn managed_binary_off_path_and_unmanaged_binary_are_help_only() {
    let mut off_path = row("ops", true);
    off_path["onPath"] = json!(false);
    let mut unmanaged = row("remote", true);
    unmanaged["installed"] = json!(false);
    unmanaged["status"] = json!("unmanaged");
    let result = commands(&actions(&[off_path, unmanaged], &[]));
    assert_eq!(
        result,
        [
            "tmt run codex",
            "tmt upgrade",
            "tmt ops --help",
            "tmt remote --help"
        ]
    );
}

#[test]
fn recovery_is_uncapped_deduplicated_and_install_argv_preserves_opaque_prefix() {
    let repairs = (0..7)
        .map(|i| {
            Action::repair(
                &i.to_string(),
                vec!["tmt".into(), "setup".into(), format!("agent-{i}")],
                "Repair",
            )
        })
        .collect();
    let rows = [json!({"name":"ops","missing":true})];
    let prefix = Path::new("/tmp/a prefix ' $(literal)");
    let guidance = project_with_agent(&rows, &[], repairs, Some(prefix), None);
    assert_eq!(guidance.actions.len(), 9);
    assert!(guidance.actions[..7].iter().all(|a| a.kind == Kind::Repair));
    assert_eq!(
        guidance.actions[8].argv.last().unwrap(),
        prefix.to_str().unwrap()
    );
    let mut text = Vec::new();
    guidance.write("Next", Terminal::PLAIN, &mut text).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert_eq!(text.matches("Next:").count(), 1);
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn bare_root_is_distinct_from_explicit_and_nested_help() {
    for (argv, expected) in [
        (vec![], true),
        (vec!["help"], false),
        (vec!["--help"], false),
        (vec!["help", "setup"], false),
    ] {
        let argv = argv.into_iter().map(Into::into).collect::<Vec<_>>();
        assert_eq!(crate::parser::parse(&argv).unwrap().start_here, expected);
    }
}

#[test]
fn printed_core_actions_and_recovery_commands_parse_through_the_real_grammar() {
    let guidance = project_with_agent(
        &[json!({"name":"ops","missing":true})],
        &[],
        vec![Action::repair(
            "ops",
            vec![
                "tmt".into(),
                "extension".into(),
                "rm".into(),
                "ops".into(),
                "--yes".into(),
                "--prefix".into(),
                "/tmp/a b".into(),
            ],
            "Remove",
        )],
        None,
        Some("codex"),
    );
    for action in guidance.actions {
        let argv = action.argv[1..].iter().map(Into::into).collect::<Vec<_>>();
        crate::parser::parse(&argv).unwrap_or_else(|error| panic!("{:?}: {error:?}", action.argv));
    }
}

#[test]
fn browser_start_uses_the_inventory_dependency_facts() {
    let mut colab = row("colab", true);
    colab["dependencies"] = json!(["digest"]);
    let remote = row("remote", true);
    assert!(
        !commands(&actions(&[remote.clone(), colab.clone()], &[]))
            .contains(&"tmt colab serve".into())
    );
    assert!(
        commands(&actions(&[remote, colab, row("digest", true)], &[]))
            .contains(&"tmt colab serve".into())
    );
}
