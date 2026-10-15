use super::*;
use crate::board::{
    app::workspace::tests::{MEMBER, ROOM, board},
    workspace::OpsObservation,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("ops-view-state-{}", crate::id::new_v4().unwrap())))
    }
    fn file(&self) -> PathBuf {
        self.0.join("view.json")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn saved() -> ViewState {
    ViewState {
        shell: OpsTab::Agents,
        expanded: [ROOM.into()].into(),
        pool_expanded: true,
        selected: Some(OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        }),
    }
}
fn cold() -> App {
    let mut app = board();
    app.shell.tab = OpsTab::Overview;
    app.workspace.expanded.clear();
    app.workspace.pool_expanded = false;
    app.workspace.selected = None;
    app.workspace_reconcile();
    app
}
fn write(path: &Path, state: &ViewState) {
    cache::replace(path, &state.bytes().unwrap()).unwrap();
}

#[test]
fn restart_round_trip_restores_the_exact_view_without_names_or_drafts() {
    let scratch = Scratch::new();
    let mut first = board();
    first.workspace.pool_expanded = true;
    let mut original = Session::load(scratch.file(), false, false);
    let now = Instant::now();
    let (restored, writer) = original.tick(&mut first, now);
    assert!(!restored);
    writer.unwrap().publish(now, false);
    let bytes = fs::read(scratch.file()).unwrap();
    assert!(!String::from_utf8(bytes.clone()).unwrap().contains("worker"));
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert!(value.get("search").is_none() && value.get("scrolls").is_none());
    let mut next = cold();
    let mut reopened = Session::load(scratch.file(), false, false);
    let (restored, job) = reopened.tick(&mut next, now);
    assert!(restored);
    assert!(
        job.is_none(),
        "unchanged restored projection writes nothing"
    );
    assert_eq!(ViewState::capture(&next), Some(saved()));
    assert_eq!(
        fs::metadata(scratch.file()).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn latest_pending_value_wins_without_a_queue_of_cursor_moves() {
    let scratch = Scratch::new();
    let writer = Writer::new(scratch.file(), None);
    let now = Instant::now();
    assert!(writer.offer(saved(), now));
    let mut latest = saved();
    latest.shell = OpsTab::Digest;
    assert!(!writer.offer(latest.clone(), now), "one outstanding job");
    writer.publish(now, false);
    assert_eq!(ViewState::read(&scratch.file()), Some(latest.clone()));
    let inode = fs::metadata(scratch.file()).unwrap().ino();
    assert!(!writer.offer(latest.clone(), now + SAVE_EVERY));
    writer.publish(now + SAVE_EVERY, false);
    assert_eq!(fs::metadata(scratch.file()).unwrap().ino(), inode);
    latest.pool_expanded = false;
    assert!(!writer.offer(latest.clone(), now + Duration::from_millis(1999)));
    assert!(writer.offer(latest.clone(), now + SAVE_EVERY));
    writer.publish(now + SAVE_EVERY, false);
    assert_eq!(ViewState::read(&scratch.file()), Some(latest));
    assert_eq!(
        fs::read_dir(&scratch.0).unwrap().count(),
        1,
        "no staging files"
    );
}

#[test]
fn failed_publication_preserves_the_old_file_and_retries_on_a_later_tick() {
    let scratch = Scratch::new();
    let now = Instant::now();
    write(&scratch.file(), &saved());
    let original = fs::read(scratch.file()).unwrap();
    let writer = Writer::new(scratch.file(), Some(saved()));
    let mut next = saved();
    next.shell = OpsTab::Digest;
    assert!(writer.offer(next.clone(), now));
    writer.publish_with(now, false, |path, bytes| {
        assert!(
            writer.0.try_lock().is_ok(),
            "input cannot wait behind file I/O"
        );
        cache::replace_if(path, bytes, || Err(io::ErrorKind::Interrupted.into()))
    });
    assert_eq!(fs::read(scratch.file()).unwrap(), original);
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
    assert!(!writer.offer(next.clone(), now + Duration::from_secs(1)));
    assert!(writer.offer(next.clone(), now + SAVE_EVERY));
    writer.publish(now + SAVE_EVERY, false);
    assert_eq!(ViewState::read(&scratch.file()), Some(next));
}

#[test]
fn a_change_during_publication_is_retained_for_the_next_tick() {
    let scratch = Scratch::new();
    let now = Instant::now();
    let writer = Writer::new(scratch.file(), None);
    assert!(writer.offer(saved(), now));
    let mut next = saved();
    next.shell = OpsTab::Digest;
    writer.publish_with(now, false, |path, bytes| {
        assert!(writer.0.try_lock().is_ok());
        assert!(!writer.offer(next.clone(), now));
        cache::replace(path, bytes)
    });
    assert_eq!(ViewState::read(&scratch.file()), Some(saved()));
    assert!(writer.offer(next.clone(), now + SAVE_EVERY));
    writer.publish(now + SAVE_EVERY, false);
    assert_eq!(ViewState::read(&scratch.file()), Some(next));
}

#[test]
fn partial_unknown_failed_and_loading_views_do_not_apply_or_clobber_the_hint() {
    for observation in [
        OpsObservation::Partial,
        OpsObservation::Unavailable,
        OpsObservation::Stale,
    ] {
        let scratch = Scratch::new();
        write(&scratch.file(), &saved());
        let original = fs::read(scratch.file()).unwrap();
        let mut app = cold();
        // The dense Overview defaults to an agent. This case tests whether a
        // pending saved hint can replace an explicit non-recipient selection.
        app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
        let initial_selection = app.workspace.selected.clone();
        assert!(app.workspace_selected_member().is_none());
        let mut session = Session::load(scratch.file(), false, false);
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .observation = observation;
        assert!(session.tick(&mut app, Instant::now()).1.is_none());
        assert!(session.hint.is_some());
        assert_eq!(app.shell.tab, OpsTab::Overview);
        assert_eq!(app.workspace.selected, initial_selection);
        assert!(
            app.workspace_selected_member().is_none(),
            "pending hint grants no recipient"
        );
        session.flush(&mut app, Instant::now());
        assert_eq!(fs::read(scratch.file()).unwrap(), original);
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .observation = OpsObservation::Complete;
        app.error = Some("Refresh failed".into());
        assert!(session.tick(&mut app, Instant::now()).1.is_none());
        app.error = None;
        app.loading_since = Some(Instant::now());
        assert!(session.tick(&mut app, Instant::now()).1.is_none());
        app.loading_since = None;
        assert!(session.tick(&mut app, Instant::now()).0);
        assert_eq!(ViewState::capture(&app), Some(saved()));
    }
    let scratch = Scratch::new();
    write(&scratch.file(), &saved());
    let mut app = App::default();
    let mut session = Session::load(scratch.file(), false, false);
    assert!(!session.tick(&mut app, Instant::now()).0);
    session.flush(&mut app, Instant::now());
    assert_eq!(ViewState::read(&scratch.file()), Some(saved()));
}

#[test]
fn removed_membership_or_room_has_the_cold_default_only_after_complete() {
    for remove in ["member", "membership", "room"] {
        let scratch = Scratch::new();
        write(&scratch.file(), &saved());
        let mut app = cold();
        let mut session = Session::load(scratch.file(), false, false);
        let roster = &mut app
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster;
        roster.observation = OpsObservation::Partial;
        match remove {
            "member" => {
                roster.members.remove(MEMBER);
            }
            "membership" => {
                roster.squads[0].members.clear();
            }
            _ => roster.squads.clear(),
        }
        assert!(session.tick(&mut app, Instant::now()).1.is_none());
        assert_eq!(session.hint.as_ref().unwrap().selected, saved().selected);
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .observation = OpsObservation::Complete;
        let (_, writer) = session.tick(&mut app, Instant::now());
        assert_eq!(app.workspace.selected, Some(OpsTarget::Pool));
        assert!(app.workspace_selected_member().is_none());
        writer.unwrap().publish(Instant::now(), false);
        let after = ViewState::read(&scratch.file()).unwrap();
        assert_eq!(after.selected, Some(OpsTarget::Pool));
        assert_eq!(after.expanded.contains(ROOM), remove != "room");
    }
}

#[test]
fn a_collapsed_parent_remains_collapsed_and_the_existing_target_is_hidden() {
    let scratch = Scratch::new();
    let mut hint = saved();
    hint.expanded.clear();
    write(&scratch.file(), &hint);
    let mut app = cold();
    let mut session = Session::load(scratch.file(), false, false);
    assert!(session.tick(&mut app, Instant::now()).0);
    assert!(app.workspace.expanded.is_empty());
    assert_eq!(app.workspace.selected, hint.selected);
    assert_eq!(
        app.workspace_selection_state(),
        crate::board::workspace::SelectionState::Hidden
    );
    assert!(!app.workspace_selection_visible());
}

#[test]
fn rename_and_reordering_do_not_change_the_saved_uuid_or_room_context() {
    let scratch = Scratch::new();
    write(&scratch.file(), &saved());
    let mut app = cold();
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    workspace.roster.members.get_mut(MEMBER).unwrap().name = "renamed agent".into();
    workspace.roster.squads[0].name = "Zulu squad".into();
    let mut earlier = workspace.roster.squads[0].clone();
    earlier.room_id = "44444444-4444-4444-8444-444444444444".into();
    earlier.name = "Alpha squad".into();
    earlier.members.clear();
    workspace.roster.squads.insert(0, earlier);
    workspace.rows.get_mut(&saved().selected.unwrap()).unwrap()["name"] = json!("renamed agent");
    let mut session = Session::load(scratch.file(), false, false);
    assert!(session.tick(&mut app, Instant::now()).0);
    assert_eq!(app.workspace.selected, saved().selected);
    assert!(app.workspace_selection_visible());
    assert_eq!(ViewState::capture(&app), Some(saved()));
}

#[test]
fn a_proven_gone_digest_target_uses_its_ordinary_visible_cold_default() {
    let scratch = Scratch::new();
    let mut hint = saved();
    hint.shell = OpsTab::Digest;
    write(&scratch.file(), &hint);
    let mut app = cold();
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let other = "44444444-4444-4444-8444-444444444444";
    let mut member = workspace.roster.members.remove(MEMBER).unwrap();
    member.id = other.into();
    member.name = "another visible agent".into();
    workspace.roster.members.insert(other.into(), member);
    workspace.roster.squads[0].members = vec![other.into()];
    workspace.rows.remove(&saved().selected.unwrap());
    workspace.rows.insert(
        OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: other.into(),
        },
        json!({"id":other,"name":"another visible agent","state":"working"}),
    );
    let mut session = Session::load(scratch.file(), false, false);
    assert!(session.tick(&mut app, Instant::now()).0);
    assert_eq!(app.shell.tab, OpsTab::Digest);
    assert_eq!(
        app.workspace.selected,
        Some(OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: other.into(),
        })
    );
    assert!(app.workspace_selection_visible());
}

