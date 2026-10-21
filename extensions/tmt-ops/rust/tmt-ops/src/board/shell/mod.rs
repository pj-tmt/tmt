//! Session-only shell selection. The host supplies the current input owner;
//! terminal input, effects, drafts and conversation state remain with App.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OpsTab {
    #[default]
    Agents,
    Digest,
}

impl OpsTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Agents => "Agents",
            Self::Digest => "Digest",
        }
    }
}

/// The right column of the Agents tab: the selected agent's conversation, or the measures of the
/// selected squad (all agents when the Agents chip is active).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RightTab {
    #[default]
    Talk,
    Measure,
}

impl RightTab {
    pub fn next(self) -> Self {
        match self {
            Self::Talk => Self::Measure,
            Self::Measure => Self::Talk,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpsInputOwner {
    List,
    Composer,
    Search,
    Completion,
    Reader,
    Popup,
}

#[derive(Debug, Default)]
pub struct OpsShell {
    pub tab: OpsTab,
}

impl OpsShell {
    /// Returns true only when the shell consumed the digit. Active editors,
    /// search and completion receive their input unchanged; the host dismisses
    /// non-typing overlays before applying the destination.
    pub fn select_digit(&mut self, digit: char, owner: OpsInputOwner) -> bool {
        if matches!(
            owner,
            OpsInputOwner::Composer | OpsInputOwner::Search | OpsInputOwner::Completion
        ) {
            return false;
        }
        let tab = match digit {
            '1' => OpsTab::Agents,
            '2' => OpsTab::Digest,
            _ => return false,
        };
        self.tab = tab;
        true
    }
}

#[cfg(test)]
mod tests;
