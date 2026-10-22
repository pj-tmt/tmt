//! What an in-place reload carries across: this session's view state, captured
//! from the `App` and put back on the first fresh view of the same tab. Nothing
//! here is data about squads; selection is an identity looked up in whatever the
//! new process loads, and a row that is gone falls back to the usual start.

use super::{App, RowTarget};
use crate::board::resume::Resume;

impl App {
    /// Cold-start hints use the same carry seam for retained controls and scroll.
    /// Composer drafts stay in this process; admission requires a fresh complete view.
    pub(in crate::board) fn carry_view_hint(
        &mut self,
        hint: &crate::board::view_state::ViewState,
        explicit_startup: bool,
    ) {
        let Some(mut resume) = self.resume() else {
            return;
        };
        resume.squads_expanded = hint.expanded.iter().cloned().collect();
        resume.pool_expanded = hint.pool_expanded;
        resume.controls = hint.controls.clone();
        resume.scrolls = hint.scrolls.clone();
        if !explicit_startup {
            resume.shell = hint.shell;
            resume.selected = hint.selected.clone().map(RowTarget::Workspace);
            // A proven-gone target has the destination's ordinary cold default.
            if hint.selected.is_none() {
                self.workspace.selected = None;
            }
        }
        self.carry(resume);
        self.restore_pending();
    }

