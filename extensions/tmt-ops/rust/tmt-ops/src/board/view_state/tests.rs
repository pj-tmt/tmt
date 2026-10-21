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
        controls: Some(Controls::capture(&board())),
        scrolls: Vec::new(),
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
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
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
    assert_eq!(value.as_object().unwrap().len(), 7);
    assert!(value.get("search").is_none());
    assert_eq!(value["controls"]["query"], "");
    assert_eq!(value["scrolls"], json!([]));
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
        assert_eq!(app.shell.tab, OpsTab::Agents);
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
    assert_eq!(app.workspace.selected, Some(OpsTarget::DigestAll));
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
        |_, _, _, _| {},
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
        |_| {},
        || Ok(()),
        || Ok(()),
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

#[test]
fn layer_names_admit_only_bounded_filename_components() {
    let root = Path::new("/isolated/board");
    assert_eq!(Layer::default().path(root), root.join("view.json"));
    assert_eq!(Layer::parse("default").unwrap(), Layer::default());
    assert_eq!(
        Layer::parse("popup-2").unwrap().path(root),
        root.join("views/popup-2.json")
    );
    for bad in [
        "",
        "../default",
        "a/b",
        "A",
        "popup.json",
        "two words",
        "é",
        "a\n",
    ] {
        assert!(Layer::parse(bad).is_err(), "{bad:?}");
    }
    assert!(Layer::parse(&"a".repeat(32)).is_ok());
    assert!(Layer::parse(&"a".repeat(33)).is_err());
}

#[test]
fn layers_share_by_name_and_reset_only_the_selected_file() {
    let scratch = Scratch::new();
    let default = Layer::default().path(&scratch.0);
    let popup = Layer::parse("popup").unwrap().path(&scratch.0);
    let untouched = saved().bytes().unwrap();
    write(&default, &saved());
    let mut first = board();
    first.workspace.filter = super::super::workspace::AgentsFilter::Pinned;
    first.search = "committed".into();
    first.workspace.query = first.search.to_string();
    first.workspace.order = Some(Order {
        agents: crate::config::OpsSort::Members,
        reversed: true,
        overview: Some(crate::config::OverviewSort::initial(
            crate::config::OverviewColumn::Cron,
        )),
    });
    first.scrolls.restore([(crate::config::Pane::Rows, 9)]);
    let mut session = Session::load(popup.clone(), false, false);
    assert!(!popup.exists(), "opening never creates a file");
    session.flush(&mut first, Instant::now());
    assert_eq!(
        fs::read(&default).unwrap(),
        untouched,
        "other layer stays byte-identical"
    );
    let mut second = cold();
    let mut reopened = Session::load(
        Layer::parse("popup").unwrap().path(&scratch.0),
        false,
        false,
    );
    assert!(reopened.tick(&mut second, Instant::now()).0);
    assert_eq!(second.search, "committed");
    assert_eq!(second.workspace.filter, first.workspace.filter);
    assert_eq!(second.workspace_order(), first.workspace_order());
    assert_eq!(second.scrolls.offset(crate::config::Pane::Rows), 9);
    reset(&popup).unwrap();
    assert!(!popup.exists());
    assert_eq!(fs::read(&default).unwrap(), untouched);
    reset(&popup).unwrap();
    let mut empty = Session::load(popup.clone(), false, false);
    let mut third = cold();
    assert!(!empty.tick(&mut third, Instant::now()).0);
    assert_eq!(third.search, "");
    assert_eq!(third.workspace.order, None);
}

