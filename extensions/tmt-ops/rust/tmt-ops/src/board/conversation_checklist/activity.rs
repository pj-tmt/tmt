//! Public checklist events held by the selected-list owner. No request or unread authority.
use super::Preview;
use crate::{
    checklist::{
        Service,
        events::{Gap, Logged, Page},
    },
    core::Core,
};
use std::{collections::BTreeMap, sync::Arc};

const PAGE_SIZE: u64 = 500;
const PAGES_PER_READ: usize = 2;
const HELD_EVENTS: usize = 1000;

/// Independent results: a failed preview must not erase acquired events, or vice versa.
#[derive(Debug)]
pub(super) struct Read {
    pub observed: bool,
    pub user: Option<String>,
    pub pages: Result<Vec<Page>, String>,
}

impl Default for Read {
    fn default() -> Self {
        Self {
            observed: false,
            user: None,
            pages: Ok(Vec::new()),
        }
    }
}

pub(super) fn read(core: &Core, service: Option<&Service>, after: u64) -> Read {
    let user = crate::me::current(core).ok().flatten().map(|user| user.id);
    Read {
        observed: true,
        user,
        pages: service
            .ok_or_else(|| "Checklist activity unavailable".to_owned())
            .and_then(|service| {
                pages(after, |cursor| {
                    service
                        .events(cursor, PAGE_SIZE)
                        .map_err(|error| error.message)
                })
            }),
    }
}

fn pages(
    mut after: u64,
    mut read: impl FnMut(u64) -> Result<Page, String>,
) -> Result<Vec<Page>, String> {
    let mut pages = Vec::new();
    for _ in 0..PAGES_PER_READ {
        let page = read(after)?;
        if page.next_cursor < after || page.more && page.next_cursor == after {
            return Err("Checklist activity cursor did not advance".into());
        }
        after = page.next_cursor;
        let more = page.more;
        pages.push(page);
        if !more {
            break;
        }
    }
    Ok(pages)
}

#[derive(Default)]
pub(super) struct Held {
    events: Vec<Logged>,
    cursor: u64,
    gap: Option<Gap>,
    more: bool,
    failed: bool,
    observed: bool,
    user: Option<String>,
    pub snapshot: Option<Arc<Snapshot>>,
}

/// Acquired facts and current labels, refreshed only on lane completion, never per frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::board) struct Snapshot {
    pub events: Vec<Logged>,
    pub user: Option<String>,
    pub titles: BTreeMap<String, String>,
    pub names: BTreeMap<String, String>,
    pub gap: Option<Gap>,
    pub more: bool,
    pub failed: bool,
}

impl Held {
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    pub fn adopt(&mut self, read: Read) {
        if !read.observed {
            return;
        }
        self.observed = true;
        self.user = read.user;
        match read.pages {
            Ok(pages) => {
                self.failed = false;
                for page in pages {
                    if let Some(gap) = page.gap {
                        self.gap = Some(gap);
                    }
                    self.more = page.more;
                    self.cursor = page.next_cursor;
                    self.events.extend(page.events);
                }
                self.events.sort_by_key(|event| event.id);
                self.events.dedup_by_key(|event| event.id);
                if self.events.len() > HELD_EVENTS {
                    self.events.drain(..self.events.len() - HELD_EVENTS);
                    self.gap = Some(Gap::Pruned);
                }
            }
            Err(_) => self.failed = true,
        }
    }

    pub fn present(&mut self, preview: Option<&Preview>) {
        if !self.observed {
            return;
        }
        let snapshot = Snapshot {
            events: self.events.clone(),
            user: self.user.clone(),
            gap: self.gap,
            more: self.more,
            failed: self.failed,
            titles: preview
                .into_iter()
                .flat_map(|preview| &preview.current.items)
                .map(|view| {
                    (
                        view.item.id.as_str().into(),
                        view.item.content.title.clone(),
                    )
                })
                .collect(),
            names: preview
                .into_iter()
                .flat_map(|preview| &preview.members)
                .map(|(id, name)| (id.as_str().into(), name.clone()))
                .collect(),
        };
        if self.snapshot.as_deref() != Some(&snapshot) {
            self.snapshot = Some(Arc::new(snapshot));
        }
    }
}

#[cfg(test)]
mod tests;
