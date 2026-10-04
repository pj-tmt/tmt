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
    config::{Board, BoardMode, Config, Direction, NotesRender, Pane},
    core::Core,
    provider::{self, Provider},
    requests,
    squad::{Member, Squad},
    tab_view::{self, roster_documents},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

/// How often an idle worker checks for changes between interval reloads.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// The event fence and its invoke stop adapter advance under one short lock.
/// Taking an obsolete token never borrows the current generation's live flag.
#[derive(Default)]
struct Generation {
    number: AtomicU64,
    stop: Mutex<crate::runner::Cancellation>,
}

impl Generation {
    fn cancellation(&self, expected: u64) -> crate::runner::Cancellation {
        let stop = self.stop.lock().unwrap();
        if self.number.load(Ordering::Acquire) == expected {
            stop.clone()
        } else {
            let obsolete = crate::runner::Cancellation::default();
            obsolete.cancel();
            obsolete
        }
    }

    fn advance(&self) -> u64 {
        let mut stop = self.stop.lock().unwrap();
        stop.cancel();
        let next = self.number.fetch_add(1, Ordering::AcqRel) + 1;
        *stop = crate::runner::Cancellation::default();
        next
    }
}

pub struct Worker {
    requests: Sender<Work>,
    generation: Arc<Generation>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    /// `tmux` selects the host preset: whether a jump can show a pane.
    pub fn spawn(core: Core, tmux: bool, events: Sender<super::BoardEvent>) -> Self {
        let (requests, pending) = mpsc::channel();
        let generation = Arc::new(Generation::default());
        let read_generation = Arc::clone(&generation);
        let thread = std::thread::spawn(move || {
            let initial = core.cancellable(read_generation.cancellation(0));
            let mut kept = Kept {
                bodies: BTreeMap::new(),
                fetch: fetcher(),
            };
            // The board's pane and squad.toml's place never change, so both
            // are read once.
            let caller = crate::me::caller(&initial).ok().flatten();
            let mut changes =
                Changes::new(Config::locate(&initial).ok(), provider::Cache::directory());
            serve(
                &pending,
                |snapshot, generation| {
                    events
                        .send(super::BoardEvent::Snapshot {
                            cancellation: read_generation.cancellation(generation),
                            snapshot: Box::new(snapshot),
                        })
                        .is_ok()
                },
                CHECK_EVERY,
                &read_generation.number,
                |generation| {
                    changes.stamp(&core.cancellable(read_generation.cancellation(generation)))
                },
                |wanted, generation, preview_panes, opening| {
                    load(
                        &core.cancellable(read_generation.cancellation(generation)),
                        tmux,
                        caller.as_ref(),
                        wanted,
                        preview_panes,
                        opening,
                        &mut kept,
                    )
                },
                |job, generation| {
                    let cancellation = read_generation.cancellation(generation);
                    let reader = core.cancellable(cancellation.clone());
                    let event = match job {
                        Deferred::Attention(job) => super::BoardEvent::Attention {
                            attention: job.complete(&reader),
                            cancellation: cancellation.clone(),
                        },
                        Deferred::Notebook { identity, revision } => super::BoardEvent::Notebook {
                            cancellation: cancellation.clone(),
                            identity: identity.clone(),
                            revision,
                            notes: member_notes(
                                reader
                                    .api("notes.read", serde_json::json!({"identityId": identity})),
                            ),
                        },
                        Deferred::Usage(MeterRead::Home) => {
                            let input = reader
                                .json(&["ls"])
                                .ok()
                                .filter(|listed| listed["identities"].is_array())
                                .map(|listed| super::rate::Input::resumes(&listed))
                                .ok_or(());
                            super::BoardEvent::HomeUsage {
                                cancellation: cancellation.clone(),
                                input,
                            }
                        }
                        Deferred::Usage(MeterRead::Squad(input)) => {
                            let sample = reader
                                .json(&["ls", "--room", &input.room])
                                .ok()
                                .filter(|listed| listed["identities"].is_array())
                                .map(|listed| input.listed(&listed))
                                .ok_or(());
                            super::BoardEvent::Usage {
                                cancellation: cancellation.clone(),
                                room: input.room,
                                input: sample,
                            }
                        }
                    };
                    cancellation.cancelled() || events.send(event).is_ok()
                },
            );
        });
        Self {
            requests,
            generation,
            thread: Some(thread),
        }
    }

