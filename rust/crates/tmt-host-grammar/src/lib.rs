//! A terminal host's name, pane-ID syntax and target syntax, as a driver
//! declares them (`contracts/driver-protocol-v1.md`). Core stores pane IDs
//! as opaque strings and asks their host about syntax, so every host's IDs
//! and targets must be told apart from tmux's and from each other's. This is
//! the one definition: core matches stored IDs with it, and the driver
//! protocol validates declarations with it. It depends on nothing.

use std::fmt;

const NAME_MAX: usize = 32;
const PREFIX_MAX: usize = 16;
const SUFFIX_MAX: usize = 64;
const TARGET_MAX: usize = 32;
const DIGITS_MAX: usize = 9;

/// A host's validated syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGrammar {
    name: String,
    prefix: String,
    target: Option<Vec<Token>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarError(String);

impl GrammarError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for GrammarError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(&self.0)
    }
}

impl std::error::Error for GrammarError {}

impl HostGrammar {
    /// `name` is the stored host token, `prefix` starts every pane ID, and
    /// `target` is the template users name panes with (`w{n}:p{n}`).
    pub fn new(name: &str, prefix: &str, target: Option<&str>) -> Result<Self, GrammarError> {
        if !valid_name(name) {
            return Err(GrammarError::new(
                "name must be a lowercase letter then up to 31 of [a-z0-9-]",
            ));
        }
        // A prefix ends in `_` or `-` and a suffix has neither, so a pane ID
        // has exactly one prefix: two different prefixes never share an ID.
        if !(2..=PREFIX_MAX).contains(&prefix.len())
            || !prefix.starts_with(|c: char| c.is_ascii_lowercase())
            || !prefix.ends_with(['_', '-'])
            || !prefix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(GrammarError::new(
                "paneId.prefix must be 2-16 of [a-z0-9_-], start with a letter and end in _ or -",
            ));
        }
        Ok(Self {
            name: name.to_owned(),
            prefix: prefix.to_owned(),
            target: target.map(parse_template).transpose()?,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn pane_id_prefix(&self) -> &str {
        &self.prefix
    }

    pub fn is_pane_id(&self, value: &str) -> bool {
        value.strip_prefix(&self.prefix).is_some_and(|suffix| {
            (1..=SUFFIX_MAX).contains(&suffix.len())
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
        })
    }

    pub fn is_target(&self, value: &str) -> bool {
        self.target
            .as_ref()
            .is_some_and(|tokens| matches(tokens, value))
    }

    /// A target with every number `1`, for comparing with other syntaxes.
    pub fn sample_target(&self) -> Option<String> {
        self.target_numbered("1")
    }

    /// A target with every number `digits`.
    pub fn target_numbered(&self, digits: &str) -> Option<String> {
        self.target.as_ref().map(|tokens| {
            tokens
                .iter()
                .map(|token| match token {
                    Token::Literal(c) => c.to_string(),
                    Token::Number => digits.to_owned(),
                })
                .collect()
        })
    }

    /// Why two hosts can't be installed together, if they can't: the same
    /// name, the same pane-ID prefix, or targets one of them would read as
    /// its own. Core also compares both against tmux's syntax.
    pub fn conflict(&self, other: &Self) -> Option<String> {
        if self.name == other.name {
            return Some(format!("both are named {}", self.name));
        }
        if self.prefix == other.prefix {
            return Some(format!("both use the pane-ID prefix {}", self.prefix));
        }
        let reads = |one: &Self, two: &Self| {
            one.sample_target()
                .is_some_and(|sample| two.is_target(&sample))
        };
        (reads(self, other) || reads(other, self))
            .then(|| format!("{} and {} targets look alike", self.name, other.name))
    }
}

fn valid_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Literals are `[a-z:]` and the template starts with a letter, so a target
/// is never a number, a pane ID of any host, or tmux's `window.pane`.
fn parse_template(template: &str) -> Result<Vec<Token>, GrammarError> {
    let error = || {
        GrammarError::new(
            "target must be up to 32 of [a-z:] and {n}, start with a letter and hold at least one {n}",
        )
    };
    if template.len() > TARGET_MAX || !template.starts_with(|c: char| c.is_ascii_lowercase()) {
        return Err(error());
    }
    let mut tokens = Vec::new();
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("{n}") {
            // Two numbers in a row would have no boundary between them.
            if tokens.last() == Some(&Token::Number) {
                return Err(error());
            }
            tokens.push(Token::Number);
            rest = after;
        } else if c.is_ascii_lowercase() || c == ':' {
            tokens.push(Token::Literal(c));
            rest = &rest[1..];
        } else {
            return Err(error());
        }
    }
    if !tokens.contains(&Token::Number) {
        return Err(error());
    }
    Ok(tokens)
}

fn matches(tokens: &[Token], value: &str) -> bool {
    let mut rest = value;
    for token in tokens {
        match token {
            Token::Literal(c) => match rest.strip_prefix(*c) {
                Some(after) => rest = after,
                None => return false,
            },
            Token::Number => {
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                if !(1..=DIGITS_MAX).contains(&digits) {
                    return false;
                }
                rest = &rest[digits..];
            }
        }
    }
    rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn herdr() -> HostGrammar {
        HostGrammar::new("herdr", "term_", Some("w{n}:p{n}")).unwrap()
    }

    #[test]
    fn pane_ids_and_targets_follow_the_declared_syntax() {
        let grammar = herdr();
        assert_eq!(grammar.name(), "herdr");
        assert_eq!(grammar.pane_id_prefix(), "term_");
        assert!(grammar.is_pane_id("term_0a9z"));
        for id in ["term_", "term_A1", "term_a_b", "%1", "xterm_1"] {
            assert!(!grammar.is_pane_id(id), "{id}");
        }
        assert!(grammar.is_pane_id(&format!("term_{}", "a".repeat(64))));
        assert!(!grammar.is_pane_id(&format!("term_{}", "a".repeat(65))));
        assert!(grammar.is_target("w1:p23"));
        for target in ["w:p1", "w1:p", "w1:p2x", "W1:p2", "w1234567890:p1"] {
            assert!(!grammar.is_target(target), "{target}");
        }
        assert_eq!(grammar.sample_target().as_deref(), Some("w1:p1"));
        assert_eq!(grammar.target_numbered("42").as_deref(), Some("w42:p42"));
        let untargeted = HostGrammar::new("plain", "pane-", None).unwrap();
        assert!(!untargeted.is_target("w1:p1"));
        assert_eq!(untargeted.sample_target(), None);
    }

    #[test]
    fn unsafe_declarations_are_refused() {
        for (name, prefix, target) in [
            ("Herdr", "term_", None),
            ("9lives", "term_", None),
            (&*"a".repeat(33), "term_", None),
            ("herdr", "term", None),
            ("herdr", "%_", None),
            ("herdr", "_x_", None),
            ("herdr", "term_", Some("{n}.{n}")),
            ("herdr", "term_", Some("w{n}{n}")),
            ("herdr", "term_", Some("pane")),
            ("herdr", "term_", Some("w{n}.p{n}")),
        ] {
            assert!(
                HostGrammar::new(name, prefix, target).is_err(),
                "{name} {prefix} {target:?}"
            );
        }
    }

    #[test]
    fn hosts_that_could_read_each_others_ids_conflict() {
        let herdr = herdr();
        let other =
            |prefix: &str, target: Option<&str>| HostGrammar::new("other", prefix, target).unwrap();
        assert_eq!(herdr.conflict(&other("pane-", Some("t{n}"))), None);
        assert!(herdr.conflict(&other("term_", None)).is_some());
        assert!(herdr.conflict(&other("pane-", Some("w{n}:p{n}"))).is_some());
        assert!(herdr.conflict(&herdr.clone()).is_some(), "same name");
    }
}
