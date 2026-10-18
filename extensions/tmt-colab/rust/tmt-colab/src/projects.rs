//! Projects: display-only names that organize the owner's page list; a page can be in several. They
//! carry no authority, are not signed and never enter the owner log (contract: Projects). This module
//! owns the grammar and the per-page set rules; the store owns the space limits.
use crate::limits::{PAGE_PROJECTS, PROJECT_POINTS};

/// Why a project set is refused. `Capacity` is a limit, never a silent truncation.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Invalid,
    Capacity,
}

/// One change to the space's projects, independent of any page (contract: Projects).
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Create(String),
    /// Onto an existing name it merges: a page in both keeps one entry.
    Rename {
        from: String,
        to: String,
    },
    /// Removes the grouping only; no page is deleted.
    Delete(String),
}

/// One name: 1 to `PROJECT_POINTS` code points, no control character and no whitespace other than
/// single interior spaces, so a name survives a filter token and never hides its edges.
pub fn valid(name: &str) -> bool {
    let points = name.chars().count();
    (1..=PROJECT_POINTS).contains(&points)
        && !name.starts_with(' ')
        && !name.ends_with(' ')
        && !name.contains("  ")
        && name
            .chars()
            .all(|c| !c.is_control() && (c == ' ' || !c.is_whitespace()))
}

/// The one stored form of a page's project set: valid, unique, sorted by code point, and at most
/// `PAGE_PROJECTS`. A repeated name is invalid rather than merged, so a request means one thing.
pub fn canonical(projects: &[String]) -> Result<Vec<String>, Refusal> {
    if projects.iter().any(|name| !valid(name)) {
        return Err(Refusal::Invalid);
    }
    let mut sorted = projects.to_vec();
    sorted.sort();
    if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(Refusal::Invalid);
    }
    if sorted.len() > PAGE_PROJECTS {
        return Err(Refusal::Capacity);
    }
    Ok(sorted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_name_is_one_to_thirty_two_points_without_control_characters_or_edge_whitespace() {
        let longest = "x".repeat(32);
        for name in ["a", "Q4 plan", "日本語", "🚀 launch", longest.as_str()] {
            assert!(valid(name), "{name:?}");
        }
        let too_long = "x".repeat(33);
        let bad = [
            "",
            " ",
            " lead",
            "trail ",
            "two  spaces",
            "tab\there",
            "line\nbreak",
            "nul\0",
            "nb\u{a0}sp",
            "wide\u{3000}space",
            "ls\u{2028}x",
            "next\u{85}line",
            too_long.as_str(),
        ];
        for name in bad {
            assert!(!valid(name), "{name:?}");
        }
    }

    /// The browser's `projectNameError` reads the same file, so the two grammars cannot drift.
    #[test]
    fn the_shared_vector_agrees_with_the_grammar() {
        let vector: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/vectors/project-name-v1.json"
        ))
        .unwrap();
        for (key, expected) in [("valid", true), ("invalid", false)] {
            let names = vector[key].as_array().unwrap();
            assert!(!names.is_empty());
            for name in names {
                let name = name.as_str().unwrap();
                assert_eq!(valid(name), expected, "{key}: {name:?}");
            }
        }
    }

    #[test]
    fn the_point_limit_counts_code_points_not_bytes() {
        assert!(valid(&"é".repeat(32)));
        assert!(valid(&"🚀".repeat(32)));
        assert!(!valid(&"🚀".repeat(33)));
    }

    #[test]
    fn a_set_is_sorted_unique_and_bounded_per_page() {
        assert_eq!(canonical(&[]), Ok(vec![]));
        assert_eq!(canonical(&set(&["b", "a"])), Ok(set(&["a", "b"])));
        assert_eq!(canonical(&set(&["a", "a"])), Err(Refusal::Invalid));
        // Case is part of a name: the two spellings are two projects.
        assert_eq!(canonical(&set(&["a", "A"])), Ok(set(&["A", "a"])));
        assert_eq!(canonical(&set(&["ok", " bad"])), Err(Refusal::Invalid));
        let full: Vec<String> = (0..PAGE_PROJECTS).map(|n| format!("l{n}")).collect();
        assert_eq!(canonical(&full).map(|v| v.len()), Ok(PAGE_PROJECTS));
        let over: Vec<String> = (0..=PAGE_PROJECTS).map(|n| format!("l{n}")).collect();
        assert_eq!(canonical(&over), Err(Refusal::Capacity));
    }

    #[test]
    fn an_invalid_name_wins_over_a_capacity_refusal() {
        let mut over: Vec<String> = (0..=PAGE_PROJECTS).map(|n| format!("l{n}")).collect();
        over.push(String::new());
        assert_eq!(canonical(&over), Err(Refusal::Invalid));
    }
}
