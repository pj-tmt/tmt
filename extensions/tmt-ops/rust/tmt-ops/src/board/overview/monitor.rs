//! One identity row over the existing acquired workspace and label/meter owners.
use super::{columns::Column, cron_counts, sort};
use crate::board::{
    app::App,
    workspace::{OpsTarget, OpsTreeRow, observe::OpsWorkspaceSnapshot},
};
use std::rc::Rc;

pub(in crate::board) struct Values {
    shared: crate::board::workspace::columns::WorkspaceColumnValues,
    pub cron: Option<cron_counts::Count>,
}
impl std::ops::Deref for Values {
    type Target = crate::board::workspace::columns::WorkspaceColumnValues;
    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}
impl Values {
    fn order<'a>(&'a self, row: &'a OpsTreeRow) -> sort::Row<'a> {
        let OpsTarget::Member { identity_id, .. } = &row.target else {
            unreachable!("monitor identity")
        };
        sort::Row {
            id: identity_id,
            agent: &row.title,
            squad: self.squad.as_deref(),
            state: row.state.as_deref(),
            tokens_per_hour: self.tokens,
            digest: self.digest.as_deref(),
            cron: self.cron.map(|c| (c.active, c.total)),
            runtime: self.runtime.as_deref(),
            model: self.model.as_deref(),
        }
    }
    pub fn text(&self, column: Column, row: &OpsTreeRow) -> String {
        if column == Column::Cron {
            cron_counts::label(self.cron, false)
        } else {
            self.shared.text(column, row)
        }
    }
}

pub(in crate::board) fn values(
    app: &App,
    snapshot: &OpsWorkspaceSnapshot,
    row: &OpsTreeRow,
) -> Values {
    let OpsTarget::Member { identity_id, .. } = &row.target else {
        unreachable!("monitor identity")
    };
    let cron = app.cron.counts.count(cron_counts::CronCountScope::Owner {
        id: identity_id,
        room: None,
    });
    let shared = crate::board::workspace::columns::project(
        crate::board::workspace::columns::WorkspaceColumnInput {
            row,
            snapshot,
            labels: &app.labels,
            usage: app.overview_member_usage(identity_id),
            agent: app.references.agents.get(identity_id),
            timers: cron.map(|count| (count.active, count.total)),
        },
    )
    .expect("monitor member projection");
    Values { shared, cron }
}

#[derive(Default)]
pub(in crate::board) struct Cache {
    held: Option<(Key, Rc<Scene>)>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::board) enum Filter {
    #[default]
    All,
    NeedsYou,
}
impl Filter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::NeedsYou,
            Self::NeedsYou => Self::All,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::NeedsYou => "Needs you",
        }
    }
}
pub(in crate::board) struct Scene {
    pub rows: Rc<[OpsTreeRow]>,
    pub summary: super::summary::Summary,
    pub recent: super::events::Recent,
}
#[derive(PartialEq, Eq)]
struct Key {
    order: Option<crate::config::OverviewSort>,
    query: String,
    filter: Filter,
    pins: std::collections::BTreeSet<String>,
}
impl Cache {
    pub fn clear(&mut self) {
        self.held = None;
    }
}

pub(in crate::board) fn rows(app: &App, snapshot: &OpsWorkspaceSnapshot) -> Rc<[OpsTreeRow]> {
    Rc::clone(&scene(app, snapshot).rows)
}

