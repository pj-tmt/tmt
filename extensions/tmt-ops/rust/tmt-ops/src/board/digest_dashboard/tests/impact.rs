//! Impact band facts, controls and decoded scenes, using the existing held-data fixture.
use super::*;
use crate::{
    board::app::{Choice, Effect},
    config::{DigestWindow, SavingsUnit, TokRange},
};

pub(in crate::board) const SCENES: &[&str] = &[
    "impact-complete",
    "impact-partial",
    "impact-unknown",
    "impact-stale",
    "impact-zero",
    "impact-mixed-prices",
    "impact-separate-coverage",
    "impact-week",
    "impact-all-time",
    "impact-basis",
];

pub(in crate::board) fn capture_case(
    scene: &str,
    look: crate::look::Look,
) -> crate::board::app::App {
    let mut app = super::layout::fixture();
    let now = crate::status::now_ms();
    let mut doc = valued_document();
    doc["observedAtMs"] = serde_json::json!(now);
    doc["tick"] = serde_json::json!({"lastAtMs":now - 30_000});
    doc["global"] = serde_json::json!({"enabled":true,"default":"auto","text":"Digest: on (auto)"});
    let snapshot = app.view.as_mut().unwrap().home.as_mut().unwrap();
    snapshot.workspace.squad_looks.clear();
    let ids: Vec<_> = snapshot.workspace.roster.members.keys().cloned().collect();
    let template = doc["members"][0].clone();
    doc["members"] = ids
        .iter()
        .map(|id| {
            let mut member = template.clone();
            member["identityId"] = serde_json::json!(id);
            member
        })
        .collect();
    snapshot.workspace.preferences.digest_window = match scene {
        "impact-week" => DigestWindow::Week,
        "impact-all-time" => DigestWindow::AllTime,
        _ => DigestWindow::Day,
    };
    let window = snapshot.workspace.preferences.digest_window;
    for (index, member) in doc["members"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        member["savings"]["window"] = serde_json::json!(window.argument());
        match scene {
            "impact-partial" if index > 0 => member["savings"] = serde_json::Value::Null,
            "impact-unknown" => member["savings"] = serde_json::Value::Null,
            "impact-zero" => {
                member["savings"]["estimatedInputTokensAvoided"] = serde_json::json!(0);
                member["savings"]["cost"]["usdMicros"] = serde_json::json!(0);
                member["savings"]["percent"]["basisPoints"] = serde_json::json!(0);
                member["savings"]["percent"]["estimatedInputTokensAvoided"] = serde_json::json!(0);
            }
            "impact-mixed-prices" if index > 0 => {
                member["savings"]["cost"]["pricingVersion"] = serde_json::json!("2026-10-15")
            }
            "impact-separate-coverage" if index > 0 => {
                member["savings"]["cost"] = serde_json::Value::Null
            }
            _ => {}
        }
    }
    app.labels = crate::labels::Supplied::from_document("digest", &doc, scene == "impact-stale");
    app.view.as_mut().unwrap().look = look;
    app.workspace_reconcile();
    app
}

#[test]
fn both_units_keep_their_own_coverage_and_whole_money_moves_to_details() {
    let _no_commands = crate::runner::forbid_commands();
    for scene in SCENES.iter().filter(|scene| **scene != "impact-basis") {
        let mut app = capture_case(scene, crate::look::Look::default());
        let shown = paint(&mut app, 160, 50);
        assert!(shown.contains("Est. ctx avoided"), "{scene}: {shown}");
        assert!(shown.contains("List-price equiv."), "{scene}: {shown}");
        if *scene == "impact-separate-coverage" {
            assert!(shown.contains("(7/7 agents)"), "{shown}");
            assert!(shown.contains("(1/7 agents · partial)"), "{shown}");
        }
        if *scene == "impact-mixed-prices" {
            assert!(shown.contains("prices differ"), "{shown}");
        }
        let narrow = paint(&mut app, 100, 32);
        let cells: Vec<_> = narrow.chars().collect();
        let lines: Vec<String> = cells
            .chunks(100)
            .map(|line| line.iter().collect())
            .collect();
        let band = lines
            .iter()
            .find(|line| line.starts_with("Impact "))
            .unwrap();
        if band.contains('$') {
            assert!(
                band.contains("List-price equiv. ≈ $86.79 (7/7 agents)")
                    || band.contains("List-price equiv. $0.00 (7/7 agents)"),
                "whole amount and coverage: {band}"
            );
        }
        let small = paint(&mut app, 80, 32);
        let small_cells: Vec<_> = small.chars().collect();
        let small_band = small_cells
            .chunks(80)
            .map(|line| line.iter().collect::<String>())
            .find(|line| line.starts_with("Impact "))
            .unwrap();
        assert!(
            !small_band.contains('$'),
            "unfit amount moves wholly: {small_band}"
        );
        let details = super::super::render::detail_lines(
            &app,
            100,
            crate::status::now_ms(),
            app.selected_row(),
        );
        assert!(
            details
                .iter()
                .any(|line| line.to_string().contains("List-price equivalent:"))
        );
        assert!(
            app.workspace_strip_hits
                .borrow()
                .iter()
                .all(|(_, choice)| *choice != Choice::SavingsUnit)
        );
    }
}

