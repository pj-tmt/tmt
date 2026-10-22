//! Controller authority and public-port regressions; no host server or provider.
use super::*;
use model::{Decode, Inventory};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

const SOCKET: &str = "/tmp/private-workspace/socket";
const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
#[test]
fn app_save_query_owns_typing_but_its_action_list_allows_destination_keys() {
    use crate::board::{app::Effect, shell::OpsTab};
    let mut app = crate::board::app::workspace::tests::board();
    let mut c = controller();
    c.screen = Screen::Save;
    app.recovery = Some(c);
    for ch in "q123".chars() {
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)),
            Effect::None
        );
    }
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert_eq!(
        app.recovery.as_ref().unwrap().form.borrow().picker.query(),
        Some("q123")
    );
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(!app.recovery.as_ref().unwrap().owns_typing());
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE)),
        Effect::None
    );
    assert_eq!(app.shell.tab, OpsTab::Digest);
    assert!(!app.recovery_shown());
    assert!(app.recovery.as_ref().unwrap().pending.is_none());
    assert_eq!(
        app.recovery.as_ref().unwrap().form.borrow().picker.query(),
        Some("q123")
    );
}
fn preview_value(name: Option<&str>) -> Value {
    json!({"version":1,"name":name,"digest":DIGEST,"snapshot":{"version":1,"capturedAtMs":1000,
        "server":{"socket":SOCKET},"sessions":[{"id":"$1","name":"work","windows":[{"index":0,"window":"@1","active":true}]}],
        "windows":[{"id":"@1","name":"editor","layout":"layout bytes","visibleLayout":"visible bytes","width":80,"height":24,"activePane":"%1"}],
        "panes":[{"id":"%1","window":"@1","index":0,"left":0,"top":0,"width":80,"height":24,"cwd":"/tmp/project","identity":null,"command":{"argv":["tmt","ops","ui","argument with spaces","$(touch should-not-run)"]}}]},
        "sessions":[{"session":"$1","action":"create"}],"panes":[{"pane":"%1","action":"relaunch_command","identity":null}]})
}
fn inventory_value() -> Value {
    json!({"version":1,"socket":SOCKET,"snapshots":[{"name":null,"digest":DIGEST,"capturedAtMs":1000},{"name":"saved","digest":DIGEST,"capturedAtMs":1000}]})
}
fn controller() -> Controller {
    let mut c = Controller::new(7);
    c.socket = Some(SOCKET.into());
    c.entries = model::Inventory::decode(&inventory_value())
        .unwrap()
        .snapshots;
    c.selected = Some("snapshot:saved".into());
    c.preview = Some(Preview::decode(&preview_value(Some("saved"))).unwrap());
    c.details = surface::topology(c.preview.as_ref().unwrap());
    c
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn render(c: &Controller, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| c.render(frame, picker_surface::evidence::looks()[0], frame.area()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
fn finish(c: &mut Controller, task: Task, result: Result<Outcome, SquadError>) -> Option<Task> {
    c.loaded(load::Completed { task, result })
}
fn review(c: &mut Controller) {
    c.screen = Screen::Preview;
    assert!(c.choose("review").is_none());
    assert_eq!(c.screen, Screen::Confirm);
    render(c, 160, 50);
    assert!(c.readable.get());
}
#[test]
fn restore_and_delete_fit_their_reviews_without_changing_inventory_height() {
    for (screen, id, expected) in [
        (Screen::Confirm, "confirm", "Restore"),
        (Screen::Delete, "delete-confirm", "Delete snapshot"),
    ] {
        let mut c = controller();
        c.screen = screen;
        c.confirmation = c.selection();
        let text = render(&c, 160, 80);
        assert!(text.contains(expected));
        assert_eq!(
            c.rows().iter().find(|row| row.id == id).unwrap().label,
            expected
        );
        assert!(
            c.readable.get(),
            "the complete target and choices remain visible"
        );
        let outer = c.panel.borrow().frame.as_ref().unwrap().areas.outer;
        assert!(outer.height < 38, "short review must not demand 38 rows");
        assert!(outer.width <= 120);
        c.screen = Screen::List;
        render(&c, 160, 80);
        assert_eq!(
            c.list.borrow().frame.as_ref().unwrap().areas.outer.height,
            38
        );
    }
}
#[test]
fn recovery_confirmation_keeps_escape_hint_whole_at_low_height() {
    for screen in [Screen::Confirm, Screen::Delete] {
        for width in [60, 100, 160] {
            for height in [8, 24, 36] {
                let mut c = controller();
                c.screen = screen;
                c.confirmation = c.selection();
                let mut app = crate::board::app::workspace::tests::board();
                app.recovery = Some(c);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| crate::board::view::render(frame, &app))
                    .unwrap();
                let c = app.recovery.as_ref().unwrap();
                let footer = c.panel.borrow().frame.as_ref().unwrap().areas.footer;
                assert_eq!(
                    footer.height, 1,
                    "one whole fitted hint row at {width}x{height}"
                );
                let text: String = (footer.x..footer.right())
                    .map(|x| terminal.backend().buffer()[(x, footer.y)].symbol())
                    .collect();
                assert!(text.starts_with("Esc cancel"), "{width}x{height}: {text:?}");
                assert!(text.contains("Enter select"));
            }
        }
    }
}
#[test]
fn short_recovery_review_keeps_its_subject_and_current_action_visible() {
    for (screen, subject) in [
        (Screen::Confirm, "Restore snapshot: saved"),
        (Screen::Delete, "Delete snapshot: saved"),
    ] {
        for height in [8, 10, 12] {
            let mut c = controller();
            c.screen = screen;
            c.confirmation = c.selection();
            let mut app = crate::board::app::workspace::tests::board();
            app.recovery = Some(c);
            let mut terminal = Terminal::new(TestBackend::new(60, height)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let c = app.recovery.as_ref().unwrap();
            let state = c.panel.borrow();
            let map = state.frame.as_ref().unwrap();
            assert_eq!(
                map.areas.outer,
                ratatui::layout::Rect::new(0, 0, 60, height)
            );
            assert!(map.areas.content.height > 0);
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains(subject), "{subject}: {text}");
            assert!(
                map.hits
                    .iter()
                    .any(|hit| hit.row_id.as_deref() == Some("cancel") && hit.rect.height > 0)
            );
            assert!(
                !c.readable.get(),
                "clipped required review must not authorize a write"
            );
            drop(state);
            assert!(
                app.recovery
                    .as_mut()
                    .unwrap()
                    .input(&Event::Key(KeyEvent::new(
                        KeyCode::Down,
                        KeyModifiers::NONE
                    )))
                    .is_none()
            );
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains(if screen == Screen::Delete {
                "Delete snapshot"
            } else {
                "Restore"
            }));
            assert!(text.contains(subject));
            assert!(!app.recovery.as_ref().unwrap().readable.get());
        }
    }
}
#[test]
fn cancel_selection_and_preview_have_no_restore_authority() {
    let mut c = controller();
    let task = c.select("snapshot:saved").unwrap();
    assert!(matches!(task.job, Job::Show { .. }));
    finish(
        &mut c,
        task,
        Ok(Outcome::Show(Box::new(
            Preview::decode(&preview_value(Some("saved"))).unwrap(),
        ))),
    );
    review(&mut c);
    c.input(&key(KeyCode::Esc));
    assert!(c.confirmation.is_none());
    assert!(c.pending.is_none());
    assert!(c.choose("confirm").is_none());
}
#[test]
fn confirmation_pins_all_fields_and_submits_only_once() {
    let mut c = controller();
    c.layout_only = false;
    review(&mut c);
    let task = c.choose("confirm").unwrap();
    assert_eq!(
        task.job,
        Job::Restore(Selection {
            socket: SOCKET.into(),
            name: Some("saved".into()),
            digest: DIGEST.into(),
            layout_only: false
        })
    );
    assert_eq!(
        task.job.argv(),
        vec![
            "workspace",
            "restore",
            "--socket",
            SOCKET,
            "--expect",
            DIGEST,
            "saved"
        ]
    );
    assert!(c.choose("confirm").is_none());
    assert!(c.confirmation.is_none());
}
#[test]
fn new_selection_mode_refresh_resize_and_close_invalidate_confirmation() {
    for change in ["selection", "mode", "refresh", "resize", "close"] {
        let mut c = controller();
        review(&mut c);
        match change {
            "selection" => {
                c.select("snapshot:latest");
            }
            "mode" => {
                c.choose("mode");
            }
            "refresh" => {
                c.refresh();
            }
            "resize" => c.invalidate(),
            _ => c.close(),
        }
        assert!(
            c.choose("confirm").is_none(),
            "{change} must not retain authority"
        );
    }
}
#[test]
fn low_confirmation_cannot_restore_without_readable_target() {
    let mut c = controller();
    c.choose("review");
    render(&c, 80, 10);
    assert!(!c.readable.get());
    assert!(c.choose("confirm").is_none());
    render(&c, 160, 50);
    assert!(c.choose("confirm").is_some());
}
#[test]
fn late_preview_cannot_replace_selection_or_reopen_closed_modal() {
    let mut c = controller();
    let saved = c.select("snapshot:saved").unwrap();
    assert!(c.select("snapshot:latest").is_none());
    let latest = finish(
        &mut c,
        saved,
        Ok(Outcome::Show(Box::new(
            Preview::decode(&preview_value(Some("saved"))).unwrap(),
        ))),
    )
    .unwrap();
    assert!(c.preview.is_none());
    assert_eq!(c.pending.as_ref(), Some(&latest));
    c.close();
    finish(
        &mut c,
        latest,
        Ok(Outcome::Show(Box::new(
            Preview::decode(&preview_value(None)).unwrap(),
        ))),
    );
    assert!(!c.active);
    assert!(c.preview.is_none());
}
#[test]
fn changed_guard_discards_confirmation_and_refreshes_only_a_preview() {
    let mut c = controller();
    review(&mut c);
    let task = c.choose("confirm").unwrap();
    let next = finish(
        &mut c,
        task,
        Err(SquadError::new(
            "WORKSPACE_SNAPSHOT_CHANGED",
            "Selected bytes changed",
        )),
    )
    .unwrap();
    assert!(matches!(next.job, Job::Show { .. }));
    assert!(c.confirmation.is_none());
    assert!(c.preview.is_none());
    assert!(c.choose("confirm").is_none());
}
#[test]
fn lost_restore_stays_unknown_after_close_reopen_and_is_never_replayed() {
    let mut c = controller();
    review(&mut c);
    let task = c.choose("confirm").unwrap();
    c.close();
    assert!(c.open().is_none());
    let inspection = finish(
        &mut c,
        task,
        Err(SquadError::new("SQUAD_CORE_UNAVAILABLE", "Outcome unknown")),
    )
    .unwrap();
    assert!(matches!(inspection.job, Job::Show { .. }));
    c.close();
    assert!(c.open().is_none());
    assert!(c.unknown.is_some());
    assert_eq!(c.pending.as_ref(), Some(&inspection));
    assert!(
        finish(
            &mut c,
            inspection,
            Ok(Outcome::Show(Box::new(
                Preview::decode(&preview_value(Some("saved"))).unwrap()
            )))
        )
        .is_none()
    );
    assert!(c.pending.is_none());
    assert!(render(&c, 160, 50).contains("Unknown"));
}
#[test]
fn duplicate_name_and_cap_do_not_hide_errors_or_evict_saves() {
    let mut c = controller();
    let task = c.issue(Job::Save {
        socket: SOCKET.into(),
        name: "saved".into(),
    });
    finish(
        &mut c,
        task,
        Err(SquadError::new(
            "WORKSPACE_SNAPSHOT_EXISTS",
            "Already saved",
        )),
    );
    assert_eq!(c.entries.len(), 2);
    assert_eq!(
        c.failure.as_ref().unwrap().code,
        "WORKSPACE_SNAPSHOT_EXISTS"
    );
    for i in 0..15 {
        c.entries.push(RecoverySnapshotSummary {
            name: Some(format!("saved-{i}")),
            digest: DIGEST.into(),
            captured_at_ms: 1000,
        });
    }
    assert!(c.rows().iter().find(|r| r.id == "save").unwrap().disabled);
    assert!(render(&c, 160, 50).contains("16 named snapshots retained"));
}
#[test]
fn unnamed_generations_are_a_valid_inventory_and_only_real_names_must_be_unique() {
    // `tmt workspace ls --json` lists every retained generation with `name: null` (#4465).
    let with = |unnamed: usize, named: usize| {
        let mut v = inventory_value();
        let entries = v["snapshots"].as_array_mut().unwrap();
        entries.clear();
        for _ in 0..unnamed {
            entries.push(json!({"name":null,"digest":OTHER,"capturedAtMs":2000}));
        }
        for i in 0..named {
            entries.push(json!({"name":format!("saved-{i}"),"digest":OTHER,"capturedAtMs":3000}));
        }
        v
    };
    let valid = |v: &Value| {
        model::Inventory::decode(v)
            .unwrap()
            .validate(Some(SOCKET))
            .is_ok()
    };
    // Ben's list: five unnamed generations beside named saves.
    assert!(valid(&with(5, 2)));
    assert!(valid(&with(2, 0)));
    // The most the CLI lists: every retained generation beside the 16 named saves.
    assert!(valid(&with(35, 16)));
    assert!(!valid(&with(36, 0)), "more generations than are retained");
    assert!(!valid(&with(0, 17)), "more named saves than are kept");
    // A real name still appears once, whatever surrounds it.
    let mut v = with(3, 2);
    v["snapshots"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"saved-0","digest":OTHER,"capturedAtMs":4000}));
    assert!(!valid(&v));
}
#[test]
fn malformed_inventory_topology_and_foreign_socket_never_gain_confirmation() {
    let mut v = inventory_value();
    v["snapshots"][1]["name"] = json!("../foreign");
    assert!(
        model::Inventory::decode(&v)
            .unwrap()
            .validate(None)
            .is_err()
    );
    let mut v = preview_value(Some("saved"));
    v["snapshot"]["server"]["socket"] = json!("/tmp/foreign");
    assert!(
        Preview::decode(&v)
            .unwrap()
            .validate(SOCKET, Some("saved"))
            .is_err()
    );
    let mut v = preview_value(Some("saved"));
    v["panes"][0]["pane"] = json!("%other");
    assert!(
        Preview::decode(&v)
            .unwrap()
            .validate(SOCKET, Some("saved"))
            .is_err()
    );
}
#[test]
fn untrusted_topology_is_inert_and_exact_argv_remains_separate() {
    let raw = "title\x1b]52;c;data\x07\r\x08\u{202e}bad";
    let mut v = preview_value(Some("saved"));
    v["snapshot"]["sessions"][0]["name"] = json!(raw);
    v["snapshot"]["panes"][0]["cwd"] = json!(raw);
    let mut c = controller();
    c.preview = Some(Preview::decode(&v).unwrap());
    c.details = surface::topology(c.preview.as_ref().unwrap());
    c.screen = Screen::Preview;
    let source = &c.preview.as_ref().unwrap().snapshot;
    assert_eq!(source.sessions[0].name, raw);
    assert_eq!(
        source.panes[0].command.as_ref().unwrap().argv[4],
        "$(touch should-not-run)"
    );
    let shown = render(&c, 160, 50);
    assert!(!shown.contains('\x1b') && !shown.contains('\x07') && !shown.contains('\u{202e}'));
    assert!(shown.contains("\\u{1b}"));
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ops-recovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn core(&self, value: &Value, success: bool) -> crate::core::Core {
        let exe = self.0.join("core");
        std::fs::write(self.0.join("reply.json"), value.to_string()).unwrap();
        crate::test_support::write_ready_executable(
            &exe,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat '{}'\nexit {}\n",
                self.0.join("argv").display(),
                self.0.join("reply.json").display(),
                if success { 0 } else { 1 }
            ),
        );
        crate::core::Core::at(exe)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn restore_value(digest: &str, status: &str) -> Value {
    json!({"version":1,"socket":SOCKET,"name":"saved","digest":digest,"layoutOnly":true,"status":status,
        "sessions":[{"recorded":"$1","name":"work","action":"created","native":"$7","retainedBootstrap":"%9"}],
        "windows":[{"recorded":"@1","native":"@7"}],"panes":[{"recorded":"%1","native":"%7"}],
        "failures":if status=="partial"{vec!["link failed"]}else{vec![]}})
}
#[test]
fn public_client_preserves_partial_nonzero_report_and_exact_layout_guard() {
    let f = Fixture::new();
    let core = f.core(&restore_value(DIGEST, "partial"), false);
    let task = Task {
        owner: 1,
        serial: 1,
        job: Job::Restore(Selection {
            socket: SOCKET.into(),
            name: Some("saved".into()),
            digest: DIGEST.into(),
            layout_only: true,
        }),
    };
    let result = load::complete(&core, task);
    let Ok(Outcome::Restored(r)) = result.result else {
        panic!("partial report must survive exit 1");
    };
    assert_eq!(r.failures, vec!["link failed"]);
    assert_eq!(r.sessions[0].retained_bootstrap.as_deref(), Some("%9"));
    assert_eq!(
        std::fs::read_to_string(f.0.join("argv")).unwrap(),
        format!(
            "workspace\nrestore\n--socket\n{SOCKET}\n--expect\n{DIGEST}\n--layout-only\nsaved\n--json\n"
        )
    );
}

#[test]
fn full_restore_retains_live_skip_needs_you_and_uncertain_startup_per_item() {
    let f = Fixture::new();
    let mut value = restore_value(DIGEST, "partial");
    value["layoutOnly"] = json!(false);
    value["sessions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"recorded":"$2","name":"existing","action":"skip_existing","native":"$8","retainedBootstrap":null}));
    value["revival"] = json!([
        {"recorded":"%1","native":"%7","name":"moved","status":"skipped","message":"skipped (already live elsewhere)"},
        {"recorded":"%2","native":null,"name":"renamed","status":"needs_you","message":"needs you (identity name changed; resume it explicitly)"},
        {"recorded":"%3","native":null,"name":"retired","status":"needs_you","message":"needs you (original identity is missing)"},
        {"recorded":"%4","native":null,"name":"stale","status":"needs_you","message":"needs you (stale conversation; explicit retry required)"},
        {"recorded":"%5","native":"%9","name":"launch","status":"needs_you","message":"needs you (startup refused or uncertain: startup result lost)"}
    ]);
    let core = f.core(&value, false);
    let task = Task {
        owner: 1,
        serial: 1,
        job: Job::Restore(Selection {
            socket: SOCKET.into(),
            name: Some("saved".into()),
            digest: DIGEST.into(),
            layout_only: false,
        }),
    };
    let result = load::complete(&core, task);
    let Ok(Outcome::Restored(r)) = result.result else {
        panic!("partial full report")
    };
    let lines = surface::results(&r);
    assert!(
        lines
            .iter()
            .any(|s| s.contains("Not applied · session existing"))
    );
    assert!(
        lines
            .iter()
            .any(|s| s.contains("Not applied") && s.contains("moved"))
    );
    // Core's needs_you includes both admission refusal and uncertain startup.
    // Preserve its detail without inferring effect certainty from message text.
    for name in ["renamed", "retired", "stale"] {
        assert!(
            lines
                .iter()
                .any(|s| s.contains("Unknown") && s.contains("needs you") && s.contains(name))
        );
    }
    assert!(
        lines
            .iter()
            .any(|s| s.contains("Unknown") && s.contains("startup result lost"))
    );
    assert_eq!(
        std::fs::read_to_string(f.0.join("argv")).unwrap(),
        format!("workspace\nrestore\n--socket\n{SOCKET}\n--expect\n{DIGEST}\nsaved\n--json\n")
    );
}
#[test]
fn restore_matching_positive_control_rejects_substituted_digest_and_exit_status() {
    for (digest, status, success, accepted) in [
        (DIGEST, "completed", true, true),
        (OTHER, "completed", true, false),
        (DIGEST, "partial", true, false),
    ] {
        let f = Fixture::new();
        let core = f.core(&restore_value(digest, status), success);
        let task = Task {
            owner: 1,
            serial: 1,
            job: Job::Restore(Selection {
                socket: SOCKET.into(),
                name: Some("saved".into()),
                digest: DIGEST.into(),
                layout_only: true,
            }),
        };
        assert_eq!(load::complete(&core, task).result.is_ok(), accepted);
    }
}
#[test]
fn unsupported_and_empty_are_distinct_public_results() {
    let f = Fixture::new();
    let core = f.core(
        &json!({"error":{"code":"WORKSPACE_HOST_UNSUPPORTED","message":"tmux only"}}),
        false,
    );
    let task = Task {
        owner: 1,
        serial: 1,
        job: Job::List(None),
    };
    assert_eq!(
        load::complete(&core, task).result.unwrap_err().code,
        "WORKSPACE_HOST_UNSUPPORTED"
    );
    let f = Fixture::new();
    let core = f.core(&json!({"version":1,"socket":SOCKET,"snapshots":[]}), true);
    let task = Task {
        owner: 1,
        serial: 1,
        job: Job::List(None),
    };
    let mut c = Controller::new(1);
    c.pending = Some(task.clone());
    assert!(c.loaded(load::complete(&core, task)).is_none());
    assert!(c.entries.is_empty());
    assert!(c.failure.is_none());
    assert!(c.notice.contains("No snapshots"));
}

#[test]
fn repeated_navigation_and_refresh_keep_only_one_active_and_one_latest_read() {
    let mut c = controller();
    let active = c.select("snapshot:saved").unwrap();
    for _ in 0..100 {
        assert!(c.select("snapshot:latest").is_none());
        assert!(c.refresh().is_none());
    }
    assert_eq!(c.pending.as_ref(), Some(&active));
    let next = finish(
        &mut c,
        active,
        Err(SquadError::new("OLD_READ", "superseded")),
    )
    .unwrap();
    assert_eq!(next.job, Job::List(Some(SOCKET.into())));
    assert!(c.failure.is_none());
    assert!(c.requested_read.is_none());
}
#[test]
fn detail_pages_keep_every_recorded_item_and_never_create_jobs() {
    let mut c = controller();
    c.screen = Screen::Preview;
    c.details = (0..401)
        .map(|i| format!("Pane source-{i} cwd /tmp/path-{i}"))
        .collect();
    render(&c, 160, 50);
    assert!(c.choose("previous-details").is_none());
    assert_eq!(c.detail_page, 0);
    assert!(c.choose("next-details").is_none());
    assert_eq!(c.detail_page, 1);
    assert!(c.choose("next-details").is_none());
    assert_eq!(c.detail_page, 2);
    assert!(
        c.rows()
            .iter()
            .find(|r| r.id == "next-details")
            .unwrap()
            .disabled
    );
    assert!(c.pending.is_none());
    assert_eq!(
        c.details.last().unwrap(),
        "Pane source-400 cwd /tmp/path-400"
    );
}
#[test]
fn selecting_saved_after_list_refresh_keeps_component_and_controller_target_equal() {
    let mut c = controller();
    let task = c.refresh().unwrap();
    finish(
        &mut c,
        task,
        Ok(Outcome::List(
            model::Inventory::decode(&inventory_value()).unwrap(),
        )),
    );
    assert_eq!(c.selected.as_deref(), Some("snapshot:saved"));
    assert_eq!(
        c.list.borrow().picker.list.selected(),
        Some("snapshot:saved")
    );
}

#[test]
fn closed_report_reopens_without_reading_or_replaying_and_unknown_blocks_reload() {
    let mut c = controller();
    review(&mut c);
    let task = c.choose("confirm").unwrap();
    let result = model::Restore::decode(&restore_value(DIGEST, "completed")).unwrap();
    finish(&mut c, task, Ok(Outcome::Restored(result)));
    c.close();
    assert!(c.open().is_none());
    assert_eq!(c.screen, Screen::Results);
    assert!(c.restored.is_some());
    assert!(render(&c, 160, 50).contains("Applied"));
    c.unknown = Some("Original result unknown".into());
    c.close();
    assert!(c.blocks_reload());
}

#[test]
fn missing_named_snapshot_refreshes_once_without_a_read_retry_loop() {
    let mut c = controller();
    review(&mut c);
    let restore = c.choose("confirm").unwrap();
    let fresh = finish(
        &mut c,
        restore,
        Err(SquadError::new(
            "WORKSPACE_SNAPSHOT_NOT_FOUND",
            "Named save missing",
        )),
    )
    .unwrap();
    assert!(matches!(fresh.job, Job::Show { .. }));
    assert!(
        finish(
            &mut c,
            fresh,
            Err(SquadError::new(
                "WORKSPACE_SNAPSHOT_NOT_FOUND",
                "Named save still missing"
            ))
        )
        .is_none()
    );
    assert!(c.pending.is_none());
    assert!(c.confirmation.is_none());
    assert!(c.notice.contains("Not applied"));
}
#[test]
fn missing_latest_or_ambiguous_restore_failure_inspects_once_and_retains_unknown() {
    let mut c = controller();
    c.selected = Some("snapshot:latest".into());
    c.preview = Some(Preview::decode(&preview_value(None)).unwrap());
    review(&mut c);
    let restore = c.choose("confirm").unwrap();
    let fresh = finish(
        &mut c,
        restore,
        Err(SquadError::new(
            "WORKSPACE_RESTORE_UNAVAILABLE",
            "Could not acquire latest",
        )),
    )
    .unwrap();
    assert_eq!(
        fresh.job,
        Job::Show {
            socket: SOCKET.into(),
            name: None
        }
    );
    assert!(
        finish(
            &mut c,
            fresh,
            Err(SquadError::new(
                "WORKSPACE_PLAN_UNAVAILABLE",
                "Latest missing"
            ))
        )
        .is_none()
    );
    assert!(c.unknown.is_some());
    assert!(c.pending.is_none());
    assert!(c.confirmation.is_none());
}
#[test]
fn mouse_choice_close_retains_the_submitted_write_on_every_pending_screen() {
    for job in [
        Job::Save {
            socket: SOCKET.into(),
            name: "new".into(),
        },
        Job::Remove {
            socket: SOCKET.into(),
            name: "saved".into(),
        },
        Job::Restore(Selection {
            socket: SOCKET.into(),
            name: Some("saved".into()),
            digest: DIGEST.into(),
            layout_only: true,
        }),
    ] {
        let mut c = controller();
        c.screen = Screen::Save;
        let task = c.issue(job);
        assert!(!c.rows().iter().find(|r| r.id == "close").unwrap().disabled);
        assert!(c.choose("close").is_none());
        assert!(!c.active);
        assert_eq!(c.pending.as_ref(), Some(&task));
        assert!(c.open().is_none());
        assert_eq!(c.pending.as_ref(), Some(&task));
    }
}

#[test]
fn app_snapshot_action_is_available_on_every_tab_without_replacing_its_owner() {
    use crate::{
        action::{OpsUiAction, Verb},
        board::{app::Effect, shell::OpsTab},
    };
    for tab in [OpsTab::Agents, OpsTab::Digest] {
        let mut app = crate::board::app::workspace::tests::board();
        app.shell.tab = tab;
        assert!(
            app.bindings()
                .values()
                .any(|a| a.verb == Verb::Ui(OpsUiAction::WorkspaceSnapshots))
        );
        let Effect::Recovery(read) = app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        else {
            panic!("{tab:?}: global snapshots key must emit the existing read job");
        };
        assert_eq!(read.job, Job::List(None));
        assert!(app.recovery_shown());
    }
    let mut app = crate::board::app::workspace::tests::board();
    app.shell.tab = OpsTab::Agents;
    app.focus = 0;
    assert_eq!(
        app.bindings()["w"].verb,
        Verb::Ui(OpsUiAction::WorkspaceSnapshots)
    );
    app.context_menu();
    assert!(
        app.menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.key == "w" && entry.label == "workspace-snapshots")
    );
    app.menu = None;
    app.focus = 1;
    assert!(
        app.bindings()
            .values()
            .any(|a| a.verb == Verb::Ui(OpsUiAction::WorkspaceSnapshots))
    );
    app.focus = 0;
    let Effect::Recovery(read) = app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
    else {
        panic!("Agents w must emit the public read job")
    };
    assert_eq!(read.job, Job::List(None));
    assert!(app.modal_open());
    assert!(app.reload_blocked());
    // Esc is consumed by the modal, never replayed as board Quit.
    assert!(matches!(
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Effect::None
    ));
    assert!(!app.recovery_shown());
    assert!(!app.modal_open());
    assert_eq!(app.focus, 0);
    assert!(
        app.reload_blocked(),
        "the active read still belongs to the lane"
    );
    assert!(matches!(
        app.finished_recovery(load::Completed {
            task: read,
            result: Ok(Outcome::List(
                Inventory::decode(&inventory_value()).unwrap()
            ))
        }),
        Effect::None
    ));
    assert!(!app.recovery_shown());
    assert!(!app.reload_blocked());
}

#[test]
fn rapid_close_reopen_retains_one_active_read_and_coalesces_one_refresh() {
    let mut c = controller();
    let active = c.select("snapshot:saved").unwrap();
    for _ in 0..100 {
        c.close();
        assert!(c.open().is_none());
        assert_eq!(c.pending.as_ref(), Some(&active));
    }
    let refresh = finish(
        &mut c,
        active,
        Ok(Outcome::Show(Box::new(
            Preview::decode(&preview_value(Some("saved"))).unwrap(),
        ))),
    )
    .unwrap();
    assert_eq!(refresh.job, Job::List(Some(SOCKET.into())));
    c.close();
    assert!(
        finish(
            &mut c,
            refresh,
            Ok(Outcome::List(
                Inventory::decode(&inventory_value()).unwrap()
            ))
        )
        .is_none()
    );
    assert!(c.pending.is_none());
    assert!(!c.active);
    assert!(c.preview.is_none());
}

#[test]
fn top_bar_snapshot_padding_uses_the_existing_recovery_owner_on_every_tab() {
    use crate::{
        action::{OpsUiAction, Verb},
        board::{
            app::{Choice, Effect},
            shell::OpsTab,
        },
    };
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    for tab in [OpsTab::Agents, OpsTab::Digest] {
        let mut app = crate::board::app::workspace::tests::board();
        app.shell.tab = tab;
        let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let area = app.workspace_strip_hits.borrow().iter().find_map(|(area, choice)| {
            matches!(choice, Choice::Action(action) if action.verb == Verb::Ui(OpsUiAction::WorkspaceSnapshots)).then_some(*area)
        }).expect("whole top-bar Snapshots control");
        assert_eq!(area.y, 0);
        let Effect::Recovery(read) = app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        ) else {
            panic!("left padding must invoke the existing read lane")
        };
        assert_eq!(read.job, Job::List(None));
        assert!(app.recovery_shown());
    }
}

