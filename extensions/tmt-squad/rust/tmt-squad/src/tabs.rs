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
    matches!(key, LEADS | ALL)
}

pub fn user_key(name: &str) -> String {
    format!("@tab:{name}")
}
pub fn user_name(key: &str) -> Option<&str> {
    key.strip_prefix("@tab:")
}
pub fn aggregate(key: &str) -> bool {
    builtin(key) || user_name(key).is_some()
}
pub fn reserved(name: &str) -> bool {
    ["leads", "all", "colors", "order", "pin", "hide"].contains(&name)
}

/// What the tab line shows for a key.
pub fn label(key: &str) -> &str {
    user_name(key).unwrap_or_else(|| key.strip_prefix('@').unwrap_or(key))
}

/// The tabs in order, and how many of them are pinned: pinned tabs first,
/// in `pin`'s order, then the configured `order` (entries naming no squad
/// are skipped, since squads come and go), then the other squads in core's
/// order, then the built-in tabs not placed. Hidden tabs are left out, even
/// when pinned; a hidden squad is still reachable by name.
pub fn arrange(squads: &[String], tabs: &Tabs) -> (Vec<String>, usize) {
    let users = tabs
        .user
        .iter()
        .map(|tab| user_key(&tab.name))
        .collect::<Vec<_>>();
    let exists = |key: &&String| builtin(key) || users.contains(key) || squads.contains(key);
    let shown = |key: &&String| !tabs.hide.contains(key);
    let mut keys: Vec<String> = tabs
        .pin
        .iter()
        .filter(exists)
        .filter(shown)
        .cloned()
        .collect();
    let pinned = keys.len();
    let rest = tabs
        .order
        .iter()
        .filter(exists)
        .cloned()
        .chain(squads.iter().cloned())
        .chain([LEADS.to_owned(), ALL.to_owned()])
        .chain(users.iter().cloned());
    for key in rest {
        if !keys.contains(&key) && !tabs.hide.contains(&key) {
            keys.push(key);
        }
    }
    (keys, pinned)
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
            pin: Vec::new(),
            ..Tabs::default()
        }
    }

    fn keys(arranged: (Vec<String>, usize)) -> Vec<String> {
        assert_eq!(arranged.1, 0, "nothing pinned");
        arranged.0
    }

    #[test]
    fn configured_tabs_come_first_then_squads_then_built_ins_minus_hidden() {
        let squads: Vec<String> = ["product", "infra", "quiet"].map(String::from).to_vec();
        assert_eq!(
            arrange(&squads, &Tabs::default()),
            (
                [ALL, LEADS, "product", "infra", "quiet"]
                    .map(String::from)
                    .to_vec(),
                1
            )
        );
        assert_eq!(
            keys(arrange(&squads, &tabs(&[ALL, LEADS, "infra", "gone"], &[]))),
            [ALL, LEADS, "infra", "product", "quiet"],
            "a squad that no longer exists is skipped"
        );
        assert_eq!(
            keys(arrange(
                &squads,
                &tabs(&["quiet"], &["product", LEADS, ALL])
            )),
            ["quiet", "infra"]
        );
        let mut pinned = tabs(&["infra"], &["quiet"]);
        pinned.pin = [ALL, "quiet", "product", "gone"].map(String::from).to_vec();
        assert_eq!(
            arrange(&squads, &pinned),
            (
                [ALL, "product", "infra", LEADS].map(String::from).to_vec(),
                2
            ),
            "pins first; a hidden or missing pin is skipped"
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
