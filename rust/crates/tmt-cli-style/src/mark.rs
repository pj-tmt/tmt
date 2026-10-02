//! State marks. Each mark has exactly one meaning everywhere it appears.

use crate::palette::Token;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// `●` running or active.
    Running,
    /// `○` offline or ended.
    Offline,
    /// `◌` bound to a pane with no agent running.
    Idle,
    /// `↻` a remembered session can be resumed; it leads a row's trailing
    /// action (`↻ tmt resume <name>`), never the row itself.
    Resumable,
    /// `✓` done.
    Done,
    /// `✗` failed or blocked.
    Failed,
    /// `!` warning.
    Warning,
    /// `◆` waits on the reader's decision, such as a squad member that owes
    /// the reader an answer.
    Decision,
}

impl Mark {
    pub const ALL: [Self; 8] = [
        Self::Running,
        Self::Offline,
        Self::Idle,
        Self::Resumable,
        Self::Done,
        Self::Failed,
        Self::Warning,
        Self::Decision,
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
            Self::Decision => "◆",
        }
    }

    /// The canonical meaning registered in the design tokens.
    pub fn description(self) -> &'static str {
        match self {
            Self::Running => "running or active",
            Self::Offline => "offline or ended",
            Self::Idle => "bound to a pane, no agent running",
            Self::Resumable => "leads a resume action, never a row's state",
            Self::Done => "done",
            Self::Failed => "failed or blocked",
            Self::Warning => "warning",
            Self::Decision => "waits on your decision",
        }
    }

    pub fn token(self) -> Token {
        match self {
            Self::Running | Self::Resumable | Self::Decision => Token::Accent,
            Self::Done => Token::Ok,
            Self::Offline | Self::Idle => Token::Dim,
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
        assert_eq!(symbols, ["●", "○", "◌", "↻", "✓", "✗", "!", "◆"]);
    }
}
