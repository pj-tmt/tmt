//! Terminal-cell placement for the shared list and conversation. Preferences
//! choose a direction; minimum usable cell budgets always take precedence.
use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preference {
    #[default]
    Auto,
    Side,
    Stack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub list: Option<Rect>,
    pub conversation: Option<Rect>,
}

/// The Agents section bands precede both panes, as in the fixed-cell mock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentsBands {
    pub title: Rect,
    pub filters: Rect,
    pub actions: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkspacePlacement {
    panes: Placement,
    pub agents: Option<AgentsBands>,
}

impl From<Placement> for WorkspacePlacement {
    fn from(panes: Placement) -> Self {
        Self {
            panes,
            agents: None,
        }
    }
}
impl std::ops::Deref for WorkspacePlacement {
    type Target = Placement;
    fn deref(&self) -> &Self::Target {
        &self.panes
    }
}

/// Agents-only geometry; Digest retains `agents` and Overview retains `overview`.
pub fn agents_tree(
    area: Rect,
    preference: Preference,
    conversation_focused: bool,
) -> WorkspacePlacement {
    const TABLE_MIN: u16 = 1 + 1 + 1; // Header, range, one target.
    const CONVERSATION_MIN: u16 = 4 + 1 + 3; // Existing composer, chrome, response/position.
    // Optional section controls yield before the compact conversation loses
    // its host metadata or response position. Narrow single-pane screens can
    // still afford these controls above their list.
    let bands = (area.height >= 8
        && (area.width < 80 || area.height >= TABLE_MIN + CONVERSATION_MIN + 6))
        .then_some(AgentsBands {
            title: Rect::new(area.x, area.y, area.width, 1),
            filters: Rect::new(area.x, area.y + 1, area.width, 1),
            actions: Rect::new(area.x, area.y + 2, area.width, 1),
        });
    let reserved = if bands.is_some() { 3 } else { 0 };
    let body = Rect::new(
        area.x,
        area.y + reserved,
        area.width,
        area.height - reserved,
    );
    let dual_fits = body.height >= TABLE_MIN + CONVERSATION_MIN;
    let side_fits = area.width >= 100 && dual_fits;
    let stack_fits = area.width >= 80 && dual_fits;
    let wants_side =
        preference == Preference::Side || preference == Preference::Auto && area.width >= 120;
    let panes = if side_fits && (wants_side || !stack_fits) {
        let left = ((u32::from(area.width) * 47 / 100) as u16).max(36);
        Placement {
            list: Some(Rect::new(body.x, body.y, left, body.height)),
            conversation: Some(Rect::new(
                body.x + left + 2,
                body.y,
                body.width - left - 2,
                body.height,
            )),
        }
    } else if stack_fits {
        // The mock's range is at min(16, floor(terminal_rows * .42)). Its
        // four shell rows and three footer rows precede/follow this area.
        let top = ((u32::from(area.height) + 7) * 42 / 100)
            .saturating_sub(6)
            .min(10) as u16;
        let top = top.clamp(TABLE_MIN, body.height - CONVERSATION_MIN);
        Placement {
            list: Some(Rect::new(body.x, body.y, body.width, top)),
            conversation: Some(Rect::new(
                body.x,
                body.y + top,
                body.width,
                body.height - top,
            )),
        }
    } else {
        Placement {
            list: (!conversation_focused).then_some(body),
            conversation: conversation_focused.then_some(body),
        }
    };
    WorkspacePlacement {
        panes,
        agents: bands,
    }
}

fn side(area: Rect) -> Placement {
    let left = ((u32::from(area.width) * 2 / 5) as u16).max(36);
    Placement {
        list: Some(Rect::new(area.x, area.y, left, area.height)),
        conversation: Some(Rect::new(
            area.x + left + 1,
            area.y,
            area.width - left - 1,
            area.height,
        )),
    }
}

