//! The home summary: two display-only lines the space's maintainer agent writes for Today's
//! greeting (contract: Home summary). Like page labels it carries no authority, is not signed and
//! never enters the owner log or any page stream. This module owns the grammar and the shape the
//! store, the serve route and the CLI share; the store owns the one row.
use crate::limits::{HOME_SUMMARY_FIRST_POINTS, HOME_SUMMARY_SECOND_POINTS};
use serde::{Deserialize, Serialize};

pub mod ipc;

/// The stored summary and the object the owner-served discovery reply carries as `homeSummary`.
/// `author` is the writing agent's display name (omitted when the writer had no agent identity)
/// and `at_ms` the owner's clock at the write; a request names neither.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HomeSummary {
    pub lines: [String; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub at_ms: u64,
}

/// The most code points each line holds, first line first.
pub const LINE_POINTS: [usize; 2] = [HOME_SUMMARY_FIRST_POINTS, HOME_SUMMARY_SECOND_POINTS];

/// One line: 1 to `max` code points, not blank, no control character (so it cannot break the two
/// line layout). Whitespace inside it is kept as written; length is refused, never truncated.
pub fn line_valid(line: &str, max: usize) -> bool {
    (1..=max).contains(&line.chars().count())
        && !line.trim().is_empty()
        && !line.chars().any(char::is_control)
}

/// Exactly two lines, each within its own limit.
pub fn lines_valid(lines: &[String]) -> bool {
    lines.len() == LINE_POINTS.len()
        && lines
            .iter()
            .zip(LINE_POINTS)
            .all(|(line, max)| line_valid(line, max))
}

/// The writer's display name as `page create` records it, or none: an invalid name is dropped
/// rather than invented around.
pub fn author_valid(author: &str) -> bool {
    crate::decoder::valid_publisher_agent(author)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(first: &str, second: &str) -> Vec<String> {
        vec![first.to_owned(), second.to_owned()]
    }

    #[test]
    fn each_line_has_its_own_limit_counted_in_code_points() {
        assert!(lines_valid(&pair(&"字".repeat(24), &"é".repeat(60))));
        assert!(lines_valid(&pair("🚀", "x")));
        assert!(!lines_valid(&pair(&"字".repeat(25), "ok")));
        assert!(!lines_valid(&pair("ok", &"é".repeat(61))));
        // The first line's limit is not the second's.
        assert!(!lines_valid(&pair(&"x".repeat(60), "ok")));
    }

    #[test]
    fn a_summary_is_exactly_two_nonblank_lines_without_control_characters() {
        assert!(!lines_valid(&["one".to_owned()]));
        assert!(!lines_valid(&[
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned()
        ]));
        for bad in ["", "   ", "\t", "line\nbreak", "bell\u{7}", "nul\u{0}"] {
            assert!(!lines_valid(&pair(bad, "ok")), "{bad:?}");
            assert!(!lines_valid(&pair("ok", bad)), "{bad:?}");
        }
        // Inner spacing is the writer's own.
        assert!(lines_valid(&pair(" two  spaces ", "ok")));
    }

    #[test]
    fn the_wire_shape_names_lines_author_and_time_and_omits_a_missing_author() {
        let with = HomeSummary {
            lines: ["a".to_owned(), "b".to_owned()],
            author: Some("Ada".to_owned()),
            at_ms: 7,
        };
        assert_eq!(
            serde_json::to_value(&with).unwrap(),
            serde_json::json!({"lines":["a","b"],"author":"Ada","atMs":7})
        );
        let without = HomeSummary {
            author: None,
            ..with
        };
        assert_eq!(
            serde_json::to_value(&without).unwrap(),
            serde_json::json!({"lines":["a","b"],"atMs":7})
        );
        assert!(
            serde_json::from_str::<HomeSummary>(r#"{"lines":["a","b"],"atMs":7,"extra":1}"#)
                .is_err()
        );
    }
}
