//! Terminal-cell placement for the shared list and conversation. Preferences
//! choose a direction; minimum usable cell budgets always take precedence.
use ratatui::layout::Rect;

pub(in crate::board) const LIST_MIN_ROWS: u16 = 3;
pub(in crate::board) const CONVERSATION_MIN_ROWS: u16 = 8;
pub(in crate::board) const LIST_MIN_COLUMNS: u16 = 36;
pub(in crate::board) const CONVERSATION_MIN_COLUMNS: u16 = 36;
// Keep the Agents-list token header whole; the optional model track yields first.
pub(in crate::board) const OVERVIEW_MIN_COLUMNS: u16 = 1 + 23 + 1 + 11 + 1 + 12 + 1 + 7 + 3 + 3;

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

/// Agents-only geometry; The list's controls sit on its own section rule.
pub fn agents_tree(area: Rect, preference: Preference, conversation_focused: bool) -> Placement {
    let body = area;
    let dual_fits = body.height >= LIST_MIN_ROWS + CONVERSATION_MIN_ROWS;
    let side_fits = area.width >= 100 && dual_fits;
    let stack_fits = area.width >= 80 && dual_fits;
    let wants_side =
        preference == Preference::Side || preference == Preference::Auto && area.width >= 120;
    if side_fits && (wants_side || !stack_fits) {
        let left = (((u32::from(area.width) * 47 + 50) / 100) as u16).max(LIST_MIN_COLUMNS);
        Placement {
            list: Some(Rect::new(body.x, body.y, left, body.height)),
            conversation: Some(Rect::new(
                body.x + left + 1,
                body.y,
                body.width - left - 1,
                body.height,
            )),
        }
    } else if stack_fits {
        // The mock's range is at min(16, floor(terminal_rows * .42)). Its
        // four shell rows and three footer rows precede/follow this area.
        let top = ((u32::from(area.height) + 7) * 42 / 100)
            .saturating_sub(6)
            .min(10) as u16;
        let top = top.clamp(LIST_MIN_ROWS, body.height - CONVERSATION_MIN_ROWS);
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
    }
}

fn side(area: Rect) -> Placement {
    let left = ((u32::from(area.width) * 2 / 5) as u16).max(LIST_MIN_COLUMNS);
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
    fn agents_tree_keeps_the_conversation_minimum() {
        let wide = agents_tree(Rect::new(0, 4, 160, 37), Preference::Auto, false);
        assert_eq!(wide.list, Some(Rect::new(0, 4, 75, 37)));
        assert_eq!(wide.conversation, Some(Rect::new(76, 4, 84, 37)));
        let medium = agents_tree(Rect::new(0, 4, 100, 17), Preference::Auto, false);
        assert_eq!(medium.list, Some(Rect::new(0, 4, 100, 4)));
        assert_eq!(medium.conversation, Some(Rect::new(0, 8, 100, 13)));
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
    fn auto_stacks_at_medium_width_and_preferences_degrade_without_losing_focus() {
        let medium = agents(Rect::new(0, 0, 100, 36), Preference::Auto, false);
        assert_eq!(medium.list.unwrap().width, 100);
        let side = agents(Rect::new(0, 0, 100, 36), Preference::Side, false);
        assert_eq!(side.list.unwrap().height, 36);
        let low = agents(Rect::new(0, 0, 160, 8), Preference::Stack, true);
        assert!(low.list.is_none());
        assert_eq!(low.conversation.unwrap(), Rect::new(0, 0, 160, 8));
    }
}