#[test]
fn exec_resume_and_explicit_startup_win_over_the_saved_destination_and_target() {
    let scratch = Scratch::new();
    write(&scratch.file(), &saved());
    let mut app = cold();
    app.shell.tab = OpsTab::Digest;
    let mut exec = Session::load(scratch.file(), true, false);
    assert!(!exec.tick(&mut app, Instant::now()).0);
    assert_eq!(app.shell.tab, OpsTab::Digest);
    let mut app = cold();
    app.shell.tab = OpsTab::Agents;
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    let mut explicit = Session::load(scratch.file(), false, true);
    assert!(explicit.tick(&mut app, Instant::now()).0);
    assert_eq!(app.workspace.selected, Some(OpsTarget::Squad(ROOM.into())));
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert!(app.workspace.expanded.contains(ROOM));
}

#[test]
fn missing_corrupt_oversized_nonregular_and_foreign_hints_are_cache_misses() {
    let scratch = Scratch::new();
    assert!(ViewState::read(&scratch.file()).is_none());
    cache::replace(&scratch.file(), b"{").unwrap();
    assert!(ViewState::read(&scratch.file()).is_none());
    let mut value: Value = serde_json::from_slice(&saved().bytes().unwrap()).unwrap();
    value["version"] = json!(2);
    assert!(ViewState::decode(&value).is_none());
    value["version"] = json!(1);
    value["squadsExpanded"] = json!(["not-a-uuid"]);
    assert!(ViewState::decode(&value).is_none());
    value["squadsExpanded"] = json!(vec![ROOM; MAX_ROOMS + 1]);
    assert!(ViewState::decode(&value).is_none());
    value["squadsExpanded"] = json!([ROOM, ROOM]);
    assert_eq!(ViewState::decode(&value).unwrap().expanded.len(), 1);
    value["selected"] =
        json!({"kind":"member","tab":"name","section":0,"squad":"name","id":MEMBER});
    assert!(
        ViewState::decode(&value).is_none(),
        "no legacy/name-addressed target wire form"
    );
    cache::replace(&scratch.file(), &vec![b' '; MAX_BYTES as usize + 1]).unwrap();
    assert!(ViewState::read(&scratch.file()).is_none());
    fs::remove_file(scratch.file()).unwrap();
    let other = scratch.0.join("foreign.json");
    write(&other, &saved());
    symlink(&other, scratch.file()).unwrap();
    assert!(ViewState::read(&scratch.file()).is_none());
    fs::remove_file(scratch.file()).unwrap();
    fs::create_dir(scratch.file()).unwrap();
    assert!(ViewState::read(&scratch.file()).is_none());
}

