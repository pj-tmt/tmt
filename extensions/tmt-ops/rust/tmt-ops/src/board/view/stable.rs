//! What the open tab has already laid out, so a refresh cannot move it.
//!
//! A snapshot owns its derived caches and is replaced whole, so anything that
//! must outlive one lives here, beside the application: the widest each cell
//! has needed (widths only grow while the tab stays open at this terminal width)
//! and the last model and age each row reported (shown dim while a refresh has
//! not read a new one). Layout never reads a value as it is loaded now; a
//! placeholder, a carried value and a fresh value therefore occupy the same cells.

use std::collections::BTreeMap;

/// How long a value from an earlier read stands in for a missing one.
const CARRY_MS: u64 = 10 * 60 * 1000;

#[derive(Default)]
pub(in crate::board) struct Stable {
    tab: String,
    width: u16,
    widths: BTreeMap<String, usize>,
    seen: BTreeMap<String, Seen>,
}

#[derive(Default)]
struct Seen {
    /// The row's model and when it was last read.
    model: Option<(String, u64)>,
    /// When the row last changed, as milliseconds since the epoch.
    since: Option<(u64, u64)>,
}

/// A value for one cell and whether it is a remembered one.
pub(in crate::board) struct Carried<T> {
    pub value: T,
    pub carried: bool,
}

impl Stable {
    /// Starts a frame. Another tab starts over; another terminal width keeps the
    /// remembered values and re-derives the widths.
    pub fn begin(&mut self, tab: &str, width: u16) {
        if self.tab != tab {
            *self = Self {
                tab: tab.to_owned(),
                width,
                ..Self::default()
            };
        } else if self.width != width {
            self.width = width;
            self.widths.clear();
        }
    }

    /// The cell's width: the widest it has needed since the tab opened at this width.
    pub fn width(&mut self, cell: &str, wanted: usize) -> usize {
        let width = self.widths.entry(cell.to_owned()).or_default();
        *width = (*width).max(wanted);
        *width
    }

    /// The row's model, or the last one read for it while this read has none.
    pub fn model(&mut self, id: Option<&str>, current: &str, now: u64) -> Carried<String> {
        let Some(id) = id else {
            return Carried {
                value: current.to_owned(),
                carried: false,
            };
        };
        let seen = self.seen.entry(id.to_owned()).or_default();
        if !current.is_empty() {
            seen.model = Some((current.to_owned(), now));
        }
        match &seen.model {
            Some((model, at)) if current.is_empty() && now.saturating_sub(*at) <= CARRY_MS => {
                Carried {
                    value: model.clone(),
                    carried: true,
                }
            }
            _ => Carried {
                value: current.to_owned(),
                carried: false,
            },
        }
    }

    /// When the row last changed. A read that could not say (`unknown`) shows the
    /// last time read for it instead; one that says there is none forgets it.
    pub fn since(
        &mut self,
        id: Option<&str>,
        current: Option<u64>,
        unknown: bool,
        now: u64,
    ) -> Carried<Option<u64>> {
        let Some(id) = id else {
            return Carried {
                value: current,
                carried: false,
            };
        };
        let seen = self.seen.entry(id.to_owned()).or_default();
        if let Some(since) = current {
            seen.since = Some((since, now));
        } else if !unknown {
            seen.since = None;
        }
        match seen.since {
            Some((since, at)) if current.is_none() && now.saturating_sub(at) <= CARRY_MS => {
                Carried {
                    value: Some(since),
                    carried: true,
                }
            }
            _ => Carried {
                value: current,
                carried: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_only_grow_until_the_tab_or_the_terminal_width_changes() {
        let mut stable = Stable::default();
        stable.begin("a", 100);
        assert_eq!(stable.width("model", 3), 3);
        assert_eq!(
            stable.width("model", 0),
            3,
            "a smaller need keeps the width"
        );
        assert_eq!(stable.width("model", 5), 5);
        stable.begin("a", 100);
        assert_eq!(
            stable.width("model", 0),
            5,
            "the same tab and width keep it"
        );
        stable.begin("a", 80);
        assert_eq!(stable.width("model", 0), 0, "a resize derives it again");
        stable.width("model", 4);
        stable.begin("b", 80);
        assert_eq!(stable.width("model", 0), 0, "another tab starts over");
    }

    #[test]
    fn a_missing_value_shows_the_last_one_read_for_that_row_until_it_expires() {
        let mut stable = Stable::default();
        let id = Some("a");
        assert!(!stable.model(id, "", 1_000).carried, "nothing was read yet");
        assert_eq!(stable.model(id, "sol", 1_000).value, "sol");
        let carried = stable.model(id, "", 2_000);
        assert_eq!((carried.value.as_str(), carried.carried), ("sol", true));
        assert!(!stable.model(id, "", 1_000 + CARRY_MS + 1).carried);
        assert!(
            !stable.model(Some("b"), "", 2_000).carried,
            "rows are separate"
        );
        assert_eq!(stable.since(id, Some(500), false, 2_000).value, Some(500));
        let carried = stable.since(id, None, true, 3_000);
        assert_eq!((carried.value, carried.carried), (Some(500), true));
        assert_eq!(stable.since(id, Some(900), false, 4_000).value, Some(900));
        assert!(
            !stable.since(id, None, false, 5_000).carried,
            "a read that says there is no age drops the old one"
        );
        assert!(!stable.since(id, None, true, 6_000).carried);
    }
}