#[test]
fn old_default_v1_hint_needs_no_migration_and_retains_config_sort_defaults() {
    let scratch = Scratch::new();
    let mut old: Value = serde_json::from_slice(&saved().bytes().unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("controls");
    old.as_object_mut().unwrap().remove("scrolls");
    let bytes = serde_json::to_vec(&old).unwrap();
    cache::replace(&scratch.file(), &bytes).unwrap();
    let mut session = Session::load(scratch.file(), false, false);
    assert_eq!(
        fs::read(scratch.file()).unwrap(),
        bytes,
        "read does not migrate"
    );
    let mut app = cold();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .sort = crate::config::OpsSort::Members;
    assert!(session.tick(&mut app, Instant::now()).0);
    assert_eq!(
        app.workspace_order().agents,
        crate::config::OpsSort::Members
    );
    assert_eq!(app.workspace.selected, saved().selected);
    assert!(
        app.workspace.order.is_none(),
        "no retained order overrides authored defaults"
    );
}

#[test]
fn search_draft_and_invalid_controls_never_replace_a_good_hint() {
    let scratch = Scratch::new();
    write(&scratch.file(), &saved());
    let bytes = fs::read(scratch.file()).unwrap();
    let mut session = Session::load(scratch.file(), false, false);
    let mut app = board();
    session.tick(&mut app, Instant::now());
    app.searching = true;
    app.search = "draft".into();
    app.workspace.query = app.search.to_string();
    session.flush(&mut app, Instant::now());
    assert_eq!(fs::read(scratch.file()).unwrap(), bytes);
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    for (key, bad) in [
        ("query", json!("a".repeat(4097))),
        ("agentsFilter", json!("invented")),
        ("reversed", json!(1)),
        ("sort", json!("random")),
    ] {
        let mut corrupt = value.clone();
        corrupt["controls"][key] = bad;
        assert!(ViewState::decode(&corrupt).is_none(), "{key}");
    }
    for bad in [
        json!([["Rows", 1], ["Rows", 2]]),
        json!([["invented", 1]]),
        json!([["Rows", -1]]),
    ] {
        let mut corrupt = value.clone();
        corrupt["scrolls"] = bad;
        assert!(ViewState::decode(&corrupt).is_none());
    }
}

#[test]
fn original_reset_flag_is_never_replayed_after_exec_and_reset_errors_preserve_bytes() {
    let scratch = Scratch::new();
    write(&scratch.file(), &saved());
    let bytes = fs::read(scratch.file()).unwrap();
    let _exec = Session::open(scratch.file(), true, false, true).unwrap();
    assert_eq!(fs::read(scratch.file()).unwrap(), bytes);
    let _cold = Session::open(scratch.file(), false, false, true).unwrap();
    assert!(!scratch.file().exists());
    fs::create_dir_all(scratch.file()).unwrap();
    let child = scratch.file().join("foreign");
    fs::write(&child, b"preserve").unwrap();
    assert!(Session::open(scratch.file(), false, false, true).is_err());
    assert_eq!(fs::read(child).unwrap(), b"preserve");
}

#[test]
fn replaced_overview_filter_does_not_erase_other_layer_controls() {
    let mut app = board();
    app.search = "committed".into();
    app.workspace.filter = super::super::workspace::AgentsFilter::Pinned;
    let mut encoded = Controls::capture(&app).json();
    encoded["overviewFilter"] = json!("needs-you");
    let decoded =
        Controls::decode(&encoded).expect("old controls retain query, sort and Agents filter");
    assert_eq!(decoded.query, "committed");
    assert_eq!(
        decoded.agents_filter,
        super::super::workspace::AgentsFilter::Pinned
    );
    let current = decoded.json();
    assert!(current.get("overviewView").is_none());
    assert!(current.get("overviewFilter").is_none());
    // The Overview tab's saved Squads/Agents mode is ignored; the chip is `agentsFilter`.
    encoded["overviewView"] = json!("squads");
    assert_eq!(
        Controls::decode(&encoded).unwrap().agents_filter,
        super::super::workspace::AgentsFilter::Pinned
    );
}

#[test]
fn retired_agents_attention_filter_loads_as_all_without_discarding_other_controls() {
    let mut app = board();
    app.search = "worker".into();
    let mut value = Controls::capture(&app).json();
    value["agentsFilter"] = json!("needs-you");
    let controls = Controls::decode(&value).unwrap();
    assert_eq!(
        controls.agents_filter,
        super::super::workspace::AgentsFilter::All
    );
    assert_eq!(controls.query, "worker");
    assert_eq!(controls.json()["agentsFilter"], "all");
}

#[test]
fn pane_ratios_restore_from_the_selected_named_layer_and_exec_controls() {
    use crate::action::ResizeBoundary as Id;
    let scratch = Scratch::new();
    let left = Layer::parse("wide").unwrap().path(&scratch.0);
    let right = Layer::parse("review").unwrap().path(&scratch.0);
    let now = Instant::now();
    let mut first = board();
    first
        .pane_resize
        .restore([(Id::AgentsColumns, 7000)].into());
    Session::load(left.clone(), false, false).flush(&mut first, now);
    first
        .pane_resize
        .restore([(Id::AgentsColumns, 4500)].into());
    Session::load(right.clone(), false, false).flush(&mut first, now);
    let right_bytes = fs::read(&right).unwrap();
    let mut next = cold();
    assert!(
        Session::load(left.clone(), false, false)
            .tick(&mut next, now)
            .0
    );
    assert_eq!(
        next.pane_resize.ratios().get(&Id::AgentsColumns),
        Some(&7000)
    );
    assert_eq!(fs::read(&right).unwrap(), right_bytes);
    let directory = scratch.0.join("reload");
    next.resume().unwrap().write(&directory, 17, 1_000).unwrap();
    let resume = crate::board::resume::Resume::take(&directory, 17, 1_000).unwrap();
    let mut exec = board();
    exec.carry(resume);
    let fresh = exec.view.take().unwrap();
    let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    snapshot.view = Ok(fresh);
    exec.apply(snapshot);
    assert_eq!(exec.pane_resize.ratios(), next.pane_resize.ratios());
    assert!(Session::open(left.clone(), false, false, true).is_ok());
    assert!(!left.exists());
    assert_eq!(fs::read(&right).unwrap(), right_bytes);
}

#[test]
fn quota_window_is_layer_local_survives_exec_and_resets_with_only_its_layer() {
    use crate::board::limits::QuotaWindow;
    let scratch = Scratch::new();
    let left = Layer::parse("quota-short").unwrap().path(&scratch.0);
    let right = Layer::parse("quota-weekly").unwrap().path(&scratch.0);
    let now = Instant::now();
    let mut first = board();
    let token_request = first.retained_history_request();
    first
        .usage_chart
        .borrow_mut()
        .restore_quota_window(QuotaWindow::Short);
    assert_eq!(first.retained_history_request(), token_request);
    Session::load(left.clone(), false, false).flush(&mut first, now);
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&left).unwrap()).unwrap()["controls"]["quotaWindow"],
        "5h"
    );
    first
        .usage_chart
        .borrow_mut()
        .restore_quota_window(QuotaWindow::Weekly);
    Session::load(right.clone(), false, false).flush(&mut first, now);
    let right_bytes = fs::read(&right).unwrap();

    let mut next = cold();
    assert!(
        Session::load(left.clone(), false, false)
            .tick(&mut next, now)
            .0
    );
    assert_eq!(next.usage_chart.borrow().quota_window(), QuotaWindow::Short);
    assert_eq!(fs::read(&right).unwrap(), right_bytes);
    let directory = scratch.0.join("reload");
    next.resume().unwrap().write(&directory, 17, 1_000).unwrap();
    let resume = crate::board::resume::Resume::take(&directory, 17, 1_000).unwrap();
    let mut exec = board();
    exec.carry(resume);
    let fresh = exec.view.take().unwrap();
    let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    snapshot.view = Ok(fresh);
    exec.apply(snapshot);
    assert_eq!(exec.usage_chart.borrow().quota_window(), QuotaWindow::Short);

    Session::open(left.clone(), false, false, true).unwrap();
    assert!(!left.exists());
    let mut reset = cold();
    assert!(!Session::load(left, false, false).tick(&mut reset, now).0);
    assert_eq!(
        reset.usage_chart.borrow().quota_window(),
        QuotaWindow::Weekly
    );
    assert_eq!(fs::read(&right).unwrap(), right_bytes);
}

