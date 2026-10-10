//! What an in-place reload carries across: this session's view state, captured
//! from the `App` and put back on the first fresh view of the same tab. Nothing
//! here is data about squads; selection is an identity looked up in whatever the
//! new process loads, and a row that is gone falls back to the usual start.

use super::{App, RowTarget};
use crate::board::{pick::Picks, resume::Resume};

impl App {
    /// Whether replacing the process now would lose something the user has open
    /// or is waiting on: a draft, an overlay, a search being typed, a request in
    /// flight, a view still loading or a resumed state not yet applied.
    pub(in crate::board) fn reload_blocked(&self) -> bool {
        self.current.is_none()
            || self.loading_since.is_some()
            || self.pending_resume.is_some()
            || self.in_flight > 0
            || self.pending_send.is_some()
            || self.input.is_some()
            || self.status_draft.is_some()
            || self.cron_draft.is_some()
            || self.searching
            || self.settings.is_some()
            || self.help
            || self.menu.is_some()
            || self.view_picker.is_some()
            || self.theme_picker.is_some()
            || self.switcher.is_some()
            || self.cron_list.is_some()
            || self.checklist_shown()
            || self.row_details.reader.is_some()
    }

    /// A reload is due and something open holds it back, which the footer says.
    pub(in crate::board) fn reload_waiting(&self) -> bool {
        self.reload.pending() && self.reload_blocked()
    }

    /// The state a reload carries; `None` before any tab is shown.
    pub(in crate::board) fn resume(&self) -> Option<Resume> {
        let tab = self.current.clone()?;
        Some(Resume {
            selected: self.row_target(self.selected),
            focus: self.focus,
            follow: self.follow,
            scrolls: self.scrolls.offsets(),
            expanded: self.row_details.expanded.clone(),
            folds: self
                .folds
                .get(&tab)
                .map(|state| {
                    state
                        .overrides
                        .iter()
                        .map(|(pane, fold)| (*pane, *fold))
                        .collect()
                })
                .unwrap_or_default(),
            picks: self.picks.keys(),
            search: self.search.clone(),
            tab,
        })
    }

    /// Takes a reload's state in at startup; it applies when the first fresh view of
    /// its tab (the one `App::new` was given) lands.
    pub(in crate::board) fn carry(&mut self, resume: Resume) {
        self.picks = Picks::from_keys(resume.picks.clone());
        self.pending_resume = Some(resume);
    }

    /// Puts the carried state back once the view it describes exists.
    pub(super) fn restore_pending(&mut self) {
        if self
            .pending_resume
            .as_ref()
            .is_none_or(|resume| self.current.as_deref() != Some(resume.tab.as_str()))
        {
            return;
        }
        let Some(resume) = self.pending_resume.take() else {
            return;
        };
        self.search = resume.search;
        if let Some(state) = self
            .current
            .as_ref()
            .and_then(|tab| self.folds.get_mut(tab))
        {
            state.overrides.extend(resume.folds);
            self.reconcile_folds();
        }
        self.row_details.expanded = resume.expanded;
        let panes = self.view.as_ref().map_or(0, |view| view.board.panes.len());
        self.focus = resume.focus.min(panes.saturating_sub(1));
        self.scrolls.restore(resume.scrolls);
        self.follow = resume.follow;
        match resume.selected {
            Some(RowTarget::Home(target)) => {
                if let Some(index) = self
                    .home_entries()
                    .iter()
                    .position(|entry| entry.target == target)
                {
                    // The cursor target set by the opening placement would win over the index.
                    self.home_target = None;
                    self.selected = index;
                    self.home_start = false;
                }
            }
            Some(anchor) => self.follow_member(&anchor),
            None => {}
        }
        // Expanded details of rows that no longer exist are dropped here.
        self.clamp();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        board::{
            app::{Snapshot, tests::snapshot},
            row_detail::Target as Detail,
        },
        config::Pane,
    };
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use serde_json::{Value, json};
    use std::time::Instant;

    /// Puts something the user could have open on a board.
    type Open = fn(&mut App);

    fn rows(names: &[&str]) -> Value {
        let rows: Vec<Value> = names
            .iter()
            .map(|name| json!({"id": name, "name": name, "fields": {"task": ""}}))
            .collect();
        json!([{"title": null, "rows": rows}])
    }

    fn product(names: &[&str]) -> Snapshot {
        snapshot("product", rows(names))
    }

