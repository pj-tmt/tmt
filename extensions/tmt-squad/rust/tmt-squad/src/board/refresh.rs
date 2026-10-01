//! One background loader: the board paints from what it has and never waits
//! on core. Queued requests collapse to the newest, and each load re-reads
//! squad.toml, so configuration edits appear on the next refresh. Between
//! requests it reloads early when core's records or squad.toml changed.

use super::{
    ALL, LEADS,
    app::{Notes, Snapshot, View},
    changes::{Changes, Stamp},
    notes::sanitize,
    tabs,
};
use crate::{
    attention::Attention,
    config::{Board, BoardMode, Config, Direction, Layout, NotesRender, Pane},
    core::Core,
    provider::{self, Provider},
    requests,
    squad::{Member, Squad},
    status,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    time::Duration,
};

/// How often an idle worker checks for changes between interval reloads.
const CHECK_EVERY: Duration = Duration::from_secs(1);

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
            let mut kept = Kept {
                bodies: BTreeMap::new(),
                fetch: fetcher(),
            };
            // The board's pane and squad.toml's place never change, so both
            // are read once.
            let caller = crate::me::caller(&core).ok().flatten();
            let mut changes = Changes::new(
                core.clone(),
                Config::locate(&core).ok(),
                provider::Cache::directory(),
            );
            serve(
                &pending,
                &sender,
                CHECK_EVERY,
                || changes.stamp(),
                |wanted| load(&core, tmux, caller.as_ref(), wanted, &mut kept),
            );
        });
        Self { requests, results }
    }

    /// None loads the first squad.
    pub fn request(&self, squad: Option<String>) {
        let _ = self.requests.send(squad);
    }
}

/// What one worker keeps across loads.
struct Kept {
    /// Reply bodies never change once submitted.
    bodies: BTreeMap<String, String>,
    fetch: Sender<Fetch>,
}

/// One squad's providers and members, for the fetcher.
struct Fetch {
    squad: String,
    providers: Vec<Provider>,
    members: Vec<Member>,
}