#[test]
fn missing_or_unknown_quota_choice_keeps_other_committed_controls() {
    use crate::board::limits::QuotaWindow;
    let mut app = board();
    app.search = "worker".into();
    app.workspace.filter = crate::board::workspace::AgentsFilter::All;
    app.usage_chart
        .borrow_mut()
        .restore_quota_window(QuotaWindow::Short);
    let controls = Controls::capture(&app);
    assert_eq!(Controls::decode(&controls.json()), Some(controls.clone()));
    let mut expected = controls.clone();
    expected.quota_window = QuotaWindow::Weekly;
    let mut old = controls.json();
    old.as_object_mut().unwrap().remove("quotaWindow");
    assert_eq!(Controls::decode(&old), Some(expected.clone()));
    for unknown in [Value::Null, json!(7), json!("future-window"), json!({})] {
        let mut value = controls.json();
        value["quotaWindow"] = unknown;
        assert_eq!(Controls::decode(&value), Some(expected.clone()));
    }
}

#[test]
fn startup_tab_precedence_is_flag_then_preference_then_retained_view() {
    use crate::config::StartupTab;
    let fixed = [
        (StartupTab::Agents, OpsTab::Agents),
        (StartupTab::Digest, OpsTab::Digest),
    ];
    assert_eq!(startup_destination(None, StartupTab::Last, false), None);
    for (preference, expected) in fixed {
        assert_eq!(startup_destination(None, preference, false), Some(expected));
        for (flag, tab) in fixed {
            assert_eq!(
                startup_destination(Some(flag), preference, false),
                Some(tab)
            );
            assert_eq!(startup_destination(Some(flag), preference, true), None);
        }
    }
    let scratch = Scratch::new();
    write(&scratch.file(), &saved());
    for (_, tab) in fixed {
        let mut app = cold();
        app.workspace_startup = Some(crate::board::app::WorkspaceStartup {
            room_id: Some(ROOM.into()),
            tab: Some(tab),
        });
        app.workspace_reconcile();
        assert_eq!(
            app.shell.tab, tab,
            "explicit tab beats scoped Agents launch"
        );
        assert!(app.workspace_startup.is_none());
        let mut session = Session::load(scratch.file(), false, false);
        session.explicit_startup();
        assert!(session.prepare(&mut app).is_some());
        assert_eq!(
            app.shell.tab, tab,
            "saved Agents tab cannot override fixed startup"
        );
        app.shell.tab = OpsTab::Agents;
        app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
        app.workspace_reconcile();
        assert_eq!(app.shell.tab, OpsTab::Agents, "startup is consumed once");
    }
    let mut app = cold();
    let mut session = Session::load(scratch.file(), false, false);
    session.prepare(&mut app).unwrap();
    assert_eq!(
        app.shell.tab,
        OpsTab::Agents,
        "last retains the existing layer destination"
    );
}

