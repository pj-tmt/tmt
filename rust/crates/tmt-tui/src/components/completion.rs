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
    matched_ranges: Vec<Vec<Range<usize>>>,
    range: Option<Range<usize>>,
    dismissed: Option<(usize, String)>,
}

impl Completion {
    pub fn open(&self) -> bool {
        self.range.is_some()
    }
    /// UTF-8 source ranges in the matched label, for presentation emphasis only.
    pub fn matched_ranges(&self, index: usize) -> &[Range<usize>] {
        self.matched_ranges.get(index).map_or(&[], Vec::as_slice)
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
            self.matched_ranges.clear();
            return;
        }
        let prior = self
            .matches
            .get(self.selected)
            .map(|candidate| candidate.id.clone());
        let query = normalized(query.unwrap()).chars;
        let mut matches: Vec<_> = candidates
            .iter()
            .filter_map(|candidate| {
                match_label(&candidate.label, &query)
                    .map(|(rank, ranges)| (rank, candidate.clone(), ranges))
            })
            .collect();
        // Stable sort preserves the application's inventory order within a rank.
        matches.sort_by_key(|(rank, _, _)| *rank);
        (self.matches, self.matched_ranges) = matches
            .into_iter()
            .map(|(_, candidate, ranges)| (candidate, ranges))
            .unzip();
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
        self.matched_ranges.clear();
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
        self.matched_ranges.clear();
        self.dismissed = None;
        true
    }
}

struct Normalized {
    chars: Vec<char>,
    source: Vec<Range<usize>>,
    initials: Vec<usize>,
}

fn normalized(text: &str) -> Normalized {
    let mut value = Normalized {
        chars: Vec::new(),
        source: Vec::new(),
        initials: Vec::new(),
    };
    let mut word_start = true;
    for (at, ch) in text.char_indices() {
        if matches!(ch, '-' | '_' | '.' | ' ') {
            word_start = true;
            continue;
        }
        if word_start {
            value.initials.push(value.chars.len());
            word_start = false;
        }
        for lower in ch.to_lowercase() {
            value.chars.push(lower);
            value.source.push(at..at + ch.len_utf8());
        }
    }
    value
}

/// Rank by match quality, never by application identity or routing information.
fn match_label(label: &str, query: &[char]) -> Option<(u8, Vec<Range<usize>>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let label = normalized(label);
    let (rank, positions): (_, Vec<_>) = if label.chars == query {
        (0, (0..query.len()).collect())
    } else if label.chars.starts_with(query) {
        (1, (0..query.len()).collect())
    } else if query.len() <= label.initials.len()
        && query
            .iter()
            .zip(&label.initials)
            .all(|(ch, at)| *ch == label.chars[*at])
    {
        (2, label.initials[..query.len()].to_vec())
    } else if let Some(start) = label
        .chars
        .windows(query.len())
        .position(|part| part == query)
    {
        (3, (start..start + query.len()).collect())
    } else {
        let mut at = 0;
        let mut positions = Vec::with_capacity(query.len());
        for ch in query {
            at += label.chars[at..]
                .iter()
                .position(|candidate| candidate == ch)?;
            positions.push(at);
            at += 1;
        }
        (4, positions)
    };
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for at in positions {
        let source = label.source[at].clone();
        if let Some(last) = ranges.last_mut().filter(|last| last.end >= source.start) {
            last.end = last.end.max(source.end);
        } else {
            ranges.push(source);
        }
    }
    Some((rank, ranges))
}

#[cfg(test)]
mod tests;
