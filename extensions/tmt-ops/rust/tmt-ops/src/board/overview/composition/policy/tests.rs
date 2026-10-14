use super::*;
fn height(tracks: &[(Panel, u16)], panel: Panel) -> u16 {
    tracks
        .iter()
        .find(|(id, _)| *id == panel)
        .map_or(0, |(_, height)| *height)
}
#[test]
fn default_budgets_keep_the_existing_height_rules() {
    let policy = OverviewPanels::default();
    for h in 0..=44 {
        for quota in 0..=9 {
            let budget = Budget::new(h, quota, &policy);
            let tracks = budget.tracks(&policy, budget.heights[3]);
            let old = super::super::panels(ratatui::layout::Rect::new(0, 0, 100, h), quota);
            for (panel, rect) in old {
                assert_eq!(height(&tracks, panel), rect.height, "{h}/{quota}/{panel:?}");
            }
            assert_eq!(tracks.iter().map(|(_, h)| h).sum::<u16>(), h);
        }
    }
}
#[test]
fn hidden_panels_and_unused_chart_rows_return_to_the_target() {
    let mut policy = OverviewPanels::default();
    let original = Budget::new(44, 7, &policy).tracks(&policy, 1);
    let old = height(&original, Panel::Table);
    for id in [
        Id::Summary,
        Id::Digest,
        Id::Subscriptions,
        Id::Usage,
        Id::Exchanges,
    ] {
        policy.set_shown(id, false).unwrap();
    }
    let hidden = Budget::new(44, 7, &policy).tracks(&policy, 0);
    assert_eq!(hidden, vec![(Panel::Table, 42), (Panel::Legend, 2)]);
    assert!(height(&hidden, Panel::Table) > old);
    policy.set_shown(Id::Monitor, false).unwrap();
    assert!(Budget::new(44, 7, &policy).tracks(&policy, 0).is_empty());
}
#[test]
fn moving_panels_does_not_change_their_budget_or_create_a_hidden_target() {
    for h in 0..=44 {
        let mut policy = OverviewPanels::default();
        let reference = Budget::new(h, 4, &policy).tracks(&policy, 1);
        for _ in 0..5 {
            policy.move_panel(Id::Monitor, false);
        }
        let moved = Budget::new(h, 4, &policy).tracks(&policy, 1);
        for (panel, count) in &reference {
            assert_eq!(height(&moved, *panel), *count);
        }
        assert_eq!(moved.iter().map(|(_, h)| h).sum::<u16>(), h);
        policy.set_shown(Id::Monitor, false).unwrap();
        assert!(
            !Budget::new(h, 4, &policy)
                .tracks(&policy, 1)
                .iter()
                .any(|(id, _)| *id == Panel::Table)
        );
    }
}

#[test]
fn every_order_visibility_and_height_preserves_a_shown_target_and_bounded_rows() {
    use crate::config::Panel as _;
    fn permutations(values: &mut [Id], index: usize, output: &mut Vec<Vec<Id>>) {
        if index == values.len() {
            output.push(values.to_vec());
            return;
        }
        for next in index..values.len() {
            values.swap(index, next);
            permutations(values, index + 1, output);
            values.swap(index, next);
        }
    }
    let mut orders = Vec::new();
    permutations(&mut Id::ALL.to_vec(), 0, &mut orders);
    assert_eq!(orders.len(), 720);
    for order in orders {
        let mut ordered = OverviewPanels::default();
        for (position, panel) in order.iter().enumerate() {
            while ordered.order().iter().position(|id| id == panel).unwrap() > position {
                ordered.move_panel(*panel, false);
            }
        }
        assert_eq!(ordered.order(), order);
        for mask in 0..64u8 {
            let mut policy = ordered.clone();
            for (bit, panel) in Id::ALL.iter().enumerate() {
                policy.set_shown(*panel, mask & (1 << bit) == 0).unwrap();
            }
            for h in 0..=44 {
                let budget = Budget::new(h, 7, &policy);
                for demand in [0, 1, 9, 10, 11] {
                    let tracks = budget.tracks(&policy, demand);
                    let total: u16 = tracks.iter().map(|(_, height)| height).sum();
                    assert!(total <= h, "{order:?}/{mask}/{h}/{demand}");
                    assert_eq!(
                        tracks.iter().filter(|(id, _)| *id == Panel::Table).count(),
                        usize::from(policy.shown(Id::Monitor) && h > 0)
                    );
                    if policy.shown(Id::Monitor) {
                        assert_eq!(total, h);
                    }
                    let vertical: Vec<_> = tracks
                        .iter()
                        .filter(|(id, _)| *id != Panel::Legend)
                        .map(|(id, _)| *id)
                        .collect();
                    assert!(vertical.windows(2).all(|pair| pair[0] != pair[1]));
                }
            }
        }
    }
}