#[test]
fn chart_preferences_are_per_layer_authored_and_tolerant_of_old_or_invalid_hints() {
    use crate::board::overview::chart::model::Each;
    for each in [Each::Auto, Each::Fixed(60)] {
        let first = board();
        first
            .usage_chart
            .borrow_mut()
            .restore_chart_controls(10_080, each);
        let controls = Controls::capture(&first);
        let encoded = controls.json();
        let decoded = Controls::decode(&encoded).unwrap();
        let mut second = board();
        decoded.apply(&mut second);
        assert_eq!(second.usage_chart.borrow().chart_controls(), (10_080, each));
        assert_eq!(decoded, controls);
        for chart in [
            None,
            Some(json!({"rangeMinutes":43_200,"eachMinutes":5})),
            Some(json!({"rangeMinutes":0,"eachMinutes":null})),
        ] {
            let mut old = encoded.clone();
            if let Some(chart) = chart {
                old["chart"] = chart;
            } else {
                old.as_object_mut().unwrap().remove("chart");
            }
            let restored = Controls::decode(&old).unwrap();
            assert_eq!(restored.chart, (60, Each::Auto));
            assert_eq!(restored.query, controls.query);
            assert_eq!(restored.order, controls.order);
        }
    }
}

fn preferences_snapshot(path: &Path) -> crate::board::app::Snapshot {
    let mut fixture = board();
    let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    snapshot.tabs.push(crate::board::ALL.into());
    snapshot.view = Ok(fixture.view.take().unwrap());
    snapshot
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .configure(&crate::config::Config::read(path.to_path_buf()).unwrap())
        .unwrap();
    snapshot
}

