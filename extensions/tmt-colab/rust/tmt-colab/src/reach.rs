//! Where a person opens a page, for every command that names one. One door lookup per command;
//! the wording lives in `door.rs`, so `serve` and the page commands say the same thing.
use crate::door::{Door, Lookup, Pairing};
use serde_json::{Value, json};

pub struct Reach {
    lookup: Lookup,
    unavailable_hint: &'static str,
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
            unavailable_hint: Door::INSTALL_HINT,
            pairing,
            pages: Vec::new(),
        }
    }
    pub fn from_creation(observation: crate::door::CreationObservation) -> Self {
        let pairing = matches!(observation.lookup, Lookup::Running(_)).then(Pairing::lookup);
        Self {
            lookup: observation.lookup,
            unavailable_hint: "Remote link unavailable from the creation snapshot",
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
                .unwrap_or_else(|| self.hint(&format!("x/colab/p/{id}")));
        }
        self.hint(path)
    }
    fn hint(&self, path: &str) -> String {
        match self.lookup {
            Lookup::Unknown => format!("{path} ({})", self.unavailable_hint),
            _ => Door::hint(&self.lookup, path),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_creation_is_neutral_and_never_supplies_a_link_or_next_action() {
        let path = Reach::path("space", "10000000-0000-4000-8000-000000000001");
        let creation = Reach::from_creation(crate::door::CreationObservation {
            lookup: Lookup::Unknown,
            machine_id: None,
        });
        let text = creation.text(&path);
        assert!(text.contains("Remote link unavailable from the creation snapshot"));
        assert!(!text.contains("install") && !text.contains("serve") && !text.contains("pair"));
        let mut value = json!({});
        creation.annotate(&mut value, &path);
        assert_eq!(value["link"], Value::Null);
        assert_eq!(value["shortLink"], Value::Null);
        assert_eq!(value["paired"], Value::Null);
        assert_eq!(value["next"], json!([]));
        let ordinary = Reach {
            lookup: Lookup::Unknown,
            pages: Vec::new(),
            pairing: None,
            unavailable_hint: Door::INSTALL_HINT,
        };
        assert!(ordinary.text(&path).contains(Door::INSTALL_HINT));
        let stopped = Reach::from_creation(crate::door::CreationObservation {
            lookup: Lookup::Stopped(None),
            machine_id: None,
        });
        assert!(stopped.text(&path).contains("run tmt colab serve"));
        assert_eq!(stopped.step(), None);
    }
}
