//! One background loader: the board paints from what it has and never waits
//! on core. Queued requests collapse to the newest, and each load re-reads
//! squad.toml, so configuration edits appear on the next refresh.

use super::app::{Snapshot, View};
use crate::{config::Config, core::Core, squad::Squad, status};
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
        Ok(View {
            columns: config.columns(&squad.name)?,
            colors: config.state_colors(&squad.name, layout)?,
            document: status::document(
                &squad,
                layout,
                &config.sections(&squad.name)?,
                squad.members(core)?,
            ),
        })
    })()
    .map_err(|error: crate::core::SquadError| error.to_string());
    Snapshot {
        squads: names,
        squad: Some(squad.name),
        view,
    }
}
