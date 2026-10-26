//! Counts and their known/unknown presentation, shared by the checklist, rows and the header.
use crate::checklist::{ItemView, model::Completion};

/// Open items and their overlapping reference attribute.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::board) struct Counts {
    pub links: usize,
    pub todo: usize,
}

impl Counts {
    /// References are an attribute of open items, never an additional todo.
    pub fn of<'a>(items: impl IntoIterator<Item = &'a ItemView>) -> Self {
        let mut counts = Self::default();
        for view in items.into_iter().filter(|view| open_item(view)) {
            counts.links += usize::from(view.item.content.reference.is_some());
            counts.todo += 1;
        }
        counts
    }

    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// Display admission only: retain the full source for references and revision-checked writes.
pub(in crate::board) fn open_item(view: &ItemView) -> bool {
    !view.item.archived && view.item.completion == Completion::Open
}

/// What the board knows of one room or conversation list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) enum Summary {
    /// Asked, nothing back yet.
    Pending,
    Counts(Counts),
    /// The admitted read's words for why the list could not be read.
    Unavailable(String),
}

/// One cached projection of the admitted summaries, published with their change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::board) struct Total {
    pub todo: usize,
    known: usize,
    pending: usize,
    unavailable: usize,
}

impl Total {
    pub fn incomplete(mut self) -> Self {
        self.pending = self.pending.saturating_add(1);
        self
    }

    pub(super) fn observe(&mut self, summary: Option<&Summary>) {
        match summary {
            Some(Summary::Counts(counts)) => {
                self.todo = self.todo.saturating_add(counts.todo);
                self.known += 1;
            }
            Some(Summary::Unavailable(_)) => self.unavailable += 1,
            None | Some(Summary::Pending) => self.pending += 1,
        }
    }

    pub fn label(self) -> String {
        if self.known == 0 && self.unavailable > 0 {
            "— todo".into()
        } else if self.known == 0 && self.pending > 0 {
            "… todo".into()
        } else if self.pending + self.unavailable > 0 {
            format!("{} todo · partial", self.todo)
        } else {
            format!("{} todo", self.todo)
        }
    }
}
