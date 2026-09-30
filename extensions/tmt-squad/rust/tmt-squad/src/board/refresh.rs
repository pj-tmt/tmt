//! One background loader: the board paints from what it has and never waits
//! on core. Queued requests collapse to the newest, and each load re-reads
//! squad.toml, so configuration edits appear on the next refresh.

use super::{
    LEADS,
    app::{Notes, Snapshot, View},
    notes::sanitize,
    tabs,
};
use crate::{
    attention::Attention,
    config::{Board, BoardMode, Config, Direction, Layout, NotesRender, Pane},
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
                tabs: Vec::new(),
                attention: BTreeMap::new(),
                squad: wanted,
                view: Err(error.to_string()),
            };
        }
    };
    let names: Vec<String> = squads.iter().map(|squad| squad.name.clone()).collect();
    let config = Config::load(core);
    // An invalid [tabs] still shows every squad; the view reports the error.
    let tabs = if names.is_empty() {
        Vec::new()
    } else {
        let settings = config.as_ref().ok().and_then(|config| config.tabs().ok());
        tabs::arrange(&names, &settings.unwrap_or_default())
    };
    // A hidden squad is still shown when asked for by name.
    let chosen = match &wanted {
        Some(key) if (key == LEADS && !names.is_empty()) || names.contains(key) => {
            Some(key.clone())
        }
        Some(_) => None,
        None => tabs.first().cloned(),
    };
    let Some(key) = chosen else {
        return Snapshot {
            tabs,
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
        let config = config?;
        let me = crate::me::you(crate::me::current(core, &config)?, caller);
        let (view, found) = if key == LEADS {
            leads_view(core, tmux, &config, &squads, &tabs, me)?
        } else {
            let squad = squads
                .iter()
                .find(|squad| squad.name == key)
                .expect("chosen from the listed squads");
            squad_view(core, tmux, &config, &squads, squad, me, bodies)?
        };
        attention = found;
        Ok(view)
    })()
    .map_err(|error: crate::core::SquadError| error.to_string());
    Snapshot {
        tabs,
        attention,
        squad: Some(key),
        view,
    }
}

/// One squad's full view, and every tab's attention.
fn squad_view(
    core: &Core,
    tmux: bool,
    config: &Config,
    squads: &[Squad],
    squad: &Squad,
    me: Option<crate::me::Me>,
    bodies: &mut BTreeMap<String, String>,
) -> Result<(View, BTreeMap<String, Attention>), crate::core::SquadError> {
    let layout = config.layout(&squad.name)?;
    let states = config.states(&squad.name, layout)?;
    let board = config.board(&squad.name, layout)?;
    let sections = config.sections(&squad.name)?;
    let mut document = status::document(squad, layout, &states, &sections, squad.members(core)?);
    let sent = requests::overlay(core, squad, me.as_ref(), &mut document)?;
    let others: Vec<&Squad> = squads
        .iter()
        .filter(|other| other.name != squad.name)
        .collect();
    let mut documents = roster_documents(core, config, &others, me.as_ref(), None);
    documents.insert(squad.name.clone(), document.clone());
    let attention = tab_attention(&documents);
    let mut replies = match &sent {
        Some(sent) if board.panes.contains(&Pane::Replies) => requests::replies(sent, &document),
        _ => Vec::new(),
    };
    requests::bodies(|id| requests::show_request(core, id), &mut replies, bodies)?;
    let notes = if board.panes.contains(&Pane::Notes) {
        lead_notes(core, &document["squad"]["lead"])
    } else {
        Notes::NotShown
    };
    let view = View {
        rows: config.rows(&squad.name)?,
        colors: states.colors,
        render: config.notes_render(&squad.name)?,
        bindings: config.bindings(tmux)?,
        section_bindings: sections.into_iter().map(|section| section.bind).collect(),
        opener: config.program("opener")?,
        clipboard: config.program("clipboard")?,
        tab_colors: config.tabs()?.colors,
        me: me.map(|me| me.name),
        replies,
        refresh: config.refresh(&squad.name)?,
        board,
        notes,
        document,
    };
    Ok((view, attention))
}

/// The built-in leads tab: every squad's lead, with presence from one `ls`
/// read, in tab order. Enter jumps to the lead as on a squad's own tab.
fn leads_view(
    core: &Core,
    tmux: bool,
    config: &Config,
    squads: &[Squad],
    tabs: &[String],
    me: Option<crate::me::Me>,
) -> Result<(View, BTreeMap<String, Attention>), crate::core::SquadError> {
    let settings = config.tabs()?;
    let listed = core.json(&["ls"])?;
    let all: Vec<&Squad> = squads.iter().collect();
    let documents = roster_documents(core, config, &all, me.as_ref(), Some(&listed));
    let document = leads_document(tabs, &documents);
    let attention = tab_attention(&documents);
    let mut bindings = config.bindings(tmux)?;
    bindings.extend(settings.leads);
    let view = View {
        rows: crate::rows::Rows::leads(),
        colors: config.states(LEADS, Layout::Crew)?.colors,
        render: NotesRender::Markdown,
        bindings,
        section_bindings: Vec::new(),
        opener: config.program("opener")?,
        clipboard: config.program("clipboard")?,
        tab_colors: settings.colors,
        me: me.map(|me| me.name),
        replies: Vec::new(),
        refresh: config.refresh(LEADS)?,
        board: Board::simple(
            BoardMode::Split,
            Direction::LeftRight,
            vec![Pane::Rows],
            &[100],
        ),
        notes: Notes::NotShown,
        document,
    };
    Ok((view, attention))
}

