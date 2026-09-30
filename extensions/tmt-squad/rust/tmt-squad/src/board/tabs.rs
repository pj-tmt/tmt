//! The board's tabs (#507): every squad, and the built-in tabs, in the order
//! `[tabs]` sets. A tab is a key: a squad's name, or a built-in key that
//! starts with `@`, which no squad name can.

use crate::config::Tabs;

/// The built-in tab with one row per squad lead.
pub const LEADS: &str = "@leads";

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
    for key in squads.iter().cloned().chain([LEADS.to_owned()]) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.retain(|key| !tabs.hide.contains(key));
    keys
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
            ["product", "infra", "quiet", LEADS]
        );
        assert_eq!(
            arrange(&squads, &tabs(&[LEADS, "infra", "gone"], &[])),
            [LEADS, "infra", "product", "quiet"],
            "a squad that no longer exists is skipped"
        );
        assert_eq!(
            arrange(&squads, &tabs(&["quiet"], &["product", LEADS])),
            ["quiet", "infra"]
        );
        assert_eq!(label(LEADS), "leads");
        assert_eq!(label("product"), "product");
        assert!(builtin(LEADS) && !builtin("leads"));
    }
}
