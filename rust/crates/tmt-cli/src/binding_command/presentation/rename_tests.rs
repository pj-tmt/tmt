use super::*;
use tmt_core::{binding::RenamedIdentity, identity::Lifetime};

fn identity(name: &str) -> Identity {
    Identity {
        id: "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f".into(),
        name: name.into(),
        canonical_name: name.to_lowercase(),
        lifetime: Lifetime::Saved,
        created_at: "t0".into(),
        updated_at: "t1".into(),
    }
}

fn renamed(from: &str, to: &str, pane: Option<(&str, PaneRefresh)>) -> Report {
    Report::Renamed {
        result: RenamedIdentity {
            previous: identity(from),
            identity: identity(to),
            binding: None,
        },
        pane: pane.map(|(id, refresh)| (id.into(), refresh)),
    }
}

fn human(report: &Report) -> (String, String) {
    let mut output = Vec::new();
    text(&mut output, Terminal::PLAIN, report).unwrap();
    let mut errors = tmt_cli_style::stream::Stream::new(Terminal::PLAIN, Vec::new());
    warnings(&mut errors, report).unwrap();
    (
        String::from_utf8(output).unwrap(),
        String::from_utf8(errors.into_inner()).unwrap(),
    )
}

#[test]
fn a_rename_reports_both_names_the_same_uuid_and_whether_its_pane_shows_it() {
    let report = renamed(
        "opus-tmt-peer-2",
        "tmt-peer-2",
        Some(("%12", PaneRefresh::Updated)),
    );
    assert_eq!(
        document(&report),
        json!({
            "renamed": true,
            "previousName": "opus-tmt-peer-2",
            "identity": identity_document(&identity("tmt-peer-2")),
            "pane": {"id": "%12", "updated": true},
        })
    );
    assert_eq!(
        human(&report),
        (
            "✓ Renamed opus-tmt-peer-2 to tmt-peer-2\n".into(),
            String::new()
        )
    );
    assert_eq!(document(&renamed("Old", "New", None))["pane"], Value::Null);
}

#[test]
fn a_pane_that_kept_the_old_name_is_a_warning_with_the_command_that_refreshes_it() {
    let report = renamed("Alice", "Ada", Some(("%12", PaneRefresh::Failed)));
    assert_eq!(
        document(&report)["pane"],
        json!({"id": "%12", "updated": false})
    );
    assert_eq!(
        human(&report),
        (
            "✓ Renamed Alice to Ada\n".into(),
            "warning: Pane %12 still shows the previous name Alice\n\
             hint: In pane %12, run: tmt this Ada\n"
                .into()
        )
    );
}

#[test]
fn renaming_to_the_current_name_changes_nothing() {
    let report = renamed("Ada", "Ada", None);
    assert_eq!(document(&report)["renamed"], false);
    assert_eq!(
        human(&report),
        ("Ada already has this name\n".into(), String::new())
    );
}

#[test]
fn the_refresh_hint_quotes_a_name_the_shell_would_split() {
    let report = renamed("Alice", "Code Reviewer", Some(("%3", PaneRefresh::Failed)));
    assert!(
        human(&report)
            .1
            .ends_with("hint: In pane %3, run: tmt this 'Code Reviewer'\n")
    );
}
