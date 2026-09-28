//! One background loader: the board paints from what it has and never waits
//! on core. Queued requests collapse to the newest, and each load re-reads
//! squad.toml, so configuration edits appear on the next refresh.

use super::{
    app::{Notes, Snapshot, View},
    notes::sanitize,
};
use crate::{
    config::{Config, Pane},
    core::Core,
    squad::Squad,
    status,
};
use serde_json::{Value, json};
use std::sync::mpsc::{self, Receiver, Sender};

pub struct Worker {
    requests: Sender<Option<String>>,
    pub results: Receiver<Snapshot>,
}

impl Worker {
    pub fn spawn(core: Core) -> Self {
        let (requests, pending) = mpsc::channel::<Option<String>>();
        let (sender, results) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(mut wanted) = pending.recv() {
                while let Ok(newer) = pending.try_recv() {
                    wanted = newer;
                }
                if sender.send(load(&core, wanted)).is_err() {
                    break;
                }
            }
        });
        Self { requests, results }
    }

    /// None loads the first squad.
    pub fn request(&self, squad: Option<String>) {
        let _ = self.requests.send(squad);
    }
}

fn load(core: &Core, wanted: Option<String>) -> Snapshot {
    let squads = match Squad::list(core) {
        Ok(squads) => squads,
        Err(error) => {
            return Snapshot {
                squads: Vec::new(),
                squad: wanted,
                view: Err(error.to_string()),
            };
        }
    };
    let names: Vec<String> = squads.iter().map(|squad| squad.name.clone()).collect();
    let chosen = match &wanted {
        Some(name) => squads.into_iter().find(|squad| &squad.name == name),
        None => squads.into_iter().next(),
    };
    let Some(squad) = chosen else {
        return Snapshot {
            squads: names,
            view: Err(match &wanted {
                Some(name) => format!("Squad '{name}' does not exist; run: tmt squad init {name}"),
                None => "No squad exists yet; run: tmt squad init <name>".into(),
            }),
            squad: wanted,
        };
    };
    let view = (|| {
        let config = Config::load(core)?;
        let layout = config.layout(&squad.name)?;
        let states = config.states(&squad.name, layout)?;
        let board = config.board(&squad.name, layout)?;
        let document = status::document(
            &squad,
            layout,
            &states,
            &config.sections(&squad.name)?,
            squad.members(core)?,
        );
        let notes = if board.panes.contains(&Pane::Notes) {
            lead_notes(core, &document["squad"]["lead"])
        } else {
            Notes::NotShown
        };
        Ok(View {
            columns: config.columns(&squad.name)?,
            colors: states.colors,
            render: config.notes_render(&squad.name)?,
            board,
            notes,
            document,
        })
    })()
    .map_err(|error: crate::core::SquadError| error.to_string());
    Snapshot {
        squads: names,
        squad: Some(squad.name),
        view,
    }
}

/// The lead's notebook through `tmt api notes.read`: bounded, read-only, and
/// never creates a missing notebook.
fn lead_notes(core: &Core, lead: &Value) -> Notes {
    let Some(id) = lead["id"].as_str() else {
        return Notes::NoLead;
    };
    match core.api("notes.read", json!({"identityId": id})) {
        Ok(note) => Notes::Text(sanitize(note["content"].as_str().unwrap_or_default())),
        Err(error) if error.code == "NOTEBOOK_NOT_FOUND" => Notes::Missing,
        Err(error) => Notes::Failed(error.to_string()),
    }
}
