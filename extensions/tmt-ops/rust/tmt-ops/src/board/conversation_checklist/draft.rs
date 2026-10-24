//! The unsent form of one conversation's checklist: a new note, or an edit of one item. It belongs to
//! the conversation, not to the overlay, so closing the overlay keeps it.
use crate::checklist::{Preview, model::Id};
use tmt_tui::components::Editor;

/// Same bound as the board's other one-line fields.
pub(super) const MAX_BYTES: usize = crate::board::search_field::MAX_BYTES;

#[derive(Debug, Clone)]
pub(in crate::board) enum Aim {
    Add,
    /// The item and the revision the edit was started at; a conflict re-aims it.
    Edit {
        item: Id,
        revision: u64,
    },
}

#[derive(Debug, Clone)]
pub(in crate::board) struct Draft {
    pub seed: Option<Seed>,
    pub aim: Aim,
    /// The note's words, then its link.
    pub fields: [Editor; 2],
    pub focus: usize,
    original: [String; 2],
    sent: bool,
}

/// What a form means once sent: the title and the reference of the item.
pub(in crate::board) struct Content {
    pub title: String,
    pub reference: Option<String>,
}

fn editor(text: &str) -> Editor {
    let mut editor = Editor::default();
    editor.insert(text, MAX_BYTES);
    editor
}

fn is_url(text: &str) -> bool {
    (text.starts_with("https://") || text.starts_with("http://"))
        && !text.chars().any(char::is_whitespace)
}

impl Draft {
    pub fn add() -> Self {
        Self::new(Aim::Add, "", "")
    }

    pub fn edit(item: Id, revision: u64, title: &str, reference: Option<&str>) -> Self {
        Self::new(Aim::Edit { item, revision }, title, reference.unwrap_or(""))
    }

    fn new(aim: Aim, title: &str, reference: &str) -> Self {
        Self {
            aim,
            seed: None,
            fields: [editor(title), editor(reference)],
            focus: 0,
            original: [title.to_owned(), reference.to_owned()],
            sent: false,
        }
    }

    pub fn text(&self, field: usize) -> &str {
        self.fields[field].as_str()
    }

    /// Nothing worth keeping: an empty new note, or an edit that changed nothing.
    pub fn untouched(&self) -> bool {
        match self.aim {
            Aim::Add => self.fields.iter().all(|field| field.as_str().is_empty()),
            Aim::Edit { .. } => (0..2).all(|field| self.text(field) == self.original[field]),
        }
    }

    pub fn sent(&self) -> bool {
        self.sent
    }

    pub fn send(&mut self) {
        self.sent = true;
    }

    /// Core refused the write: the words stay, aimed at the item as it now stands.
    pub fn rejected(&mut self, preview: Option<&Preview>) {
        self.sent = false;
        if let Aim::Edit { item, revision } = &mut self.aim
            && let Some(view) = preview
                .and_then(|preview| preview.current.items.iter().find(|v| &v.item.id == item))
        {
            *revision = view.item.revision;
        }
    }

    /// A link typed in the second field makes a link titled by the note (by the link when the
    /// note is empty); a lone web address makes a link too; any other words make a todo.
    pub fn content(&self) -> Result<Content, &'static str> {
        let (note, link) = (self.text(0).trim(), self.text(1).trim());
        if note.is_empty() && link.is_empty() {
            return Err("Type a note or a link first.");
        }
        if let Some(seed) = &self.seed {
            return match seed.kind {
                NoteKind::Todo if note.is_empty() => Err("Type a title first."),
                NoteKind::Todo => Ok(Content {
                    title: note.into(),
                    reference: None,
                }),
                NoteKind::Link if link.is_empty() => Err("Type a link first."),
                NoteKind::Link => Ok(Content {
                    title: if note.is_empty() { link } else { note }.into(),
                    reference: Some(link.into()),
                }),
            };
        }
        Ok(match (note.is_empty(), link.is_empty()) {
            (_, false) => Content {
                title: if note.is_empty() { link } else { note }.to_owned(),
                reference: Some(link.to_owned()),
            },
            (false, true) if is_url(note) => Content {
                title: note.to_owned(),
                reference: Some(note.to_owned()),
            },
            _ => Content {
                title: note.to_owned(),
                reference: None,
            },
        })
    }
}

/// An explicit kind for the Add to checklist entry points; ordinary checklist forms retain inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::board) enum NoteKind {
    Todo,
    Link,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) struct Seed {
    pub key: super::Key,
    pub actor: Option<String>,
    pub kind: NoteKind,
    pub title: String,
    pub reference: String,
    pub source: crate::board::conversation::note_source::Source,
}

impl Draft {
    pub fn seeded(seed: Seed) -> Self {
        let mut draft = Self::new(Aim::Add, &seed.title, &seed.reference);
        draft.seed = Some(seed);
        draft.focus = 1; // Title follows the kind selector.
        draft
    }

    pub fn field(&self) -> Option<usize> {
        if self.seed.is_some() {
            self.focus.checked_sub(1)
        } else {
            Some(self.focus)
        }
    }
}
