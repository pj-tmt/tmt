use super::*;
use toml_edit::DocumentMut;

fn overview(text: &str) -> Result<OverviewPanels, String> {
    let document: DocumentMut = text.parse().unwrap();
    OverviewPanels::parse(&document["workspace"]["overview_panels"])
}

#[test]
fn omitted_fields_and_both_table_forms_have_the_same_defaults() {
    for text in [
        "[workspace.overview_panels]\n",
        "[workspace]\noverview_panels = {}\n",
    ] {
        assert_eq!(overview(text).unwrap(), OverviewPanels::default());
    }
    let normal = overview("[workspace.overview_panels]\nhidden = ['exchanges']\n").unwrap();
    let inline = overview("[workspace]\noverview_panels = { hidden = ['exchanges'] }\n").unwrap();
    assert_eq!(normal, inline);
    assert!(!normal.shown(OverviewPanel::Exchanges));
    assert!(normal.side_shown());
}

#[test]
fn explicit_order_is_a_complete_permutation_and_hidden_is_a_unique_subset() {
    let policy = overview("[workspace.overview_panels]\norder = ['monitor','usage','summary','digest','exchanges','subscriptions']\nhidden = ['usage','exchanges']\nconversation = false\n").unwrap();
    assert_eq!(policy.order()[0], OverviewPanel::Monitor);
    assert!(!policy.shown(OverviewPanel::Usage));
    assert!(!policy.side_shown());
    for field in [
        "order = ['monitor']",
        "order = ['summary','digest','subscriptions','usage','monitor','monitor']",
        "hidden = ['usage','usage']",
        "hidden = ['future-panel']",
        "hidden = ['Summary']",
        "order = 5",
        "hidden = false",
        "hidden = [1]",
        "conversation = 'true'",
        "future = true",
    ] {
        assert!(
            overview(&format!("[workspace.overview_panels]\n{field}\n")).is_err(),
            "{field}"
        );
    }
    assert!(overview("[workspace]\noverview_panels = 5\n").is_err());
}

#[test]
fn dashboards_have_distinct_ids_and_side_panes() {
    let document: DocumentMut = "[workspace.digest_panels]\norder = ['list','status','usage','subscriptions','chart']\nhidden = ['chart']\ndetails = false\n".parse().unwrap();
    let policy = DigestPanels::parse(&document["workspace"]["digest_panels"]).unwrap();
    assert_eq!(policy.order()[0], DigestPanel::List);
    assert!(!policy.shown(DigestPanel::Chart));
    assert!(!policy.side_shown());
    assert!(OverviewPanel::parse("chart").is_none());
    assert!(DigestPanel::parse("monitor").is_none());
    let wrong: DocumentMut = "[workspace.digest_panels]\nconversation = false\n"
        .parse()
        .unwrap();
    assert!(DigestPanels::parse(&wrong["workspace"]["digest_panels"]).is_err());
}

#[test]
fn last_surface_refusal_never_changes_the_draft() {
    let mut policy = OverviewPanels::default();
    policy.set_side_shown(false).unwrap();
    let before = policy.clone();
    assert!(policy.set_shown(OverviewPanel::Monitor, false).is_err());
    assert_eq!(policy, before);
    policy.set_side_shown(true).unwrap();
    policy.set_shown(OverviewPanel::Monitor, false).unwrap();
    let before = policy.clone();
    assert!(policy.set_side_shown(false).is_err());
    assert_eq!(policy, before);
    assert!(
        overview("[workspace.overview_panels]\nhidden = ['monitor']\nconversation = false\n")
            .is_err()
    );
    let document: DocumentMut = "[workspace.digest_panels]\nhidden = ['list']\ndetails = false\n"
        .parse()
        .unwrap();
    assert!(DigestPanels::parse(&document["workspace"]["digest_panels"]).is_err());
}