    /// Whether replacing the process now would lose something the user has open
    /// or is waiting on: a draft, an overlay, a search being typed, a request in
    /// flight, a view still loading or a resumed state not yet applied.
    pub(in crate::board) fn reload_blocked(&self) -> bool {
        self.current.is_none()
            || self.pane_resize.dragging()
            || self.loading_since.is_some()
            || self.pending_resume.is_some()
            || self.in_flight > 0
            || self.input.is_some()
            || self.workspace_form.is_some()
            || self.composer.has_drafts()
            || self.status_draft.is_some()
            || self.cron_draft.is_some()
            || self.searching
            || self.settings.is_some()
            || self.quota_details.is_some()
            || self.monitor_details.is_some()
            || self.cron_review.is_some()
            || self.usage_controls
            || (self.usage_chart_available() && self.usage_chart.borrow().focused())
            || self.help
            || self.menu.is_some()
            || self.theme_picker.is_some()
            || self.panels_picker.is_some()
            || self.cron_list.is_some()
            || self.checklist_shown()
            || self.recovery.as_ref().is_some_and(|c| c.blocks_reload())
            || self.discovery.as_ref().is_some_and(|c| c.reload_blocked())
            || self.discovered_uuid.is_some()
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
            layer: self.view_layer.clone(),
            controls: Some(crate::board::view_state::Controls::capture(self)),
            visibility: self.workspace.visibility.clone(),
            shell: self.shell.tab,
            squads_expanded: self.workspace.expanded.iter().cloned().collect(),
            pool_expanded: self.workspace.pool_expanded,
            selected: if self.workspace_snapshot().is_some() {
                self.workspace.selected.clone().map(RowTarget::Workspace)
            } else {
                self.row_target(self.selected)
            },
            focus: self.focus,
            follow: self.follow,
            scrolls: self.scrolls.offsets(),
            expanded: self.row_details.expanded.clone(),
            search: self.search.to_string(),
            tab,
        })
    }

    /// Takes a reload's state in at startup; it applies when the first fresh view of
    /// its tab (the one `App::new` was given) lands.
    pub(in crate::board) fn carry(&mut self, resume: Resume) {
        // A reload's carried scope, including an explicitly cleared scope,
        // supersedes the original argv selection before the first projection.
        self.workspace_startup = None;
        self.view_layer = resume.layer.clone();
        self.workspace.visibility = resume.visibility.clone();
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
        self.search = resume.search.into();
        if let Some(controls) = resume.controls {
            controls.apply(self);
        }
        self.shell.tab = resume.shell;
        self.workspace.expanded = resume.squads_expanded.into_iter().collect();
        self.workspace.pool_expanded = resume.pool_expanded;
        self.row_details.expanded = resume.expanded;
        self.focus = resume.focus.min(self.focus_panes().len().saturating_sub(1));
        self.scrolls.restore(resume.scrolls);
        self.follow = resume.follow;
        match resume.selected {
            Some(RowTarget::Workspace(target)) => {
                self.workspace.selected = Some(target);
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
    fn agent_details_holds_reload_until_the_user_closes_it() {
        let mut app = crate::board::app::workspace::tests::board();
        assert!(!app.reload_blocked());
        app.overview_details();
        assert!(app.monitor_details.is_some());
        assert!(app.reload_blocked(), "open agent details must defer reload");
        press(&mut app, KeyCode::Esc);
        assert!(app.monitor_details.is_none());
        assert!(!app.reload_blocked(), "closed details release the reload");
    }

    #[test]
    fn timer_review_holds_reload_until_the_user_closes_it() {
        let mut app = crate::board::app::workspace::tests::board();
        assert!(!app.reload_blocked());
        let request = crate::board::cronboard::CronRequest::Add {
            actor: crate::board::cronboard::test_cron(vec![], tmt_ops::cron::ClockStatus::NoClock)
                .actor
                .unwrap(),
            squad: "product".into(),
            room_id: crate::board::app::workspace::tests::ROOM.into(),
            owner: crate::board::app::workspace::tests::MEMBER.into(),
            message: "Review this work".into(),
            schedule: "every 30m".into(),
            zone: "UTC".into(),
        };
        assert!(matches!(
            app.cron_review_request(request),
            crate::board::app::Effect::Act(crate::board::app::Request::CronPreview(_))
        ));
        assert!(app.cron_review.is_some());
        assert_eq!(app.in_flight, 0, "the open review alone must defer reload");
        assert!(app.reload_blocked(), "open timer review must defer reload");
        press(&mut app, KeyCode::Esc);
        assert!(app.cron_review.is_none());
        assert!(!app.reload_blocked(), "closed review releases the reload");
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
                    draft_key: None,

                    inline_quotes: Default::default(),

                    row_send: None,
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
                app.row_details.reader = Some(crate::board::row_detail::Reader::new(
                    Detail::Job("c1".into()),
                    None,
                ));
            }),
            ("carried state not yet applied", |app| {
                app.carry(Resume {
                    layer: Default::default(),
                    controls: None,
                    visibility: Default::default(),
                    shell: crate::board::shell::OpsTab::Agents,
                    squads_expanded: vec![],
                    pool_expanded: false,
                    tab: "product".into(),
                    selected: None,
                    focus: 0,
                    follow: true,
                    scrolls: Vec::new(),
                    expanded: Vec::new(),
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
            after.row_details.expanded.contains(&Detail::Row(target)),
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
    fn workspace_reload_keeps_reader_target_tree_expansion_and_scroll_without_tab_picks() {
        use crate::board::{
            ALL,
            app::workspace::tests::{ROOM, board},
            shell::OpsTab,
        };
        let mut before = board();
        before.focus = 1;
        before.follow = false;
        before.workspace.pool_expanded = true;
        before
            .scrolls
            .restore([(Pane::Rows, 3), (Pane::Replies, 5)]);
        let target = before.workspace.selected.clone();
        let resume = arrive(&before.resume().unwrap());
        assert_eq!(resume.tab, ALL);
        let mut after = App::new(Some(ALL.into()));
        after.carry(resume);
        let mut fresh = board();
        let mut snapshot = snapshot(ALL, json!([]));
        snapshot.tabs = fresh.tabs.clone();
        snapshot.view = Ok(fresh.view.take().unwrap());
        after.apply(snapshot);
        assert_eq!(after.shell.tab, OpsTab::Agents);
        assert_eq!(after.workspace.selected, target);
        assert!(after.workspace.expanded.contains(ROOM));
        assert!(after.workspace.pool_expanded);
        assert_eq!(
            after.focused(),
            Pane::Replies,
            "authored CLI pane count cannot reset the current Reader focus"
        );
        assert_eq!(after.scrolls.offset(Pane::Rows), 3);
        assert_eq!(after.scrolls.offset(Pane::Replies), 5);
        assert!(!after.follow);
        assert!(after.pending_resume.is_none());
        assert!(!after.reload_blocked());
    }
    #[test]
    fn named_layer_controls_and_session_scope_survive_real_exec_snapshot_round_trip() {
        use crate::board::{
            app::workspace::tests::{ROOM, board},
            view_state::{Layer, Order},
            workspace::AgentsFilter,
        };
        let mut first = board();
        first.view_layer = Layer::parse("popup").unwrap();
        first.workspace.visibility.scope = vec![ROOM.into()];
        first.workspace.visibility.reveal_hidden = true;
        first.search = "worker".into();
        first.workspace_reconcile();
        first.workspace.order = Some(Order {
            agents: crate::config::OpsSort::Attention,
            reversed: true,
            overview: None,
        });
        first.workspace.filter = AgentsFilter::Pinned;
        let arrived = arrive(&first.resume().unwrap());
        let mut next = board();
        next.carry(arrived);
        next.restore_pending();
        assert_eq!(next.view_layer, first.view_layer);
        assert_eq!(next.workspace_order(), first.workspace_order());
        assert_eq!(next.search, "worker");
        assert_eq!(next.workspace.visibility, first.workspace.visibility);
        assert_eq!(next.workspace.filter, first.workspace.filter);
    }
}
