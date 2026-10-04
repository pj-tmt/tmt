//! What `tmt colab serve` reports once the local space is mounted: the state, and the next step,
//! for a person (detail rows) and for an agent (one stable JSON line).
use crate::{
    door::{Door, Lookup, Pairing},
    supervisor::Access,
};
use serde_json::{Value, json};
use std::path::Path;

const PAIR: &str = "tmt remote pair";
const CREATE: &str = "tmt colab page create --title <title>";

pub struct Status<'a> {
    pub space: &'a str,
    pub socket: &'a Path,
    pub access: &'a Access,
    /// `None` when no door is reachable: there is nothing to pair with.
    pub pairing: Option<Pairing>,
    /// Ids of the pages that are not archived; `None` when the catalog could not be read.
    pub pages: Option<Vec<String>>,
}

impl Status<'_> {
    fn door(&self) -> Option<&Door> {
        match self.access {
            Access::Attached(door) | Access::Started { door, .. } => Some(door),
            Access::Unavailable { .. } => None,
        }
    }
    fn state(&self) -> &'static str {
        match self.access {
            Access::Attached(_) => "attached",
            Access::Started { .. } => "started",
            Access::Unavailable { .. } => "unavailable",
        }
    }
    /// The first page's path under the Remote door address, if the space has a page.
    fn first_page(&self) -> Option<String> {
        let id = self.pages.as_ref()?.first()?;
        Some(format!(
            "x/colab/#space={}&path=%2Fpages%2F{id}",
            self.space
        ))
    }
    /// A full link when a door runs, else the relative path with how to get a full one.
    fn page_link(&self) -> Option<String> {
        let relative = self.first_page()?;
        Some(match self.door() {
            Some(door) => door.url(&relative),
            None => Door::hint(&Lookup::Unknown, &relative),
        })
    }
    fn needs_pairing(&self) -> bool {
        self.door().is_some() && !matches!(self.pairing, Some(Pairing::Paired(_)))
    }
    fn needs_page(&self) -> bool {
        self.pages.as_ref().is_some_and(Vec::is_empty)
    }
    /// The one JSON line an agent reads. Keys are stable; absent facts are `null`.
    pub fn json(&self) -> Value {
        let (paired, devices) = match &self.pairing {
            Some(Pairing::Paired(n)) => (json!(true), json!(n)),
            Some(Pairing::Unpaired) => (json!(false), json!(0)),
            _ => (Value::Null, Value::Null),
        };
        let mut next = Vec::new();
        if self.needs_pairing() {
            next.push(PAIR);
        }
        if self.needs_page() {
            next.push(CREATE);
        }
        json!({
            "spaceId": self.space,
            "socket": self.socket,
            "profile": "colab-sync-v1",
            "state": "mounted",
            "door": self.state(),
            "origin": self.door().map(Door::origin),
            "url": self.door().map(|door| door.url("x/colab/")),
            "paired": paired,
            "devices": devices,
            "pages": self.pages.as_ref().map(Vec::len),
            "page": self.page_link(),
            "next": next,
            "warning": self.access.warning().map(|(what, _)| what),
        })
    }
    /// Detail rows in reading order: facts first, then the next step.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("space", format!("{} (ready)", self.space)),
            ("socket", self.socket.display().to_string()),
            (
                "door",
                match self.door() {
                    Some(door) => format!("{} · {}", self.state(), door.origin()),
                    None => "unavailable (local only)".into(),
                },
            ),
        ];
        if self.door().is_some() {
            rows.push((
                "paired",
                match &self.pairing {
                    Some(Pairing::Paired(1)) => "yes (1 device)".into(),
                    Some(Pairing::Paired(n)) => format!("yes ({n} devices)"),
                    Some(Pairing::Unpaired) => "no".into(),
                    _ => "unknown".into(),
                },
            ));
        }
        rows.push((
            "open",
            match (self.page_link(), self.pages.as_ref()) {
                (Some(link), _) => link,
                (None, Some(_)) => format!("create one: {CREATE}"),
                (None, None) => "pages unknown: tmt colab ls".into(),
            },
        ));
        if self.needs_pairing() {
            rows.push((
                "pair",
                match self.pairing {
                    Some(Pairing::Unpaired) => format!("Pair this browser once: {PAIR}"),
                    _ => format!("If this browser is new: {PAIR}"),
                },
            ));
        }
        rows
    }
}
