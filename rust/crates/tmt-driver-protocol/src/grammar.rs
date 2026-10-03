//! A driver's declared syntax and the text rules every string from a driver
//! meets. The host's name, pane-ID and target syntax are `tmt-host-grammar`'s,
//! the one definition core also matches stored IDs with; this adds what only
//! the protocol has, the environment `caller` may read.

use crate::wire::Capabilities;
pub use tmt_host_grammar::{GrammarError, HostGrammar};

const ENV_MAX: usize = 4;
const ENV_NAME_MAX: usize = 64;

/// A driver's validated declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grammar {
    host: HostGrammar,
    caller_env: Vec<String>,
}

impl Grammar {
    pub fn from_capabilities(capabilities: &Capabilities) -> Result<Self, GrammarError> {
        let host = HostGrammar::new(
            &capabilities.name,
            &capabilities.pane_id.prefix,
            capabilities.target.as_deref(),
        )?;
        if capabilities.caller_env.len() > ENV_MAX {
            return Err(GrammarError::new("callerEnv names at most 4 variables"));
        }
        for variable in &capabilities.caller_env {
            if !valid_env_name(variable) {
                return Err(GrammarError::new(format!(
                    "callerEnv entry {variable:?} must be [A-Z][A-Z0-9_]* and not TMT_*"
                )));
            }
        }
        Ok(Self {
            host,
            caller_env: capabilities.caller_env.clone(),
        })
    }

    /// The host syntax, for core's registry of hosts.
    pub fn host(&self) -> &HostGrammar {
        &self.host
    }

    pub fn name(&self) -> &str {
        self.host.name()
    }

    pub fn caller_env(&self) -> &[String] {
        &self.caller_env
    }

    pub fn pane_id_prefix(&self) -> &str {
        self.host.pane_id_prefix()
    }

    pub fn is_pane_id(&self, value: &str) -> bool {
        self.host.is_pane_id(value)
    }

    pub fn is_target(&self, value: &str) -> bool {
        self.host.is_target(value)
    }

    pub fn sample_target(&self) -> Option<String> {
        self.host.sample_target()
    }

    pub fn target_numbered(&self, digits: &str) -> Option<String> {
        self.host.target_numbered(digits)
    }

    /// Why two drivers can't be installed together, if they can't.
    pub fn conflict(&self, other: &Self) -> Option<String> {
        self.host.conflict(&other.host)
    }
}

pub(crate) fn valid_env_name(name: &str) -> bool {
    (1..=ENV_NAME_MAX).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with("TMT_")
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
    fn the_declared_syntax_is_the_host_grammar() {
        let grammar = with(|_| {}).unwrap();
        assert_eq!(grammar.name(), "herdr");
        assert!(grammar.is_pane_id("term_1") && !grammar.is_pane_id("%1"));
        assert!(grammar.is_target("w1:p2"));
        assert_eq!(
            grammar.host(),
            &HostGrammar::new("herdr", "term_", Some("w{n}:p{n}")).unwrap()
        );
        let mut other = herdr_like();
        other.name = "other".into();
        let other = Grammar::from_capabilities(&other).unwrap();
        assert!(grammar.conflict(&other).is_some(), "same prefix");
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