pub(in crate::board) fn scene(app: &App, snapshot: &OpsWorkspaceSnapshot) -> Rc<Scene> {
    let key = Key {
        order: snapshot.preferences.overview_sort,
        query: app.workspace.query.clone(),
        filter: app.overview_filter,
        pins: snapshot.preferences.agent_pins.clone(),
    };
    let mut cache = app.overview_rows.borrow_mut();
    if let Some((old, scene)) = &cache.held
        && *old == key
    {
        return Rc::clone(scene);
    }
    // Choose a deterministic admitted membership occurrence for each UUID.
    // Cursor movement must not change row facts, ordering or rebuild this scene.
    // The existing projection owns pool/partial-roster admission.
    let rows = crate::board::digest_dashboard::rows(snapshot, &app.workspace.query, None)
        .into_iter()
        .filter(|row| key.filter == Filter::All || sort::needs_attention(row.state.as_deref()));
    let mut projected: Vec<_> = rows
        .map(|row| {
            let v = values(app, snapshot, &row);
            (row, v)
        })
        .collect();
    projected.sort_by(
        |(a, av), (b, bv)| {
            let pinned = |row: &OpsTreeRow| matches!(&row.target, OpsTarget::Member { identity_id, .. } if key.pins.contains(identity_id));
            pinned(b).cmp(&pinned(a)).then_with(|| match snapshot.preferences.overview_sort {
            Some(order) => sort::compare(&av.order(a), &bv.order(b), order),
            None => sort::needs_you(&av.order(a), &bv.order(b)),
            })
        },
    );
    let rows: Rc<[OpsTreeRow]> = projected
        .into_iter()
        .map(|(row, _)| row)
        .collect::<Vec<_>>()
        .into();
    let scene = Rc::new(Scene {
        rows,
        summary: super::summary::project(snapshot, &app.labels),
        recent: super::events::project(&app.home_leads),
    });
    cache.held = Some((key, Rc::clone(&scene)));
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        board::{
            app::{Effect, Request},
            shell::OpsTab,
            workspace::{OpsMember, OpsMembershipState},
        },
        config::{OverviewColumn, OverviewSort},
    };
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    fn board() -> App {
        let mut app = crate::board::app::workspace::tests::board();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot.roster.members.insert(
            "other".into(),
            OpsMember {
                id: "other".into(),
                name: "A lead".into(),
                state: Some("blocked".into()),
                waiting: false,
                presence: Some(false),
                usage: None,
            },
        );
        snapshot.roster.squads[0].members.push("other".into());
        snapshot.roster.squads[0].member_states.insert(
            "other".into(),
            OpsMembershipState {
                state: Some("blocked".into()),
                ..Default::default()
            },
        );
        snapshot.rows.insert(
            OpsTarget::Member {
                room_id: Some(snapshot.roster.squads[0].room_id.clone()),
                identity_id: "other".into(),
            },
            json!({"id":"other","name":"A lead","fields":{"model":"claude"}}),
        );
        app.overview_rows.borrow_mut().clear();
        app.shell.tab = OpsTab::Overview;
        app.workspace_reconcile();
        app
    }
    #[test]
    fn header_hover_uses_pointer_motion_without_sorting_or_retargeting() {
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let mut app = board();
        let selected = app.workspace.selected.clone();
        let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let area = app
            .overview_sort_hits
            .borrow()
            .iter()
            .find(|(_, column)| *column == Column::Agent)
            .unwrap()
            .0;
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Moved,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        assert_eq!(app.workspace_header_hover.get(), Some(Column::Agent));
        assert_eq!(app.workspace.selected, selected);
        assert!(
            app.workspace_snapshot()
                .unwrap()
                .preferences
                .overview_sort
                .is_none()
        );
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let shown = (area.x..area.right())
            .map(|x| terminal.backend().buffer()[(x, area.y)].symbol())
            .collect::<String>();
        assert!(shown.contains('↕'));
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Moved,
                column: area.x,
                row: area.y + 2,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        assert!(app.workspace_header_hover.get().is_none());
        assert_eq!(app.workspace.selected, selected);
        app.workspace_header_hover.set(Some(Column::Agent));
        app.invalidate_frame_geometry();
        assert!(app.workspace_header_hover.get().is_none());
        assert!(app.overview_sort_hits.borrow().is_empty());
    }

    #[test]
    fn filter_keeps_hidden_identity_and_uses_the_scoped_key_table() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = board();
        let selected = app.workspace.selected.clone();
        let before = app.workspace_rows();
        assert_eq!(before.len(), 2);
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)),
            Effect::None
        );
        assert_eq!(app.overview_filter.label(), "Needs you");
        assert_eq!(app.workspace_rows().len(), 1);
        assert_eq!(app.workspace_rows()[0].state.as_deref(), Some("blocked"));
        assert_eq!(app.workspace.selected, selected);
        assert_eq!(
            app.workspace_selection_state(),
            crate::board::workspace::SelectionState::Hidden
        );
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)),
            Effect::None
        );
        assert_eq!(app.workspace_rows().len(), 2);
        assert_eq!(app.workspace.selected, selected);
        assert_eq!(
            app.workspace_selection_state(),
            crate::board::workspace::SelectionState::Visible
        );
        let snapshot = app.view.as_mut().unwrap().home.as_mut().unwrap();
        snapshot
            .workspace
            .keys
            .get_mut(&crate::config::OpsKeyScope::Overview)
            .unwrap()
            .insert("f".into(), crate::action::Action::parse("search").unwrap());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert!(
            app.searching,
            "explicit binding wins over the filter default"
        );
        assert_eq!(app.overview_filter.label(), "All");
    }

    #[test]
    fn admitted_exchange_read_invalidates_one_shared_scene() {
        let mut app = board();
        let snapshot = app.workspace_snapshot().unwrap();
        let before = scene(&app, snapshot);
        assert!(matches!(
            before.recent,
            super::super::events::Recent::Unavailable
        ));
        assert!(Rc::ptr_eq(&before, &scene(&app, snapshot)));
        let read = crate::board::home_leads::Fetch::results(None).complete(&crate::core::Core::at(
            "/nonexistent/overview-no-source".into(),
        ));
        app.apply_home_leads(read);
        let after = scene(&app, app.workspace_snapshot().unwrap());
        assert!(!Rc::ptr_eq(&before, &after));
        assert!(matches!(
            after.recent,
            super::super::events::Recent::Unavailable
        ));
        assert_eq!(before.summary, after.summary);
    }

    #[test]
    fn cursor_movement_reuses_the_scene_without_changing_membership_facts_or_order() {
        let mut app = board();
        let before = app.workspace_rows();
        let targets = before
            .iter()
            .map(|row| row.target.clone())
            .collect::<Vec<_>>();
        let selected = app.workspace.selected.clone();
        assert_eq!(
            app.key(ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Up,
                ratatui::crossterm::event::KeyModifiers::NONE
            )),
            Effect::None
        );
        assert_ne!(
            app.workspace.selected, selected,
            "positive control: cursor actually moved"
        );
        let after = app.workspace_rows();
        assert!(
            Rc::ptr_eq(&before, &after),
            "selection-only input must not rebuild the monitor"
        );
        assert_eq!(
            after
                .iter()
                .map(|row| row.target.clone())
                .collect::<Vec<_>>(),
            targets
        );
        assert_eq!(app.workspace.selected.as_ref(), Some(&targets[0]));
    }
    #[test]
    fn monitor_consumes_persisted_order_without_retargeting_selection() {
        let mut app = board();
        let selected = app.workspace.selected.clone();
        assert_eq!(app.workspace_rows()[0].title, "A lead");
        let effect = app.overview_sort_column(OverviewColumn::Agent);
        let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) = effect else {
            panic!("sort must go through the Config lane");
        };
        assert_eq!(expected.overview_sort, None);
        assert_eq!(
            next.overview_sort,
            Some(OverviewSort::initial(OverviewColumn::Agent))
        );
        assert_eq!(app.workspace.selected, selected);
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences = next;
        let effect = app.overview_sort_column(OverviewColumn::Agent);
        let Effect::Act(Request::WorkspacePreferences { next, .. }) = effect else {
            panic!("header toggle");
        };
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences = next;
        assert_eq!(app.workspace_rows()[0].title, "worker");
        assert_eq!(app.workspace.selected, selected);
        app.workspace.query = "A lead".into();
        assert_eq!(
            app.workspace_selection_state(),
            crate::board::workspace::SelectionState::Hidden
        );
    }
    #[test]
    fn real_monitor_geometry_is_one_row_per_identity_and_clipped_at_every_width() {
        let mut app = board();
        for width in [60, 80, 100, 140, 160] {
            app.set_body_width(width);
            let mut terminal = Terminal::new(TestBackend::new(width, 28)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let hits = app.overview_sort_hits.borrow();
            assert!(!hits.is_empty());
            assert!(
                hits.iter()
                    .all(|(rect, _)| rect.height == 1 && rect.right() <= width)
            );
            assert_eq!(app.row_starts.borrow().len(), 2);
            let rows = app.workspace_rows();
            let snapshot = app.workspace_snapshot().unwrap();
            let observed = values(&app, snapshot, &rows[0]);
            assert_eq!(
                observed.runtime, None,
                "content/model cannot choose a runtime"
            );
            assert_eq!(observed.text(Column::TokensPerHour, &rows[0]), "—");
        }
    }
    #[test]
    fn stale_header_geometry_cannot_change_the_order() {
        let app = board();
        let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        assert!(!app.overview_sort_hits.borrow().is_empty());
        app.invalidate_frame_geometry();
        assert!(app.overview_sort_hits.borrow().is_empty());
        assert!(app.cron_scope_hits.borrow().is_empty());
    }
    #[test]
    fn painted_header_click_and_authored_sort_keys_use_the_same_config_requests() {
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let mut clicked = board();
        let mut keyed = board();
        let keys = keyed
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .keys
            .get_mut(&crate::config::OpsKeyScope::Overview)
            .unwrap();
        keys.insert("s".into(), crate::action::Action::parse("sort").unwrap());
        keys.insert(
            "S".into(),
            crate::action::Action::parse("reverse-sort").unwrap(),
        );
        let selected = clicked.workspace.selected.clone();
        for key in ['s', 'S'] {
            let mut terminal = Terminal::new(TestBackend::new(160, 28)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &clicked))
                .unwrap();
            let area = clicked
                .overview_sort_hits
                .borrow()
                .iter()
                .find(|(_, column)| *column == OverviewColumn::Agent)
                .unwrap()
                .0;
            let click = clicked.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                },
                std::time::Instant::now(),
            );
            let press = keyed.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
            let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) = click else {
                panic!("painted sort header must request the Config lane")
            };
            let Effect::Act(Request::WorkspacePreferences {
                expected: key_expected,
                next: key_next,
                ..
            }) = press
            else {
                panic!("sort key must request the same Config lane")
            };
            assert_eq!(expected, key_expected);
            assert_eq!(next, key_next);
            assert_eq!(clicked.workspace.selected, selected);
            assert_eq!(keyed.workspace.selected, selected);
            for app in [&mut clicked, &mut keyed] {
                app.view
                    .as_mut()
                    .unwrap()
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .preferences = next.clone();
            }
        }
    }
    #[test]
    fn long_observed_metadata_is_clipped_by_cells_but_keeps_full_values() {
        let mut app = board();
        let runtime = "fixture-runtime-with-long-name";
        // Wide fixture glyphs exercise cell geometry, not a new Core driver.
        let model = "model-界界-with-a-long-observed-label";
        let OpsTarget::Member { identity_id, .. } = app.workspace.selected.clone().unwrap() else {
            panic!("selected observed member")
        };
        app.references = crate::board::inventory::Inventory::from_listed(
            &[],
            &Ok(json!({
                "identities": [{"id":identity_id,"name":"worker","runningDriver":runtime,
                    "resume":{"model":model}}]
            })),
        );
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences
            .layout = crate::config::OpsLayout::Stack;
        let mut clipped = [false; 2];
        for width in [60, 80, 100, 140, 160] {
            let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let rows = app.workspace_rows();
            let index = rows
                .iter()
                .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
                .unwrap();
            let observed = values(&app, app.workspace_snapshot().unwrap(), &rows[index]);
            assert_eq!(observed.runtime.as_deref(), Some(runtime));
            assert_eq!(observed.model.as_deref(), Some(model));
            assert_eq!(app.row_starts.borrow().len(), 2);

            let row_y = app
                .hits
                .borrow()
                .iter()
                .find(|hit| hit.row() == Some(index))
                .unwrap()
                .y;
            for (at, column) in [OverviewColumn::Runtime, OverviewColumn::Model]
                .into_iter()
                .enumerate()
            {
                let area = app
                    .overview_sort_hits
                    .borrow()
                    .iter()
                    .find(|(_, c)| *c == column)
                    .map(|(area, _)| *area);
                if let Some(area) = area {
                    let text = (area.x..area.right())
                        .map(|x| terminal.backend().buffer()[(x, row_y)].symbol())
                        .collect::<String>();
                    assert!(text.contains('…'), "{column:?}: {text}");
                    clipped[at] = true;
                }
            }
        }
        assert_eq!(clipped, [true, true]);
    }
    #[test]
    fn every_repaint_replaces_sort_and_timer_hit_geometry() {
        let app = board();
        for width in [100, 160] {
            let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let sorts = app.overview_sort_hits.borrow().clone();
            let counts = app.cron_scope_hits.borrow().len();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            assert_eq!(*app.overview_sort_hits.borrow(), sorts);
            assert_eq!(app.cron_scope_hits.borrow().len(), counts);
            assert!(sorts.iter().all(|(area, _)| area.right() <= width));
            if width == 160 {
                assert!(
                    sorts
                        .iter()
                        .any(|(_, column)| *column == OverviewColumn::Squad)
                );
                assert!(
                    sorts
                        .iter()
                        .any(|(_, column)| *column == OverviewColumn::Runtime)
                );
                assert!(
                    sorts
                        .iter()
                        .any(|(_, column)| *column == OverviewColumn::Model)
                );
            }
        }
    }
    #[test]
    fn saved_pins_rebuild_the_scene_and_divider_hits_keep_uuid_rows() {
        use crate::board::app::HitTarget;
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = board();
        let old = app.workspace_rows();
        assert_eq!(old[0].title, "A lead");
        let pin = old
            .iter()
            .find(|row| row.title != "A lead")
            .unwrap()
            .target
            .clone();
        let OpsTarget::Member { identity_id, .. } = &pin else {
            unreachable!()
        };
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences
            .agent_pins
            .insert(identity_id.clone());
        let pinned = app.workspace_rows();
        assert!(!Rc::ptr_eq(&old, &pinned));
        assert_eq!(pinned[0].target, pin);
        assert!(Rc::ptr_eq(&pinned, &app.workspace_rows()));
        let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        assert_eq!(*app.row_starts.borrow(), [0, 2]);
        let hits: Vec<_> = app
            .hits
            .borrow()
            .iter()
            .filter_map(|hit| {
                if let HitTarget::Row(index) = hit.target {
                    Some((index, hit.x, hit.y))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(hits.len(), 2);
        let header_y = app
            .overview_sort_hits
            .borrow()
            .iter()
            .find(|(_, column)| *column == Column::Agent)
            .unwrap()
            .0
            .y;
        assert_eq!(
            hits[0].2,
            header_y + 2,
            "first pin immediately follows header and divider, with no blank row"
        );
        assert_eq!(hits[1].2, hits[0].2 + 2);
        let selected = app.workspace.selected.clone();
        let outcome = app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: hits[0].1,
                row: hits[0].2 + 1,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        assert_eq!(outcome, Effect::None);
        assert_eq!(app.workspace.selected, selected);
        assert_eq!(app.workspace_rows()[hits[1].0].title, "A lead");
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences
            .agent_pins
            .insert("00000000-0000-4000-8000-000000000000".into());
        assert_eq!(app.workspace_rows().len(), 2);
    }
    #[test]
    fn pin_requests_use_agent_identity_and_success_only_sanitized_text() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = board();
        let row = app
            .workspace_rows()
            .iter()
            .find(|row| row.title != "A lead")
            .unwrap()
            .target
            .clone();
        app.workspace.selected = Some(row.clone());
        let OpsTarget::Member { identity_id, .. } = row else {
            unreachable!()
        };
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot.roster.members.get_mut(&identity_id).unwrap().name = "Safe\nname\u{1b}[31m".into();
        app.overview_rows.borrow_mut().clear();
        let selected = app.workspace.selected.clone();
        for (word, should_pin) in [("Pinned", true), ("Unpinned", false)] {
            let effect = app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
            let Effect::Act(Request::WorkspacePreferences { next, saved, .. }) = effect else {
                panic!("pin requires the existing save lane")
            };
            assert_eq!(saved.as_deref(), Some(format!("{word} Safename").as_str()));
            assert_eq!(next.agent_pins.contains(&identity_id), should_pin);
            assert!(next.pinned.is_empty());
            assert_eq!(app.workspace.selected, selected);
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .preferences = next;
            app.overview_rows.borrow_mut().clear();
        }
        app.workspace.query = "no matching agent".into();
        assert!(!matches!(
            app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)),
            Effect::Act(_)
        ));
        assert_eq!(app.workspace.selected, selected);
    }
    #[test]
    fn navigation_crosses_the_decorative_divider_without_selecting_it() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = board();
        let pin = app
            .workspace_rows()
            .iter()
            .find(|row| row.title != "A lead")
            .unwrap()
            .target
            .clone();
        let OpsTarget::Member { identity_id, .. } = &pin else {
            unreachable!()
        };
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences
            .agent_pins
            .insert(identity_id.clone());
        app.workspace.selected = Some(pin.clone());
        app.workspace_reconcile();
        let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let last = app.workspace_rows()[1].target.clone();
        for (key, target) in [
            (KeyCode::Down, last.clone()),
            (KeyCode::Up, pin.clone()),
            (KeyCode::End, last.clone()),
            (KeyCode::Home, pin.clone()),
            (KeyCode::PageDown, last),
            (KeyCode::PageUp, pin),
        ] {
            assert_eq!(
                app.key(KeyEvent::new(key, KeyModifiers::NONE)),
                Effect::None
            );
            assert_eq!(app.workspace.selected, Some(target));
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            assert_eq!(*app.row_starts.borrow(), [0, 2]);
        }
    }
}

