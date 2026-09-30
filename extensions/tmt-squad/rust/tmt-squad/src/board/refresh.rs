//! One background loader: the board paints from what it has and never waits
//! on core. Queued requests collapse to the newest, and each load re-reads
//! squad.toml, so configuration edits appear on the next refresh.

use super::{
    app::{Notes, Snapshot, View},
    notes::sanitize,
};
use crate::{
    attention::Attention,
    config::{Config, Pane},
    core::Core,
    requests,
    squad::Squad,
    status,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, Sender},
};

pub struct Worker {
    requests: Sender<Option<String>>,
    pub results: Receiver<Snapshot>,
}

impl Worker {
    /// `tmux` selects the host preset: whether a jump can show a pane.
    pub fn spawn(core: Core, tmux: bool) -> Self {
        let (requests, pending) = mpsc::channel::<Option<String>>();
        let (sender, results) = mpsc::channel();
        std::thread::spawn(move || {
            // Reply bodies never change once submitted; keep them per worker.
            let mut bodies = BTreeMap::new();
            // The board's pane never changes, so its identity is read once.
            let caller = crate::me::caller(&core).ok().flatten();
            while let Ok(mut wanted) = pending.recv() {
                while let Ok(newer) = pending.try_recv() {
                    wanted = newer;
                }
                if sender
                    .send(load(&core, tmux, caller.as_ref(), wanted, &mut bodies))
                    .is_err()
                {
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

fn load(
    core: &Core,
    tmux: bool,
    caller: Option<&crate::me::Caller>,
    wanted: Option<String>,
    bodies: &mut BTreeMap<String, String>,
) -> Snapshot {
    let squads = match Squad::list(core) {
        Ok(squads) => squads,
        Err(error) => {
            return Snapshot {
                squads: Vec::new(),
                attention: BTreeMap::new(),
                squad: wanted,
                view: Err(error.to_string()),
            };
        }
    };
    let names: Vec<String> = squads.iter().map(|squad| squad.name.clone()).collect();
    let chosen = match &wanted {
        Some(name) => squads.iter().find(|squad| &squad.name == name),
        None => squads.first(),
    };
    let Some(squad) = chosen.cloned() else {
        return Snapshot {
            squads: names,
            attention: BTreeMap::new(),
            view: Err(match &wanted {
                Some(name) => format!("Squad '{name}' does not exist; run: tmt squad init {name}"),
                None => "No squad exists yet; run: tmt squad init <name>".into(),
            }),
            squad: wanted,
        };
    };
    let mut attention = BTreeMap::new();
    let view = (|| {
        let config = Config::load(core)?;
        let layout = config.layout(&squad.name)?;
        let states = config.states(&squad.name, layout)?;
        let board = config.board(&squad.name, layout)?;
        let sections = config.sections(&squad.name)?;
        let me = crate::me::you(crate::me::current(core, &config)?, caller);
        let mut document =
            status::document(&squad, layout, &states, &sections, squad.members(core)?);
        let sent = requests::overlay(core, &squad, me.as_ref(), &mut document)?;
        attention = others(core, &config, &squads, &squad.name, me.as_ref());
        attention.insert(squad.name.clone(), Attention::of(&document));
        let mut replies = match &sent {
            Some(sent) if board.panes.contains(&Pane::Replies) => {
                requests::replies(sent, &document)
            }
            _ => Vec::new(),
        };
        requests::bodies(|id| requests::show_request(core, id), &mut replies, bodies)?;
        let notes = if board.panes.contains(&Pane::Notes) {
            lead_notes(core, &document["squad"]["lead"])
        } else {
            Notes::NotShown
        };
        Ok(View {
            rows: config.rows(&squad.name)?,
            colors: states.colors,
            render: config.notes_render(&squad.name)?,
            bindings: config.bindings(tmux)?,
            section_bindings: sections.into_iter().map(|section| section.bind).collect(),
            opener: config.program("opener")?,
            clipboard: config.program("clipboard")?,
            tab_colors: config.tab_colors()?,
            me: me.map(|me| me.name),
            replies,
            refresh: config.refresh(&squad.name)?,
            board,
            notes,
            document,
        })
    })()
    .map_err(|error: crate::core::SquadError| error.to_string());
    Snapshot {
        squads: names,
        attention,
        squad: Some(squad.name),
        view,
    }
}

/// The attention of every squad but the one shown, from its roster alone:
/// one `rooms.roster` read each, and one inbox read shared by all. A squad
/// that cannot be read has no entry, so its tab shows no state.
fn others(
    core: &Core,
    config: &Config,
    squads: &[Squad],
    shown: &str,
    me: Option<&crate::me::Me>,
) -> BTreeMap<String, Attention> {
    let others: Vec<&Squad> = squads.iter().filter(|squad| squad.name != shown).collect();
    if others.is_empty() {
        return BTreeMap::new();
    }
    let waiting = match me {
        Some(me) => match requests::inbox(core, &me.id) {
            Ok(inbox) => Some((me.id.as_str(), inbox)),
            Err(_) => return BTreeMap::new(),
        },
        None => None,
    };
    others
        .into_iter()
        .filter_map(|squad| {
            let roster = squad.roster(core).ok()?;
            let waiting = waiting.as_ref().map(|(me, inbox)| (*me, inbox));
            Some((
                squad.name.clone(),
                attention(config, squad, roster, waiting).ok()?,
            ))
        })
        .collect()
}

/// One squad's attention from its roster: the same document `status` builds,
/// so a tab and `ls --json` never disagree.
fn attention(
    config: &Config,
    squad: &Squad,
    roster: Vec<crate::squad::Member>,
    waiting: Option<(&str, &requests::Window)>,
) -> Result<Attention, crate::core::SquadError> {
    let layout = config.layout(&squad.name)?;
    let states = config.states(&squad.name, layout)?;
    let sections = config.sections(&squad.name)?;
    let mut document = status::document(squad, layout, &states, &sections, roster);
    if let Some((me, inbox)) = waiting {
        requests::apply_waiting(&mut document, &squad.name, me, inbox);
    }
    Ok(Attention::of(&document))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::squad::Member;

    fn member(id: &str, fields: &[(&str, &str)]) -> Member {
        Member {
            id: id.into(),
            name: id.to_lowercase(),
            lifetime: "saved".into(),
            presence: "unknown".into(),
            pane: Value::Null,
            activity: Value::Null,
            fields: fields
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
        }
    }

    #[test]
    fn a_hidden_squad_s_attention_comes_from_its_roster_and_the_shared_inbox() {
        let directory = std::env::temp_dir().join(format!("squad-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("squad.toml");
        // A user section repeats a member: it still counts once.
        std::fs::write(
            &path,
            "[[squad.infra.section]]\ntitle = \"Blocked\"\nfilter = \"state = blocked\"\n",
        )
        .unwrap();
        let config = Config::read(path).unwrap();
        let squad = Squad {
            name: "infra".into(),
            room_id: "room-infra".into(),
        };
        let roster = || {
            vec![
                member("L", &[("role", "lead"), ("pending", "approve")]),
                member("A", &[("state", "blocked")]),
                member("B", &[("state", "working")]),
            ]
        };
        let inbox = requests::Window {
            items: vec![
                json!({"requestId": "q1", "from": {"identityId": "B"}, "preview": "?", "preparedAtMs": 1}),
                json!({"requestId": "q2", "from": {"identityId": "X"}, "preview": "?", "preparedAtMs": 2}),
            ],
            complete: true,
        };
        assert_eq!(
            attention(&config, &squad, roster(), Some(("ME", &inbox))).unwrap(),
            Attention {
                waiting: 2,
                blocked: 1
            },
            "the lead's decision and B's request; X is not in the squad"
        );
        // Without a known user only member-set decisions count.
        assert_eq!(
            attention(&config, &squad, roster(), None).unwrap(),
            Attention {
                waiting: 1,
                blocked: 1
            }
        );
        let _ = std::fs::remove_dir_all(directory);
    }
}
