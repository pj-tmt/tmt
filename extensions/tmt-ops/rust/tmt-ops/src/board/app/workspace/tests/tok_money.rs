//! The TOK figures of the Agents list are a control: a click switches every figure between
//! tokens and the list price of the input tokens behind them. The choice is the one shared
//! unit preference the Digest estimates use, so it never needs a new acquisition.
use super::*;
use crate::{
    board::{
        app::{Choice, Request},
        digest_dashboard::tests::valued_board,
        usage_report::{
            Delivery, Report,
            model::{HourlySeries, Seat, Who},
        },
        workspace::AgentsFilter,
    },
    config::{SavingsUnit, TokRange},
};
use ratatui::{
    crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};
use std::time::Instant;

fn hold(app: &mut App, unit: SavingsUnit) {
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .savings_unit = unit;
}

fn figures(app: &App) -> Vec<Rect> {
    app.workspace_strip_hits
        .borrow()
        .iter()
        .filter_map(|(area, choice)| (*choice == Choice::TokMoney).then_some(*area))
        .collect()
}

fn click(app: &mut App, area: Rect) -> Effect {
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        },
        Instant::now(),
    )
}

/// One complete day for `MEMBER` with one million priced input tokens (`$2.00` at the held quote).
fn report_for_member(app: &mut App) {
    let seat = Seat {
        who: Who {
            id: Some(MEMBER.into()),
            name: None,
            retired: false,
        },
        model_inputs: Some(vec![("gpt-6-sol".into(), 1_000_000)]),
        input: 1_000_000,
        output: 100,
        cached: 0,
        covered_ms: 86_400_000,
        window_ms: 86_400_000,
        from_ms: 100_000_000,
        to_ms: 186_400_000,
        observed_at_ms: 186_400_000,
        starts_at_ms: Some(100_000_000),
        complete: true,
        series: HourlySeries::new(100_000_000, [0; 24], [true; 24]),
    };
    app.usage_report.apply(Delivery {
        since: TokRange::Day,
        result: Ok(Report {
            since: TokRange::Day,
            observed_at_ms: 186_400_000,
            window_from_ms: Some(100_000_000),
            seats: vec![seat],
            unavailable: vec![],
            exchange: None,
            warnings: vec![],
        }),
    });
}

fn lists() -> [(&'static str, AgentsFilter); 2] {
    [
        ("squads", AgentsFilter::All),
        ("agents", AgentsFilter::Agents),
    ]
}

#[test]
fn a_click_on_a_tok_figure_asks_for_the_other_unit_and_selects_nothing() {
    for (name, filter) in lists() {
        for (held, wanted) in [
            (SavingsUnit::Tokens, SavingsUnit::Cost),
            (SavingsUnit::Percent, SavingsUnit::Cost),
            (SavingsUnit::Cost, SavingsUnit::Tokens),
        ] {
            let mut app = board();
            app.workspace.filter = filter;
            hold(&mut app, held);
            paint(&app, 160, 40);
            let area = *figures(&app)
                .first()
                .unwrap_or_else(|| panic!("{name}: no TOK figure hit"));
            let selected = app.workspace.selected.clone();
            let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) =
                click(&mut app, area)
            else {
                panic!("{name} {held:?}: the figure must use the preferences writer");
            };
            assert_eq!(next.savings_unit, wanted, "{name} {held:?}");
            let mut rest = next.clone();
            rest.savings_unit = held;
            assert_eq!(rest, expected, "{name}: only the unit changes");
            assert_eq!(
                app.workspace.selected, selected,
                "{name}: a click selects nothing"
            );
        }
    }
}

#[test]
fn tokens_stay_the_default_and_the_heading_names_the_unit() {
    for (name, filter) in lists() {
        let mut app = board();
        app.workspace.filter = filter;
        let tokens = paint(&app, 160, 40).join("\n");
        assert!(tokens.contains("TOK 1h"), "{name}: {tokens}");
        assert!(!tokens.contains("≈$"), "{name}: {tokens}");
        hold(&mut app, SavingsUnit::Cost);
        let money = paint(&app, 160, 40).join("\n");
        assert!(money.contains("≈$ 1h"), "{name}: {money}");
        assert!(!money.contains("TOK 1h"), "{name}: {money}");
    }
}

#[test]
fn money_is_the_list_price_of_the_input_tokens_or_a_dash_never_a_partial_sum() {
    let mut app = valued_board();
    app.workspace.filter = AgentsFilter::Agents;
    hold(&mut app, SavingsUnit::Cost);
    // Until a complete report arrives the figure is unknown, not zero.
    let before = paint(&app, 160, 40).join("\n");
    assert!(!before.contains("≈$2"), "{before}");
    assert!(before.contains("working          —"), "{before}");
    report_for_member(&mut app);
    let after = paint(&app, 160, 40).join("\n");
    assert!(after.contains("≈$2.00"), "{after}");
    // The same range word stays its own control after the money caption.
    let chip = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find_map(|(area, choice)| matches!(choice, Choice::TokRange(_)).then_some(*area))
        .unwrap();
    let line = &paint(&app, 160, 40)[usize::from(chip.y)];
    let word: String = line
        .chars()
        .skip(usize::from(chip.x))
        .take(usize::from(chip.width))
        .collect();
    assert_eq!(word, "1d", "{line}");
    // An incomplete report must not look like a price.
    if let crate::board::usage_report::View::Report(_) = app.usage_report.view_for(TokRange::Day) {
        let mut incomplete = app
            .usage_report
            .retained_report(TokRange::Day)
            .unwrap()
            .as_ref()
            .clone();
        incomplete.seats[0].complete = false;
        app.usage_report.apply(Delivery {
            since: TokRange::Day,
            result: Ok(incomplete),
        });
    }
    let partial = paint(&app, 160, 40).join("\n");
    assert!(!partial.contains("≈$2"), "{partial}");
}

#[test]
fn the_squads_tree_sums_the_members_money_the_same_way() {
    let mut app = valued_board();
    app.workspace.filter = AgentsFilter::All;
    hold(&mut app, SavingsUnit::Cost);
    report_for_member(&mut app);
    let shown = paint(&app, 160, 40).join("\n");
    assert!(shown.contains("≈$ 1d"), "{shown}");
    assert!(
        shown
            .lines()
            .any(|line| line.contains("product") && line.contains("≈$2.00")),
        "the squad row carries its only member's price: {shown}"
    );
    assert!(!figures(&app).is_empty(), "a money figure stays a control");
}

#[test]
fn the_usage_report_is_wanted_on_the_agents_tab_only_while_money_is_shown() {
    let mut app = board();
    app.shell.tab = crate::board::shell::OpsTab::Agents;
    app.right_tab = crate::board::shell::RightTab::Talk;
    hold(&mut app, SavingsUnit::Tokens);
    assert!(app.usage_report_want().is_none());
    hold(&mut app, SavingsUnit::Cost);
    assert_eq!(
        app.usage_report_want().map(|want| want.since()),
        Some(app.tok_range())
    );
}
