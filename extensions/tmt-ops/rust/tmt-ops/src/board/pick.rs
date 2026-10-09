//! This board's transient tab policy; global arrangement and hide remain config-owned.

use crate::{core::SquadError, tabs};
use std::collections::BTreeSet;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Picks(Option<BTreeSet<String>>);

impl Picks {
    /// Names are resolved against the board inventory, never the `ls --tab` resolver.
    pub fn parse(input: Option<&str>, inventory: &[String]) -> Result<Self, SquadError> {
        let Some(input) = input else {
            return Ok(Self::default());
        };
        if input == "all" {
            return Ok(Self::default());
        }
        let valid = inventory
            .iter()
            .map(|key| match key.as_str() {
                tabs::ALL => "all",
                tabs::LEADS => "leads",
                _ => key,
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut picked = BTreeSet::new();
        for name in input.split(',') {
            let key = inventory
                .iter()
                .find(|key| key.as_str() == name)
                .or_else(|| {
                    inventory.iter().find(|key| {
                        tabs::label(key) == name
                            || tabs::user_name(key)
                                .is_some_and(|user| name == format!("tab:{user}"))
                    })
                });
            if name.is_empty() || name == "all" || key.is_none() {
                return Err(SquadError::new(
                    "USAGE_ERROR",
                    format!(
                        "Invalid --tabs name '{name}'. Use all for the default set, or select from: {valid}."
                    ),
                ));
            }
            picked.insert(key.unwrap().clone());
        }
        Ok(Self(Some(picked)))
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.as_ref().is_none_or(|picked| picked.contains(key))
    }

    pub fn include(&mut self, key: &str) {
        if let Some(picked) = &mut self.0 {
            picked.insert(key.to_owned());
        }
    }

    pub fn toggle(&mut self, key: &str, inventory: &[String]) {
        let picked = self
            .0
            .get_or_insert_with(|| inventory.iter().cloned().collect());
        if !picked.remove(key) {
            picked.insert(key.to_owned());
        }
    }

    pub fn reconcile(&mut self, inventory: &[String]) {
        if let Some(picked) = &mut self.0 {
            picked.retain(|key| inventory.contains(key));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory() -> Vec<String> {
        [
            tabs::ALL,
            tabs::LEADS,
            "product",
            "infra",
            "@tab:needs-me",
            "@tab:product",
        ]
        .map(String::from)
        .to_vec()
    }

    #[test]
    fn omitted_and_all_follow_the_inventory_and_explicit_names_resolve_once() {
        let keys = inventory();
        for input in [None, Some("all")] {
            let picks = Picks::parse(input, &keys).unwrap();
            assert!(keys.iter().all(|key| picks.contains(key)));
            assert!(picks.contains("new-squad"));
        }
        let picks = Picks::parse(Some("product,@tab:needs-me,leads,product"), &keys).unwrap();
        assert!(
            picks.contains("product")
                && picks.contains("@tab:needs-me")
                && picks.contains(tabs::LEADS)
        );
        assert!(
            !picks.contains(tabs::ALL)
                && !picks.contains("infra")
                && !picks.contains("@tab:product")
        );
        assert_eq!(
            Picks::parse(Some("needs-me"), &keys),
            Picks::parse(Some("tab:needs-me"), &keys)
        );
    }

    #[test]
    fn invalid_lists_report_usage_and_available_names() {
        for input in [
            "",
            ",product",
            "product,",
            "all,product",
            "gone",
            "product, infra",
        ] {
            let error = Picks::parse(Some(input), &inventory()).unwrap_err();
            assert_eq!(error.code, "USAGE_ERROR");
            assert!(error.message.contains("product") && error.message.contains("@tab:needs-me"));
        }
    }

    #[test]
    fn a_squad_name_wins_over_a_user_label_regardless_of_arrangement() {
        let keys = ["@tab:product", "product", tabs::ALL].map(String::from);
        let picks = Picks::parse(Some("product"), &keys).unwrap();
        assert!(picks.contains("product") && !picks.contains("@tab:product"));
        let picks = Picks::parse(Some("@tab:product"), &keys).unwrap();
        assert!(picks.contains("@tab:product") && !picks.contains("product"));
    }

    #[test]
    fn toggling_freezes_only_this_board_and_removed_keys_are_pruned() {
        let mut picks = Picks::default();
        let other = picks.clone();
        let mut keys = inventory();
        picks.toggle("product", &keys);
        assert!(!picks.contains("product") && other.contains("product"));
        assert!(!picks.contains("new-squad"));
        picks.toggle("product", &keys);
        assert!(picks.contains("product"));
        keys.retain(|key| key != "product");
        picks.reconcile(&keys);
        assert!(!picks.contains("product"));
    }
}
