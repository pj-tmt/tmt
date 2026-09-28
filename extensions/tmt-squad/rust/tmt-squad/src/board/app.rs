//! Board state and key handling, independent of the terminal. Every keypress
//! works on what is already loaded; loading happens in the refresh worker,
//! and actions leave as fully resolved requests.

use crate::{
    action::{Action, Bindings, Verb},
    config::{Board, Column, NotesRender, Pane},
    effects,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::collections::BTreeMap;

/// What the refresh worker loaded for one squad.
pub struct View {
    /// The `status --json` document, so the board and `status` never differ.
    pub document: Value,
    pub columns: Vec<Column>,
    pub colors: BTreeMap<String, String>,
    pub board: Board,
    pub notes: Notes,
    pub render: NotesRender,
    /// The host preset overridden by `[bind]`.
    pub bindings: Bindings,
    /// Each document section's own bindings, in document order.
    pub section_bindings: Vec<Bindings>,
    pub opener: Option<Vec<String>>,
    pub clipboard: Option<Vec<String>>,
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
    pub squads: Vec<String>,
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

/// The row's action menu: its bindings, by event.
pub struct Menu {
    pub title: String,
    pub entries: Vec<(String, Action)>,
    pub selected: usize,
}

pub const LATER: &str = "That action is available in a later version.";

#[derive(Default)]
pub struct App {
    pub squads: Vec<String>,
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
    /// Index of the focused pane (split) or visible tab (tabs).
    pub focus: usize,
    pub notes_scroll: u16,
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

    /// A late result for a squad the user already left is ignored.
    pub fn apply(&mut self, snapshot: Snapshot) {
        self.squads = snapshot.squads;
        if self.current.is_some() && snapshot.squad != self.current {
            return;
        }
        self.current = snapshot.squad;
        match snapshot.view {
            Ok(view) => {
                self.focus = self.focus.min(view.board.panes.len().saturating_sub(1));
                self.view = Some(view);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        self.clamp();
    }

    fn switch(&mut self, step: isize) -> Effect {
        let Some(position) = self
            .current
            .as_ref()
            .and_then(|current| self.squads.iter().position(|squad| squad == current))
        else {
            return Effect::None;
        };
        let count = self.squads.len() as isize;
        let next = self.squads[(position as isize + step).rem_euclid(count) as usize].clone();
        if Some(&next) == self.current.as_ref() {
            return Effect::None;
        }
        self.current = Some(next.clone());
        self.view = None;
        self.menu = None;
        self.selected = 0;
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
    /// the action with a notice; nothing runs half-filled.
    pub fn perform(&mut self, action: &Action) -> Effect {
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
            Verb::Talk | Verb::Reply | Verb::Annotate => return self.say(LATER),
            _ => {}
        }
        let Some(row) = self.selected_row().cloned() else {
            return self.say("No row is selected.");
        };
        let view = self.view.as_ref().expect("a selected row has a view");
        let request = match action.verb {
            Verb::Menu => {
                let mut entries: Vec<(String, Action)> = self
                    .bindings()
                    .into_iter()
                    .filter(|(_, action)| !matches!(action.verb, Verb::Menu | Verb::NextPane))
                    .collect();
                entries.sort_by_key(|(event, _)| event.chars().count() > 1);
                self.menu = Some(Menu {
                    title: row["name"].as_str().unwrap_or_default().to_owned(),
                    entries,
                    selected: 0,
                });
                return Effect::None;
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
            _ => unreachable!("handled above"),
        };
        match request {
            Ok(request) => Effect::Act(request),
            Err(reason) => self.say(format!("{}: {reason}.", action.verb.name())),
        }
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
            KeyCode::Enter => menu.entries.get(menu.selected).map(|(_, a)| a.clone()),
            _ => {
                let event = event_name(key);
                match menu.entries.iter().find(|(e, _)| Some(e) == event.as_ref()) {
                    Some((_, action)) => Some(action.clone()),
                    None => return Effect::None,
                }
            }
        };
        self.menu = None;
        chosen.map_or(Effect::None, |action| self.perform(&action))
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        self.notice = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Effect::Quit;
        }
        if self.menu.is_some() {
            return self.menu_key(key);
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
            KeyCode::Up | KeyCode::Char('k') if self.focused() == Pane::Notes => {
                self.notes_scroll = self.notes_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if self.focused() == Pane::Notes => {
                self.notes_scroll = self.notes_scroll.saturating_add(1);
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected += 1;
                self.clamp();
            }
            KeyCode::Left => return self.switch(-1),
            KeyCode::Right => return self.switch(1),
            KeyCode::Char('/') => self.searching = true,
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

    pub fn selected_row(&self) -> Option<&Value> {
        self.rows().get(self.selected).map(|(_, row)| *row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn press(app: &mut App, code: KeyCode) -> Effect {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn view(sections: Value) -> View {
        View {
            document: json!({"squad": {"name": "product"}, "sections": sections}),
            columns: Vec::new(),
            colors: BTreeMap::new(),
            board: crate::config::Board {
                mode: crate::config::BoardMode::Split,
                direction: crate::config::Direction::LeftRight,
                panes: vec![crate::config::Pane::Rows],
                sizes: vec![100],
            },
            notes: super::Notes::NotShown,
            render: crate::config::NotesRender::Markdown,
            bindings: crate::action::preset(true),
            section_bindings: Vec::new(),
            opener: None,
            clipboard: None,
        }
    }

    fn snapshot(squad: &str, sections: Value) -> Snapshot {
        Snapshot {
            squads: vec!["infra".into(), "product".into()],
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
    fn squad_switch_loads_and_ignores_stale_results() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("a", "")]}]),
        ));
        let Effect::Load(next) = press(&mut app, KeyCode::Right) else {
            panic!("switch");
        };
        assert_eq!(next, "infra");
        assert!(app.view.is_none());
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("late", "")]}]),
        ));
        assert!(
            app.view.is_none(),
            "a late product result must not show under infra"
        );
        app.apply(snapshot(
            "infra",
            json!([{"title": null, "rows": [row("i", "")]}]),
        ));
        assert_eq!(names(&app), ["i"]);
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
        assert_eq!(app.notice.as_deref(), Some(LATER));
        assert_eq!(press(&mut app, KeyCode::Char('x')), Effect::None);
        assert_eq!(app.notice, None, "an unbound key does nothing");
        assert_eq!(press(&mut app, KeyCode::Char('q')), Effect::Quit);
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
        let sections = vec![Bindings::new(), bind(&[("o", "copy {name} ({task})")])];
        let mut app = crew(global, sections);
        assert_eq!(press(&mut app, KeyCode::F(5)), Effect::Refresh);
        assert!(matches!(
            press(&mut app, KeyCode::Char('o')),
            Effect::Act(Request::Open { .. })
        ));
        press(&mut app, KeyCode::Down);
        assert_eq!(
            press(&mut app, KeyCode::Char('o')),
            Effect::Act(Request::Copy {
                text: "docs (guide)".into(),
                program: Some(vec!["pbcopy".into()])
            })
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
        let events: Vec<&str> = menu.entries.iter().map(|(e, _)| e.as_str()).collect();
        assert!(events.contains(&"y") && events.contains(&"backspace"));
        assert!(!events.iter().any(|e| ["enter", "tab"].contains(e)));
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
    fn failed_refreshes_keep_the_last_view() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("a", "")]}]),
        ));
        app.apply(Snapshot {
            squads: vec!["product".into()],
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