/// Each squad's roster-only status document, with what waits on the user
/// from one inbox read shared by all, and presence when an `ls` document is
/// given. A squad that cannot be read is left out.
fn roster_documents(
    core: &Core,
    config: &Config,
    squads: &[&Squad],
    me: Option<&crate::me::Me>,
    listed: Option<&Value>,
) -> BTreeMap<String, Value> {
    if squads.is_empty() {
        return BTreeMap::new();
    }
    let waiting = match me {
        Some(me) => match requests::inbox(core, &me.id) {
            Ok(inbox) => Some((me.id.as_str(), inbox)),
            Err(_) => return BTreeMap::new(),
        },
        None => None,
    };
    squads
        .iter()
        .filter_map(|squad| {
            let mut roster = squad.roster(core).ok()?;
            if let Some(listed) = listed {
                crate::squad::join_presence(&mut roster, listed);
            }
            let waiting = waiting.as_ref().map(|(me, inbox)| (*me, inbox));
            Some((
                squad.name.clone(),
                roster_document(config, squad, roster, waiting).ok()?,
            ))
        })
        .collect()
}

/// One squad's document from its roster: the same document `status`
/// builds, so a tab and `ls --json` never disagree.
fn roster_document(
    config: &Config,
    squad: &Squad,
    roster: Vec<crate::squad::Member>,
    waiting: Option<(&str, &requests::Window)>,
) -> Result<Value, crate::core::SquadError> {
    let layout = config.layout(&squad.name)?;
    let states = config.states(&squad.name, layout)?;
    let sections = config.sections(&squad.name)?;
    let mut document = status::document(squad, layout, &states, &sections, roster);
    if let Some((me, inbox)) = waiting {
        requests::apply_waiting(&mut document, &squad.name, me, inbox);
    }
    Ok(document)
}

/// The leads tab's document: one row per squad that has a lead, in tab
/// order (hidden squads last), each carrying its squad as `squad` and as
/// the `squad` field.
fn leads_document(tabs: &[String], documents: &BTreeMap<String, Value>) -> Value {
    let mut order: Vec<&String> = tabs
        .iter()
        .filter(|key| documents.contains_key(*key))
        .collect();
    order.extend(documents.keys().filter(|key| !tabs.contains(key)));
    let rows: Vec<Value> = order
        .into_iter()
        .filter_map(|name| {
            let mut lead = documents[name]["squad"]["lead"].clone();
            if !lead.is_object() {
                return None;
            }
            lead["squad"] = json!(name);
            lead["fields"]["squad"] = json!(name);
            Some(lead)
        })
        .collect();
    json!({
        "squad": {"name": "leads", "lead": null},
        "sections": [{"title": null, "rows": rows}],
    })
}

/// Every squad tab's attention, and the leads tab's from its leads.
fn tab_attention(documents: &BTreeMap<String, Value>) -> BTreeMap<String, Attention> {
    let mut attention: BTreeMap<String, Attention> = documents
        .iter()
        .map(|(name, document)| (name.clone(), Attention::of(document)))
        .collect();
    attention.insert(
        LEADS.to_owned(),
        Attention::of(&leads_document(&[], documents)),
    );
    attention
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
            Attention::of(
                &roster_document(&config, &squad, roster(), Some(("ME", &inbox))).unwrap()
            ),
            Attention {
                waiting: 2,
                blocked: 1
            },
            "the lead's decision and B's request; X is not in the squad"
        );
        // Without a known user only member-set decisions count.
        assert_eq!(
            Attention::of(&roster_document(&config, &squad, roster(), None).unwrap()),
            Attention {
                waiting: 1,
                blocked: 1
            }
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn the_leads_tab_lists_each_squad_s_lead_in_tab_order_with_its_squad() {
        let squad = |lead: Option<Value>| json!({"squad": {"lead": lead}, "sections": []});
        let documents = BTreeMap::from([
            (
                "infra".to_owned(),
                squad(Some(
                    json!({"id": "R", "name": "rin", "state": "blocked", "fields": {}}),
                )),
            ),
            ("quiet".to_owned(), squad(None)),
            (
                "product".to_owned(),
                squad(Some(
                    json!({"id": "S", "name": "sol", "pending": "approve", "fields": {}}),
                )),
            ),
            (
                "hidden".to_owned(),
                squad(Some(json!({"id": "S", "name": "sol", "fields": {}}))),
            ),
        ]);
        let tabs = ["product", LEADS, "infra", "quiet"].map(String::from);
        let document = leads_document(&tabs, &documents);
        let rows = document["sections"][0]["rows"].as_array().unwrap();
        // Tab order, then squads off the tab line; a squad without a lead has
        // no row, and one person can lead two squads.
        let listed: Vec<(&str, &str)> = rows
            .iter()
            .map(|row| {
                (
                    row["squad"].as_str().unwrap(),
                    row["name"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [("product", "sol"), ("infra", "rin"), ("hidden", "sol")]
        );
        assert_eq!(rows[0]["fields"]["squad"], "product", "the squad column");

        let attention = tab_attention(&documents);
        assert_eq!(
            attention[LEADS],
            Attention {
                waiting: 1,
                blocked: 1
            }
        );
        assert_eq!(attention["quiet"], Attention::default());
        assert_eq!(
            attention["infra"],
            Attention {
                waiting: 0,
                blocked: 1
            }
        );
    }
}