#[test]
fn moving_keeps_identity_and_does_not_cross_a_boundary() {
    let mut policy = OverviewPanels::default();
    assert!(!policy.move_panel(OverviewPanel::Summary, false));
    assert!(policy.move_panel(OverviewPanel::Monitor, false));
    assert_eq!(policy.order()[3], OverviewPanel::Monitor);
    assert!(policy.move_panel(OverviewPanel::Monitor, true));
    assert_eq!(policy, OverviewPanels::default());
    assert!(!policy.move_panel(OverviewPanel::Exchanges, true));
}

#[test]
fn observation_distinguishes_partial_explicit_and_malformed_values() {
    let partial: DocumentMut = "[workspace.overview_panels]\nhidden = []\n"
        .parse()
        .unwrap();
    let setting = PanelSetting::<OverviewPanel>::observe(&partial["workspace"]["overview_panels"]);
    assert_eq!(setting.effective(), OverviewPanels::default());
    assert_ne!(setting, PanelSetting::configured(OverviewPanels::default()));
    let first: DocumentMut = "[workspace]\noverview_panels = 5\n".parse().unwrap();
    let second: DocumentMut = "[workspace]\noverview_panels = 6\n".parse().unwrap();
    let a = PanelSetting::<OverviewPanel>::observe(&first["workspace"]["overview_panels"]);
    let b = PanelSetting::<OverviewPanel>::observe(&second["workspace"]["overview_panels"]);
    assert_ne!(a, b);
    assert!(a.error().is_some());
    assert_eq!(a.effective(), OverviewPanels::default());
}

#[test]
fn explicit_save_keeps_table_form_comments_and_unrelated_bytes() {
    for text in [
        "# workspace\n[workspace]\nsort = 'name' # untouched\noverview_panels = { hidden = ['exchanges'], conversation = true } # policy\n",
        "# workspace\n[workspace]\nsort = 'name' # untouched\n\n# panel header\n[workspace.overview_panels] # policy\n# hidden note\nhidden = ['exchanges'] # keep this\nconversation = true # side\n",
    ] {
        let mut document: DocumentMut = text.parse().unwrap();
        let before = document["workspace"]["overview_panels"].is_inline_table();
        let mut policy = OverviewPanels::default();
        policy.set_shown(OverviewPanel::Usage, false).unwrap();
        let setting = PanelSetting::configured(policy.clone());
        setting.write(&mut document["workspace"]["overview_panels"]);
        let output = document.to_string();
        let parsed: DocumentMut = output.parse().unwrap();
        assert_eq!(
            OverviewPanels::parse(&parsed["workspace"]["overview_panels"]).unwrap(),
            policy
        );
        assert_eq!(
            parsed["workspace"]["overview_panels"].is_inline_table(),
            before
        );
        for comment in ["# workspace", "sort = 'name' # untouched", "# policy"] {
            assert!(output.contains(comment), "{output}");
        }
        if !before {
            for comment in ["# panel header", "# hidden note", "# keep this", "# side"] {
                assert!(output.contains(comment), "{output}");
            }
        }
        assert_eq!(
            PanelSetting::<OverviewPanel>::observe(&parsed["workspace"]["overview_panels"]),
            setting
        );
    }
}

#[test]
fn saving_changed_ids_and_repairing_unknown_fields_keep_displaced_comments() {
    let mut document: DocumentMut = "[workspace.overview_panels]\n# unknown leading\nfuture = '# not a comment' # unknown trailing\nhidden = [\n  'exchanges', # image note\n] # hidden suffix\n".parse().unwrap();
    let setting = PanelSetting::configured(OverviewPanels::default());
    setting.write(&mut document["workspace"]["overview_panels"]);
    let output = document.to_string();
    let parsed: DocumentMut = output.parse().unwrap();
    assert_eq!(
        OverviewPanels::parse(&parsed["workspace"]["overview_panels"]).unwrap(),
        OverviewPanels::default()
    );
    for comment in [
        "# unknown leading",
        "# unknown trailing",
        "# image note",
        "# hidden suffix",
    ] {
        assert!(output.contains(comment), "{output}");
    }
    assert!(!output.contains("# not a comment"));
    assert!(
        parsed["workspace"]["overview_panels"]
            .get("future")
            .is_none()
    );
}
