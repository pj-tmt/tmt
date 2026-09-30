//! The syntax a driver declares for its pane IDs and targets, and the text
//! rules every string from a driver meets. Core stores pane IDs as opaque
//! strings and asks their host about syntax, so each driver's IDs must be
//! told apart from tmux's and from every other driver's.

use crate::wire::Capabilities;
use std::fmt;

const NAME_MAX: usize = 32;
const PREFIX_MAX: usize = 16;
const SUFFIX_MAX: usize = 64;
const TARGET_MAX: usize = 32;
const ENV_MAX: usize = 4;
const ENV_NAME_MAX: usize = 64;
const DIGITS_MAX: usize = 9;

/// A driver's validated syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grammar {
    name: String,
    prefix: String,
    target: Option<Vec<Token>>,
    caller_env: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarError(String);

impl fmt::Display for GrammarError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(&self.0)
    }
}

impl std::error::Error for GrammarError {}

fn invalid(message: impl Into<String>) -> GrammarError {
    GrammarError(message.into())
}

impl Grammar {
    pub fn from_capabilities(capabilities: &Capabilities) -> Result<Self, GrammarError> {
        let name = &capabilities.name;
        if !valid_name(name) {
            return Err(invalid(
                "name must be a lowercase letter then up to 31 of [a-z0-9-]",
            ));
        }
        // A prefix ends in `_` or `-` and a suffix has neither, so a pane ID
        // has exactly one prefix: two different prefixes never share an ID.
        let prefix = &capabilities.pane_id.prefix;
        if !(2..=PREFIX_MAX).contains(&prefix.len())
            || !prefix.starts_with(|c: char| c.is_ascii_lowercase())
            || !prefix.ends_with(['_', '-'])
            || !prefix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(invalid(
                "paneId.prefix must be 2-16 of [a-z0-9_-], start with a letter and end in _ or -",
            ));
        }
        let target = capabilities
            .target
            .as_deref()
            .map(parse_template)
            .transpose()?;
        if capabilities.caller_env.len() > ENV_MAX {
            return Err(invalid("callerEnv names at most 4 variables"));
        }
        for variable in &capabilities.caller_env {
            if !valid_env_name(variable) {
                return Err(invalid(format!(
                    "callerEnv entry {variable:?} must be [A-Z][A-Z0-9_]* and not TMT_*"
                )));
            }
        }
        Ok(Self {
            name: name.clone(),
            prefix: prefix.clone(),
            target,
            caller_env: capabilities.caller_env.clone(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn caller_env(&self) -> &[String] {
        &self.caller_env
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

    pub fn pane_id_prefix(&self) -> &str {
        &self.prefix
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

    /// Why two drivers can't be installed together, if they can't: the same
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

fn valid_env_name(name: &str) -> bool {
    (1..=ENV_NAME_MAX).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with("TMT_")
}

/// Literals are `[a-z:]` and the template starts with a letter, so a target
/// is never a number, a pane ID of any driver, or tmux's `window.pane`.
fn parse_template(template: &str) -> Result<Vec<Token>, GrammarError> {
    let error = || {
        invalid(
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

/// Text a driver reports: bounded, and free of control characters, so it
/// can be stored and shown without escaping surprises.
pub(crate) fn plain(value: &str, max_bytes: usize) -> bool {
    value.len() <= max_bytes && !value.chars().any(char::is_control)
}

/// An absolute path without control characters.
pub(crate) fn socket(value: &str) -> bool {
    value.starts_with('/') && value.len() > 1 && plain(value, 1024)
}

/// A process ID that fits core's stored integers.
pub(crate) fn pid(value: u64) -> bool {
    value > 0 && value < (1 << 53)
}

/// A lowercase hyphenated UUID; core checks the version when it compares.
pub(crate) fn uuid_shaped(value: &str) -> bool {
    value.len() == 36
        && value.char_indices().all(|(index, c)| match index {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::wire::PaneIdSyntax;

    pub(crate) fn herdr_like() -> Capabilities {
        Capabilities {
            protocols: vec![1],
            kind: "host".into(),
            name: "herdr".into(),
            version: "0.1.0".into(),
            ops: vec!["snapshot".into()],
            pane_id: PaneIdSyntax {
                prefix: "term_".into(),
            },
            target: Some("w{n}:p{n}".into()),
            caller_env: vec!["HERDR_PANE_ID".into(), "HERDR_SOCKET_PATH".into()],
        }
    }

    fn with(edit: impl FnOnce(&mut Capabilities)) -> Result<Grammar, GrammarError> {
        let mut capabilities = herdr_like();
        edit(&mut capabilities);
        Grammar::from_capabilities(&capabilities)
    }

    #[test]
    fn pane_ids_and_targets_follow_the_declared_syntax() {
        let grammar = with(|_| {}).unwrap();
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
    }

    #[test]
    fn unsafe_declarations_are_refused() {
        for edit in [
            (|c: &mut Capabilities| c.name = "Herdr".into()) as fn(&mut Capabilities),
            |c| c.name = "9lives".into(),
            |c| c.name = "a".repeat(33),
            |c| c.pane_id.prefix = "term".into(),
            |c| c.pane_id.prefix = "%_".into(),
            |c| c.pane_id.prefix = "_x_".into(),
            |c| c.target = Some("{n}.{n}".into()),
            |c| c.target = Some("w{n}{n}".into()),
            |c| c.target = Some("pane".into()),
            |c| c.target = Some("w{n}.p{n}".into()),
            |c| c.caller_env = vec!["TMT_IDENTITY".into()],
            |c| c.caller_env = vec!["lower".into()],
            |c| c.caller_env = ["A", "B", "C", "D", "E"].map(String::from).to_vec(),
        ] {
            assert!(with(edit).is_err(), "{:?}", {
                let mut c = herdr_like();
                edit(&mut c);
                c
            });
        }
        assert!(with(|c| c.target = None).is_ok());
    }

    #[test]
    fn drivers_that_could_read_each_others_ids_conflict() {
        let herdr = with(|_| {}).unwrap();
        let other = |prefix: &str, target: Option<&str>| {
            with(|c| {
                c.name = "other".into();
                c.pane_id.prefix = prefix.into();
                c.target = target.map(String::from);
            })
            .unwrap()
        };
        assert_eq!(herdr.conflict(&other("pane-", Some("t{n}"))), None);
        assert!(herdr.conflict(&other("term_", None)).is_some());
        assert!(herdr.conflict(&other("pane-", Some("w{n}:p{n}"))).is_some());
        assert!(herdr.conflict(&herdr.clone()).is_some(), "same name");
    }

    #[test]
    fn text_rules() {
        assert!(plain("zsh", 8) && !plain("a\u{1b}[2J", 64) && !plain("toolong", 3));
        assert!(socket("/tmp/h.sock") && !socket("tmp/h.sock") && !socket("/"));
        assert!(pid(1) && !pid(0) && !pid(1 << 53));
        assert!(uuid_shaped("0f8fad5b-d9cb-469f-a165-70867728950e"));
        assert!(!uuid_shaped("0F8FAD5B-d9cb-469f-a165-70867728950e"));
    }
}
