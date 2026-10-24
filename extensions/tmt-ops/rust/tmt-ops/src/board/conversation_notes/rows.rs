//! What the list overlay shows of each item: its kind, its title and who added it when.
use crate::checklist::{
    Preview,
    model::{Completion, Id, Stamp},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Link,
    Todo,
    Done,
}

impl Kind {
    pub fn mark(self) -> &'static str {
        match self {
            Self::Link => "↗",
            Self::Todo => "○",
            Self::Done => "✓",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub id: Id,
    pub kind: Kind,
    pub title: String,
    pub reference: Option<String>,
    /// The host of a link; empty for a todo.
    pub host: String,
    /// `who · age` or `who · done age`; empty when nothing is known.
    pub byline: String,
}

impl Row {
    /// What the right-hand column shows: the host (cut to `host_cells`) when there is room for
    /// it, then the byline.
    pub fn meta(&self, host_cells: usize) -> String {
        let host = super::super::view::fit(&self.host, host_cells);
        [host.trim_end(), self.byline.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("  ")
    }
}

/// The items the overlay lists, archived ones left out, in the list's own order.
pub(super) fn rows(preview: &Preview, now_ms: u64) -> Vec<Row> {
    preview
        .current
        .items
        .iter()
        .filter(|view| !view.item.archived)
        .map(|view| {
            let item = &view.item;
            let kind = match (&item.content.reference, item.completion) {
                (Some(_), _) => Kind::Link,
                (None, Completion::Open) => Kind::Todo,
                (None, Completion::Complete) => Kind::Done,
            };
            let author = item
                .added
                .as_ref()
                .map(|stamp| format!("{} · {}", who(preview, stamp), age(now_ms, stamp)));
            let byline = match kind {
                Kind::Link | Kind::Todo => author.unwrap_or_default(),
                Kind::Done => {
                    let done = item.last_changed.as_ref().map_or_else(
                        || "done".to_owned(),
                        |stamp| format!("done {}", age(now_ms, stamp)),
                    );
                    match &item.added {
                        Some(stamp) => format!("{} · {done}", who(preview, stamp)),
                        None => done,
                    }
                }
            };
            let host = match kind {
                Kind::Link => host(item.content.reference.as_deref().unwrap_or_default()),
                _ => String::new(),
            };
            Row {
                id: item.id.clone(),
                kind,
                title: item.content.title.clone(),
                reference: item.content.reference.clone(),
                host,
                byline,
            }
        })
        .collect()
}

/// You, a member of the room by name, or the start of an identity the room no longer lists.
fn who(preview: &Preview, stamp: &Stamp) -> String {
    if stamp.by == preview.actor_id {
        return "you".into();
    }
    preview
        .members
        .iter()
        .find(|(id, _)| *id == stamp.by)
        .map_or_else(
            || stamp.by.as_str().chars().take(8).collect(),
            |(_, name)| name.clone(),
        )
}

fn age(now_ms: u64, stamp: &Stamp) -> String {
    crate::requests::age(now_ms, stamp.at_ms)
}

/// The host of a reference: what follows `://` up to the path, or the reference itself.
fn host(reference: &str) -> String {
    let rest = reference
        .split_once("://")
        .map_or(reference, |(_, rest)| rest);
    rest.split(['/', '?', '#'])
        .next()
        .unwrap_or(rest)
        .to_owned()
}

#[cfg(test)]
mod tests;
