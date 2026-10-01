//! Board state and key handling, independent of the terminal. Every keypress
//! works on what is already loaded; loading happens in the refresh worker,
//! and actions leave as fully resolved requests.

use super::scroll::{Scrolls, Step, WHEEL_LINES};
use crate::{
    action::{Action, Bindings, Verb},
    attention::Attention,
    config::{Board, NotesRender, Pane},
    effects,
};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// What the refresh worker loaded for one squad.
pub struct View {
    /// The `status --json` document, so the board and `status` never differ.
    pub document: Value,
    pub rows: crate::rows::Rows,
    pub colors: BTreeMap<String, String>,
    pub board: Board,
    /// The full-reload interval; `None` reloads only on F5 and actions.
    pub refresh: Option<std::time::Duration>,
    pub notes: Notes,
    pub render: NotesRender,
    /// The host preset overridden by `[bind]`.
    pub bindings: Bindings,
    /// Each document section's own bindings, in document order.
    pub section_bindings: Vec<Bindings>,
    pub opener: Option<Vec<String>>,
    pub clipboard: Option<Vec<String>>,
    /// `[tabs.colors]`, re-read with every load like the rest of squad.toml.
    pub tab_colors: crate::config::TabColors,
    /// The user's saved identity, the sender of talk, reply and annotate.
    pub me: Option<String>,
    /// Finals to the user's squad requests, newest first (replies pane).
    pub replies: Vec<Value>,
    /// The squad's theme at the terminal's depth: every color the board draws.
    pub look: crate::look::Look,
    /// Why the board uses the default theme, when the global one is wrong.
    pub theme_notice: Option<String>,
}

/// The lead's notebook, already sanitized for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notes {
    /// The notes pane is not shown, so nothing was read.
    NotShown,
    NoLead,
    /// The lead has no notebook yet; the board never creates one.
    Missing,
    Text(String),
    Failed(String),
}

pub struct Snapshot {
    /// Tab keys in order: squad names and built-in tabs (`board::tabs`).
    pub tabs: Vec<String>,
    /// Hidden tabs, still reachable through the switcher.
    pub hidden: Vec<String>,
    /// How many tabs at the front are pinned.
    pub pinned: usize,
    /// Every listed squad's attention, for its tab's color and counts.
    pub attention: BTreeMap<String, Attention>,
    pub squad: Option<String>,
    pub view: Result<View, String>,
}

/// An action with every value already taken from the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Show this member's pane in the invoking client.
    Jump(String),
    /// Return the invoker's client to where its last jump came from.
    Back,
    Open {
        link: String,
        opener: Option<Vec<String>>,
    },
    Copy {
        text: String,
        program: Option<Vec<String>>,
    },
    Run(Vec<String>),
    Talk {
        me: String,
        squad: String,
        to: String,
        text: String,
    },
    Annotate {
        me: String,
        squad: String,
        to: String,
        row: String,
        text: String,
    },
    Reply {
        me: String,
        request: String,
        from: String,
        text: String,
    },
    /// Save this tab order to `[tabs] order` (tab keys, in order).
    Reorder(Vec<String>),
}

impl Request {
    /// Sending changes request state, and a saved order changes the tabs, so
    /// the view reloads afterwards.
    pub fn sends(&self) -> bool {
        matches!(
            self,
            Self::Talk { .. } | Self::Annotate { .. } | Self::Reply { .. } | Self::Reorder(_)
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    /// Load this squad now (a squad switch).
    Load(String),
    Refresh,
    Act(Request),
}

pub enum Item<'a> {
    Header(&'a str),
    Row(&'a Value),
}

/// What a menu entry does: run a binding, or reply to one open request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Action(Action),
    Reply { request: String, from: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    /// The key that chooses it directly.
    pub key: String,
    pub label: String,
    pub choice: Choice,
}

/// The row's action menu, or the choice among a member's open requests.
pub struct Menu {
    pub title: String,
    pub entries: Vec<MenuEntry>,
    pub selected: usize,
}

/// Where composed text goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compose {
    Talk { to: String },
    Annotate { to: String, row: String },
    Reply { request: String, from: String },
}

/// The one-line composer: Enter sends, Esc cancels, empty sends nothing.
pub struct Input {
    pub prompt: String,
    pub text: String,
    pub compose: Compose,
    /// The squad the text is sent in: the row's own on the leads tab.
    pub squad: String,
}

/// Longest text the composer accepts, in characters.
const INPUT_LIMIT: usize = 4000;

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The quick switcher: a filter over every tab, hidden ones included.
#[derive(Debug, Default)]
pub struct Switcher {
    pub query: String,
    pub selected: usize,
}

/// Where one tab was drawn on the tab line, for clicks and drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabHit {
    pub y: u16,
    pub x: u16,
    pub width: u16,
    pub tab: usize,
}

/// A screen line of the rows pane and the visible row it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub y: u16,
    pub x: u16,
    pub width: u16,
    pub row: usize,
}

#[derive(Default)]
pub struct App {
    pub tabs: Vec<String>,
    pub hidden: Vec<String>,
    pub pinned: usize,
    /// The quick switcher (`s`), while open.
    pub switcher: Option<Switcher>,
    pub attention: BTreeMap<String, Attention>,
    pub current: Option<String>,
    pub view: Option<View>,
    pub error: Option<String>,
    pub search: String,
    pub searching: bool,
    /// Index among visible rows (headers excluded).
    pub selected: usize,
    pub notice: Option<String>,
    pub help: bool,
    pub menu: Option<Menu>,
    pub input: Option<Input>,
    /// Index of the focused pane (split) or visible tab (tabs).
    pub focus: usize,
    /// Every pane's scroll position, from one owner.
    pub scrolls: Scrolls,
    /// The rows pane keeps the selection on screen until the wheel moves it.
    pub follow: bool,
    /// Opened as a tmux popup: a successful jump closes the board.
    pub popup: bool,
    /// Views of squads already visited, so switching back is instant while
    /// the worker refreshes them.
    cache: BTreeMap<String, View>,
    /// Set from a squad switch until that squad's data arrives.
    pub loading_since: Option<Instant>,
    /// The squad `view` belongs to.
    shown: Option<String>,
    last_click: Option<(usize, Instant)>,
    /// Where rows were last drawn, for mouse events.
    pub hits: RefCell<Vec<Hit>>,
    /// Where tabs were last drawn.
    pub tab_hits: RefCell<Vec<TabHit>>,
    /// The first tab the tab line showed, so it scrolls only as needed.
    pub tab_start: std::cell::Cell<usize>,
    /// The tab a left button went down on, until it is released.
    dragging: Option<usize>,
}