#[test]
fn help_has_one_effective_snapshot_binding_and_no_retired_reservation() {
    use crate::board::shell::OpsTab;
    let mut app = crate::board::app::workspace::tests::board();
    for (tab, expected) in [
        (OpsTab::Agents, 1),
        (OpsTab::Agents, 1),
        (OpsTab::Digest, 1),
    ] {
        app.shell.tab = tab;
        let value = crate::board::help::model(&app).value().to_string();
        assert_eq!(value.matches("\"keys\":\"w\"").count(), expected);
        assert!(!value.contains("reserved for Workspace snapshots"));
        assert!(!value.contains("no list binding"));
    }
}

#[test]
fn production_lane_reads_public_snapshot_and_routes_typed_completion_to_the_modal() {
    use crate::board::{BoardEvent, app::Effect, lane, shell::OpsTab};
    let f = Fixture::new();
    let core = f.core(&inventory_value(), true);
    let mut app = crate::board::app::workspace::tests::board();
    app.shell.tab = OpsTab::Agents;
    let Effect::Recovery(list) = app.open_recovery() else {
        panic!("inventory job")
    };
    let BoardEvent::Recovery(completed) = lane::run(&core, None, lane::Job::Recovery(list)) else {
        panic!("typed event")
    };
    let Effect::Recovery(show) = app.finished_recovery(completed) else {
        panic!("selected read-only preview")
    };
    assert_eq!(
        show.job,
        Job::Show {
            socket: SOCKET.into(),
            name: None
        }
    );
    let core = f.core(&preview_value(None), true);
    let BoardEvent::Recovery(completed) = lane::run(&core, None, lane::Job::Recovery(show)) else {
        panic!("preview event")
    };
    assert!(matches!(app.finished_recovery(completed), Effect::None));
    assert!(app.recovery_shown());
    let c = app.recovery.as_mut().unwrap();
    review(c);
    app.invalidate_frame_geometry();
    let c = app.recovery.as_mut().unwrap();
    assert!(!c.readable.get());
    assert!(c.choose("confirm").is_none());
    assert!(c.pending.is_none());
}