    /// None loads the first squad.
    pub fn request(&self, squad: Option<String>, preempt: bool, preview_panes: bool) {
        let generation = if preempt {
            self.generation.advance()
        } else {
            self.generation.number.load(Ordering::Acquire)
        };
        let _ = self.requests.send(Work::Reload(Reload {
            squad,
            generation,
            preview_panes,
        }));
    }
    pub fn notebook(&self, revision: u64, identity: Option<String>) {
        let generation = self.generation.number.load(Ordering::Acquire);
        let _ = self.requests.send(Work::Notebook {
            identity,
            revision,
            generation,
        });
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.generation.advance();
        // Disconnect before joining: the worker exits its bounded cancelled
        // child read, and an idle worker exits recv immediately.
        let (replacement, _) = mpsc::channel();
        drop(std::mem::replace(&mut self.requests, replacement));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Work {
    Reload(Reload),
    Notebook {
        identity: Option<String>,
        revision: u64,
        generation: u64,
    },
}

#[derive(Clone)]
struct Reload {
    preview_panes: bool,
    squad: Option<String>,
    generation: u64,
}

struct Loaded {
    snapshot: Snapshot,
    attention: Option<AttentionJob>,
}

impl Loaded {
    fn only(snapshot: Snapshot) -> Self {
        Self {
            snapshot,
            attention: None,
        }
    }
}

/// The existing worker's lower-priority work, behind full reloads.
enum Deferred {
    Attention(Box<AttentionJob>),
    Usage(MeterRead),
    Notebook { identity: String, revision: u64 },
}

#[derive(Clone)]
enum MeterRead {
    Squad(super::rate::Input),
    Home,
}

/// Other tabs' attention follows the shown snapshot on the same worker.
/// A new switch cancels this lower-priority work through the same core reader.
struct AttentionJob {
    config: Config,
    squads: Vec<Squad>,
    shown: String,
    me: Option<crate::me::Me>,
    document: Value,
}

impl AttentionJob {
    fn complete(self, core: &Core) -> BTreeMap<String, Attention> {
        let others = self
            .squads
            .iter()
            .filter(|squad| squad.name != self.shown)
            .collect::<Vec<_>>();
        let mut documents = roster_documents(core, &self.config, &others, self.me.as_ref(), None);
        documents.include(&self.config, &self.shown, self.document);
        documents.attention(&self.config, &[])
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
    pending: &Receiver<Work>,
    mut publish: impl FnMut(Snapshot, u64) -> bool,
    check_every: Duration,
    generation: &AtomicU64,
    mut stamp: impl FnMut(u64) -> Stamp,
    mut load: impl FnMut(Option<String>, u64, bool, bool) -> Loaded,
    mut deferred: impl FnMut(Deferred, u64) -> bool,
) {
    // The squad last loaded, whether it reloads automatically, and the
    // stamp taken just before that load.
    let mut last: Option<(Reload, bool, Stamp)> = None;
    let mut sampling: Option<(MeterRead, Duration, Instant)> = None;
    loop {
        let wait = sampling.as_ref().map_or(check_every, |(_, _, next)| {
            check_every.min(next.saturating_duration_since(Instant::now()))
        });
        let received = match pending.recv_timeout(wait) {
            Ok(wanted) => wanted,
            Err(RecvTimeoutError::Timeout) => match &last {
                Some((reload, true, seen)) if seen.moved(&stamp(reload.generation)) => {
                    Work::Reload(reload.clone())
                }
                _ => {
                    if let (Some((reload, _, _)), Some((input, every, next))) =
                        (&last, &mut sampling)
                        && Instant::now() >= *next
                    {
                        if generation.load(Ordering::Acquire) == reload.generation
                            && !deferred(Deferred::Usage(input.clone()), reload.generation)
                        {
                            break;
                        }
                        *next = Instant::now() + *every;
                    }
                    continue;
                }
            },
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let mut wanted = None;
        let mut notebook = None;
        for work in std::iter::once(received).chain(pending.try_iter()) {
            match work {
                Work::Reload(reload) => wanted = Some(reload),
                Work::Notebook {
                    identity,
                    revision,
                    generation,
                } => notebook = Some((identity, revision, generation)),
            }
        }
        let Some(wanted) = wanted else {
            if let Some((Some(identity), revision, expected)) = notebook
                && generation.load(Ordering::Acquire) == expected
                && !deferred(Deferred::Notebook { identity, revision }, expected)
            {
                break;
            }
            continue;
        };
        // Reloads take priority; their snapshot schedules a fresh selected read.
        // Taken before the load, so a change during it shows at the next check.
        let seen = stamp(wanted.generation);
        if generation.load(Ordering::Acquire) != wanted.generation {
            continue;
        }
        let Loaded {
            snapshot,
            attention: job,
        } = load(
            wanted.squad.clone(),
            wanted.generation,
            wanted.preview_panes,
            last.as_ref()
                .is_none_or(|(previous, _, _)| previous.squad != wanted.squad),
        );
        if generation.load(Ordering::Acquire) != wanted.generation {
            continue;
        }
        // A squad that failed to load keeps the default interval.
        let automatic = snapshot
            .view
            .as_ref()
            .map_or(true, |view| view.refresh.is_some());
        last = Some((
            Reload {
                squad: snapshot.squad.clone(),
                preview_panes: wanted.preview_panes,
                generation: wanted.generation,
            },
            automatic,
            seen,
        ));
        sampling = snapshot
            .view
            .as_ref()
            .ok()
            .and_then(|view| view.token_rate.as_ref())
            .filter(|rate| rate.settings.enabled)
            .map(|rate| {
                (
                    MeterRead::Squad(rate.input.clone()),
                    rate.settings.every,
                    Instant::now() + rate.settings.every,
                )
            })
            .or_else(|| {
                if snapshot.squad.as_deref() != Some(ALL) {
                    return None;
                }
                snapshot
                    .view
                    .as_ref()
                    .ok()?
                    .home_rate
                    .values()
                    .filter(|rate| rate.settings.enabled)
                    .map(|rate| rate.settings.every)
                    .min()
                    .map(|every| (MeterRead::Home, every, Instant::now() + every))
            });
        if !publish(snapshot, wanted.generation) {
            break;
        }
        if let Some(job) = job
            && generation.load(Ordering::Acquire) == wanted.generation
            && !deferred(Deferred::Attention(Box::new(job)), wanted.generation)
        {
            break;
        }
    }
}

fn load(
    core: &Core,
    tmux: bool,
    caller: Option<&crate::me::Caller>,
    wanted: Option<String>,
    preview_panes: bool,
    opening: bool,
    kept: &mut Kept,
) -> Loaded {
    let squads = match Squad::list(core) {
        Ok(squads) => squads,
        Err(error) => {
            return Loaded::only(Snapshot {
                squad_keys: Vec::new(),
                tabs: Vec::new(),
                hidden: Vec::new(),
                pinned: 0,
                attention: BTreeMap::new(),
                squad: wanted,
                view: Err(error.to_string()),
            });
        }
    };
    let names: Vec<String> = squads.iter().map(|squad| squad.name.clone()).collect();
    let config = Config::load(core);
    // An invalid [tabs] still shows every squad; the view reports the error.
    let settings = config
        .as_ref()
        .ok()
        .and_then(|config| config.tabs().ok())
        .unwrap_or_default();
    let users = settings
        .user
        .iter()
        .map(|tab| tabs::user_key(&tab.name))
        .collect::<Vec<_>>();
    let (tabs, pinned) = if names.is_empty() && users.is_empty() {
        (Vec::new(), 0)
    } else {
        tabs::arrange(&names, &settings)
    };
    let hidden: Vec<String> = if names.is_empty() && users.is_empty() {
        Vec::new()
    } else {
        names
            .iter()
            .cloned()
            .chain([LEADS.to_owned(), ALL.to_owned()])
            .chain(users.iter().cloned())
            .filter(|key| !tabs.contains(key))
            .collect()
    };
    // A hidden squad is still shown when asked for by name.
    let chosen = match &wanted {
        Some(key)
            if (tabs::builtin(key) && !names.is_empty())
                || users.contains(key)
                || names.contains(key) =>
        {
            Some(key.clone())
        }
        Some(_) => None,
        None => tabs.first().cloned(),
    };
    let Some(key) = chosen else {
        return Loaded::only(Snapshot {
            squad_keys: names,
            tabs,
            hidden,
            pinned,
            attention: BTreeMap::new(),
            view: Err(match &wanted {
                Some(name) => format!("Squad '{name}' does not exist; run: tmt squad init {name}"),
                None => "No squad exists yet; run: tmt squad init <name>".into(),
            }),
            squad: wanted,
        });
    };
    let mut attention = BTreeMap::new();
    let mut deferred = None;
    let view = (|| {
        let config = config.as_ref().map_err(Clone::clone)?;
        let me = crate::me::you(crate::me::current(core, config)?, caller);
        let (mut view, found) = if key == LEADS {
            leads_view(core, tmux, config, &squads, &tabs, me)?
        } else if key == ALL {
            all_view(core, config, &squads, &tabs, me)?
        } else if tabs::user_name(&key).is_some() {
            member_view(core, tmux, config, &squads, &tabs, me, &key)?
        } else {
            let squad = squads
                .iter()
                .find(|squad| squad.name == key)
                .expect("chosen from the listed squads");
            let result = squad_view(core, tmux, config, squad, me.clone(), preview_panes, kept)?;
            deferred = Some((me, result.0.document.clone()));
            result
        };
        attention = found;
        if opening {
            let enabled: Vec<_> = view
                .token_rate
                .iter()
                .chain(view.home_rate.values())
                .filter(|rate| rate.settings.enabled)
                .collect();
            let longest = enabled
                .iter()
                .map(|rate| rate.settings.windows[2])
                .max_by_key(|window| window.milliseconds());
            if let Some(longest) = longest {
                let ids = enabled
                    .iter()
                    .flat_map(|rate| rate.input.resumes.keys().cloned())
                    .collect();
                let seeds = super::rate::history::load(core, ids, longest);
                for rate in view
                    .token_rate
                    .iter_mut()
                    .chain(view.home_rate.values_mut())
                {
                    if rate.settings.enabled {
                        rate.history = Some(
                            rate.input
                                .resumes
                                .keys()
                                .map(|id| {
                                    (id.clone(), seeds.get(id).and_then(Option::as_ref).cloned())
                                })
                                .collect(),
                        );
                    }
                }
            }
        }
        Ok(view)
    })()
    .map_err(|error: crate::core::SquadError| error.to_string());
    let job = config
        .ok()
        .zip(deferred)
        .map(|(config, (me, document))| AttentionJob {
            config,
            squads,
            shown: key.clone(),
            me,
            document,
        });
    Loaded {
        snapshot: Snapshot {
            squad_keys: names,
            tabs,
            hidden,
            pinned,
            attention,
            squad: Some(key),
            view,
        },
        attention: job,
    }
}

/// One squad's full view and its attention; other tabs follow publication.
fn squad_view(
    core: &Core,
    tmux: bool,
    config: &Config,
    squad: &Squad,
    me: Option<crate::me::Me>,
    preview_panes: bool,
    kept: &mut Kept,
) -> Result<(View, BTreeMap<String, Attention>), crate::core::SquadError> {
    let layout = config.layout(&squad.name)?;
    let (theme, theme_notice) = config.theme(&squad.name)?;
    let states = config.states(&squad.name, layout)?;
    let board = config.board(&squad.name)?;
    let sections = config.sections(&squad.name)?;
    let rows = config.rows(&squad.name)?;
    let providers = config.providers(&squad.name)?;
    let reminders = config.reminders(&squad.name)?;
    let shows_notes = preview_panes || board.panes.contains(&Pane::Notes);
    let observation = crate::observe::observe(
        core,
        config.path(),
        squad,
        reminders,
        &providers,
        crate::observe::Mode::Read(crate::observe::Reads {
            metadata: preview_panes || rows.reads_metadata(),
            notes: shows_notes,
        }),
    )?;
    // Providers run on the fetcher thread, never while the board draws.
    if !providers.is_empty() {
        let _ = kept.fetch.send(Fetch {
            squad: squad.name.clone(),
            providers: providers.clone(),
            members: observation.members.clone(),
        });
    }
    let settings = config.token_rate(&squad.name)?;
    let token_rate = settings.enabled.then(|| super::app::RateView {
        history: None,
        settings,
        input: super::rate::Input::observed(&squad.room_id, &observation.members),
    });
    let crate::observe::Projected {
        document,
        sent,
        notes,
    } = observation.document(
        core,
        squad,
        me.as_ref(),
        &providers,
        crate::observe::Shape {
            layout,
            states: &states,
            sections: &sections,
            rows: &rows,
        },
    )?;
    let attention = BTreeMap::from([(squad.name.clone(), Attention::of(&document))]);
    let mut replies = match &sent {
        Some(sent) if preview_panes || board.panes.contains(&Pane::Replies) => {
            requests::replies(sent, &document)
        }
        _ => Vec::new(),
    };
    requests::bodies(
        |id| requests::show_request(core, id),
        &mut replies,
        &mut kept.bodies,
    )?;
    let notes = if shows_notes {
        lead_notes(notes)
    } else {
        Notes::NotShown
    };
    let view = View {
        home: None,
        token_rate,
        home_rate: Default::default(),
        derived: Default::default(),
        rows,
        render: config.notes_render(&squad.name)?,
        bindings: config.bindings_for_tab(&squad.name, tmux, &board.panes)?,
        section_bindings: sections.into_iter().map(|section| section.bind).collect(),
        configured_bindings: config.configured_bindings()?,
        opener: config.program("opener")?,
        clipboard: config.program("clipboard")?,
        links: config.links()?,
        tab_colors: config.tabs()?.colors,
        look: crate::look::Look::new(theme),
        theme_notice,
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
    member_view(core, tmux, config, squads, tabs, me, LEADS)
}

fn member_view(
    core: &Core,
    tmux: bool,
    config: &Config,
    squads: &[Squad],
    tabs: &[String],
    me: Option<crate::me::Me>,
    key: &str,
) -> Result<(View, BTreeMap<String, Attention>), crate::core::SquadError> {
    let settings = config.tabs()?;
    // The cross-squad tabs have no squad table: the global theme alone.
    let (theme, theme_notice) = config.theme("")?;
    let loaded = tab_view::load(core, config, squads, tabs, me.as_ref(), key)?;
    let bindings = config.bindings_for_tab(key, tmux, &[])?;
    let tab = settings
        .user
        .iter()
        .find(|tab| Some(tab.name.as_str()) == tabs::user_name(key));
    let section_bindings = tab.map_or_else(Vec::new, |tab| {
        tab.sections
            .iter()
            .map(|section| section.bind.clone())
            .collect()
    });
    let view = View {
        token_rate: None,
        home_rate: Default::default(),
        home: None,
        derived: Default::default(),
        rows: loaded.rows,
        render: NotesRender::Markdown,
        bindings,
        section_bindings,
        configured_bindings: config.configured_bindings()?,
        opener: config.program("opener")?,
        clipboard: config.program("clipboard")?,
        links: config.links()?,
        tab_colors: settings.colors,
        look: crate::look::Look::new(theme),
        theme_notice,
        me: me.map(|me| me.name),
        replies: Vec::new(),
        refresh: config.refresh(key)?,
        board: Board::simple(
            BoardMode::Split,
            Direction::LeftRight,
            vec![Pane::Rows],
            &[100],
        ),
        notes: Notes::NotShown,
        document: loaded.document,
    };
    Ok((view, loaded.attention))
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
    // The cross-squad tabs have no squad table: the global theme alone.
    let (theme, theme_notice) = config.theme("")?;
    let (loaded, home, home_rate) = super::home::load(core, config, squads, tabs, me.as_ref())?;
    let bindings = config.bindings_for_tab(ALL, false, &[])?;
    let view = View {
        token_rate: None,
        home_rate,
        home: Some(home),
        derived: Default::default(),
        rows: loaded.rows,
        render: NotesRender::Markdown,
        bindings,
        section_bindings: Vec::new(),
        configured_bindings: Default::default(),
        opener: None,
        clipboard: None,
        links: Default::default(),
        tab_colors: settings.colors,
        look: crate::look::Look::new(theme),
        theme_notice,
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
        document: loaded.document,
    };
    Ok((view, loaded.attention))
}

fn member_notes(read: Result<Value, crate::core::SquadError>) -> Notes {
    match read {
        Err(error) if error.code == "NOTEBOOK_SAVED_IDENTITY_REQUIRED" => {
            Notes::Failed("(temporary identity: no notebook)".into())
        }
        other => lead_notes(Some(other)),
    }
}

/// The lead's notebook, from the `notes.read` the observation made: bounded,
/// read-only, and never creating a missing notebook.
fn lead_notes(read: Option<Result<Value, crate::core::SquadError>>) -> Notes {
    match read {
        None => Notes::NoLead,
        Some(Ok(note)) => Notes::Text(sanitize(note["content"].as_str().unwrap_or_default())),
        Some(Err(error)) if error.code == "NOTEBOOK_NOT_FOUND" => Notes::Missing,
        Some(Err(error)) => Notes::Failed(sanitize(&error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::squad::Member;
    use crate::tab_view::{all_document, leads_document, roster_document, tab_attention};
    use serde_json::json;

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
            colors: Default::default(),
            failed: Default::default(),
        }
    }

    /// Runs `serve` on its own thread with a stamp the test sets, and
    /// returns the requests channel, the loaded squads and the stamp.
    fn serving(
        automatic: bool,
    ) -> (
        Sender<Work>,
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
                |snapshot, _| sender.send(snapshot).is_ok(),
                Duration::from_millis(10),
                &AtomicU64::new(0),
                |_| Stamp::cursor(read.load(Ordering::SeqCst)),
                |wanted, _, _, _| {
                    let _ = loaded.send(wanted.clone());
                    let mut snapshot = crate::board::app::tests::snapshot(
                        wanted.as_deref().unwrap_or("first"),
                        json!([]),
                    );
                    if let Ok(view) = &mut snapshot.view {
                        view.refresh = automatic.then_some(Duration::from_secs(3600));
                    }
                    Loaded::only(snapshot)
                },
                |_, _| true,
            );
            drop(results);
        });
        (requests, loads, cursor)
    }

    const WAIT: Duration = Duration::from_millis(300);

    #[test]
    fn selected_notebook_reader_uses_public_api_and_shutdown_cancels_the_child() {
        let dir = std::env::temp_dir().join(format!("squad-selected-notes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("tmt");
        let calls = dir.join("calls");
        crate::test_support::write_ready_executable(
            &fake,
            &format!(
                r###"#!/bin/sh
if [ "$1" = api ]; then
  request=$(cat)
  case "$request" in
    *'"notes.read"'*) printf '%s' "$request" > '{}'; printf '%s\n' '{{"content":"## Now\nSafe"}}'; exit 0 ;;
  esac
fi
printf '%s\n' '{{}}'
"###,
                calls.display()
            ),
        );
        let (events, input) = mpsc::channel();
        let worker = Worker::spawn(Core::at(fake.clone()), false, events);
        worker.notebook(7, Some("selected-id".into()));
        let super::super::BoardEvent::Notebook {
            identity,
            revision,
            notes,
            cancellation,
        } = input.recv_timeout(Duration::from_secs(30)).unwrap()
        else {
            panic!("notebook event")
        };
        assert_eq!((identity.as_str(), revision), ("selected-id", 7));
        assert_eq!(notes, Notes::Text("## Now\nSafe".into()));
        assert!(!cancellation.cancelled());
        let request: Value = serde_json::from_slice(&std::fs::read(&calls).unwrap()).unwrap();
        assert_eq!(
            request,
            json!({"version":1,"operation":"notes.read","input":{"identityId":"selected-id"}})
        );
        drop(worker);
        assert!(cancellation.cancelled());
        let ready = dir.join("started");
        let slow = dir.join("slow");
        crate::test_support::write_ready_executable(
            &slow,
            &format!(
                r###"#!/bin/sh
if [ "$1" = api ]; then
  request=$(cat)
  case "$request" in *'"notes.read"'*) echo $$ > '{}'; exec sleep 30 ;; esac
fi
printf '%s\n' '{{}}'
"###,
                ready.display()
            ),
        );
        let (events, _) = mpsc::channel();
        let worker = Worker::spawn(Core::at(slow), false, events);
        worker.notebook(8, Some("selected-id".into()));
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "reader did not start");
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid = std::fs::read_to_string(&ready).unwrap();
        let started = Instant::now();
        drop(worker);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            !std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn notebooks_coalesce_behind_reload_and_obey_the_existing_generation() {
        let (sender, pending) = mpsc::channel();
        let generation = AtomicU64::new(1);
        let queue = |identity: Option<&str>, revision, expected| {
            sender
                .send(Work::Notebook {
                    identity: identity.map(str::to_owned),
                    revision,
                    generation: expected,
                })
                .unwrap()
        };
        queue(Some("old"), 1, 1);
        sender
            .send(Work::Reload(Reload {
                squad: Some("product".into()),
                generation: 1,
                preview_panes: false,
            }))
            .unwrap();
        let steps = std::cell::RefCell::new(Vec::new());
        serve(
            &pending,
            |_, _| {
                steps.borrow_mut().push("snapshot".to_owned());
                queue(Some("stale"), 2, 0);
                queue(Some("old"), 3, 1);
                queue(Some("selected"), 4, 1);
                true
            },
            CHECK_EVERY,
            &generation,
            |_| Stamp::cursor(0),
            |_, _, _, _| Loaded::only(crate::board::app::tests::snapshot("product", json!([]))),
            |job, expected| {
                let Deferred::Notebook { identity, revision } = job else {
                    panic!("other work")
                };
                assert_eq!((identity.as_str(), revision, expected), ("selected", 4, 1));
                steps.borrow_mut().push(identity);
                false
            },
        );
        assert_eq!(*steps.borrow(), ["snapshot", "selected"]);
        // No selection and obsolete generation perform no deferred read.
        let (sender, pending) = mpsc::channel();
        sender
            .send(Work::Notebook {
                identity: Some("stale".into()),
                revision: 5,
                generation: 0,
            })
            .unwrap();
        drop(sender);
        serve(
            &pending,
            |_, _| panic!("no snapshot"),
            CHECK_EVERY,
            &generation,
            |_| Stamp::cursor(0),
            |_, _, _, _| panic!("no load"),
            |_, _| panic!("no read"),
        );
        for (code, expected) in [
            ("NOTEBOOK_NOT_FOUND", "(no notes yet)"),
            (
                "NOTEBOOK_SAVED_IDENTITY_REQUIRED",
                "(temporary identity: no notebook)",
            ),
        ] {
            let note = member_notes(Err(crate::core::SquadError::new(code, "ignored")));
            let lines = super::super::view::notebook_lines(
                &note,
                80,
                crate::look::Look::default(),
                NotesRender::Markdown,
            );
            assert_eq!(lines[0].to_string(), expected);
        }
        assert_eq!(
            member_notes(Ok(json!({"content":"## Now\n\u{1b}[31mSafe"}))),
            Notes::Text("## Now\nSafe".into())
        );
    }

    #[test]
    fn built_in_board_documents_equal_ls_tab_documents() {
        let root = std::env::temp_dir().join(format!("squad-tab-parity-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("tmt");
        crate::test_support::write_ready_executable(
            &executable,
            r##"#!/bin/sh
case "$1" in
room) printf '%s\n' '{"rooms":[{"id":"P","name":"squad-product"},{"id":"Q","name":"squad-quiet"}]}' ;;
whoami) printf '%s\n' '{"bound":false}' ;;
ls) printf '%s\n' '{"identities":[{"id":"S","name":"sol","presence":"offline"}]}' ;;
api)
 input=$(cat)
 case "$input" in
 *'"room":"P"'*) printf '%s\n' '{"members":[{"id":"S","name":"sol","lifetime":"saved","metadata":{"squad.product.role":"lead","squad.product.state":"blocked","squad.product.pending":"approve"}},{"id":"W","name":"worker","lifetime":"saved","metadata":{"squad.product.task":"ship"}}]}' ;;
 *'"room":"Q"'*) printf '%s\n' '{"members":[]}' ;;
 *) exit 2 ;;
 esac ;;
*) exit 2 ;;
esac
"##,
        );
        let core = Core::at(executable);
        let path = root.join("squad.toml");
        std::fs::write(
            &path,
            "[tabs]\norder = ['all', 'product', 'leads']\nhide = ['quiet']\n",
        )
        .unwrap();
        let mut config = Config::read(path).unwrap();
        let squads = Squad::list(&core).unwrap();
        let names = squads
            .iter()
            .map(|squad| squad.name.clone())
            .collect::<Vec<_>>();
        let (tabs, _) = tabs::arrange(&names, &config.tabs().unwrap());
        for (name, key) in [("leads", LEADS), ("all", ALL)] {
            let view = if key == LEADS {
                leads_view(&core, false, &config, &squads, &tabs, None)
                    .unwrap()
                    .0
            } else {
                all_view(&core, &config, &squads, &tabs, None).unwrap().0
            };
            let mut listed = crate::ls_tab_document(&core, &mut config, name)
                .unwrap()
                .document;
            listed.as_object_mut().unwrap().remove("you");
            assert_eq!(
                view.document, listed,
                "board and ls share complete projected documents"
            );
            assert_eq!(view.home.is_some(), key == ALL);
            if let Some(home) = &view.home {
                assert_eq!(home.squads.len(), 2);
                assert_eq!(home.summary.members, 2);
                assert_eq!(home.summary.waiting, 1);
            }
            assert_eq!(view.rows.value()["columns"], listed["columns"]);
            assert_eq!(view.rows.value()["lines"], listed["lines"]);
            assert_eq!(listed["sections"][0]["rows"][0]["squad"], "product");
            assert!(
                crate::status::text(&listed, tmt_cli_style::Terminal::PLAIN).contains("product")
            );
            if key == LEADS {
                assert_eq!(listed["sections"][0]["rows"].as_array().unwrap().len(), 1);
                assert_eq!(listed["sections"][0]["rows"][0]["pending"], "approve");
            } else {
                assert_eq!(
                    listed["sections"][0]["rows"].as_array().unwrap().len(),
                    2,
                    "hidden squads stay in all"
                );
                assert_eq!(
                    listed["sections"][0]["rows"][1]["fields"]["lead"],
                    Value::Null
                );
            }
        }
        assert_eq!(
            crate::ls_tab_document(&core, &mut config, "missing")
                .err()
                .unwrap()
                .code,
            "SQUAD_TAB_NOT_FOUND"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn user_board_and_ls_share_members_sections_bindings_and_failed_reads() {
        let root = std::env::temp_dir().join(format!("squad-user-tab-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("tmt");
        crate::test_support::write_ready_executable(
            &executable,
            r##"#!/bin/sh
root=${0%/*}
case "$1" in
room) printf '%s\n' '{"rooms":[{"id":"P","name":"squad-product"},{"id":"Q","name":"squad-quiet"}]}' ;;
whoami) printf '%s\n' '{"bound":false}' ;;
identity) printf '%s\n' '{"identity":{"id":"7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f","name":"ben"}}' ;;
ls) printf '%s\n' '{"identities":[]}' ;;
inbox)
 if test -f "$root/fail-inbox"; then printf '%s\n' '{"error":{"code":"READ_FAILED","message":"inbox unavailable"}}'; exit 1; fi
 printf '%s\n' '{"items":[{"requestId":"R","from":{"identityId":"S"},"preview":"approve","preparedAtMs":1}],"more":false}' ;;
api)
 input=$(cat)
 case "$input" in
 *'"room":"P"'*) printf '%s\n' '{"members":[{"id":"S","name":"sol","lifetime":"saved","metadata":{"squad.product.role":"lead","squad.product.state":"blocked","squad.product.task":"ship","usage.count":"1200"}},{"id":"L","name":"legacy","lifetime":"saved","metadata":{"squad.product.role":"lead","squad.product.pending":"review","usage.count":"950"}},{"id":"W","name":"worker","lifetime":"saved","metadata":{"squad.product.task":"hidden"}}]}' ;;
 *'"room":"Q"'*)
 if test -f "$root/fail-squad"; then printf '%s\n' '{"error":{"code":"READ_FAILED","message":"roster unavailable"}}'; exit 1; fi
 printf '%s\n' '{"members":[{"id":"S","name":"sol","lifetime":"saved","metadata":{"squad.quiet.pending":"quiet approval","squad.quiet.state":"blocked"}}]}' ;;
 *) exit 2 ;;
 esac ;;
*) exit 2 ;;
esac
"##,
        );
        let core = Core::at(executable);
        let path = root.join("squad.toml");
        std::fs::write(
            &path,
            r#"me = "ben"
me_id = "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f"
[tabs]
hide = ["quiet", "tab:needs-me"]
[bind]
enter = "tab"
[tabs.needs-me]
filter = "pending or waiting_on_you"
sort = ["ctx", "squad", "name"]
[tabs.needs-me.bind]
enter = "jump"
[[tabs.needs-me.section]]
title = "Blocked"
filter = "state = blocked"
[tabs.needs-me.section.bind]
o = "tab"
[[tabs.needs-me.section]]
title = "All decisions"
filter = "pending or waiting_on_you"
[[squad.product.section]]
title = "Repeated"
filter = "task"
[[squad.quiet.section]]
title = "First copy"
filter = "pending"
[[squad.quiet.section]]
title = "Second copy"
filter = "pending"
[squad.product.rows]
columns = [{ name = "member" }, { name = "ctx", from = "meta.usage.count", format = "tokens" }]
"#,
        )
        .unwrap();
        let mut config = Config::read(path).unwrap();
        let squads = Squad::list(&core).unwrap();
        let (tabs, _) = tabs::arrange(&["product".into(), "quiet".into()], &config.tabs().unwrap());
        let key = tabs::user_key("needs-me");
        for partial in [false, true, false] {
            for marker in ["fail-inbox", "fail-squad"] {
                if partial {
                    std::fs::write(root.join(marker), "").unwrap();
                } else {
                    let _ = std::fs::remove_file(root.join(marker));
                }
            }
            let view = member_view(
                &core,
                false,
                &config,
                &squads,
                &tabs,
                Some(crate::me::Me {
                    id: "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f".into(),
                    name: "ben".into(),
                }),
                &key,
            )
            .unwrap()
            .0;
            let mut listed = crate::ls_tab_document(&core, &mut config, "needs-me")
                .unwrap()
                .document;
            listed.as_object_mut().unwrap().remove("you");
            assert_eq!(view.document, listed);
            let mut expected = crate::rows::Rows::leads().value();
            expected["columns"][1]["title"] = json!("MEMBER");
            assert_eq!(view.rows.value(), expected);
            assert_eq!(view.bindings["enter"].verb, crate::action::Verb::Jump);
            assert_eq!(view.section_bindings[0]["o"].verb, crate::action::Verb::Tab);
            if partial {
                assert_eq!(listed["partial"], true);
                assert_eq!(listed["failures"].as_array().unwrap().len(), 2);
                assert_eq!(listed["sections"][1]["rows"][0]["name"], "legacy");
                let text = crate::status::text(&listed, tmt_cli_style::Terminal::PLAIN);
                assert!(text.contains("inbox unavailable") && text.contains("roster unavailable"));
            } else {
                assert!(listed.get("partial").is_none());
                assert_eq!(listed["sections"][0]["rows"].as_array().unwrap().len(), 2);
                let rows = listed["sections"][1]["rows"].as_array().unwrap();
                assert_eq!(
                    rows.iter()
                        .map(|row| (
                            row["squad"].as_str().unwrap(),
                            row["name"].as_str().unwrap()
                        ))
                        .collect::<Vec<_>>(),
                    [("product", "legacy"), ("product", "sol"), ("quiet", "sol")]
                );
                assert_eq!(rows[1]["waitingOnYou"][0]["requestId"], "R");
                assert_eq!(rows[0]["fields"]["ctx"], "950");
                assert_eq!(rows[1]["fields"]["ctx"], "1k");
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saved_views_acquire_notes_from_effective_board_without_changing_workflow() {
        let root = std::env::temp_dir().join(format!("squad-view-notes-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("tmt");
        crate::test_support::write_ready_executable(
            &executable,
            r##"#!/bin/sh
root=${0%/*}
case "$1" in
  api)
    input=$(cat)
    case "$input" in
      *'"rooms.roster"'*) printf '%s\n' '{"members":[{"id":"SOL","name":"sol","lifetime":"saved","metadata":{"squad.product.role":"lead","squad.product.task":"unchanged task"}}]}' ;;
      *'"notes.read"'*) printf '%s\n' notes >> "$root/calls"; printf '%s\n' '{"identityId":"SOL","content":"# Lead notebook\nFinal notebook sentinel"}' ;;
      *) exit 2 ;;
    esac ;;
  ls) printf '%s\n' '{"identities":[]}' ;;
  *) exit 2 ;;
esac
"##,
        );
        let core = Core::at(executable);
        let squad = Squad {
            name: "product".into(),
            room_id: "33333333-3333-4333-8333-333333333333".into(),
        };
        let (fetch, _pending) = mpsc::channel();
        let mut kept = Kept {
            bodies: BTreeMap::new(),
            fetch,
        };
        for workflow in ["minimal", "crew"] {
            let path = root.join("squad.toml");
            std::fs::write(&path, format!("[squad.product]\nlayout = '{workflow}'\n")).unwrap();
            let mut config = Config::read(path).unwrap();
            let baseline = squad_view(&core, false, &config, &squad, None, false, &mut kept)
                .unwrap()
                .0;
            if workflow == "minimal" {
                assert_eq!(baseline.notes, Notes::NotShown);
                assert!(!root.join("calls").exists());
                let preview = squad_view(&core, false, &config, &squad, None, true, &mut kept)
                    .unwrap()
                    .0;
                assert_eq!(preview.document, baseline.document);
                assert_eq!(
                    preview.board, baseline.board,
                    "preview reads never resolve another board"
                );
                assert!(
                    matches!(preview.notes, Notes::Text(_)),
                    "preview acquires a missing notes pane"
                );
            }
            config
                .set_view(
                    &crate::view::ViewScope::Squad("product".into()),
                    crate::view::ViewName::Notes,
                )
                .unwrap();
            let view = squad_view(&core, false, &config, &squad, None, false, &mut kept)
                .unwrap()
                .0;
            assert_eq!(
                view.document, baseline.document,
                "workflow projection stays identical"
            );
            assert_eq!(
                view.notes,
                Notes::Text("# Lead notebook\nFinal notebook sentinel".into())
            );
            assert!(view.board.panes.contains(&Pane::Notes));
            assert_eq!(config.layout("product").unwrap().as_str(), workflow);
        }
        assert_eq!(
            std::fs::read_to_string(root.join("calls"))
                .unwrap()
                .lines()
                .count(),
            4,
            "one read for each effective notes pane or explicit preview request"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn advancing_generation_stops_all_old_readers_and_never_resets_them() {
        let generation = Generation::default();
        let old = generation.cancellation(0);
        assert!(!old.cancelled());
        assert_eq!(generation.advance(), 1);
        assert!(old.cancelled());
        assert!(generation.cancellation(0).cancelled());
        let current = generation.cancellation(1);
        assert!(!current.cancelled());
        assert_eq!(generation.advance(), 2);
        assert!(old.cancelled() && current.cancelled());
        assert!(!generation.cancellation(2).cancelled());
    }

    /// A moved stamp reloads the last squad at once, well before its
    /// hour-long interval; an unmoved one never does.
    #[test]
    fn the_worker_reloads_early_only_when_the_stamp_moves() {
        use std::sync::atomic::Ordering;
        let (requests, loads, cursor) = serving(true);
        requests
            .send(Work::Reload(Reload {
                preview_panes: false,
                squad: Some("product".into()),
                generation: 0,
            }))
            .unwrap();
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("product".into())));
        assert!(loads.recv_timeout(WAIT).is_err(), "nothing changed");
        cursor.store(2, Ordering::SeqCst);
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("product".into())));
        assert!(loads.recv_timeout(WAIT).is_err(), "once per change");
        // A request is served as before and becomes the squad to watch.
        requests
            .send(Work::Reload(Reload {
                preview_panes: false,
                squad: Some("infra".into()),
                generation: 0,
            }))
            .unwrap();
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("infra".into())));
        cursor.store(3, Ordering::SeqCst);
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("infra".into())));
    }

    /// `refresh = "off"` means ctrl-r and actions only: no early reload either.
    #[test]
    fn a_view_with_automatic_reload_off_is_never_reloaded_early() {
        use std::sync::atomic::Ordering;
        let (requests, loads, cursor) = serving(false);
        requests
            .send(Work::Reload(Reload {
                preview_panes: false,
                squad: Some("product".into()),
                generation: 0,
            }))
            .unwrap();
        assert_eq!(loads.recv_timeout(WAIT), Ok(Some("product".into())));
        cursor.store(2, Ordering::SeqCst);
        assert!(loads.recv_timeout(WAIT).is_err());
        requests
            .send(Work::Reload(Reload {
                preview_panes: false,
                squad: Some("product".into()),
                generation: 0,
            }))
            .unwrap();
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
                    json!({"id": "R", "name": "rin", "state": "blocked", "fields": {}, "colors": {"state": "red"}}),
                )),
            ),
            ("quiet".to_owned(), squad(None)),
            (
                "product".to_owned(),
                squad(Some(
                    json!({"id": "S", "name": "sol", "pending": "approve", "fields": {}, "colors": {"state": "review"}}),
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
        assert_eq!(rows[0]["colors"]["state"], "review");
        assert_eq!(rows[1]["colors"]["state"], "red");
        assert!(rows[2].get("colors").is_none());

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
    #[test]
    fn a_superseded_load_is_never_published_and_queued_switches_collapse() {
        let (sender, pending) = mpsc::channel();
        let mut sender = Some(sender);
        sender
            .as_ref()
            .unwrap()
            .send(Work::Reload(Reload {
                preview_panes: false,
                squad: Some("old".into()),
                generation: 0,
            }))
            .unwrap();
        let generation = AtomicU64::new(0);
        let mut loaded = Vec::new();
        let mut published = Vec::new();
        serve(
            &pending,
            |snapshot, _| {
                published.push(snapshot.squad);
                true
            },
            Duration::from_secs(1),
            &generation,
            |_| Stamp::cursor(1),
            |squad, _, _, _| {
                loaded.push(squad.clone());
                if squad.as_deref() == Some("old") {
                    generation.store(1, Ordering::Release);
                    for name in ["middle", "new"] {
                        sender
                            .as_ref()
                            .unwrap()
                            .send(Work::Reload(Reload {
                                preview_panes: false,
                                squad: Some(name.into()),
                                generation: 1,
                            }))
                            .unwrap();
                    }
                } else {
                    drop(sender.take());
                }
                Loaded::only(crate::board::app::tests::snapshot(
                    squad.as_deref().unwrap(),
                    json!([]),
                ))
            },
            |_, _| panic!("no attention job"),
        );
        assert_eq!(loaded, [Some("old".into()), Some("new".into())]);
        assert_eq!(published, [Some("new".into())]);
    }

    #[test]
    fn the_shown_snapshot_is_published_before_other_tab_attention() {
        let (sender, pending) = mpsc::channel();
        sender
            .send(Work::Reload(Reload {
                preview_panes: false,
                squad: Some("product".into()),
                generation: 0,
            }))
            .unwrap();
        drop(sender);
        let steps = std::cell::RefCell::new(Vec::new());
        let config = Config::read(
            std::env::temp_dir().join(format!("squad-attention-order-{}.toml", std::process::id())),
        )
        .unwrap();
        let mut config = Some(config);
        serve(
            &pending,
            |_, _| {
                steps.borrow_mut().push("shown");
                true
            },
            Duration::from_secs(1),
            &AtomicU64::new(0),
            |_| Stamp::cursor(1),
            |_, _, _, _| Loaded {
                snapshot: crate::board::app::tests::snapshot("product", json!([])),
                attention: Some(AttentionJob {
                    config: config.take().unwrap(),
                    squads: Vec::new(),
                    shown: "product".into(),
                    me: None,
                    document: json!({}),
                }),
            },
            |_, _| {
                steps.borrow_mut().push("attention");
                true
            },
        );
        assert_eq!(*steps.borrow(), ["shown", "attention"]);
    }
    #[test]
    fn squad_leads_and_all_default_to_ctrl_r_refresh_and_keep_their_override_owners() {
        let path = std::env::temp_dir().join(format!("squad-ctrl-r-{}.toml", std::process::id()));
        let executable = path.with_extension("tmt");
        crate::test_support::write_ready_executable(
            &executable,
            "#!/bin/sh\n[ \"$1\" = ls ] || exit 2\nprintf \"%s\\n\" \'{\"identities\":[]}\'\n",
        );
        let core = Core::at(executable.clone());
        for (body, squad, leads, all) in [
            (
                "",
                crate::action::Verb::Refresh,
                crate::action::Verb::Refresh,
                crate::action::Verb::Refresh,
            ),
            (
                "[bind]\nctrl-r = \"copy\"\n[tabs.leads.bind]\nctrl-r = \"notes\"\n[tabs.all.bind]\nctrl-r = \"notes\"\n",
                crate::action::Verb::Copy,
                crate::action::Verb::Notes,
                crate::action::Verb::Notes,
            ),
        ] {
            std::fs::write(&path, body).unwrap();
            let config = Config::read(path.clone()).unwrap();
            for tmux in [false, true] {
                let squad_bindings = config.bindings(tmux, &[]).unwrap();
                assert_eq!(squad_bindings["ctrl-r"].verb, squad);
                assert!(!squad_bindings.contains_key("f5"));
                let leads_bindings = leads_view(&core, tmux, &config, &[], &[], None)
                    .unwrap()
                    .0
                    .bindings;
                assert_eq!(leads_bindings["ctrl-r"].verb, leads);
                assert!(!leads_bindings.contains_key("f5"));
                let all_bindings = all_view(&core, &config, &[], &[], None).unwrap().0.bindings;
                assert_eq!(all_bindings["ctrl-r"].verb, all);
                assert!(!all_bindings.contains_key("f5"));
            }
        }
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(executable).unwrap();
    }
    #[test]
    fn home_sampling_requires_the_open_home_tab_and_an_enabled_policy() {
        let (sender, pending) = mpsc::channel();
        let reload = |squad: &str| {
            Work::Reload(Reload {
                preview_panes: false,
                squad: Some(squad.into()),
                generation: 0,
            })
        };
        sender.send(reload(ALL)).unwrap();
        let published = std::cell::Cell::new(0);
        let mut samples = 0;
        serve(
            &pending,
            |_, _| {
                let count = published.get() + 1;
                published.set(count);
                match count {
                    1 => sender.send(reload("product")).unwrap(),
                    2 => sender.send(reload(ALL)).unwrap(),
                    _ => {}
                }
                true
            },
            Duration::from_millis(1),
            &AtomicU64::new(0),
            |_| Stamp::cursor(1),
            |wanted, _, _, _| {
                let mut snapshot =
                    crate::board::app::tests::snapshot(wanted.as_deref().unwrap(), json!([]));
                let view = snapshot.view.as_mut().unwrap();
                view.refresh = None;
                // Even a retained HOME template must not schedule HOME reads on another tab.
                view.home_rate.insert(
                    "product".into(),
                    super::super::app::RateView {
                        history: None,
                        settings: crate::config::TokenRate {
                            enabled: published.get() > 0,
                            every: Duration::from_millis(2),
                            ..Default::default()
                        },
                        input: super::super::rate::tests::input(100),
                    },
                );
                Loaded::only(snapshot)
            },
            |job, _| {
                assert_eq!(published.get(), 3);
                assert!(matches!(job, Deferred::Usage(MeterRead::Home)));
                samples += 1;
                false
            },
        );
        assert_eq!(samples, 1);
    }

    #[test]
    fn meter_only_sampling_uses_the_selected_roster_after_queued_full_loads() {
        let (sender, pending) = mpsc::channel();
        for name in ["old", "new"] {
            sender
                .send(Work::Reload(Reload {
                    preview_panes: false,
                    squad: Some(name.into()),
                    generation: 0,
                }))
                .unwrap();
        }
        let mut loads = Vec::new();
        let mut samples = 0;
        serve(
            &pending,
            |_, _| true,
            Duration::from_millis(1),
            &AtomicU64::new(0),
            |_| Stamp::cursor(1),
            |wanted, _, _, _| {
                loads.push(wanted.clone());
                let mut snapshot =
                    crate::board::app::tests::snapshot(wanted.as_deref().unwrap(), json!([]));
                let view = snapshot.view.as_mut().unwrap();
                view.refresh = None;
                view.token_rate = Some(super::super::app::RateView {
                    history: None,
                    settings: crate::config::TokenRate {
                        enabled: true,
                        every: Duration::from_millis(2),
                        reduced_motion: true,
                        window: crate::config::TokenWindow::MINUTE,
                        windows: crate::config::TokenWindow::DEFAULTS,
                    },
                    input: super::super::rate::tests::input(100),
                });
                Loaded::only(snapshot)
            },
            |job, generation| {
                let Deferred::Usage(MeterRead::Squad(input)) = job else {
                    panic!("unexpected attention work")
                };
                assert_eq!(generation, 0);
                assert_eq!(input.resumes.len(), 1);
                samples += 1;
                false
            },
        );
        assert_eq!(loads, [Some("new".into())]);
        assert_eq!(samples, 1);
    }
    #[test]
    fn opening_history_is_requested_once_per_entry_and_not_on_ordinary_reload() {
        let (sender, pending) = mpsc::channel();
        let reload = |name: &str| {
            Work::Reload(Reload {
                squad: Some(name.into()),
                generation: 0,
                preview_panes: false,
            })
        };
        let mut names = ["product", "product", "infra", "product"].into_iter();
        sender.send(reload(names.next().unwrap())).unwrap();
        let mut openings = Vec::new();
        serve(
            &pending,
            |_, _| {
                if let Some(next) = names.next() {
                    sender.send(reload(next)).unwrap();
                    true
                } else {
                    false
                }
            },
            CHECK_EVERY,
            &AtomicU64::new(0),
            |_| Stamp::cursor(0),
            |wanted, _, _, opening| {
                openings.push(opening);
                let mut snapshot =
                    crate::board::app::tests::snapshot(wanted.as_deref().unwrap(), json!([]));
                snapshot.view.as_mut().unwrap().refresh = None;
                Loaded::only(snapshot)
            },
            |_, _| panic!("no deferred work"),
        );
        assert_eq!(openings, [true, false, true, true]);
    }
}
