//! Session-only shell selection. The host supplies the current input owner;
//! terminal input, effects, drafts and conversation state remain with App.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OpsTab {
    #[default]
    Overview,
    Agents,
    Digest,
}

impl OpsTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "1 Overview",
            Self::Agents => "2 Agents",
            Self::Digest => "3 Digest",
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
    /// Returns true only when the shell consumed the digit. Editors, search,
    /// readers and modal surfaces receive their input unchanged.
    pub fn select_digit(&mut self, digit: char, owner: OpsInputOwner) -> bool {
        if owner != OpsInputOwner::List {
            return false;
        }
        let tab = match digit {
            '1' => OpsTab::Overview,
            '2' => OpsTab::Agents,
            '3' => OpsTab::Digest,
            _ => return false,
        };
        self.tab = tab;
        true
    }
}

#[cfg(test)]
mod tests;
