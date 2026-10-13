//! Source-range completion without routing semantics. The application supplies
//! stable candidates and decides which fields admit references.
use std::ops::Range;

use super::Editor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub insertion: String,
}

#[derive(Clone, Debug, Default)]
pub struct Completion {
    pub matches: Vec<Candidate>,
    pub selected: usize,
    range: Option<Range<usize>>,
    dismissed: Option<(usize, String)>,
}

impl Completion {
    pub fn open(&self) -> bool {
        self.range.is_some()
    }
    /// Only the token immediately before the caret participates. Addresses in
    /// ordinary words and references inside folded source do not open a picker.
    pub fn update(&mut self, editor: &Editor, candidates: &[Candidate]) {
        let cursor = editor.cursor();
        let prefix = &editor.as_str()[..cursor];
        let start = prefix
            .rfind(char::is_whitespace)
            .map_or(0, |at| at + prefix[at..].chars().next().unwrap().len_utf8());
        let token = &prefix[start..];
        let query = token.strip_prefix('@').filter(|query| !query.contains('@'));
        let folded = editor
            .folds()
            .iter()
            .any(|fold| !fold.expanded && fold.source.start <= start && cursor <= fold.source.end);
        if query.is_none()
            || folded
            || self
                .dismissed
                .as_ref()
                .is_some_and(|(at, value)| *at == cursor && value == token)
        {
            self.range = None;
            self.matches.clear();
            return;
        }
        let prior = self
            .matches
            .get(self.selected)
            .map(|candidate| candidate.id.clone());
        let query = query.unwrap().to_lowercase();
        self.matches = candidates
            .iter()
            .filter(|candidate| candidate.label.to_lowercase().contains(&query))
            .cloned()
            .collect();
        self.selected = prior
            .and_then(|id| self.matches.iter().position(|candidate| candidate.id == id))
            .unwrap_or(0);
        self.range = Some(start..cursor);
        self.dismissed = None;
    }
    pub fn move_by(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.matches.len().saturating_sub(1));
    }
    pub fn dismiss(&mut self, editor: &Editor) {
        if let Some(range) = self.range.take() {
            self.dismissed = Some((editor.cursor(), editor.as_str()[range].to_owned()));
        }
        self.matches.clear();
    }
    pub fn accept(&mut self, editor: &mut Editor, max_bytes: usize) -> bool {
        let Some(range) = self.range.clone() else {
            return false;
        };
        let Some(candidate) = self.matches.get(self.selected) else {
            return false;
        };
        if !editor.replace_range(range, &format!("{} ", candidate.insertion), max_bytes) {
            return false;
        }
        self.range = None;
        self.matches.clear();
        self.dismissed = None;
        true
    }
}

#[cfg(test)]
mod tests;