/// Field providers run on their own thread, so a slow `gh` never delays a
/// load. Queued work collapses to the newest; each run rereads the cache,
/// so work another run finished meanwhile is not repeated. Between loads it
/// runs the last squad's providers again at their shortest `every`, so
/// values stay current whatever the board's own interval. Saved values move
/// the cache directory's stamp, and the next check reloads the board.
fn fetcher() -> Sender<Fetch> {
    let (sender, pending) = mpsc::channel::<Fetch>();
    std::thread::spawn(move || {
        let mut last: Option<Fetch> = None;
        loop {
            let wait = last
                .as_ref()
                .and_then(|fetch| fetch.providers.iter().map(Provider::every).min());
            let received = match wait {
                Some(wait) => pending.recv_timeout(wait),
                None => pending.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match received {
                Ok(mut fetch) => {
                    while let Ok(newer) = pending.try_recv() {
                        fetch = newer;
                    }
                    last = Some(fetch);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if let Some(fetch) = &last {
                provider::refresh(
                    &fetch.squad,
                    &fetch.providers,
                    &fetch.members,
                    crate::status::now_ms(),
                );
            }
        }
    });
    sender
}

/// Loads each request, newest first, until the board goes away. While idle
/// it reloads the last squad early when the change stamp moved since that
/// squad's load, unless its view has automatic reload off. The input loop's
/// interval reloads are requests like any other and never wait on this.
fn serve(
    pending: &Receiver<Option<String>>,
    sender: &Sender<Snapshot>,
    check_every: Duration,
    mut stamp: impl FnMut() -> Stamp,
    mut load: impl FnMut(Option<String>) -> Snapshot,
) {
    // The squad last loaded, whether it reloads automatically, and the
    // stamp taken just before that load.
    let mut last: Option<(Option<String>, bool, Stamp)> = None;
    loop {
        let mut wanted = match pending.recv_timeout(check_every) {
            Ok(wanted) => wanted,
            Err(RecvTimeoutError::Timeout) => match &last {
                Some((squad, true, seen)) if seen.moved(&stamp()) => squad.clone(),
                _ => continue,
            },
            Err(RecvTimeoutError::Disconnected) => break,
        };
        while let Ok(newer) = pending.try_recv() {
            wanted = newer;
        }
        // Taken before the load, so a change during it shows at the next check.
        let seen = stamp();
        let snapshot = load(wanted);
        // A squad that failed to load keeps the default interval.
        let automatic = snapshot
            .view
            .as_ref()
            .map_or(true, |view| view.refresh.is_some());
        last = Some((snapshot.squad.clone(), automatic, seen));
        if sender.send(snapshot).is_err() {
            break;
        }
    }
}

fn load(
    core: &Core,
    tmux: bool,
    caller: Option<&crate::me::Caller>,
    wanted: Option<String>,
    kept: &mut Kept,
) -> Snapshot {
    let squads = match Squad::list(core) {
        Ok(squads) => squads,
        Err(error) => {
            return Snapshot {
                tabs: Vec::new(),
                hidden: Vec::new(),
                pinned: 0,
                attention: BTreeMap::new(),
                squad: wanted,
                view: Err(error.to_string()),
            };
        }
    };
    let names: Vec<String> = squads.iter().map(|squad| squad.name.clone()).collect();
    let config = Config::load(core);
    // An invalid [tabs] still shows every squad; the view reports the error.
    let (tabs, pinned) = if names.is_empty() {
        (Vec::new(), 0)
    } else {
        let settings = config.as_ref().ok().and_then(|config| config.tabs().ok());
        tabs::arrange(&names, &settings.unwrap_or_default())
    };
    let hidden: Vec<String> = if names.is_empty() {
        Vec::new()
    } else {
        names
            .iter()
            .cloned()
            .chain([LEADS.to_owned(), ALL.to_owned()])
            .filter(|key| !tabs.contains(key))
            .collect()
    };
    // A hidden squad is still shown when asked for by name.
    let chosen = match &wanted {
        Some(key) if (tabs::builtin(key) && !names.is_empty()) || names.contains(key) => {
            Some(key.clone())
        }
        Some(_) => None,
        None => tabs.first().cloned(),
    };
    let Some(key) = chosen else {
        return Snapshot {
            tabs,
            hidden,
            pinned,
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
        } else if key == ALL {
            all_view(core, &config, &squads, &tabs, me)?
        } else {
            let squad = squads
                .iter()
                .find(|squad| squad.name == key)
                .expect("chosen from the listed squads");
            squad_view(core, tmux, &config, &squads, squad, me, kept)?
        };
        attention = found;
        Ok(view)
    })()
    .map_err(|error: crate::core::SquadError| error.to_string());
    Snapshot {
        tabs,
        hidden,
        pinned,
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
    kept: &mut Kept,
) -> Result<(View, BTreeMap<String, Attention>), crate::core::SquadError> {
    let layout = config.layout(&squad.name)?;
    let states = config.states(&squad.name, layout)?;
    let board = config.board(&squad.name, layout)?;
    let sections = config.sections(&squad.name)?;
    let rows = config.rows(&squad.name)?;
    let providers = config.providers(&squad.name)?;
    let mut members = squad.members(core, rows.reads_metadata())?;
    if !providers.is_empty() {
        let _ = kept.fetch.send(Fetch {
            squad: squad.name.clone(),
            providers: providers.clone(),
            members: members.clone(),
        });
    }
    provider::apply(
        &providers,
        &mut members,
        &provider::Cache::load(&squad.name),
    );
    let mut document = status::document(squad, layout, &states, &sections, &rows, members);
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
    requests::bodies(
        |id| requests::show_request(core, id),
        &mut replies,
        &mut kept.bodies,
    )?;
    let notes = if board.panes.contains(&Pane::Notes) {
        lead_notes(core, &document["squad"]["lead"])
    } else {
        Notes::NotShown
    };
    let view = View {
        rows,
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

/// The built-in `all` tab: one row per squad with its lead, member count
/// and attention, in tab order. Enter opens that squad's tab; the rows are
/// squads, not members, so no member binding applies here.
fn all_view(
    core: &Core,
    config: &Config,
    squads: &[Squad],
    tabs: &[String],
    me: Option<crate::me::Me>,
) -> Result<(View, BTreeMap<String, Attention>), crate::core::SquadError> {
    let settings = config.tabs()?;
    let all: Vec<&Squad> = squads.iter().collect();
    let documents = roster_documents(core, config, &all, me.as_ref(), None);
    let attention = tab_attention(&documents);
    let mut bindings = crate::action::parse_bindings(
        [
            ("enter", Some("tab")),
            ("double-click", Some("tab")),
            ("f5", Some("refresh")),
        ]
        .into_iter(),
        "tabs.all",
    )
    .expect("the all tab's preset");
    bindings.extend(settings.all);
    let view = View {
        rows: crate::rows::Rows::overview(),
        colors: BTreeMap::new(),
        render: NotesRender::Markdown,
        bindings,
        section_bindings: Vec::new(),
        opener: None,
        clipboard: None,
        tab_colors: settings.colors,
        me: me.map(|me| me.name),
        replies: Vec::new(),
        refresh: config.refresh(ALL)?,
        board: Board::simple(
            BoardMode::Split,
            Direction::LeftRight,
            vec![Pane::Rows],
            &[100],
        ),
        notes: Notes::NotShown,
        document: all_document(tabs, &documents, &attention),
    };
    Ok((view, attention))
}

/// The `all` tab's document: a row per squad, `name` the squad's, with its
/// lead, member count and attention counts as fields.
fn all_document(
    tabs: &[String],
    documents: &BTreeMap<String, Value>,
    attention: &BTreeMap<String, Attention>,
) -> Value {
    let rows: Vec<Value> = in_tab_order(tabs, documents)
        .into_iter()
        .map(|name| {
            let document = &documents[name];
            let lead = document["squad"]["lead"]["name"].as_str();
            let members = document["sections"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|section| section["rows"].as_array().into_iter().flatten())
                .filter_map(|row| row["name"].as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let attention = attention.get(name).copied().unwrap_or_default();
            json!({
                "name": name,
                "squad": name,
                "state": attention.state(),
                "fields": {
                    "squad": name,
                    "lead": lead,
                    "members": members.to_string(),
                    "waiting": attention.waiting.to_string(),
                    "blocked": attention.blocked.to_string(),
                },
            })
        })
        .collect();
    json!({
        "squad": {"name": "all", "lead": null},
        "sections": [{"title": null, "rows": rows}],
    })
}

/// Squads in tab order, then squads off the tab line.
fn in_tab_order<'a>(tabs: &'a [String], documents: &'a BTreeMap<String, Value>) -> Vec<&'a String> {
    let mut order: Vec<&String> = tabs
        .iter()
        .filter(|key| documents.contains_key(*key))
        .collect();
    order.extend(documents.keys().filter(|key| !tabs.contains(key)));
    order
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
    let rows = config.rows(&squad.name)?;
    let mut document = status::document(squad, layout, &states, &sections, &rows, roster);
    if let Some((me, inbox)) = waiting {
        requests::apply_waiting(&mut document, &squad.name, me, inbox);
    }
    Ok(document)
}

/// The leads tab's document: one row per squad that has a lead, in tab
/// order (hidden squads last), each carrying its squad as `squad` and as
/// the `squad` field.
fn leads_document(tabs: &[String], documents: &BTreeMap<String, Value>) -> Value {
    let rows: Vec<Value> = in_tab_order(tabs, documents)
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
    // Every squad's members, each counted once per squad.
    let all = attention
        .iter()
        .filter(|(key, _)| !tabs::builtin(key))
        .fold(Attention::default(), |sum, (_, one)| Attention {
            waiting: sum.waiting + one.waiting,
            blocked: sum.blocked + one.blocked,
        });
    attention.insert(ALL.to_owned(), all);
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
            lead_marker: None,
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
            meta: Default::default(),
            seen: Value::Null,
            numbers: Default::default(),
            failed: Default::default(),
        }
    }

    /// Runs `serve` on its own thread with a stamp the test sets, and
    /// returns the requests channel, the loaded squads and the stamp.
    fn serving(
        automatic: bool,
    ) -> (
        Sender<Option<String>>,
        Receiver<Option<String>>,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) {
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        let (requests, pending) = mpsc::channel();
        let (sender, results) = mpsc::channel();
        let (loaded, loads) = mpsc::channel();
        let cursor = Arc::new(AtomicU64::new(1));
        let read = Arc::clone(&cursor);
        std::thread::spawn(move || {
            serve(
                &pending,
                &sender,
                Duration::from_millis(10),
                || Stamp::cursor(read.load(Ordering::SeqCst)),
                |wanted| {
                    let _ = loaded.send(wanted.clone());
                    let mut snapshot = crate::board::app::tests::snapshot(
                        wanted.as_deref().unwrap_or("first"),
                        json!([]),
                    );
                    if let Ok(view) = &mut snapshot.view {
                        view.refresh = automatic.then_some(Duration::from_secs(3600));
                    }
                    snapshot
                },
            );
            drop(results);
        });
        (requests, loads, cursor)
    }

    const WAIT: Duration = Duration::from_millis(300);

    /// A moved stamp reloads the last squad at once, well before its
    /// hour-long interval; an unmoved one never does.
    #[test]
    fn the_worker_reloads_early_only_when_the_stamp_moves() {
        use std::sync::atomic::Ordering;
        let (requests, loads, cursor) = serving(true);
        requests.send(Some("product".into())).unwrap();
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("product".into())));
        assert!(loads.recv_timeout(WAIT).is_err(), "nothing changed");
        cursor.store(2, Ordering::SeqCst);
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("product".into())));
        assert!(loads.recv_timeout(WAIT).is_err(), "once per change");
        // A request is served as before and becomes the squad to watch.
        requests.send(Some("infra".into())).unwrap();
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("infra".into())));
        cursor.store(3, Ordering::SeqCst);
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("infra".into())));
    }

    /// `refresh = "off"` means F5 and actions only: no early reload either.
    #[test]
    fn a_view_with_automatic_reload_off_is_never_reloaded_early() {
        use std::sync::atomic::Ordering;
        let (requests, loads, cursor) = serving(false);
        requests.send(Some("product".into())).unwrap();
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("product".into())));
        cursor.store(2, Ordering::SeqCst);
        assert!(loads.recv_timeout(WAIT).is_err());
        requests.send(Some("product".into())).unwrap();
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("product".into())));
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
        // Presence is unknown from the roster alone, and never matters:
        // blocked and waiting come from fields and requests only (#568).
        let mut seen = roster();
        for member in &mut seen {
            member.presence = "active".into();
        }
        assert_eq!(
            Attention::of(&roster_document(&config, &squad, seen, Some(("ME", &inbox))).unwrap()),
            Attention::of(
                &roster_document(&config, &squad, roster(), Some(("ME", &inbox))).unwrap()
            )
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
        // The all tab sums its squads: hidden's sol is not waiting there.
        assert_eq!(
            attention[ALL],
            Attention {
                waiting: 1,
                blocked: 1
            }
        );
        let all = all_document(&tabs, &documents, &attention);
        let rows = all["sections"][0]["rows"].as_array().unwrap();
        let listed: Vec<(&str, Option<&str>, &str)> = rows
            .iter()
            .map(|row| {
                (
                    row["name"].as_str().unwrap(),
                    row["fields"]["lead"].as_str(),
                    row["state"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                ("product", Some("sol"), "waiting"),
                ("infra", Some("rin"), "blocked"),
                ("quiet", None, "normal"),
                ("hidden", Some("sol"), "normal"),
            ],
            "every squad, with or without a lead, in tab order"
        );
        assert_eq!(rows[0]["squad"], "product", "Enter opens this tab");
        assert_eq!(rows[0]["fields"]["waiting"], "1");
        assert_eq!(rows[2]["fields"]["lead"], Value::Null);

        let mut snapshot = crate::board::app::tests::snapshot(ALL, json!([]));
        let view = snapshot.view.as_mut().unwrap();
        view.document = all;
        view.rows = crate::rows::Rows::overview();
        let mut app = crate::board::app::App::new(Some(ALL.into()));
        app.apply(snapshot);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(64, 10)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let screen: Vec<String> = terminal
            .backend()
            .buffer()
            .content()
            .chunks(64)
            .map(|line| line.iter().map(|cell| cell.symbol()).collect())
            .collect();
        let quiet = screen
            .iter()
            .find(|line| line.trim_start().starts_with("quiet"))
            .expect("the leadless squad is visible on the all tab");
        assert_eq!(
            quiet.split_whitespace().collect::<Vec<_>>(),
            ["quiet", "–", "0", "0", "0"],
            "the view renders the missing lead as a display placeholder"
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