#[test]
fn a_clean_exit_flushes_the_last_changed_projection_inside_the_coalescing_window() {
    let scratch = Scratch::new();
    write(&scratch.file(), &saved());
    let mut app = board();
    app.workspace.pool_expanded = true;
    let mut session = Session::load(scratch.file(), false, false);
    let now = Instant::now();
    session.tick(&mut app, now);
    app.shell.tab = OpsTab::Digest;
    session.flush(&mut app, now);
    assert_eq!(
        ViewState::read(&scratch.file()).unwrap().shell,
        OpsTab::Digest
    );
}

#[test]
fn background_publication_does_not_become_an_action_or_repaint() {
    use crate::board::{BoardEvent, lane};
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::{cell::Cell, sync::atomic::AtomicUsize, sync::mpsc::channel};

    let scratch = Scratch::new();
    let mut app = board();
    app.view.as_mut().unwrap().refresh = None;
    let mut state = Session::load(scratch.file(), false, false);
    let (events, input) = channel();
    // Both inputs are already queued; a broken publication cannot hang waiting
    // for a mocked worker to finish the chain.
    events.send(BoardEvent::ViewStatePublished).unwrap();
    events
        .send(BoardEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        ))))
        .unwrap();
    let paints = Cell::new(0);
    let publications = Cell::new(0);
    crate::board::session(
        &mut app,
        &AtomicUsize::new(0),
        &input,
        |_, _, _| {},
        |_, _| {},
        |_| {},
        |job| match job {
            lane::Job::ViewState(writer) => {
                publications.set(publications.get() + 1);
                writer.publish(Instant::now(), false);
            }
            _ => panic!("cache persistence cannot submit a user action"),
        },
        |_| {},
        Some(&mut state),
        |app| {
            assert_eq!(app.in_flight, 0);
            assert!(app.notice.is_none());
            paints.set(paints.get() + 1);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(publications.get(), 1);
    assert_eq!(paints.get(), 1, "publication completion is paint-neutral");
    assert!(ViewState::read(&scratch.file()).is_some());
}
