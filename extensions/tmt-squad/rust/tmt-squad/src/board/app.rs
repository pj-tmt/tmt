//! Board state and key handling, independent of the terminal. Every keypress
//! works on what is already loaded; loading happens in the refresh worker.

use crate::config::{Board, Column, NotesRender, Pane};
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

pub enum Effect {
    None,
    Quit,
    /// Load this squad now (a squad switch).
    Load(String),
}

pub enum Item<'a> {
    Header(&'a str),
    Row(&'a Value),
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
    pub notice: Option<&'static str>,
    pub help: bool,
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

impl App {
    pub fn new(squad: Option<String>) -> Self {
        Self {
            current: squad,
            ..Self::default()
        }
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

    fn row_count(&self) -> usize {
        self.items()
            .iter()
            .filter(|item| matches!(item, Item::Row(_)))
            .count()
    }

    fn clamp(&mut self) {
        self.selected = self.selected.min(self.row_count().saturating_sub(1));
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

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        self.notice = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Effect::Quit;
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
            KeyCode::Tab => self.next_pane(),
            KeyCode::Left => return self.switch(-1),
            KeyCode::Right => return self.switch(1),
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('?') => self.help = !self.help,
            KeyCode::Enter
            | KeyCode::Backspace
            | KeyCode::Char('t' | 'r' | 'a' | 'o' | 'y' | 'n') => self.notice = Some(LATER),
            _ => {}
        }
        Effect::None
    }

    pub fn selected_row(&self) -> Option<&Value> {
        self.items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Row(row) => Some(row),
                Item::Header(_) => None,
            })
            .nth(self.selected)
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

    #[test]
    fn action_keys_say_they_arrive_later_and_errors_keep_the_last_view() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("a", "")]}]),
        ));
        for code in [
            KeyCode::Enter,
            KeyCode::Char('t'),
            KeyCode::Char('o'),
            KeyCode::Backspace,
        ] {
            assert!(matches!(press(&mut app, code), Effect::None));
            assert_eq!(app.notice, Some(LATER));
        }
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
