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
        side(area)
    } else {
        Placement {
            list: Some(area),
            conversation: None,
        }
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
                area.y + top + 1,
                area.width,
                area.height - top - 1,
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
                                    || list.bottom() + 1 == conversation.y
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
        assert!(overview(Rect::new(0, 0, 100, 36)).conversation.is_none());
        assert!(overview(Rect::new(0, 0, 160, 36)).conversation.is_some());
    }
}