#[cfg(test)]
mod legend_tests {
    use crate::board::{app::workspace::tests::board, shell::OpsTab, view::footer};
    use crate::{action::Action, config::OpsKeyScope};
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn idle_composer_keeps_overview_legend_and_input_owners_hide_it_without_losing_draft() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for owner in ["typing", "modal", "search", "chart", "replies"] {
            let mut app = board();
            app.shell.tab = OpsTab::Overview;
            let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
            let draw = |terminal: &mut Terminal<TestBackend>, app: &crate::board::app::App| {
                terminal
                    .draw(|frame| crate::board::view::render(frame, app))
                    .unwrap();
            };
            draw(&mut terminal, &app);
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            app.paste("retained Overview draft");
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            // Inspect the idle hints after the transient Escape notice is dismissed.
            app.notice = None;
            draw(&mut terminal, &app);
            assert!(app.input.is_none() && app.input_band.get().is_some());
            let key = app.composer_projection().unwrap().key.unwrap();
            let [first, second] = footer::overview_legend(&app, 160).unwrap();
            assert!(first.contains("⏎ chat") && second.contains("? help"));
            let lines: Vec<String> = terminal
                .backend()
                .buffer()
                .content
                .chunks(160)
                .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                .collect();
            assert!(lines[..43].iter().any(|line| line.contains("⏎ chat")));
            assert_eq!(lines[43].trim_end(), second);
            assert!(!lines[43].contains("⏎ chat"));
            let event = match owner {
                "typing" => KeyCode::Enter,
                "modal" => KeyCode::Char('?'),
                "search" => KeyCode::Char('/'),
                "chart" => KeyCode::Char('u'),
                "replies" => KeyCode::Tab,
                _ => unreachable!(),
            };
            app.key(KeyEvent::new(event, KeyModifiers::NONE));
            match owner {
                "typing" => assert!(app.input.is_some()),
                "modal" => assert!(app.modal_open()),
                "search" => assert!(app.searching),
                "chart" => assert!(app.usage_chart.borrow().focused()),
                "replies" => assert_eq!(app.focused_pane(), Some(crate::config::Pane::Replies)),
                _ => unreachable!(),
            }
            draw(&mut terminal, &app);
            assert!(footer::overview_legend(&app, 160).is_none(), "{owner}");
            if owner == "typing" {
                app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            }
            let draft = app.composer_projection().unwrap();
            assert_eq!(draft.key.as_ref(), Some(&key), "{owner}");
            assert_eq!(
                draft.text.unwrap().as_str(),
                "retained Overview draft",
                "{owner}"
            );
        }
    }

    #[test]
    fn overview_legend_is_disjoint_and_names_only_live_bindings() {
        let mut app = board();
        app.shell.tab = OpsTab::Overview;
        let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let [first, second] = footer::overview_legend(&app, 160).unwrap();
        assert!(first.contains("↑↓ select") && first.contains("⏎ chat") && first.contains("p pin"));
        assert!(
            second.contains("/ search")
                && second.contains("f needs you")
                && second.contains("u chart")
        );
        assert!(
            !first.contains("f needs you")
                && !first.contains("u chart")
                && !second.contains("p pin")
        );
        let keys = &mut app
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .keys;
        keys.get_mut(&OpsKeyScope::Overview)
            .unwrap()
            .insert("enter".into(), Action::parse("inspect").unwrap());
        keys.get_mut(&OpsKeyScope::Overview)
            .unwrap()
            .insert("/".into(), Action::parse("copy {name}").unwrap());
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let [first, second] = footer::overview_legend(&app, 160).unwrap();
        assert!(first.contains("⏎ inspect") && !first.contains("t chat"));
        assert!(!first.contains("chat") && !second.contains("/ search"));
        for width in [60, 80, 100, 160] {
            let [first, second] = footer::overview_legend(&app, width).unwrap();
            assert!(unicode_width::UnicodeWidthStr::width(first.as_str()) <= width);
            assert!(unicode_width::UnicodeWidthStr::width(second.as_str()) <= width);
            assert!(second.contains("? help") && second.contains("q quit"));
        }
    }
}