fn click(c: &mut Controller, id: &str) -> Option<Task> {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    render(c, 160, 50);
    let rect = {
        let state = if c.screen == Screen::List {
            &c.list
        } else if c.screen == Screen::Save {
            &c.form
        } else {
            &c.panel
        };
        let state = state.borrow();
        state
            .frame
            .as_ref()
            .unwrap()
            .list
            .as_ref()
            .unwrap()
            .geometry
            .iter()
            .find(|row| row.id == id)
            .unwrap()
            .visible
    };
    assert!(rect.width > 0 && rect.height > 0);
    c.input(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    }))
}

#[test]
fn painted_mouse_actions_review_confirm_once_and_close_submitted_work() {
    let mut c = controller();
    assert!(click(&mut c, "details").is_none());
    assert_eq!(c.screen, Screen::Preview);
    assert!(click(&mut c, "review").is_none());
    assert_eq!(c.screen, Screen::Confirm);
    let submitted = click(&mut c, "confirm").unwrap();
    assert!(matches!(submitted.job, Job::Restore(_)));
    assert!(click(&mut c, "close").is_none());
    assert!(!c.active);
    assert_eq!(c.pending.as_ref(), Some(&submitted));
    assert!(c.open().is_none());
    assert_eq!(c.pending.as_ref(), Some(&submitted));
}