    fn loaded(names: &[&str]) -> App {
        let mut app = App::new(Some("product".into()));
        app.apply(product(names));
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn selected_name(app: &App) -> &str {
        app.selected_row().unwrap()["name"].as_str().unwrap()
    }

    /// Selects the row named `name`, as the cursor would.
    fn select(app: &mut App, name: &str) {
        app.selected = (0..app.rows().len())
            .find(|&index| app.rows()[index].1["name"] == name)
            .unwrap();
    }

    /// What the next process reads, through the real file.
    fn arrive(resume: &Resume) -> Resume {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "ops-carry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        resume.write(&directory, 7, 1_000).unwrap();
        let taken = Resume::take(&directory, 7, 1_000).unwrap();
        let _ = std::fs::remove_dir_all(directory);
        taken
    }

    #[test]
    fn a_reload_waits_for_anything_the_user_has_open() {
        assert!(App::default().reload_blocked(), "no tab is shown yet");
        assert!(App::default().resume().is_none());
        assert!(
            !loaded(&["a", "b"]).reload_blocked(),
            "a quiet board reloads"
        );
        let cases: [(&str, Open); 7] = [
            ("a composer", |app| {
                app.input = Some(crate::board::app::Input {
                    row_send: None,
                    others: Vec::new(),
                    quote: None,
                    link: None,
                    prompt: String::new(),
                    text: "a draft".into(),
                    compose: crate::board::app::Compose::AskLead {
                        to: "lead".into(),
                        sender: "ben".into(),
                    },
                    squad: "product".into(),
                    hint: None,
                });
            }),
            ("a search being typed", |app| press(app, KeyCode::Char('/'))),
            ("help", |app| app.help = true),
            ("a request in flight", |app| app.in_flight = 1),
            ("a view still loading", |app| {
                app.loading_since = Some(Instant::now());
            }),
            ("a reply being read", |app| {
                app.row_details.reader = Some(crate::board::row_detail::Reader {
                    target: Detail::Job("c1".into()),
                    scroll: Default::default(),
                });
            }),
            ("carried state not yet applied", |app| {
                app.carry(Resume {
                    tab: "product".into(),
                    selected: None,
                    focus: 0,
                    follow: true,
                    scrolls: Vec::new(),
                    expanded: Vec::new(),
                    folds: Vec::new(),
                    picks: None,
                    search: String::new(),
                });
            }),
        ];
        for (name, open) in cases {
            let mut app = loaded(&["a", "b"]);
            open(&mut app);
            assert!(app.reload_blocked(), "{name} defers the reload");
        }
    }

    #[test]
    fn a_reload_brings_back_the_tab_selection_scroll_and_expansion() {
        let mut before = loaded(&["a", "b", "c", "d"]);
        select(&mut before, "c");
        before.follow = false;
        before.scrolls.restore([(Pane::Rows, 3), (Pane::Notes, 5)]);
        let target = before.row_target(before.selected).unwrap();
        before.row_details.toggle(Detail::Row(target.clone()));
        before.search = "".into();
        let resume = arrive(&before.resume().unwrap());
        assert_eq!(resume.tab, "product");

        // The new process lists the squad in another order.
        let mut after = App::new(Some("product".into()));
        after.carry(resume.clone());
        assert!(after.reload_blocked(), "until the state is applied");
        after.apply(product(&["d", "x", "a", "c", "b"]));
        assert_eq!(
            selected_name(&after),
            "c",
            "selection follows the member, not the index"
        );
        assert!(!after.follow);
        assert_eq!(after.scrolls.offset(Pane::Rows), 3);
        assert_eq!(after.scrolls.offset(Pane::Notes), 5);
        assert!(
            after.row_details.contains(&Detail::Row(target)),
            "expansion returns"
        );
        assert!(after.pending_resume.is_none() && !after.reload_blocked());

        // Applied once: a later refresh only follows the cursor.
        select(&mut after, "x");
        after.apply(product(&["x", "d", "a", "c", "b"]));
        assert_eq!(selected_name(&after), "x");
    }

    #[test]
    fn a_row_that_is_gone_starts_where_a_normal_start_would_and_state_is_consumed() {
        let mut before = loaded(&["a", "b", "c"]);
        select(&mut before, "c");
        before.search = "c".into();
        let resume = arrive(&before.resume().unwrap());

        let mut after = App::new(Some("product".into()));
        after.carry(resume);
        let normal = loaded(&["a", "b"]).selected;
        after.apply(product(&["a", "b"]));
        assert_eq!(after.selected, normal, "the departed row is not guessed");
        assert_eq!(after.search, "c", "other state still returns");
        assert!(after.pending_resume.is_none());
    }

    #[test]
    fn state_for_a_view_that_failed_is_dropped_rather_than_applied_later() {
        let mut before = loaded(&["a", "b", "c"]);
        select(&mut before, "c");
        let resume = arrive(&before.resume().unwrap());

        let mut after = App::new(Some("product".into()));
        after.carry(resume);
        let mut failed = product(&["a"]);
        failed.view = Err("Core is unreachable".into());
        after.apply(failed);
        assert!(after.pending_resume.is_none());
        after.apply(product(&["a", "b", "c"]));
        assert_eq!(
            after.selected,
            loaded(&["a", "b", "c"]).selected,
            "a later load starts normally"
        );
    }

    #[test]
    fn the_admitted_tabs_and_a_fold_come_back() {
        let mut before = loaded(&["a", "b"]);
        before.picks = Picks::from_keys(Some(vec!["product".into(), "infra".into()]));
        before
            .folds
            .get_mut("product")
            .unwrap()
            .overrides
            .insert(Pane::Detail, true);
        let resume = arrive(&before.resume().unwrap());
        assert_eq!(resume.folds, [(Pane::Detail, true)]);

        let mut after = App::new(Some("product".into()));
        after.carry(resume);
        assert_eq!(
            after.picks.keys(),
            Some(vec!["infra".into(), "product".into()])
        );
        after.apply(product(&["a", "b"]));
        assert_eq!(
            after.folds["product"].overrides.get(&Pane::Detail),
            Some(&true)
        );
    }
}
