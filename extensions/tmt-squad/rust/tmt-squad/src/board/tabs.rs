//! The board's tabs (#507): every squad, and the built-in tabs, in the order
//! `[tabs]` sets. A tab is a key: a squad's name, or a built-in key that
//! starts with `@`, which no squad name can.

use crate::config::Tabs;

/// The built-in tab with one row per squad lead.
pub const LEADS: &str = "@leads";
/// The built-in overview tab with one row per squad.
pub const ALL: &str = "@all";

/// Whether the key is a built-in tab rather than a squad.
pub fn builtin(key: &str) -> bool {
    key.starts_with('@')
}

/// What the tab line shows for a key.
pub fn label(key: &str) -> &str {
    key.strip_prefix('@').unwrap_or(key)
}

/// The tabs in order: the configured `order` first (entries naming no
/// squad are skipped, since squads come and go), then the other squads in
/// core's order, then the built-in tabs not placed. Hidden tabs are left
/// out; a hidden squad is still reachable by name.
pub fn arrange(squads: &[String], tabs: &Tabs) -> Vec<String> {
    let exists = |key: &String| builtin(key) || squads.contains(key);
    let mut keys: Vec<String> = tabs
        .order
        .iter()
        .filter(|key| exists(key))
        .cloned()
        .collect();
    for key in squads
        .iter()
        .cloned()
        .chain([LEADS.to_owned(), ALL.to_owned()])
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.retain(|key| !tabs.hide.contains(key));
    keys
}

/// The switcher's choices for `query`: every key whose label holds the
/// query's characters in order, ignoring case. A label that starts with the
/// query ranks first, then one that contains it whole, then the rest; ties
/// keep `keys`' order.
pub fn matching<'a>(keys: &'a [String], query: &str) -> Vec<&'a String> {
    let query = query.to_lowercase();
    let mut found: Vec<(u8, usize, &String)> = keys
        .iter()
        .enumerate()
        .filter_map(|(index, key)| {
            let name = label(key).to_lowercase();
            let rank = if name.starts_with(&query) {
                0
            } else if name.contains(&query) {
                1
            } else {
                let mut rest = name.chars();
                if !query.chars().all(|wanted| rest.any(|seen| seen == wanted)) {
                    return None;
                }
                2
            };
            Some((rank, index, key))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, _, key)| key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(order: &[&str], hide: &[&str]) -> Tabs {
        Tabs {
            order: order.iter().map(|key| (*key).to_owned()).collect(),
            hide: hide.iter().map(|key| (*key).to_owned()).collect(),
            ..Tabs::default()
        }
    }

    #[test]
    fn configured_tabs_come_first_then_squads_then_built_ins_minus_hidden() {
        let squads: Vec<String> = ["product", "infra", "quiet"].map(String::from).to_vec();
        assert_eq!(
            arrange(&squads, &Tabs::default()),
            ["product", "infra", "quiet", LEADS, ALL]
        );
        assert_eq!(
            arrange(&squads, &tabs(&[ALL, LEADS, "infra", "gone"], &[])),
            [ALL, LEADS, "infra", "product", "quiet"],
            "a squad that no longer exists is skipped"
        );
        assert_eq!(
            arrange(&squads, &tabs(&["quiet"], &["product", LEADS, ALL])),
            ["quiet", "infra"]
        );
        assert_eq!(label(LEADS), "leads");
        assert_eq!(label(ALL), "all");
        assert_eq!(label("product"), "product");
        assert!(builtin(LEADS) && !builtin("leads"));
    }

    #[test]
    fn the_switcher_matches_prefix_then_substring_then_in_order_letters() {
        let keys: Vec<String> = ["infra", "product", "platform", LEADS, "prod-ops"]
            .map(String::from)
            .to_vec();
        let names = |query: &str| {
            matching(&keys, query)
                .into_iter()
                .map(|key| label(key))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(""),
            ["infra", "product", "platform", "leads", "prod-ops"]
        );
        assert_eq!(names("prod"), ["product", "prod-ops"]);
        assert_eq!(names("PL"), ["platform"]);
        assert_eq!(names("ops"), ["prod-ops"]);
        assert_eq!(names("pdt"), ["product"], "letters in order");
        assert_eq!(names("ra"), ["infra"], "platform has no a after its r");
        assert!(names("zz").is_empty());
    }
}