#[test]
fn saved_range_tok_restarts_from_confirmed_preferences_and_survives_refresh() {
    use crate::{
        board::app::{Effect, Request},
        config::{Config, TokRange},
    };
    let _no_commands = crate::runner::forbid_commands();
    for range in TokRange::ALL {
        let scratch = Scratch::new();
        fs::create_dir_all(&scratch.0).unwrap();
        let path = scratch.0.join("ops.toml");
        fs::write(&path, "# keep the viewer's preferences\n").unwrap();
        let mut first = App::new(Some(crate::board::ALL.into()));
        first.apply(preferences_snapshot(&path));
        let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) =
            first.select_tok_range(range)
        else {
            panic!("a range change must use the confirmed workspace writer");
        };
        assert_eq!(
            first.tok_range(),
            TokRange::Hour,
            "dispatch is not a saved choice"
        );
        Config::read(path.clone())
            .unwrap()
            .save_workspace_preferences(&expected, &next)
            .unwrap();
        first.finished_workspace_preferences(next, "Saved".into());
        assert_eq!(first.tok_range(), range);
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("# keep the viewer's preferences")
        );

        let mut restarted = App::new(Some(crate::board::ALL.into()));
        restarted.apply(preferences_snapshot(&path));
        for tab in [OpsTab::Agents, OpsTab::Digest] {
            restarted.shell.tab = tab;
            assert_eq!(restarted.tok_range(), range, "{tab:?} after restart");
            restarted.apply(preferences_snapshot(&path));
            assert_eq!(restarted.tok_range(), range, "{tab:?} after refresh");
        }
    }
}

#[test]
fn saved_range_refused_stale_write_does_not_change_the_viewer_or_saved_preference() {
    use crate::{
        board::app::{Effect, Request},
        config::{Config, TokRange},
    };
    let _no_commands = crate::runner::forbid_commands();
    let scratch = Scratch::new();
    fs::create_dir_all(&scratch.0).unwrap();
    let path = scratch.0.join("ops.toml");
    fs::write(&path, "").unwrap();
    let mut app = App::new(Some(crate::board::ALL.into()));
    app.apply(preferences_snapshot(&path));
    let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) =
        app.select_tok_range(TokRange::Week)
    else {
        panic!("expected a range save");
    };
    let mut other = expected.clone();
    other.tok_range = TokRange::SixHours;
    Config::read(path.clone())
        .unwrap()
        .save_workspace_preferences(&expected, &other)
        .unwrap();
    let bytes = fs::read(&path).unwrap();
    let error = Config::read(path.clone())
        .unwrap()
        .save_workspace_preferences(&expected, &next)
        .unwrap_err();
    assert!(error.message.contains("changed"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        app.tok_range(),
        TokRange::Hour,
        "a refusal is not a completion"
    );
    let mut restarted = App::new(Some(crate::board::ALL.into()));
    restarted.apply(preferences_snapshot(&path));
    assert_eq!(restarted.tok_range(), TokRange::SixHours);
}

#[test]
fn saved_range_agents_chart_restarts_through_the_selected_layer_writer() {
    use crate::board::overview::chart::model::Each;
    let _no_commands = crate::runner::forbid_commands();
    for each in [Each::Auto, Each::Fixed(60)] {
        let scratch = Scratch::new();
        let mut first = board();
        first
            .usage_chart
            .borrow_mut()
            .restore_chart_controls(10_080, each);
        let now = Instant::now();
        let mut original = Session::load(scratch.file(), false, false);
        let (_, writer) = original.tick(&mut first, now);
        writer.unwrap().publish(now, false);
        assert_eq!(
            ViewState::read(&scratch.file())
                .unwrap()
                .controls
                .unwrap()
                .chart,
            (10_080, each)
        );

        let mut restarted = cold();
        assert_eq!(
            restarted.usage_chart.borrow().chart_controls(),
            (60, Each::Auto)
        );
        let mut selected = Session::load(scratch.file(), false, false);
        assert!(selected.tick(&mut restarted, now).0);
        assert_eq!(restarted.shell.tab, OpsTab::Agents);
        assert_eq!(
            restarted.usage_chart.borrow().chart_controls(),
            (10_080, each)
        );
        restarted.apply(crate::board::app::Snapshot {
            view: Ok(board().view.take().unwrap()),
            ..crate::board::app::tests::snapshot(crate::board::ALL, json!([]))
        });
        assert_eq!(
            restarted.usage_chart.borrow().chart_controls(),
            (10_080, each),
            "refresh retains the restored chart choice"
        );

        let mut other = cold();
        Session::load(scratch.0.join("other-view.json"), false, false).tick(&mut other, now);
        assert_eq!(
            other.usage_chart.borrow().chart_controls(),
            (60, Each::Auto),
            "another layer does not inherit this chart range"
        );
    }
}
