//! Where a person opens a page, for every command that names one. One door lookup per command;
//! the wording lives in `door.rs`, so `serve` and the page commands say the same thing.
use crate::door::{Door, Lookup, Pairing};
use serde_json::{Value, json};

pub struct Reach {
    lookup: Lookup,
    pages: Vec<String>,
    /// Read only while a door runs: there is nothing to pair with otherwise.
    pairing: Option<Pairing>,
}

impl Reach {
    pub fn gather() -> Self {
        let lookup = Door::lookup();
        let pairing = matches!(lookup, Lookup::Running(_)).then(Pairing::lookup);
        Self {
            lookup,
            pairing,
            pages: Vec::new(),
        }
    }
    pub fn with_pages(mut self, pages: &[String]) -> Self {
        self.pages = pages.to_vec();
        self
    }
    fn short_id<'a>(&self, path: &'a str) -> Option<&'a str> {
        let page = path
            .strip_prefix("x/colab/#space=")?
            .split_once("&path=%2Fpages%2F")?
            .1;
        tmt_colab_model::values::generated_id(page).ok()?;
        Some(tmt_colab::short_links::shortest_id(page, &self.pages))
    }
    pub fn short_link(&self, path: &str) -> Option<String> {
        let id = self.short_id(path)?;
        match &self.lookup {
            Lookup::Running(door) => Some(format!("{}/p/{id}", door.origin())),
            _ => None,
        }
    }
    /// The page path under the Remote door address for a space and page.
    pub fn path(space: &str, page: &str) -> String {
        format!("x/colab/#space={space}&path=%2Fpages%2F{page}")
    }
    /// The full link, only while a door runs.
    pub fn link(&self, path: &str) -> Option<String> {
        match &self.lookup {
            Lookup::Running(door) => Some(door.url(path)),
            _ => None,
        }
    }
    /// The link, or the path with why there is no full link.
    pub fn text(&self, path: &str) -> String {
        if let Some(id) = self.short_id(path) {
            return self
                .short_link(path)
                .unwrap_or_else(|| Door::hint(&self.lookup, &format!("x/colab/p/{id}")));
        }
        Door::hint(&self.lookup, path)
    }
    /// The explicit pairing step, while a door runs and no browser is known to be paired.
    pub fn step(&self) -> Option<&'static str> {
        self.pairing.as_ref().and_then(Pairing::step)
    }
    /// Adds `path` (relative) and `link` (full, `null` without a door) to a JSON object.
    pub fn annotate_link(&self, value: &mut Value, path: &str) {
        value["path"] = json!(path);
        value["link"] = json!(self.link(path));
        if self.short_id(path).is_some() {
            value["shortLink"] = json!(self.short_link(path));
        }
    }
    /// The link facts plus `paired` and `next`, for a JSON result that names a page.
    pub fn annotate(&self, value: &mut Value, path: &str) {
        self.annotate_link(value, path);
        value["paired"] = match &self.pairing {
            Some(Pairing::Paired(_)) => json!(true),
            Some(Pairing::Unpaired) => json!(false),
            _ => Value::Null,
        };
        value["next"] = match self.step() {
            Some(_) => json!(["tmt remote pair"]),
            None => json!([]),
        };
    }
}
