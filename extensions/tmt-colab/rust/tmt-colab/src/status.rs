//! What `tmt colab serve` reports once the local space is mounted: the state, and the next step,
//! for a person (detail rows) and for an agent (one stable JSON line).
use crate::{
    door::{Door, Pairing},
    reach::Reach,
    supervisor::{Access, Reason},
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
    /// Full catalog, including archived and deleted IDs, for collision-safe display aliases.
    pub all_pages: Option<Vec<String>>,
    /// The browser was opened on the link `open_link` named.
    pub opened: bool,
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
    /// A full link when a door runs, else the relative path.
    fn page_link(&self) -> Option<String> {
        let relative = self.first_page()?;
        Some(match self.door() {
            Some(door) => door.url(&relative),
            None => relative,
        })
    }
    fn short_page(&self) -> Option<String> {
        let page = self.pages.as_ref()?.first()?;
        let id =
            tmt_colab::short_links::shortest_id(page, self.all_pages.as_deref().unwrap_or(&[]));
        Some(match self.door() {
            Some(door) => format!("{}/p/{id}", door.origin()),
            None => format!("x/colab/p/{id}"),
        })
    }
    /// What a person reads beside a page link: the link, or, without a door, the relative path
    /// and why there is no full link. Never a command that `serve` itself replaces.
    fn page_text(&self) -> Option<String> {
        let link = self.short_page()?;
        Some(match self.access {
            Access::Unavailable {
                reason: Reason::Missing,
            } => format!("{link} ({})", Door::INSTALL_HINT),
            Access::Unavailable {
                reason: Reason::Remote(failure),
            } => {
                // The warning below carries the whole instruction; the row only points at it.
                if failure.code == "REMOTE_SERVE_OUTDATED" {
                    format!("{link} (Remote serve is outdated; see warning)")
                } else {
                    format!("{link} (browser access unavailable: see warning)")
                }
            }
            Access::Unavailable {
                reason: Reason::WouldNotStart,
            } => format!("{link} (browser access unavailable: see warning)"),
            _ => link,
        })
    }
    /// Where to send the browser: the page when the space has exactly one, else the space home.
    fn target_path(&self) -> String {
        Reach::landing(self.space, self.pages.as_deref())
    }
    /// The full link to open in a browser, only while a door runs.
    pub fn open_link(&self) -> Option<String> {
        self.door().map(|door| match self.pages.as_deref() {
            Some([only]) => format!(
                "{}/p/{}",
                door.origin(),
                tmt_colab::short_links::shortest_id(only, self.all_pages.as_deref().unwrap_or(&[]))
            ),
            _ => door.url(&self.target_path()),
        })
    }
    /// What the `open` row says: the link, or without a door the relative page path and why.
    fn target_text(&self) -> Option<String> {
        match (self.open_link(), self.pages.as_deref()) {
            (Some(link), _) => Some(link),
            (None, Some([_, ..])) => self.page_text(),
            _ => None,
        }
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
            "shortLink": self.door().and_then(|_| self.short_page()),
            "next": next,
            "opened": self.opened,
            "warning": self.access.warning().map(|(what, _)| what),
            "memoryLimit": tmt_colab::decoder::memory_limit(),
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
        // Next steps come in the order they are needed: the link opens only once a browser is paired.
        if self.needs_pairing() {
            rows.push((
                "pair",
                self.pairing
                    .as_ref()
                    .and_then(Pairing::step)
                    .unwrap_or("if this browser is new, pair for page access: tmt remote pair")
                    .into(),
            ));
        }
        match (self.target_text(), self.pages.as_ref()) {
            (Some(text), _) if self.opened => {
                rows.push(("open", format!("opened in your browser: {text}")))
            }
            (Some(text), _) => rows.push(("open", text)),
            (None, None) => rows.push(("open", "pages unknown: tmt colab ls".into())),
            (None, Some(_)) => {}
        }
        if self.needs_page() {
            rows.push(("create", CREATE.into()));
        }
        rows
    }
}