/// Overview keeps its monitor at compact widths; its optional wide conversation
/// never consumes the only visible monitor surface.
pub fn overview(area: Rect) -> Placement {
    if area.width >= 140 && area.height >= 12 {
        let left = (u32::from(area.width) * 3 / 5) as u16;
        Placement {
            list: Some(Rect::new(area.x, area.y, left, area.height)),
            conversation: Some(Rect::new(
                area.x + left + 1,
                area.y,
                area.width - left - 1,
                area.height,
            )),
        }
    } else {
        Placement {
            list: Some(area),
            conversation: None,
        }
    }
}

/// Policy changes eligibility, not the default responsive split. When the
/// monitor is hidden at a compact size, its remaining context cannot displace
/// the only interactive surface. No recipient or focus is chosen here.
pub fn overview_policy(
    area: Rect,
    policy: &crate::config::OverviewPanels,
    conversation_revealed: bool,
) -> Placement {
    use crate::config::OverviewPanel;
    if !policy.side_shown() {
        if conversation_revealed {
            return if area.width >= 140 && area.height >= 12 {
                overview(area)
            } else {
                Placement {
                    list: None,
                    conversation: Some(area),
                }
            };
        }
        return Placement {
            list: Some(area),
            conversation: None,
        };
    }
    let left_shown = policy.order().iter().any(|panel| policy.shown(*panel));
    if !left_shown
        || (!policy.shown(OverviewPanel::Monitor) && (area.width < 140 || area.height < 12))
    {
        return Placement {
            list: None,
            conversation: Some(area),
        };
    }
    overview(area)
}

