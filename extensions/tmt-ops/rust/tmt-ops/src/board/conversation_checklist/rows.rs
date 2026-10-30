//! What the list overlay shows of each item: its status, its title, its link and who added it when.
use crate::checklist::{
    Preview,
    model::{Id, Stamp},
};

/// The first line's mark for an open item; ticked items leave the list.
pub(super) const OPEN_MARK: &str = "○";
/// The mark before a link on the second line, so a link never rests on its colour alone.
pub(super) const LINK_MARK: &str = "↗";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Link,
    Todo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub id: Id,
    pub kind: Kind,
    pub title: String,
    pub reference: Option<String>,
    /// The reference as the second line shows it, whole; empty for a todo.
    pub address: String,
    /// `who · age`; empty when nothing is known.
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
            let address = item
                .content
                .reference
                .as_deref()
                .map(address)
                .unwrap_or_default();
            Row {
                id: item.id.clone(),
                kind,
                title: item.content.title.clone(),
                reference: item.content.reference.clone(),
                address,
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

/// A reference as the list shows it, never shortened past what names its target: a GitHub
/// issue or pull request reads `owner/repo#N`; any other address reads whole, without its scheme.
fn address(reference: &str) -> String {
    let rest = reference
        .split_once("://")
        .map_or(reference, |(_, rest)| rest);
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let host = parts.next().unwrap_or(path);
    if host.eq_ignore_ascii_case("github.com")
        && let [owner, repo, "issues" | "pull", number] = parts.collect::<Vec<_>>().as_slice()
        && !number.is_empty()
        && number.bytes().all(|byte| byte.is_ascii_digit())
        && path.len() == rest.len()
    {
        return format!("{owner}/{repo}#{number}");
    }
    rest.to_owned()
}

#[cfg(test)]
mod tests;
