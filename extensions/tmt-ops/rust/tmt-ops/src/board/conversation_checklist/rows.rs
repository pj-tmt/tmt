//! What the list overlay shows of each item: its status, its link, its title and who added it when.
use crate::checklist::{
    Preview,
    model::{Id, Stamp},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Link,
    Todo,
}

impl Kind {
    /// The STATUS cell: the mark and its word, so a state never rests on a color alone.
    pub fn status(self) -> &'static str {
        match self {
            Self::Link => "↗ link",
            Self::Todo => "○ todo",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub id: Id,
    pub kind: Kind,
    pub title: String,
    pub reference: Option<String>,
    /// The LINK cell: the reference in short form, `—` for a todo.
    pub link: String,
    /// `who · age`, shown on the cursor row only; empty when nothing is known.
    pub byline: String,
}

/// Open, nonarchived items in the list's own order.
pub(super) fn rows(preview: &Preview, now_ms: u64) -> Vec<Row> {
    preview
        .current
        .items
        .iter()
        .filter(|view| super::open_item(view))
        .map(|view| {
            let item = &view.item;
            let kind = if item.content.reference.is_some() {
                Kind::Link
            } else {
                Kind::Todo
            };
            let byline = item
                .added
                .as_ref()
                .map(|stamp| format!("{} · {}", who(preview, stamp), age(now_ms, stamp)))
                .unwrap_or_default();
            let link = match kind {
                Kind::Link => link(item.content.reference.as_deref().unwrap_or_default()),
                Kind::Todo => "—".into(),
            };
            Row {
                id: item.id.clone(),
                kind,
                title: item.content.title.clone(),
                reference: item.content.reference.clone(),
                link,
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

/// A reference in short form. A GitHub issue or pull request reads `owner/repo#N`; any other
/// address reads `host/…/last-segment` (`host/segment` when it has one, `host` when none).
fn link(reference: &str) -> String {
    let rest = reference
        .split_once("://")
        .map_or(reference, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut parts = rest.split('/').filter(|part| !part.is_empty());
    let host = parts.next().unwrap_or(rest);
    let path: Vec<&str> = parts.collect();
    if host.eq_ignore_ascii_case("github.com")
        && let [owner, repo, "issues" | "pull", number, ..] = path.as_slice()
        && !number.is_empty()
        && number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return format!("{owner}/{repo}#{number}");
    }
    match path.as_slice() {
        [] => host.to_owned(),
        [only] => format!("{host}/{only}"),
        [.., last] => format!("{host}/…/{last}"),
    }
}

#[cfg(test)]
mod tests;