pub fn agents(area: Rect, preference: Preference, conversation_focused: bool) -> Placement {
    let side_fits = area.width >= 100 && area.height >= 12;
    let stack_fits = area.width >= 80 && area.height >= 20;
    let wants_side =
        preference == Preference::Side || preference == Preference::Auto && area.width >= 120;
    if side_fits && (wants_side || !stack_fits) {
        return side(area);
    }
    if stack_fits {
        let top = (u32::from(area.height) * 2 / 5) as u16;
        return Placement {
            list: Some(Rect::new(area.x, area.y, area.width, top)),
            conversation: Some(Rect::new(
                area.x,
                area.y + top,
                area.width,
                area.height - top,
            )),
        };
    }
    Placement {
        list: (!conversation_focused).then_some(area),
        conversation: conversation_focused.then_some(area),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_overview_policy_preserves_every_responsive_rectangle() {
        let policy = crate::config::OverviewPanels::default();
        for width in 0..=160 {
            for height in 0..=44 {
                let area = Rect::new(2, 3, width, height);
                assert_eq!(overview_policy(area, &policy, false), overview(area));
            }
        }
    }

    #[test]
    fn explicit_reveal_keeps_tiny_and_wide_conversations_in_bounds() {
        let mut policy = crate::config::OverviewPanels::default();
        policy.set_side_shown(false).unwrap();
        for width in 0..=160 {
            for height in 0..=44 {
                let area = Rect::new(2, 3, width, height);
                let placement = overview_policy(area, &policy, true);
                let conversation = placement
                    .conversation
                    .expect("explicit reveal remains visible");
                assert_eq!(conversation.intersection(area), conversation);
                if let Some(list) = placement.list {
                    assert!(list.intersection(conversation).is_empty());
                    assert_eq!(placement, overview(area));
                } else {
                    assert_eq!(conversation, area);
                }
            }
        }
    }

    #[test]
    fn every_overview_visibility_keeps_the_last_interactive_surface_in_bounds() {
        use crate::config::{OverviewPanel as Id, OverviewPanels, Panel};
        for mask in 0..64u8 {
            for side in [false, true] {
                let mut policy = OverviewPanels::default();
                for (bit, panel) in Id::ALL.iter().enumerate() {
                    policy.set_shown(*panel, mask & (1 << bit) == 0).unwrap();
                }
                if policy.set_side_shown(side).is_err() {
                    assert!(!side && !policy.shown(Id::Monitor));
                    continue;
                }
                for width in 0..=160 {
                    for height in 0..=44 {
                        let area = Rect::new(2, 3, width, height);
                        let placement = overview_policy(area, &policy, false);
                        for pane in [placement.list, placement.conversation]
                            .into_iter()
                            .flatten()
                        {
                            assert_eq!(pane.intersection(area), pane);
                        }
                        if let (Some(left), Some(right)) = (placement.list, placement.conversation)
                        {
                            assert!(left.intersection(right).is_empty());
                        }
                        if !side {
                            assert_eq!(
                                placement,
                                Placement {
                                    list: Some(area),
                                    conversation: None
                                }
                            );
                        } else if mask == 63
                            || (!policy.shown(Id::Monitor) && (width < 140 || height < 12))
                        {
                            assert_eq!(
                                placement,
                                Placement {
                                    list: None,
                                    conversation: Some(area)
                                }
                            );
                        } else {
                            assert_eq!(placement, overview(area));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_order_and_visibility_has_the_same_geometry_as_its_canonical_order() {
        use crate::config::{OverviewPanel as Id, OverviewPanels, Panel};
        fn check_order(order: &mut [Id], index: usize, count: &mut usize) {
            if index < order.len() {
                for next in index..order.len() {
                    order.swap(index, next);
                    check_order(order, index + 1, count);
                    order.swap(index, next);
                }
                return;
            }
            *count += 1;
            let mut ordered = OverviewPanels::default();
            for (position, panel) in order.iter().enumerate() {
                while ordered.order().iter().position(|id| id == panel).unwrap() > position {
                    ordered.move_panel(*panel, false);
                }
            }
            assert_eq!(ordered.order(), order);
            // Geometry has no order-dependent budget. Exercise all permutations
            // and visibility combinations at each responsive boundary, while
            // the separate canonical-order sweep covers every terminal size.
            for mask in 0..64u8 {
                let mut canonical = OverviewPanels::default();
                let mut candidate = ordered.clone();
                for (bit, panel) in Id::ALL.iter().enumerate() {
                    let shown = mask & (1 << bit) == 0;
                    canonical.set_shown(*panel, shown).unwrap();
                    candidate.set_shown(*panel, shown).unwrap();
                }
                for side in [false, true] {
                    if canonical.set_side_shown(side).is_err() {
                        assert!(candidate.set_side_shown(side).is_err());
                        continue;
                    }
                    candidate.set_side_shown(side).unwrap();
                    for width in [0, 1, 60, 139, 140, 141, 160] {
                        for height in [0, 1, 11, 12, 44] {
                            let area = Rect::new(2, 3, width, height);
                            assert_eq!(
                                overview_policy(area, &candidate, false),
                                overview_policy(area, &canonical, false)
                            );
                        }
                    }
                }
            }
        }
        let mut count = 0;
        check_order(&mut Id::ALL.to_vec(), 0, &mut count);
        assert_eq!(count, 720);
    }


    #[test]
    fn agents_tree_matches_the_mock_bands_and_keeps_the_conversation_minimum() {
        let wide = agents_tree(Rect::new(0, 4, 160, 37), Preference::Auto, false);
        assert_eq!(wide.list, Some(Rect::new(0, 7, 75, 34)));
        assert_eq!(wide.conversation, Some(Rect::new(77, 7, 83, 34)));
        let medium = agents_tree(Rect::new(0, 4, 100, 17), Preference::Auto, false);
        assert_eq!(medium.list, Some(Rect::new(0, 7, 100, 4)));
        assert_eq!(medium.conversation, Some(Rect::new(0, 11, 100, 10)));
        for width in 100..=200 {
            for height in [0, 1, 4, 8, 10, 13, 17, 29, 37] {
                let area = Rect::new(2, 3, width, height);
                for preference in [Preference::Auto, Preference::Side, Preference::Stack] {
                    for focused in [false, true] {
                        let placed = agents_tree(area, preference, focused);
                        for pane in [placed.list, placed.conversation].into_iter().flatten() {
                            assert_eq!(pane.intersection(area), pane);
                        }
                        if let (Some(list), Some(conversation)) = (placed.list, placed.conversation)
                        {
                            assert!(list.intersection(conversation).is_empty());
                            assert!(conversation.height >= 8);
                            assert!(list.height >= 3);
                        } else {
                            assert_eq!(placed.conversation.is_some(), focused);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_size_and_preference_stays_in_bounds_without_overlap_or_padding_rows() {
        for width in [0, 1, 60, 80, 100, 140, 160] {
            for height in [0, 1, 18, 24, 36, 44] {
                let area = Rect::new(2, 3, width, height);
                for preference in [Preference::Auto, Preference::Side, Preference::Stack] {
                    for focused in [false, true] {
                        let placement = agents(area, preference, focused);
                        for pane in [placement.list, placement.conversation]
                            .into_iter()
                            .flatten()
                        {
                            assert_eq!(pane.intersection(area), pane);
                        }
                        if let (Some(list), Some(conversation)) =
                            (placement.list, placement.conversation)
                        {
                            assert!(list.intersection(conversation).is_empty());
                            assert!(
                                list.right() + 1 == conversation.x
                                    || list.bottom() == conversation.y
                            );
                        } else {
                            assert_eq!(placement.conversation.is_some(), focused);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn overview_uses_the_mock_split_without_changing_admission_or_agents() {
        for width in [139, 140, 141, 160, 200] {
            for height in [11, 12, 44] {
                let area = Rect::new(2, 3, width, height);
                let placed = overview(area);
                for pane in [placed.list, placed.conversation].into_iter().flatten() {
                    assert_eq!(pane.intersection(area), pane);
                }
                if width >= 140 && height >= 12 {
                    let list = placed.list.unwrap();
                    let conversation = placed.conversation.unwrap();
                    assert_eq!(list.width, (u32::from(width) * 3 / 5) as u16);
                    assert_eq!(list.right() + 1, conversation.x);
                    assert!(list.intersection(conversation).is_empty());
                } else {
                    assert_eq!(placed.list, Some(area));
                    assert!(placed.conversation.is_none());
                }
            }
        }
        let medium = overview(Rect::new(0, 0, 140, 44));
        assert_eq!(
            (
                medium.list.unwrap().width,
                medium.conversation.unwrap().width
            ),
            (84, 55)
        );
        let wide = overview(Rect::new(0, 0, 160, 44));
        assert_eq!(
            (wide.list.unwrap().width, wide.conversation.unwrap().width),
            (96, 63)
        );
        let columns = crate::board::overview::columns::visible(medium.list.unwrap().width);
        assert_eq!(columns, crate::config::OverviewColumn::ALL);
        assert!(columns.contains(&crate::config::OverviewColumn::Squad));
        assert!(columns.contains(&crate::config::OverviewColumn::Runtime));
        assert_eq!(
            agents(Rect::new(0, 0, 160, 44), Preference::Side, false)
                .list
                .unwrap()
                .width,
            64
        );
    }

    #[test]
    fn auto_stacks_at_medium_width_and_preferences_degrade_without_losing_focus() {
        let medium = agents(Rect::new(0, 0, 100, 36), Preference::Auto, false);
        assert_eq!(medium.list.unwrap().width, 100);
        let side = agents(Rect::new(0, 0, 100, 36), Preference::Side, false);
        assert_eq!(side.list.unwrap().height, 36);
        let low = agents(Rect::new(0, 0, 160, 8), Preference::Stack, true);
        assert!(low.list.is_none());
        assert_eq!(low.conversation.unwrap(), Rect::new(0, 0, 160, 8));
        assert!(overview(Rect::new(0, 0, 100, 36)).conversation.is_none());
        assert!(overview(Rect::new(0, 0, 160, 36)).conversation.is_some());
    }
}