#[test]
fn band_controls_leave_usage_unit_selection_and_window_write_owner_intact() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = capture_case("impact-complete", crate::look::Look::default());
    let selected = app.workspace.selected.clone();
    paint(&mut app, 160, 50);
    assert_eq!(app.choose(Choice::DigestWindowMenu), Effect::None);
    let menu = app.menu.as_ref().unwrap();
    assert_eq!(menu.entries.len(), 3);
    assert!(
        menu.entries
            .iter()
            .any(|entry| entry.choice == Choice::DigestWindow(DigestWindow::AllTime))
    );
    app.menu = None;
    let change = app.choose(Choice::DigestWindow(DigestWindow::Week));
    assert_eq!(app.digest_window(), DigestWindow::Day);
    save(&mut app, change);
    assert_eq!(app.digest_window(), DigestWindow::Week);
    assert_eq!(app.tok_range(), TokRange::Day);
    assert_eq!(app.savings_unit(), SavingsUnit::Tokens);
    assert_eq!(app.workspace.selected, selected);
    paint(&mut app, 100, 32);
    app.choose(Choice::DigestBasis);
    assert!(app.scrolls.offset(crate::config::Pane::Replies) > 0);
    assert_eq!(app.workspace.selected, selected);
    app.context_menu();
    assert!(
        app.menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.choice == Choice::DigestBasis)
    );
}

#[test]
fn basis_reveals_hidden_details_via_the_existing_panel_cas() {
    use crate::config::{DigestPanels, PanelSetting};
    let mut app = capture_case("impact-complete", crate::look::Look::default());
    let mut panels = DigestPanels::default();
    panels.set_side_shown(false).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .digest_panels = Some(PanelSetting::configured(panels));
    paint(&mut app, 100, 32);
    let effect = app.choose(Choice::DigestBasis);
    let Effect::Act(crate::board::app::Request::WorkspacePreferences { expected, next, .. }) =
        effect
    else {
        panic!("the existing Details toggle must own the write");
    };
    assert!(
        !expected
            .digest_panels
            .as_ref()
            .unwrap()
            .effective()
            .side_shown()
    );
    assert!(
        next.digest_panels
            .as_ref()
            .unwrap()
            .effective()
            .side_shown()
    );
    assert_eq!(expected.digest_window, next.digest_window);
    assert_eq!(expected.savings_unit, next.savings_unit);
    assert_eq!(expected.tok_range, next.tok_range);
    assert!(
        !app.digest_panels().side_shown(),
        "a pending write never paints Saved"
    );
    app.finished_workspace_preferences(next, "saved".into());
    assert!(app.digest_panels().side_shown());
}

#[test]
fn details_show_each_impact_amount_once_and_basis_has_only_evidence() {
    let mut app = valued_board();
    press(&mut app, '2');
    select_worker(&mut app);
    let lines = super::super::render::detail_lines(&app, 160, 121_000, app.selected_row());
    let text = lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let (impact, basis) = text.split_once("Estimate basis").unwrap();
    for amount in ["18.2k", "$12.40", "18%"] {
        assert_eq!(
            text.matches(amount).count(),
            1,
            "{amount} appears once: {text}"
        );
        assert!(impact.contains(amount), "amount stays in Impact: {text}");
        assert!(
            !basis.contains(amount),
            "basis contains evidence only: {text}"
        );
    }
    assert_eq!(text.matches(super::super::band::DEFINITION).count(), 1);
    assert!(
        !basis.contains("Observed input quote"),
        "usage amount stays outside basis"
    );
    for fact in [
        "gpt-6-sol",
        "$2.00 per 1M",
        "price table 2026-10-14",
        "under 1 s",
        "1–2 min",
    ] {
        assert!(basis.contains(fact), "basis retains {fact}: {text}");
    }
}
