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
fn explicit_order_completes_known_ids_and_hidden_is_a_unique_subset() {
    let policy = overview("[workspace.overview_panels]\norder = ['monitor','usage','summary','digest','exchanges','subscriptions']\nhidden = ['usage','exchanges']\nconversation = false\n").unwrap();
    assert_eq!(policy.order()[0], OverviewPanel::Monitor);
    assert!(!policy.shown(OverviewPanel::Usage));
    assert!(!policy.side_shown());
    for field in [
        "order = ['future-panel']",
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
    let document: DocumentMut = "[workspace.digest_panels]\norder = ['list','status','usage','subscriptions','chart','report']\nhidden = ['chart']\ndetails = false\n".parse().unwrap();
    let policy = DigestPanels::parse(&document["workspace"]["digest_panels"]).unwrap();
    assert_eq!(policy.order(), DigestPanel::ALL);
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
fn retired_digest_sections_are_ignored_while_equivalent_switches_survive() {
    let document: DocumentMut = "[workspace.digest_panels]\norder=['list','status','usage','subscriptions','chart','report']\nhidden=['report','chart']\ndetails=false\n".parse().unwrap();
    let policy = DigestPanels::parse(&document["workspace"]["digest_panels"]).unwrap();
    assert_eq!(policy.order(), DigestPanel::ALL);
    assert!(!policy.shown(DigestPanel::Chart));
    assert!(!policy.side_shown());
    assert!(policy.shown(DigestPanel::List));
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

#[test]
fn older_panel_orders_respect_fixed_sections_and_round_trip_visibility_in_both_forms() {
    for (key, side, order, hidden, expected) in [
        (
            "overview_panels",
            "conversation",
            "'monitor','exchanges','digest','summary','subscriptions'",
            "summary",
            vec![
                "monitor",
                "exchanges",
                "digest",
                "usage",
                "summary",
                "subscriptions",
            ],
        ),
        (
            "digest_panels",
            "details",
            "'list','usage','status','subscriptions'",
            "chart",
            vec!["subscriptions", "chart", "list"],
        ),
    ] {
        for inline in [false, true] {
            let input = if inline {
                format!(
                    "[workspace]\nsort = 'name' # untouched\n{key} = {{ order = [{order}], hidden = ['{hidden}'], {side} = true }} # keep policy\n"
                )
            } else {
                format!(
                    "[workspace]\nsort = 'name' # untouched\n[workspace.{key}] # keep policy\norder = [{order}]\nhidden = ['{hidden}']\n{side} = true\n"
                )
            };
            let mut document: DocumentMut = input.parse().unwrap();
            fn round_trip<P: Panel>(item: &mut Item, expected: &[&str], hidden: &str) {
                let parsed = Panels::<P>::parse(item)
                    .expect("a new known panel must not discard the saved arrangement");
                assert_eq!(
                    parsed
                        .order()
                        .iter()
                        .map(|panel| panel.key())
                        .collect::<Vec<_>>(),
                    expected
                );
                assert!(
                    !parsed.shown(P::parse(hidden).unwrap()),
                    "saved hidden state survives expansion"
                );
                let missing = if P::SIDE_KEY == "conversation" {
                    "usage"
                } else {
                    "subscriptions"
                };
                assert!(
                    parsed.shown(P::parse(missing).unwrap()),
                    "new panels use the default visibility"
                );
                let authored_order = item["order"].to_string();
                PanelSetting::configured(parsed.clone()).write(item);
                assert_eq!(Panels::<P>::parse(item).unwrap(), parsed);
                if !P::ORDERED {
                    assert_eq!(
                        item["order"].to_string(),
                        authored_order,
                        "retired order remains authored migration data"
                    );
                }
            }
            if key == "overview_panels" {
                round_trip::<OverviewPanel>(&mut document["workspace"][key], &expected, hidden);
            } else {
                round_trip::<DigestPanel>(&mut document["workspace"][key], &expected, hidden);
            }
            assert_eq!(document["workspace"][key].is_inline_table(), inline);
            let output = document.to_string();
            assert!(output.contains("sort = 'name' # untouched"));
            assert!(output.contains("# keep policy"));
            assert!(output.contains(hidden));
        }
    }
}

#[test]
fn every_omitted_subset_completes_once_and_only_ordered_panels_keep_saved_order() {
    fn check<P: Panel>() {
        for mask in 0..1usize << P::ALL.len() {
            let saved: Vec<_> = P::ALL
                .iter()
                .copied()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, panel)| panel)
                .rev()
                .collect();
            let order = saved
                .iter()
                .map(|panel| format!("'{}'", panel.key()))
                .collect::<Vec<_>>()
                .join(",");
            let document: DocumentMut =
                format!("panels = {{ order = [{order}] }}").parse().unwrap();
            let policy = Panels::<P>::parse(&document["panels"]).unwrap();
            assert_eq!(policy.order.len(), P::ALL.len());
            assert_eq!(
                policy.order.iter().copied().collect::<BTreeSet<_>>(),
                P::ALL.iter().copied().collect()
            );
            if P::ORDERED {
                assert_eq!(
                    policy
                        .order
                        .iter()
                        .copied()
                        .filter(|panel| saved.contains(panel))
                        .collect::<Vec<_>>(),
                    saved
                );
            } else {
                assert_eq!(policy.order(), P::ALL);
                let mut moved = policy.clone();
                assert!(!moved.move_panel(P::ALL[0], true));
                assert_eq!(moved, policy);
            }
            for (index, panel) in P::ALL.iter().enumerate() {
                if !saved.contains(panel) {
                    assert_eq!(&policy.order[index], panel);
                    assert!(policy.shown(*panel));
                }
            }
        }
    }
    check::<OverviewPanel>();
    check::<DigestPanel>();
}
