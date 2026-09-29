//! State marks. Each mark has exactly one meaning everywhere it appears.

use crate::palette::Token;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// `●` running or active.
    Running,
    /// `○` offline.
    Offline,
    /// `◌` bound to a pane with no agent running.
    Idle,
    /// `↻` a remembered session can be resumed.
    Resumable,
    /// `✓` done.
    Done,
    /// `✗` failed.
    Failed,
    /// `!` needs attention.
    Warning,
}

impl Mark {
    pub const ALL: [Self; 7] = [
        Self::Running,
        Self::Offline,
        Self::Idle,
        Self::Resumable,
        Self::Done,
        Self::Failed,
        Self::Warning,
    ];

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::Offline => "○",
            Self::Idle => "◌",
            Self::Resumable => "↻",
            Self::Done => "✓",
            Self::Failed => "✗",
            Self::Warning => "!",
        }
    }

    pub fn token(self) -> Token {
        match self {
            Self::Running | Self::Done => Token::Ok,
            Self::Offline | Self::Idle => Token::Dim,
            Self::Resumable => Token::Accent,
            Self::Failed => Token::Error,
            Self::Warning => Token::Warn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_mark_has_its_own_symbol() {
        let symbols = Mark::ALL.map(Mark::symbol);
        assert_eq!(symbols.iter().collect::<BTreeSet<_>>().len(), symbols.len());
        assert_eq!(symbols, ["●", "○", "◌", "↻", "✓", "✗", "!"]);
    }
}
