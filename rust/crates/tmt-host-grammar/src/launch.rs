//! A host's default launch template: the argv that opens a new pane in a session and runs one
//! command there (`contracts/driver-protocol-v1.md`). A placeholder is a whole word, so the
//! value that fills it is always exactly one argument and never part of a shell script. Any
//! other word is literal text, braces included (a tmux format such as `#{pane_id}`).

use crate::GrammarError;

const WORDS_MIN: usize = 2;
const WORDS_MAX: usize = 16;
const WORD_MAX: usize = 1024;
const TOTAL_MAX: usize = 4096;

/// The word the session name fills.
pub const SESSION: &str = "{session}";
/// The word the command fills: one argument holding the command line to run.
pub const COMMAND: &str = "{command}";

/// A validated launch template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTemplate {
    argv: Vec<String>,
}

impl LaunchTemplate {
    /// 2–16 words of plain text, each at most 1024 bytes and 4096 in all; the first is a
    /// literal program; `{session}` and `{command}` appear only as whole words, the first at
    /// least once and the second exactly once. A word that only contains one of them, or is
    /// any other `{name}`, is refused, so nothing is silently left unexpanded.
    pub fn new(argv: &[String]) -> Result<Self, GrammarError> {
        if !(WORDS_MIN..=WORDS_MAX).contains(&argv.len()) {
            return Err(GrammarError::new("launch has 2 to 16 words"));
        }
        let total: usize = argv.iter().map(String::len).sum();
        if total > TOTAL_MAX {
            return Err(GrammarError::new("launch is at most 4096 bytes"));
        }
        let (mut sessions, mut commands) = (0, 0);
        for (index, word) in argv.iter().enumerate() {
            if word.is_empty() || word.len() > WORD_MAX || word.chars().any(char::is_control) {
                return Err(GrammarError::new(format!(
                    "launch word {} must be 1 to 1024 bytes without control characters",
                    index + 1
                )));
            }
            match word.as_str() {
                SESSION => sessions += 1,
                COMMAND => commands += 1,
                _ if word.contains(SESSION)
                    || word.contains(COMMAND)
                    || whole_placeholder(word) =>
                {
                    return Err(GrammarError::new(format!(
                        "launch word {} may use {SESSION} and {COMMAND} only as whole words",
                        index + 1
                    )));
                }
                _ => (),
            }
            if index == 0 && (word == SESSION || word == COMMAND) {
                return Err(GrammarError::new("launch starts with a literal program"));
            }
        }
        if sessions == 0 || commands != 1 {
            return Err(GrammarError::new(format!(
                "launch has {SESSION} at least once and {COMMAND} exactly once"
            )));
        }
        Ok(Self {
            argv: argv.to_vec(),
        })
    }

    /// The placeholder names, as the public read lists them.
    pub const PLACEHOLDERS: [&'static str; 2] = ["session", "command"];

    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// Whether `word` is one of the two words a caller fills.
    pub fn is_placeholder(word: &str) -> bool {
        word == SESSION || word == COMMAND
    }
}

/// `{name}` as a whole word: placeholder syntax that is not one of ours.
fn whole_placeholder(word: &str) -> bool {
    word.len() > 2
        && word.starts_with('{')
        && word.ends_with('}')
        && !word[1..word.len() - 1].contains(['{', '}'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn a_template_with_whole_word_placeholders_is_accepted() {
        let template = LaunchTemplate::new(&words(&[
            "herdr",
            "tab",
            "create",
            "--label",
            "{session}",
            "--",
            "{command}",
        ]))
        .unwrap();
        assert_eq!(template.argv()[4], "{session}");
        // The session may fill several words; the command exactly one.
        assert!(
            LaunchTemplate::new(&words(&["sh", "{session}", "{session}", "{command}"])).is_ok()
        );
        // Other words are literal, braces included: a tmux format is not a placeholder.
        let script = "tmux new-window -P -F '#{pane_id}' \"$2\"";
        assert!(
            LaunchTemplate::new(&words(&["sh", "-c", script, "x", "{session}", "{command}"]))
                .is_ok()
        );
    }

    #[test]
    fn an_unsafe_template_is_refused() {
        let long = "x".repeat(1025);
        for (label, argv) in [
            ("one word", words(&["tmux"])),
            ("too many words", vec!["x".to_owned(); 17]),
            ("no command", words(&["tmux", "{session}"])),
            ("no session", words(&["tmux", "{command}"])),
            (
                "two commands",
                words(&["tmux", "{session}", "{command}", "{command}"]),
            ),
            (
                "embedded placeholder",
                words(&["tmux", "-t", "{session}:", "{command}"]),
            ),
            (
                "embedded in a script",
                words(&["sh", "-c", "run {command}", "{session}"]),
            ),
            (
                "unknown placeholder",
                words(&["tmux", "{session}", "{name}", "{command}"]),
            ),
            ("placeholder program", words(&["{command}", "{session}"])),
            ("empty word", words(&["tmux", "", "{session}", "{command}"])),
            (
                "control character",
                words(&["tmux", "a\nb", "{session}", "{command}"]),
            ),
            (
                "long word",
                words(&["tmux", &long, "{session}", "{command}"]),
            ),
        ] {
            assert!(LaunchTemplate::new(&argv).is_err(), "{label}");
        }
        let mut argv = words(&["tmux", "{session}", "{command}"]);
        argv.extend(vec!["x".repeat(1000); 5]);
        assert!(LaunchTemplate::new(&argv).is_err(), "total size");
    }
}