#[test]
fn stale_mouse_geometry_cannot_acquire_restore_authority() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut c = controller();
    review(&mut c);
    let rect = c
        .panel
        .borrow()
        .frame
        .as_ref()
        .unwrap()
        .list
        .as_ref()
        .unwrap()
        .geometry
        .iter()
        .find(|r| r.id == "confirm")
        .unwrap()
        .visible;
    c.invalidate();
    assert!(
        c.input(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE
        }))
        .is_none()
    );
    assert!(c.pending.is_none());
}

/// Explicit task-owned review evidence, never a frozen fixture recorder.
#[test]
#[ignore = "recovery TUI matrix; requires TMT_RECOVERY_CAPTURE task-owned path"]
fn capture_recovery_packet() {
    let path = std::env::var("TMT_RECOVERY_CAPTURE").expect("task-owned output path");
    let mut captures = Vec::new();
    crate::status::with_now_ms(1_791_600_000_000, || {
        for scene in [
            "list",
            "preview",
            "confirm",
            "save",
            "delete",
            "pending",
            "applied",
            "partial",
            "unknown",
            "unsupported",
            "empty",
        ] {
            for width in [160, 100, 80, 60] {
                for height in [36, 18] {
                    for (theme, look) in picker_surface::evidence::looks().into_iter().enumerate() {
                        let mut c = controller();
                        match scene {
                            "preview" => c.screen = Screen::Preview,
                            "confirm" => {
                                c.screen = Screen::Confirm;
                                c.confirmation = c.selection();
                            }
                            "save" => c.screen = Screen::Save,
                            "delete" => c.screen = Screen::Delete,
                            "pending" => {
                                c.screen = Screen::Results;
                                c.issue(Job::Restore(c.selection().unwrap()));
                            }
                            "applied" | "partial" => {
                                c.screen = Screen::Results;
                                let r = model::Restore::decode(&restore_value(
                                    DIGEST,
                                    if scene == "applied" {
                                        "completed"
                                    } else {
                                        "partial"
                                    },
                                ))
                                .unwrap();
                                c.details = surface::results(&r);
                                c.restored = Some(r);
                                c.preview = None;
                            }
                            "unknown" => {
                                c.screen = Screen::Results;
                                c.preview = None;
                                c.unknown = Some("Core result unavailable. Inspect original socket before deciding another action; no automatic retry.".into());
                            }
                            "unsupported" => {
                                c.entries.clear();
                                c.preview = None;
                                c.failure = Some(SquadError::new(
                                    "WORKSPACE_HOST_UNSUPPORTED",
                                    "tmux only",
                                ));
                            }
                            "empty" => {
                                c.entries.clear();
                                c.preview = None;
                                c.notice = "No snapshots. A successful Core capture is needed before Save latest capture.".into();
                            }
                            _ => {}
                        }
                        let mut app = crate::board::app::workspace::tests::board();
                        app.shell.tab = crate::board::shell::OpsTab::Agents;
                        app.view.as_mut().unwrap().look = look;
                        app.recovery = Some(c);
                        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                        let buffer = terminal.backend().buffer();
                        let c = app.recovery.as_ref().unwrap();
                        let state = if c.screen == Screen::List {
                            &c.list
                        } else if c.screen == Screen::Save {
                            &c.form
                        } else {
                            &c.panel
                        };
                        let state = state.borrow();
                        let cells = buffer.content.iter().map(|cell| json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                        let lines = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                            .collect::<Vec<_>>();
                        captures.push(json!({"scene":scene,"theme":theme,"width":width,"height":height,"cells":cells,"lines":lines,"readable":c.readable.get(),"hits":format!("{:?}",state.frame.as_ref().map(|f|&f.hits)),"list":format!("{:?}",state.frame.as_ref().and_then(|f|f.list.as_ref()))}));
                    }
                }
            }
        }
    });
    assert_eq!(captures.len(), 352);
    std::fs::write(path, serde_json::to_vec(&captures).unwrap()).unwrap();
}

#[test]
#[ignore = "explicit fitted review matrix; requires TMT_OVERLAY_CUE_OUTPUT"]
fn capture_fitted_recovery_reviews() {
    std::env::var("TMT_OVERLAY_CUE_OUTPUT").expect("task-owned output directory");
    crate::status::with_now_ms(1_791_600_000_000, || {
        for (scene, screen) in [("restore", Screen::Confirm), ("delete", Screen::Delete)] {
            for (theme, look) in picker_surface::evidence::looks().into_iter().enumerate() {
                for width in [160, 100, 60] {
                    for height in [36, 24, 8] {
                        let mut c = controller();
                        c.screen = screen;
                        c.confirmation = c.selection();
                        let mut app = crate::board::app::workspace::tests::board();
                        app.view.as_mut().unwrap().look = look;
                        app.recovery = Some(c);
                        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                        let c = app.recovery.as_ref().unwrap();
                        picker_surface::evidence::capture(
                            &format!("recovery-{scene}-{theme}-{width}x{height}"),
                            terminal.backend().buffer(),
                            &c.panel.borrow(),
                        );
                    }
                }
            }
        }
    });
}
