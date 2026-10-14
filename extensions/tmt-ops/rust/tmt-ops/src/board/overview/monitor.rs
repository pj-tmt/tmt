//! One identity row over the existing acquired workspace and label/meter owners.
use super::{columns::Column, cron_counts, sort};
use crate::board::{
    app::App,
    workspace::{OpsTarget, OpsTreeRow, observe::OpsWorkspaceSnapshot},
};
use std::rc::Rc;

pub(in crate::board) struct Values {
    pub squad: Option<String>,
    pub tokens: Option<f64>,
    pub partial: bool,
    pub digest: Option<String>,
    pub cron: Option<cron_counts::Count>,
    pub runtime: Option<String>,
    pub model: Option<String>,
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
        match column {
            Column::Agent => row.title.clone(),
            Column::Squad => self.squad.clone().unwrap_or_else(|| "—".into()),
            Column::State => row.state.clone().unwrap_or_else(|| "—".into()),
            Column::TokensPerHour => self.tokens.map_or_else(
                || "—".into(),
                |n| {
                    format!(
                        "{}{}",
                        if self.partial { "~" } else { "" },
                        crate::source::tokens(n)
                    )
                },
            ),
            Column::Digest => self.digest.clone().unwrap_or_else(|| "—".into()),
            Column::Cron => cron_counts::label(self.cron, false),
            Column::Runtime => self.runtime.clone().unwrap_or_else(|| "—".into()),
            Column::Model => self.model.clone().unwrap_or_else(|| "—".into()),
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
    let squad = match &row.target {
        OpsTarget::Member {
            room_id: Some(room),
            ..
        } => snapshot
            .roster
            .squads
            .iter()
            .find(|s| &s.room_id == room)
            .map(|s| s.name.clone()),
        _ => Some("Common pool".into()),
    };
    let observation = app.overview_member_usage(identity_id);
    let observed = app.references.agents.get(identity_id);
    Values {
        squad,
        tokens: observation.map(|r| r.tokens as f64),
        partial: observation.is_some_and(|r| r.partial),
        digest: app
            .labels
            .of(identity_id)
            .filter(|(source, _)| *source == "digest")
            .map(|(_, label)| label.text.clone())
            .reduce(|a, b| format!("{a} · {b}")),
        cron: app.cron.counts.count(cron_counts::CronCountScope::Owner {
            id: identity_id,
            room: None,
        }),
        runtime: observed.and_then(|a| a.runtime.clone()),
        model: observed.and_then(|a| a.model.clone()),
    }
}

#[derive(Default)]
pub(in crate::board) struct Cache {
    held: Option<(Key, Rc<[OpsTreeRow]>)>,
}
#[derive(PartialEq, Eq)]
struct Key {
    order: Option<crate::config::OverviewSort>,
    query: String,
}
impl Cache {
    pub fn clear(&mut self) {
        self.held = None;
    }
}

pub(in crate::board) fn rows(app: &App, snapshot: &OpsWorkspaceSnapshot) -> Rc<[OpsTreeRow]> {
    let key = Key {
        order: snapshot.preferences.overview_sort,
        query: app.workspace.query.clone(),
    };
    let mut cache = app.overview_rows.borrow_mut();
    if let Some((old, rows)) = &cache.held
        && *old == key
    {
        return Rc::clone(rows);
    }
    // Choose a deterministic admitted membership occurrence for each UUID.
    // Cursor movement must not change row facts, ordering or rebuild this scene.
    // The existing projection owns pool/partial-roster admission.
    let rows = crate::board::digest_dashboard::rows(snapshot, &app.workspace.query, None);
    let mut projected: Vec<_> = rows
        .into_iter()
        .map(|row| {
            let v = values(app, snapshot, &row);
            (row, v)
        })
        .collect();
    projected.sort_by(
        |(a, av), (b, bv)| match snapshot.preferences.overview_sort {
            Some(order) => sort::compare(&av.order(a), &bv.order(b), order),
            None => sort::needs_you(&av.order(a), &bv.order(b)),
        },
    );
    let rows: Rc<[OpsTreeRow]> = projected
        .into_iter()
        .map(|(row, _)| row)
        .collect::<Vec<_>>()
        .into();
    cache.held = Some((key, Rc::clone(&rows)));
    rows
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
        let Effect::Act(Request::WorkspacePreferences { expected, next }) = effect else {
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
    fn painted_header_click_and_sort_keys_use_the_same_config_requests() {
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let mut clicked = board();
        let mut keyed = board();
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
            let Effect::Act(Request::WorkspacePreferences { expected, next }) = click else {
                panic!("painted sort header must request the Config lane")
            };
            let Effect::Act(Request::WorkspacePreferences {
                expected: key_expected,
                next: key_next,
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
            if width == 160 {
                for column in [OverviewColumn::Runtime, OverviewColumn::Model] {
                    let area = app
                        .overview_sort_hits
                        .borrow()
                        .iter()
                        .find(|(_, c)| *c == column)
                        .unwrap()
                        .0;
                    let text = (area.x..area.right())
                        .map(|x| {
                            terminal.backend().buffer()[(x, area.y + 1 + index as u16)].symbol()
                        })
                        .collect::<String>();
                    assert!(text.contains('…'), "{column:?}: {text}");
                }
            }
        }
    }
}