fn page_step(code: KeyCode) -> Step {
    match code {
        KeyCode::PageUp => Step::Pages(-1),
        KeyCode::PageDown => Step::Pages(1),
        KeyCode::Home => Step::Top,
        _ => Step::Bottom,
    }
}

fn matches(row: &Value, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle = needle.to_lowercase();
    let contains = |value: &Value| {
        value
            .as_str()
            .is_some_and(|text| text.to_lowercase().contains(&needle))
    };
    contains(&row["name"])
        || row["fields"]
            .as_object()
            .is_some_and(|fields| fields.values().any(contains))
}

/// The binding name of a key, as `[bind]` spells it.
pub fn event_name(key: KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    Some(match key.code {
        KeyCode::Char(character) if ctrl => format!("ctrl-{}", character.to_ascii_lowercase()),
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(character) if character.is_ascii_graphic() => character.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::F(number) => format!("f{number}"),
        _ => return None,
    })
}

impl App {
    pub fn new(squad: Option<String>) -> Self {
        Self {
            current: squad,
            follow: true,
            ..Self::default()
        }
    }

    /// Section index and row for every row matching the search.
    fn rows(&self) -> Vec<(usize, &Value)> {
        let Some(view) = &self.view else {
            return Vec::new();
        };
        view.document["sections"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .flat_map(|(index, section)| {
                section["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(move |row| (index, row))
            })
            .filter(|(_, row)| matches(row, &self.search))
            .collect()
    }

    /// Section headers and the rows matching the search, in board order. The
    /// single default section has no header.
    pub fn items(&self) -> Vec<Item<'_>> {
        let mut items = Vec::new();
        let Some(view) = &self.view else {
            return items;
        };
        for section in view.document["sections"].as_array().into_iter().flatten() {
            if let Some(title) = section["title"].as_str() {
                items.push(Item::Header(title));
            }
            for row in section["rows"].as_array().into_iter().flatten() {
                if matches(row, &self.search) {
                    items.push(Item::Row(row));
                }
            }
        }
        items
    }

    fn clamp(&mut self) {
        self.selected = self.selected.min(self.rows().len().saturating_sub(1));
    }

    /// Another squad is now on screen: its selection and scrolling start over.
    fn shown_changed(&mut self) {
        self.selected = 0;
        self.scrolls = Scrolls::default();
        self.follow = true;
        self.clamp();
    }

    /// The view on screen belongs to another squad while a switch loads.
    pub fn stale(&self) -> bool {
        self.view.is_some() && self.shown != self.current
    }

    /// Swaps in a loaded squad in one step. A result for a squad the user
    /// already left is kept for switching back, never shown.
    pub fn apply(&mut self, snapshot: Snapshot) {
        self.tabs = snapshot.tabs;
        self.hidden = snapshot.hidden;
        self.pinned = snapshot.pinned;
        self.attention = snapshot.attention;
        if self.current.is_some() && snapshot.squad != self.current {
            if let (Some(name), Ok(view)) = (snapshot.squad, snapshot.view) {
                self.cache.insert(name, view);
            }
            return;
        }
        self.current = snapshot.squad;
        self.loading_since = None;
        match snapshot.view {
            Ok(view) => {
                self.focus = self.focus.min(view.board.panes.len().saturating_sub(1));
                let changed = self.stale() || self.view.is_none();
                let previous = self.view.replace(view);
                let previous_squad = std::mem::replace(&mut self.shown, self.current.clone());
                // A result that arrived for it meanwhile is newer: keep that.
                if let (Some(previous), Some(name)) = (previous, previous_squad)
                    && Some(&name) != self.current.as_ref()
                {
                    self.cache.entry(name).or_insert(previous);
                }
                if changed {
                    self.shown_changed();
                }
                self.error = None;
            }
            Err(error) => {
                // The switch failed: the error is the state, not a stale frame
                // that keeps saying it is loading. The old view stays cached.
                if self.stale()
                    && let (Some(previous), Some(name)) = (self.view.take(), self.shown.take())
                {
                    self.cache.entry(name).or_insert(previous);
                }
                self.error = Some(error);
            }
        }
        self.clamp();
    }

    fn switch(&mut self, step: isize) -> Effect {
        let Some(position) = self
            .current
            .as_ref()
            .and_then(|current| self.tabs.iter().position(|tab| tab == current))
        else {
            return Effect::None;
        };
        let count = self.tabs.len() as isize;
        let next = self.tabs[(position as isize + step).rem_euclid(count) as usize].clone();
        self.go(next)
    }

    /// Shift+←/→: the current tab trades places with its neighbor, and the
    /// new order is saved. The ends do not wrap.
    fn move_current(&mut self, step: isize) -> Effect {
        let Some(from) = self
            .current
            .as_ref()
            .and_then(|current| self.tabs.iter().position(|tab| tab == current))
        else {
            return Effect::None;
        };
        match from
            .checked_add_signed(step)
            .filter(|to| *to < self.tabs.len())
        {
            Some(to) => self.move_tab(from, to),
            None => Effect::None,
        }
    }

    /// Moves one tab to another position and saves the order. Pinned tabs
    /// keep `[tabs] pin`'s order, so only the others move, and never among
    /// them: a saved `order` could not change where a pin is drawn.
    fn move_tab(&mut self, from: usize, to: usize) -> Effect {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return Effect::None;
        }
        if from < self.pinned || to < self.pinned {
            return self.say("Pinned tabs keep the order in [tabs] pin.");
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        Effect::Act(Request::Reorder(self.tabs.clone()))
    }

    /// Shows the tab `next`, from the cache at once when it was visited.
    fn go(&mut self, next: String) -> Effect {
        if Some(&next) == self.current.as_ref() {
            return Effect::None;
        }
        self.current = Some(next.clone());
        self.menu = None;
        self.loading_since = Some(Instant::now());
        // Never blank the screen: a visited squad shows from the cache at
        // once; otherwise the current frame stays until the new one arrives.
        if let Some(cached) = self.cache.remove(&next) {
            let previous = self.view.replace(cached);
            if let (Some(previous), Some(name)) = (previous, self.shown.replace(next.clone())) {
                self.cache.insert(name, previous);
            }
            self.shown_changed();
        }
        Effect::Load(next)
    }

    pub fn focused(&self) -> Pane {
        self.view
            .as_ref()
            .and_then(|view| view.board.panes.get(self.focus).copied())
            .unwrap_or(Pane::Rows)
    }

    fn next_pane(&mut self) {
        if let Some(view) = &self.view {
            self.focus = (self.focus + 1) % view.board.panes.len().max(1);
        }
    }

    /// The selected row's effective bindings: its section's, over `[bind]`
    /// and the host preset.
    pub fn bindings(&self) -> Bindings {
        let Some(view) = &self.view else {
            return Bindings::new();
        };
        let mut bindings = view.bindings.clone();
        if let Some(section) = self
            .rows()
            .get(self.selected)
            .and_then(|(index, _)| view.section_bindings.get(*index))
        {
            bindings.extend(section.clone());
        }
        bindings
    }

    fn say(&mut self, notice: impl Into<String>) -> Effect {
        self.notice = Some(notice.into());
        Effect::None
    }

    /// Resolves an action against the selected row. Missing values refuse
    /// the action with a notice; nothing runs half-filled. While a switch
    /// loads, the rows on screen are another squad's, so nothing acts on them.
    pub fn perform(&mut self, action: &Action) -> Effect {
        if self.stale() && !matches!(action.verb, Verb::NextPane | Verb::Refresh | Verb::Notes) {
            let loading = self.current.clone().unwrap_or_default();
            return self.say(format!("Loading {loading}…"));
        }
        match action.verb {
            Verb::NextPane => {
                self.next_pane();
                return Effect::None;
            }
            Verb::Refresh => {
                self.notice = Some("Refreshing…".into());
                return Effect::Refresh;
            }
            Verb::Notes => {
                let position = self
                    .view
                    .as_ref()
                    .and_then(|view| view.board.panes.iter().position(|p| *p == Pane::Notes));
                return match position {
                    Some(position) => {
                        self.focus = position;
                        Effect::None
                    }
                    None => self.say("The notes pane is not on this board; add it to panes."),
                };
            }
            Verb::Back => return Effect::Act(Request::Back),
            Verb::Jump if action.args.first().and_then(|arg| arg.literal()) == Some("lead") => {
                return match self.lead() {
                    Ok(lead) => Effect::Act(Request::Jump(lead)),
                    Err(reason) => self.say(format!("jump lead: {reason}.")),
                };
            }
            _ => {}
        }
        let Some(row) = self.selected_row().cloned() else {
            return self.say("No row is selected.");
        };
        let view = self.view.as_ref().expect("a selected row has a view");
        let request = match action.verb {
            Verb::Talk | Verb::Annotate | Verb::Reply => return self.compose(action, &row),
            Verb::Menu => {
                let mut entries: Vec<MenuEntry> = self
                    .bindings()
                    .into_iter()
                    .filter(|(event, action)| {
                        !matches!(event.as_str(), "click" | "double-click")
                            && !matches!(action.verb, Verb::Menu | Verb::NextPane)
                    })
                    .map(|(key, action)| MenuEntry {
                        key,
                        label: action.text.clone(),
                        choice: Choice::Action(action),
                    })
                    .collect();
                entries.sort_by_key(|entry| entry.key.chars().count() > 1);
                self.menu = Some(Menu {
                    title: row["name"].as_str().unwrap_or_default().to_owned(),
                    entries,
                    selected: 0,
                });
                return Effect::None;
            }
            Verb::Tab => {
                return match row["squad"].as_str() {
                    Some(squad) => self.go(squad.to_owned()),
                    None => self.say("tab: this row has no squad."),
                };
            }
            Verb::Jump => match row["name"].as_str() {
                Some(name) => Ok(Request::Jump(name.to_owned())),
                None => Err("This row has no member name.".to_owned()),
            },
            Verb::Open => match action.args.first() {
                Some(template) => template.fill(&row),
                None => effects::default_link(&row)
                    .map(str::to_owned)
                    .ok_or_else(|| "This row has no link field.".to_owned()),
            }
            .and_then(|link| effects::web_link(&link).map(str::to_owned))
            .map(|link| Request::Open {
                link,
                opener: view.opener.clone(),
            }),
            Verb::Copy => action.args[0].fill(&row).map(|text| Request::Copy {
                text,
                program: view.clipboard.clone(),
            }),
            Verb::Run => action.argv(&row).map(Request::Run),
            _ => unreachable!("handled above"),
        };
        match request {
            Ok(request) => Effect::Act(request),
            Err(reason) => self.say(format!("{}: {reason}.", action.verb.name())),
        }
    }

    /// The lead `jump lead` goes to: the squad's own lead on its tab; on the
    /// leads and all tabs, which list several squads, the selected row's
    /// squad's lead.
    fn lead(&self) -> Result<String, String> {
        let view = self.view.as_ref().ok_or("the board has not loaded yet")?;
        let named = |name: Option<&str>| name.filter(|name| !name.is_empty()).map(str::to_owned);
        let lead = match self.current.as_deref() {
            Some(super::LEADS) => named(self.selected_row().and_then(|row| row["name"].as_str())),
            Some(super::ALL) => named(
                self.selected_row()
                    .and_then(|row| row["fields"]["lead"].as_str()),
            ),
            _ => named(view.document["squad"]["lead"]["name"].as_str()),
        };
        lead.ok_or_else(|| match self.current.as_deref() {
            Some(super::LEADS | super::ALL) if self.selected_row().is_none() => {
                "no row is selected".to_owned()
            }
            _ => "this squad has no lead; set one with tmt squad lead <name>".to_owned(),
        })
    }

    fn ask(&mut self, prompt: String, compose: Compose, squad: String) -> Effect {
        self.input = Some(Input {
            prompt,
            text: String::new(),
            compose,
            squad,
        });
        Effect::None
    }

    /// Opens the composer for talk, annotate or reply. Reply needs an open
    /// request from the member; with several, the user picks one.
    fn compose(&mut self, action: &Action, row: &Value) -> Effect {
        let Some(view) = &self.view else {
            return Effect::None;
        };
        if view.me.is_none() {
            return self.say(
                "Who is sending? Record yourself with tmt squad me <name>, or open the board from your named pane.",
            );
        }
        let name = row["name"].as_str().unwrap_or_default().to_owned();
        // A leads-tab row carries its own squad; a squad tab's rows are its own.
        let squad = row["squad"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| self.current.clone())
            .unwrap_or_default();
        match action.verb {
            Verb::Talk => self.ask(format!("talk {name}"), Compose::Talk { to: name }, squad),
            Verb::Annotate if self.current.as_deref().is_some_and(super::tabs::builtin) => {
                self.say("Annotate from the squad's own tab.")
            }
            Verb::Annotate => {
                let to = if action.args[0].literal() == Some("member") {
                    name.clone()
                } else {
                    match view.document["squad"]["lead"]["name"].as_str() {
                        Some(lead) => lead.to_owned(),
                        None => return self.say("This squad has no lead to annotate for."),
                    }
                };
                self.ask(
                    format!("note on {name} for {to}"),
                    Compose::Annotate { to, row: name },
                    squad,
                )
            }
            _ => {
                let open: Vec<MenuEntry> = row["waitingOnYou"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(|(index, item)| {
                        Some(MenuEntry {
                            key: (index + 1).to_string(),
                            label: item["preview"].as_str().unwrap_or_default().to_owned(),
                            choice: Choice::Reply {
                                request: item["requestId"].as_str()?.to_owned(),
                                from: name.clone(),
                            },
                        })
                    })
                    .collect();
                match open.as_slice() {
                    [] => self.say(format!("{name} is not waiting on you.")),
                    [only] => self.choose(only.choice.clone()),
                    _ => {
                        self.menu = Some(Menu {
                            title: format!("reply to {name}"),
                            entries: open,
                            selected: 0,
                        });
                        Effect::None
                    }
                }
            }
        }
    }

    fn choose(&mut self, choice: Choice) -> Effect {
        match choice {
            Choice::Action(action) => self.perform(&action),
            Choice::Reply { request, from } => {
                let squad = self.current.clone().unwrap_or_default();
                self.ask(
                    format!("reply {from}"),
                    Compose::Reply { request, from },
                    squad,
                )
            }
        }
    }

    fn input_key(&mut self, key: KeyEvent) -> Effect {
        let Some(input) = &mut self.input else {
            return Effect::None;
        };
        match key.code {
            KeyCode::Esc => {
                self.input = None;
                return self.say("Nothing sent.");
            }
            KeyCode::Enter => {}
            KeyCode::Backspace => {
                input.text.pop();
                return Effect::None;
            }
            KeyCode::Char(character)
                if !character.is_control()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && input.text.chars().count() < INPUT_LIMIT =>
            {
                input.text.push(character);
                return Effect::None;
            }
            _ => return Effect::None,
        }
        let input = self.input.take().expect("composing");
        let text = input.text.trim().to_owned();
        let squad = input.squad;
        let Some(me) = self.view.as_ref().and_then(|view| view.me.clone()) else {
            return Effect::None;
        };
        if text.is_empty() {
            return self.say("Nothing sent.");
        }
        Effect::Act(match input.compose {
            Compose::Talk { to } => Request::Talk {
                me,
                squad,
                to,
                text,
            },
            Compose::Annotate { to, row } => Request::Annotate {
                me,
                squad,
                to,
                row,
                text,
            },
            Compose::Reply { request, from } => Request::Reply {
                me,
                request,
                from,
                text,
            },
        })
    }

    /// Shows how a request ended.
    pub fn finished(&mut self, outcome: Result<String, String>) {
        self.notice = Some(outcome.unwrap_or_else(|error| error));
    }

    fn menu_key(&mut self, key: KeyEvent) -> Effect {
        let Some(menu) = &mut self.menu else {
            return Effect::None;
        };
        let chosen = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => None,
            KeyCode::Up | KeyCode::Char('k') => {
                menu.selected = menu.selected.saturating_sub(1);
                return Effect::None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                menu.selected = (menu.selected + 1).min(menu.entries.len().saturating_sub(1));
                return Effect::None;
            }
            KeyCode::Enter => menu
                .entries
                .get(menu.selected)
                .map(|entry| entry.choice.clone()),
            _ => {
                let event = event_name(key);
                match menu
                    .entries
                    .iter()
                    .find(|entry| Some(&entry.key) == event.as_ref())
                {
                    Some(entry) => Some(entry.choice.clone()),
                    None => return Effect::None,
                }
            }
        };
        self.menu = None;
        chosen.map_or(Effect::None, |choice| self.choose(choice))
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        self.notice = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Effect::Quit;
        }
        if self.input.is_some() {
            return self.input_key(key);
        }
        if self.menu.is_some() {
            return self.menu_key(key);
        }
        if self.switcher.is_some() {
            return self.switcher_key(key);
        }
        if self.searching {
            match key.code {
                KeyCode::Char(character) if !character.is_control() => self.search.push(character),
                KeyCode::Backspace => {
                    self.search.pop();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Esc => {
                    self.search.clear();
                    self.searching = false;
                }
                _ => {}
            }
            self.clamp();
            return Effect::None;
        }
        match key.code {
            KeyCode::Char('q') => return Effect::Quit,
            KeyCode::Esc if self.search.is_empty() && !self.help => return Effect::Quit,
            KeyCode::Esc => {
                self.help = false;
                self.search.clear();
                self.clamp();
            }
            // Any pane but rows scrolls its text; rows moves the selection.
            KeyCode::Up | KeyCode::Char('k') if self.focused() != Pane::Rows => {
                self.scrolls.scroll(self.focused(), Step::Lines(-1));
            }
            KeyCode::Down | KeyCode::Char('j') if self.focused() != Pane::Rows => {
                self.scrolls.scroll(self.focused(), Step::Lines(1));
            }
            // Paging keys scroll unless the user bound them (then the
            // binding runs, below).
            KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
                if self.focused() != Pane::Rows && !self.bound(key) =>
            {
                self.scrolls.scroll(self.focused(), page_step(key.code));
            }
            KeyCode::Up | KeyCode::Char('k') => self.select(self.selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.select(self.selected + 1),
            KeyCode::PageUp if !self.bound(key) => {
                self.select(self.selected.saturating_sub(self.rows_page()));
            }
            KeyCode::PageDown if !self.bound(key) => self.select(self.selected + self.rows_page()),
            KeyCode::Home if !self.bound(key) => self.select(0),
            KeyCode::End if !self.bound(key) => self.select(usize::MAX),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => {
                return self.move_current(-1);
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => {
                return self.move_current(1);
            }
            KeyCode::Left => return self.switch(-1),
            KeyCode::Right => return self.switch(1),
            KeyCode::Char('/') => self.searching = true,
            // The switcher's key, unless the user bound `s` to something.
            KeyCode::Char('s') if !self.bound(key) => self.switcher = Some(Switcher::default()),
            KeyCode::Char('?') => self.help = !self.help,
            _ => {
                if let Some(action) =
                    event_name(key).and_then(|event| self.bindings().remove(&event))
                {
                    return self.perform(&action);
                }
            }
        }
        Effect::None
    }

    /// Every tab the switcher offers: the tab line's, then hidden ones.
    pub fn switchable(&self) -> Vec<String> {
        self.tabs.iter().chain(&self.hidden).cloned().collect()
    }

    fn switcher_key(&mut self, key: KeyEvent) -> Effect {
        let keys = self.switchable();
        let Some(switcher) = &mut self.switcher else {
            return Effect::None;
        };
        match key.code {
            KeyCode::Esc => {
                self.switcher = None;
                return Effect::None;
            }
            KeyCode::Up => switcher.selected = switcher.selected.saturating_sub(1),
            KeyCode::Down => switcher.selected += 1,
            KeyCode::Backspace => {
                switcher.query.pop();
                switcher.selected = 0;
            }
            KeyCode::Char(character)
                if !character.is_control() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                switcher.query.push(character);
                switcher.selected = 0;
            }
            KeyCode::Enter => {
                let chosen = super::tabs::matching(&keys, &switcher.query)
                    .get(switcher.selected)
                    .map(|key| (*key).clone());
                self.switcher = None;
                return match chosen {
                    Some(key) => self.go(key),
                    None => Effect::None,
                };
            }
            _ => {}
        }
        let count = super::tabs::matching(&keys, &switcher.query).len();
        switcher.selected = switcher.selected.min(count.saturating_sub(1));
        Effect::None
    }

    fn bound(&self, key: KeyEvent) -> bool {
        event_name(key).is_some_and(|event| self.bindings().contains_key(&event))
    }

    /// Selects a row and keeps it on screen.
    fn select(&mut self, row: usize) {
        self.selected = row;
        self.clamp();
        self.follow = true;
    }

    /// Rows one page moves: the rows pane's last viewport, less one.
    fn rows_page(&self) -> usize {
        self.scrolls.page_lines(Pane::Rows)
    }

    /// The wheel scrolls the pane under the pointer, whichever is focused.
    /// A left click selects the row under it, then runs its `click` binding;
    /// a second click on the same row soon after runs `double-click`.
    pub fn mouse(&mut self, event: MouseEvent, now: Instant) -> Effect {
        if self.menu.is_some() || self.input.is_some() || self.help || self.switcher.is_some() {
            return Effect::None;
        }
        let lines = match event.kind {
            MouseEventKind::ScrollUp => Some(-(WHEEL_LINES as isize)),
            MouseEventKind::ScrollDown => Some(WHEEL_LINES as isize),
            _ => None,
        };
        if let Some(lines) = lines {
            if let Some(pane) = self.scrolls.pane_at(event.column, event.row) {
                self.scrolls.scroll(pane, Step::Lines(lines));
                if pane == Pane::Rows {
                    self.follow = false;
                }
            }
            return Effect::None;
        }
        // A press on a tab shows it; releasing it over another tab moves it
        // there and saves the order.
        let tab = self.tab_hits.borrow().iter().copied().find(|hit| {
            hit.y == event.row && (hit.x..hit.x.saturating_add(hit.width)).contains(&event.column)
        });
        match (event.kind, tab) {
            (MouseEventKind::Down(MouseButton::Left), Some(hit)) => {
                self.dragging = Some(hit.tab);
                return match self.tabs.get(hit.tab).cloned() {
                    Some(key) => self.go(key),
                    None => Effect::None,
                };
            }
            (MouseEventKind::Up(MouseButton::Left), hit) => {
                return match (self.dragging.take(), hit) {
                    (Some(from), Some(to)) => self.move_tab(from, to.tab),
                    _ => Effect::None,
                };
            }
            _ => {}
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Effect::None;
        }
        let hit = self.hits.borrow().iter().copied().find(|hit| {
            hit.y == event.row && (hit.x..hit.x.saturating_add(hit.width)).contains(&event.column)
        });
        let Some(hit) = hit else {
            return Effect::None;
        };
        self.notice = None;
        self.select(hit.row);
        let double = self
            .last_click
            .is_some_and(|(row, at)| row == hit.row && now.duration_since(at) <= DOUBLE_CLICK);
        self.last_click = (!double).then_some((hit.row, now));
        let event = if double { "double-click" } else { "click" };
        match self.bindings().remove(event) {
            Some(action) => self.perform(&action),
            None => Effect::None,
        }
    }

    /// How the board draws now: the shown squad's theme, or the default
    /// one before the first load.
    pub fn look(&self) -> crate::look::Look {
        self.view.as_ref().map_or_else(
            || crate::look::Look::new(tmt_cli_style::Theme::default()),
            |view| view.look,
        )
    }

    pub fn selected_row(&self) -> Option<&Value> {
        self.rows().get(self.selected).map(|(_, row)| *row)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    fn press(app: &mut App, code: KeyCode) -> Effect {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn view(sections: Value) -> View {
        View {
            document: json!({"squad": {"name": "product"}, "sections": sections}),
            rows: crate::rows::Rows::preset(),
            colors: BTreeMap::new(),
            refresh: Some(crate::config::DEFAULT_REFRESH),
            board: crate::config::Board::simple(
                crate::config::BoardMode::Split,
                crate::config::Direction::LeftRight,
                vec![crate::config::Pane::Rows],
                &[100],
            ),
            notes: super::Notes::NotShown,
            render: crate::config::NotesRender::Markdown,
            bindings: crate::action::preset(true),
            section_bindings: Vec::new(),
            opener: None,
            clipboard: None,
            tab_colors: Default::default(),
            look: Default::default(),
            theme_notice: None,
            me: None,
            replies: Vec::new(),
        }
    }

    pub(crate) fn snapshot(squad: &str, sections: Value) -> Snapshot {
        Snapshot {
            tabs: vec!["infra".into(), "product".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some(squad.into()),
            view: Ok(view(sections)),
        }
    }

    fn row(name: &str, task: &str) -> Value {
        json!({"name": name, "fields": {"task": task}})
    }

    fn names(app: &App) -> Vec<&str> {
        app.items()
            .iter()
            .map(|item| match item {
                Item::Header(title) => *title,
                Item::Row(row) => row["name"].as_str().unwrap(),
            })
            .collect()
    }

    #[test]
    fn search_filters_rows_by_name_or_field_and_keeps_selection_in_range() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([
                {"title": "Needs me", "rows": [row("auth-fix", "Rotate tokens")]},
                {"title": "Everyone", "rows": [row("auth-fix", "Rotate tokens"), row("docs-sweep", "install guide")]}
            ]),
        ));
        assert_eq!(
            names(&app),
            ["Needs me", "auth-fix", "Everyone", "auth-fix", "docs-sweep"]
        );
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(
            app.selected_row().unwrap()["name"],
            "docs-sweep",
            "clamped to the last row"
        );
        press(&mut app, KeyCode::Char('/'));
        for character in "TOKEN".chars() {
            press(&mut app, KeyCode::Char(character));
        }
        assert_eq!(
            names(&app),
            ["Needs me", "auth-fix", "Everyone", "auth-fix"]
        );
        assert_eq!(app.selected, 1);
        press(&mut app, KeyCode::Esc);
        assert!(app.search.is_empty() && !app.searching);
        assert!(matches!(press(&mut app, KeyCode::Esc), Effect::Quit));
    }

    #[test]
    fn squad_switch_keeps_the_frame_caches_and_never_shows_another_squads_result() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("a", "")]}]),
        ));
        let Effect::Load(next) = press(&mut app, KeyCode::Right) else {
            panic!("switch");
        };
        assert_eq!(next, "infra");
        // Nothing is blanked: product's frame stays until infra arrives, and
        // its rows take no actions meanwhile.
        assert_eq!(names(&app), ["a"]);
        assert!(app.stale() && app.loading_since.is_some());
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.notice.as_deref(), Some("Loading infra…"));
        // A late product result is kept for switching back, never shown.
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("late", "")]}]),
        ));
        assert_eq!(names(&app), ["a"]);
        app.apply(snapshot(
            "infra",
            json!([{"title": null, "rows": [row("i", "")]}]),
        ));
        assert_eq!(names(&app), ["i"]);
        assert!(!app.stale() && app.loading_since.is_none());
        // Back to product: at once, from the cache (the late result).
        assert_eq!(
            press(&mut app, KeyCode::Left),
            Effect::Load("product".into())
        );
        assert_eq!(names(&app), ["late"]);
        assert!(!app.stale(), "a cached squad is the current one at once");
    }

    #[test]
    fn a_failed_switch_shows_its_error_instead_of_a_stale_loading_frame() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("a", "")]}]),
        ));
        press(&mut app, KeyCode::Right);
        app.apply(Snapshot {
            tabs: vec!["product".into(), "infra".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some("infra".into()),
            view: Err("infra: room not found".into()),
        });
        assert!(!app.stale() && app.loading_since.is_none());
        assert!(app.view.is_none());
        assert_eq!(app.error.as_deref(), Some("infra: room not found"));
        press(&mut app, KeyCode::Enter);
        assert_ne!(app.notice.as_deref(), Some("Loading infra…"));
        // The squad it came from is still one key away, from the cache.
        press(&mut app, KeyCode::Left);
        assert_eq!(names(&app), ["a"]);
    }

    fn member(name: &str, fields: Value) -> Value {
        json!({"name": name, "state": "working", "fields": fields})
    }

    fn crew(bindings: crate::action::Bindings, sections: Vec<crate::action::Bindings>) -> App {
        let mut app = App::new(Some("product".into()));
        let mut snapshot = snapshot(
            "product",
            json!([
                {"title": "Mine", "rows": [member("auth-fix", json!({"task": "rotate", "pr_link": "https://example.com/pull/412"}))]},
                {"title": "Others", "rows": [member("docs", json!({"task": "guide", "link": "file:///etc/passwd"}))]}
            ]),
        );
        let view = snapshot.view.as_mut().unwrap();
        view.bindings = bindings;
        view.section_bindings = sections;
        view.clipboard = Some(vec!["pbcopy".into()]);
        app.apply(snapshot);
        app
    }

    fn bind(entries: &[(&str, &str)]) -> crate::action::Bindings {
        crate::action::parse_bindings(entries.iter().map(|(e, a)| (*e, Some(*a))), "bind").unwrap()
    }

    /// `L` (`jump lead`) goes to the squad's lead on its own tab, and to the
    /// selected row's squad's lead on the leads and all tabs; without one it
    /// says why and nothing runs.
    #[test]
    fn jump_lead_goes_to_the_lead_of_the_tab_or_the_selected_row_s_squad() {
        let lead = |app: &mut App| press(app, KeyCode::Char('L'));
        let mut app = App::new(Some("product".into()));
        let mut own = snapshot(
            "product",
            json!([{"title": null, "rows": [row("rin", "x")]}]),
        );
        own.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol"});
        app.apply(own);
        assert_eq!(lead(&mut app), Effect::Act(Request::Jump("sol".into())));

        // An empty squad still has its lead.
        let mut app = App::new(Some("product".into()));
        let mut empty = snapshot("product", json!([{"title": null, "rows": []}]));
        empty.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol"});
        app.apply(empty);
        assert_eq!(lead(&mut app), Effect::Act(Request::Jump("sol".into())));

        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("rin", "x")]}]),
        ));
        assert_eq!(lead(&mut app), Effect::None);
        assert_eq!(
            app.notice.as_deref(),
            Some("jump lead: this squad has no lead; set one with tmt squad lead <name>.")
        );

        // The all tab: a row per squad, with its lead as a field.
        let mut app = App::new(Some(crate::board::ALL.into()));
        app.apply(snapshot(
            crate::board::ALL,
            json!([{"title": null, "rows": [
                {"name": "product", "squad": "product", "fields": {"lead": "sol"}},
                {"name": "quiet", "squad": "quiet", "fields": {"lead": null}},
            ]}]),
        ));
        assert_eq!(lead(&mut app), Effect::Act(Request::Jump("sol".into())));
        press(&mut app, KeyCode::Down);
        assert_eq!(lead(&mut app), Effect::None);
        assert_eq!(
            app.notice.as_deref(),
            Some("jump lead: this squad has no lead; set one with tmt squad lead <name>.")
        );

        // A missing lead is data, so a custom binding gets the same missing-field
        // notice as any other absent value instead of copying a display glyph.
        assert_eq!(
            app.perform(&Action::parse("copy {lead}").unwrap()),
            Effect::None
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("copy: lead is empty for this row.")
        );

        // The leads tab: the selected row is the lead.
        let mut app = App::new(Some(crate::board::LEADS.into()));
        app.apply(snapshot(
            crate::board::LEADS,
            json!([{"title": null, "rows": [
                {"name": "sol", "squad": "product", "fields": {}},
                {"name": "rin", "squad": "infra", "fields": {}},
            ]}]),
        ));
        press(&mut app, KeyCode::Down);
        assert_eq!(lead(&mut app), Effect::Act(Request::Jump("rin".into())));
    }

    #[test]
    fn the_leads_tab_jumps_to_a_lead_and_talks_in_that_lead_s_squad() {
        let mut app = App::new(Some(crate::board::LEADS.into()));
        let mut snapshot = snapshot(
            crate::board::LEADS,
            json!([{"title": null, "rows": [
                member("sol", json!({"squad": "product"})),
                member("rin", json!({"squad": "infra"})),
            ]}]),
        );
        for (row, squad) in [(0, "product"), (1, "infra")] {
            snapshot.view.as_mut().unwrap().document["sections"][0]["rows"][row]["squad"] =
                json!(squad);
        }
        snapshot.view.as_mut().unwrap().me = Some("Ben".into());
        snapshot.tabs = ["product", crate::board::LEADS, "infra"]
            .map(String::from)
            .to_vec();
        app.apply(snapshot);
        press(&mut app, KeyCode::Down);
        // Enter jumps through tmt focus, as on a squad's own tab.
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Jump("rin".into()))
        );
        press(&mut app, KeyCode::Char('t'));
        typed(&mut app, "status?");
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Talk {
                me: "Ben".into(),
                squad: "infra".into(),
                to: "rin".into(),
                text: "status?".into()
            }),
            "sent in the lead's own squad room, not a tab's"
        );
        press(&mut app, KeyCode::Char('a'));
        assert!(app.input.is_none());
        assert_eq!(
            app.notice.as_deref(),
            Some("Annotate from the squad's own tab.")
        );
        // ← → walk every tab, the built-in one included.
        assert_eq!(
            press(&mut app, KeyCode::Right),
            Effect::Load("infra".into())
        );
    }

    #[test]
    fn enter_on_the_all_tab_opens_that_squad_s_tab() {
        let mut app = App::new(Some(crate::board::ALL.into()));
        let mut snapshot = snapshot(
            crate::board::ALL,
            json!([{"title": null, "rows": [
                {"name": "product", "squad": "product", "fields": {}},
                {"name": "infra", "squad": "infra", "fields": {}},
            ]}]),
        );
        snapshot.view.as_mut().unwrap().bindings =
            bind(&[("enter", "tab"), ("t", "talk"), ("o", "open")]);
        app.apply(snapshot);
        press(&mut app, KeyCode::Down);
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Load("infra".into())
        );
        assert_eq!(app.current.as_deref(), Some("infra"));
        // A member's row has no squad of its own to open.
        let mut app = crew(bind(&[("enter", "tab")]), Vec::new());
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.notice.as_deref(), Some("tab: this row has no squad."));
    }

    #[test]
    fn keys_resolve_the_selected_row_into_requests_or_refuse_with_a_notice() {
        let mut app = crew(crate::action::preset(true), Vec::new());
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Jump("auth-fix".into()))
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('o')),
            Effect::Act(Request::Open {
                link: "https://example.com/pull/412".into(),
                opener: None
            })
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('y')),
            Effect::Act(Request::Copy {
                text: "auth-fix: rotate (working)".into(),
                program: Some(vec!["pbcopy".into()])
            })
        );
        press(&mut app, KeyCode::Down);
        assert_eq!(press(&mut app, KeyCode::Char('o')), Effect::None);
        assert!(
            app.notice
                .as_deref()
                .unwrap()
                .contains("Only http(s) links open"),
            "{:?}",
            app.notice
        );
        assert_eq!(press(&mut app, KeyCode::Char('t')), Effect::None);
        assert!(
            app.notice
                .as_deref()
                .unwrap()
                .starts_with("Who is sending? Record yourself with tmt squad me"),
            "sending needs me"
        );
        assert_eq!(press(&mut app, KeyCode::Char('x')), Effect::None);
        assert_eq!(app.notice, None, "an unbound key does nothing");
        assert_eq!(press(&mut app, KeyCode::Char('q')), Effect::Quit);
    }

    fn typed(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    #[test]
    fn talk_annotate_and_reply_compose_one_line_and_empty_sends_nothing() {
        let mut app = crew(crate::action::preset(true), Vec::new());
        {
            let view = app.view.as_mut().unwrap();
            view.me = Some("Ben".into());
            view.document["squad"]["lead"] = json!({"id": "L", "name": "sol"});
            view.document["sections"][0]["rows"][0]["waitingOnYou"] = json!([
                {"requestId": "q2", "preview": "approve the plan?"},
                {"requestId": "q1", "preview": "which database?"}
            ]);
            view.document["sections"][1]["rows"][0]["waitingOnYou"] =
                json!([{"requestId": "q9", "preview": "ok to merge?"}]);
        }
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.input.as_ref().unwrap().prompt, "talk auth-fix");
        typed(&mut app, "q j -rf; $(x)");
        assert!(app.input.is_some(), "q and j are text while composing");
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Talk {
                me: "Ben".into(),
                squad: "product".into(),
                to: "auth-fix".into(),
                text: "q j -rf; $(x)".into()
            })
        );
        assert!(app.input.is_none());

        press(&mut app, KeyCode::Char('t'));
        typed(&mut app, "   ");
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.notice.as_deref(), Some("Nothing sent."));
        press(&mut app, KeyCode::Char('t'));
        typed(&mut app, "draft");
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.input.is_none() && app.notice.as_deref() == Some("Nothing sent."));

        press(&mut app, KeyCode::Char('a'));
        assert_eq!(
            app.input.as_ref().unwrap().prompt,
            "note on auth-fix for sol"
        );
        typed(&mut app, "split the job");
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Annotate {
                me: "Ben".into(),
                squad: "product".into(),
                to: "sol".into(),
                row: "auth-fix".into(),
                text: "split the job".into()
            })
        );

        // Two open requests: the user picks; the newest is never assumed.
        press(&mut app, KeyCode::Char('r'));
        let menu = app.menu.as_ref().expect("picker");
        assert_eq!(menu.title, "reply to auth-fix");
        assert_eq!(menu.entries.len(), 2);
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(app.input.as_ref().unwrap().prompt, "reply auth-fix");
        typed(&mut app, "postgres");
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Reply {
                me: "Ben".into(),
                request: "q1".into(),
                from: "auth-fix".into(),
                text: "postgres".into()
            })
        );
        // One open request goes straight to the composer.
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('r'));
        assert!(app.menu.is_none());
        typed(&mut app, "yes");
        assert!(matches!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Reply { request, .. }) if request == "q9"
        ));
        assert!(!Request::Back.sends());

        // Nothing open, or no lead to annotate for, is a notice.
        app.view.as_mut().unwrap().document["sections"][1]["rows"][0]["waitingOnYou"] = json!([]);
        app.view.as_mut().unwrap().document["squad"]["lead"] = Value::Null;
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.notice.as_deref(), Some("docs is not waiting on you."));
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(
            app.notice.as_deref(),
            Some("This squad has no lead to annotate for.")
        );
        assert!(app.input.is_none());
    }

    #[test]
    fn back_is_one_request_and_outcomes_become_the_notice() {
        let mut app = crew(crate::action::preset(true), Vec::new());
        assert_eq!(
            press(&mut app, KeyCode::Backspace),
            Effect::Act(Request::Back)
        );
        app.finished(Ok("Nothing to go back to.".into()));
        assert_eq!(app.notice.as_deref(), Some("Nothing to go back to."));
        app.finished(Err("Pane '%5' was not found.".into()));
        assert_eq!(app.notice.as_deref(), Some("Pane '%5' was not found."));
    }

    #[test]
    fn section_bindings_win_over_bind_which_wins_over_the_preset() {
        let mut global = crate::action::preset(true);
        global.extend(bind(&[("o", "open {pr_link}"), ("f5", "refresh")]));
        let sections = vec![
            Bindings::new(),
            bind(&[("o", "run code {name} --task={task}")]),
        ];
        let mut app = crew(global, sections);
        assert_eq!(press(&mut app, KeyCode::F(5)), Effect::Refresh);
        assert!(matches!(
            press(&mut app, KeyCode::Char('o')),
            Effect::Act(Request::Open { .. })
        ));
        press(&mut app, KeyCode::Down);
        assert_eq!(
            press(&mut app, KeyCode::Char('o')),
            Effect::Act(Request::Run(vec![
                "code".into(),
                "docs".into(),
                "--task=guide".into()
            ]))
        );
        // A missing field refuses rather than running with a gap.
        let mut app = crew(bind(&[("o", "open {pr_link}")]), Vec::new());
        press(&mut app, KeyCode::Down);
        assert_eq!(press(&mut app, KeyCode::Char('o')), Effect::None);
        assert_eq!(
            app.notice.as_deref(),
            Some("open: pr_link is empty for this row.")
        );
    }

    #[test]
    fn the_plain_host_menu_lists_row_actions_and_runs_the_chosen_one() {
        let mut app = crew(crate::action::preset(false), Vec::new());
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        let menu = app.menu.as_ref().expect("menu");
        assert_eq!(menu.title, "auth-fix");
        let events: Vec<&str> = menu.entries.iter().map(|e| e.key.as_str()).collect();
        assert!(events.contains(&"y") && events.contains(&"backspace"));
        assert!(!events.iter().any(|e| ["click", "enter", "tab"].contains(e)));
        assert_eq!(press(&mut app, KeyCode::Char('j')), Effect::None);
        assert!(app.menu.is_some(), "j moves inside the menu");
        assert!(matches!(
            press(&mut app, KeyCode::Char('y')),
            Effect::Act(Request::Copy { .. })
        ));
        assert!(app.menu.is_none());
        press(&mut app, KeyCode::Enter);
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.menu.is_none(), "Esc closes the menu, not the board");
    }

    #[test]
    fn clicks_select_rows_and_run_click_or_double_click_bindings() {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut global = crate::action::preset(true);
        global.extend(bind(&[("click", "notes"), ("double-click", "jump")]));
        let mut app = crew(global, Vec::new());
        *app.hits.borrow_mut() = vec![
            Hit {
                y: 3,
                x: 0,
                width: 40,
                row: 0,
            },
            Hit {
                y: 5,
                x: 0,
                width: 40,
                row: 1,
            },
        ];
        let click = |row, column| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let start = Instant::now();
        assert_eq!(app.mouse(click(5, 3), start), Effect::None);
        assert_eq!(app.selected, 1);
        assert_eq!(
            app.notice.as_deref(),
            Some("The notes pane is not on this board; add it to panes.")
        );
        assert_eq!(
            app.mouse(click(5, 3), start + Duration::from_millis(200)),
            Effect::Act(Request::Jump("docs".into()))
        );
        assert_eq!(
            app.mouse(click(3, 3), start + Duration::from_millis(300)),
            Effect::None,
            "a click on another row is a new single click"
        );
        assert_eq!(app.selected, 0);
        assert_eq!(
            app.mouse(click(3, 3), start + Duration::from_secs(2)),
            Effect::None,
            "too slow for a double click"
        );
        assert_eq!(app.mouse(click(9, 3), start), Effect::None);
        assert_eq!(app.mouse(click(3, 45), start), Effect::None);
        assert_eq!(app.selected, 0, "clicks outside rows change nothing");
    }

    #[test]
    fn failed_refreshes_keep_the_last_view() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("a", "")]}]),
        ));
        app.apply(Snapshot {
            tabs: vec!["product".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some("product".into()),
            view: Err("tmt did not finish in time".into()),
        });
        assert_eq!(
            names(&app),
            ["a"],
            "a failed refresh keeps showing the last good rows"
        );
        assert!(app.error.is_some());
        assert!(matches!(
            app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Effect::Quit
        ));
    }
}
