use super::*;
use model::{BindReport, ScanObservation};
use serde_json::{Value, json};

mod performance;
#[test]
fn app_discovery_name_owns_typing_and_tab_to_actions_releases_destination_keys() {
    use crate::board::{app::Effect, shell::OpsTab};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::board::app::workspace::tests::board();
    let mut c = controller();
    assert!(c.choose("bind").is_none());
    assert_eq!(c.screen, Screen::Form);
    app.discovery = Some(c);
    for ch in "q123".chars() {
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)),
            Effect::None
        );
    }
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert_eq!(app.discovery.as_ref().unwrap().editor.as_str(), "q123");
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(!app.discovery.as_ref().unwrap().owns_typing());
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE)),
        Effect::None
    );
    assert_eq!(app.shell.tab, OpsTab::Digest);
    assert!(!app.discovery_shown());
    assert!(app.discovery.as_ref().unwrap().pending.is_none());
    assert_eq!(app.discovery.as_ref().unwrap().editor.as_str(), "q123");
}

const SOCKET: &str = "/tmp/private-discovery.sock";
const UUID: &str = "11111111-1111-4111-8111-111111111111";
const DIGEST: &str = "bf591b8c42ee4d8f2cd13fb00375c9a7807bca32370c9e9c191c9698b32dade8";
fn document() -> Value {
    json!({"version":1,"observedAtMs":1000,"truncated":false,"sources":[{"host":"tmux","socket":SOCKET,"status":"ok","reason":null}],"candidates":[{
        "host":"tmux","socket":SOCKET,"server":{"pid":123,"start":"server-native-start","identity":null},"session":"work","window":"0","pane":"%3","target":"work:0.0","native":{"pid":456,"start":"pane-native-start"},
        "runtime":{"driver":"codex","state":"unknown","reason":"executable_hint_is_advisory","recordedState":null},"identity":null,"state":"unknown","reason":null,"duplicate":false,
        "display":{"title":"#{run-shell:never} $(never)\nnew row\tstill one title","command":"codex","cwd":"/tmp/private","truncated":false},"adoptable":true,"expect":DIGEST
    }]})
}
fn scope() -> ScanScope {
    ScanScope {
        host: Some("tmux".into()),
        socket: Some(SOCKET.into()),
    }
}
fn controller() -> Controller {
    let mut c = Controller::new(1, scope());
    let task = c.scan(scope()).unwrap();
    c.loaded(load::Completed {
        task,
        result: Ok(Outcome::Scan(ScanObservation::decode(&document()).unwrap())),
    });
    let key = c.observation.as_ref().unwrap().panes[0].key.clone();
    c.select(&key);
    c
}
fn bind(c: &mut Controller) -> Task {
    assert!(c.review(AgentSelector::NewName("worker".into()), false));
    c.readable.set(true);
    c.confirm().unwrap()
}
fn finished(c: &mut Controller, task: Task, result: Result<Outcome, SquadError>) -> Completion {
    c.loaded(load::Completed { task, result })
}

#[test]
fn scan_preserves_unknown_runtime_and_literal_display_without_readiness_claim() {
    let v = document();
    let scan = ScanObservation::decode(&v).unwrap();
    let p = &scan.panes[0];
    assert!(p.adoptable);
    assert_eq!(p.runtime_state, "unknown");
    assert_eq!(p.title, v["candidates"][0]["display"]["title"]);
    assert!(p.identity.is_none());
    assert_eq!(scan.observed_at_ms, 1000);
}
#[test]
fn unavailable_sources_are_observations_not_empty_success() {
    for status in ["unsupported", "permission_denied", "offline", "unknown"] {
        let v = json!({"version":1,"observedAtMs":1000,"truncated":false,"sources":[{"host":"herdr","socket":null,"status":status,"reason":"Not available"}],"candidates":[]});
        let s = ScanObservation::decode(&v).unwrap();
        assert_eq!(s.sources[0].status, status);
        assert_eq!(s.sources[0].reason.as_deref(), Some("Not available"));
    }
}
#[test]
fn unverified_hook_only_stale_and_duplicate_evidence_cannot_bind() {
    for state in ["hook_only", "stale", "duplicate", "unknown"] {
        let mut v = document();
        let p = &mut v["candidates"][0];
        p["state"] = state.into();
        p["reason"] = "unverified".into();
        p["adoptable"] = false.into();
        p["expect"] = Value::Null;
        let scan = ScanObservation::decode(&v).unwrap();
        let mut c = controller();
        c.observation = Some(scan);
        assert!(!c.review(AgentSelector::NewName("worker".into()), false));
        assert!(c.confirm().is_none());
    }
}
#[test]
fn incomplete_scan_cannot_carry_adoption_authority() {
    let mut v = document();
    v["truncated"] = true.into();
    assert!(ScanObservation::decode(&v).is_err());
    v["candidates"][0]["adoptable"] = false.into();
    v["candidates"][0]["expect"] = Value::Null;
    assert!(ScanObservation::decode(&v).unwrap().truncated);
}
#[test]
fn malformed_digest_source_native_and_duplicate_keys_refuse() {
    for field in ["expect", "native", "server", "pane"] {
        let mut v = document();
        v["candidates"][0][field] = "not-valid".into();
        assert!(ScanObservation::decode(&v).is_err(), "{field}");
    }
    let mut v = document();
    v["sources"][0]["socket"] = "/tmp/another.sock".into();
    assert!(ScanObservation::decode(&v).is_err());
    let mut v = document();
    let duplicate = v["candidates"][0].clone();
    v["candidates"].as_array_mut().unwrap().push(duplicate);
    assert!(ScanObservation::decode(&v).is_err());
}
#[test]
fn opening_and_cancel_have_no_command_or_mutation() {
    let mut c = Controller::new(1, scope());
    c.open();
    assert!(c.pending.is_none());
    assert_eq!(c.serial, 0);
    c.close();
    assert!(!c.reload_blocked());
    assert!(c.confirm().is_none());
    assert_eq!(c.serial, 0);
}
#[test]
fn confirmation_freezes_target_digest_uuid_and_lifetime_once() {
    let mut c = controller();
    assert!(c.review(AgentSelector::ExistingUuid(UUID.into()), true));
    c.readable.set(true);
    let task = c.confirm().unwrap();
    let Job::Bind(selection) = &task.job else {
        panic!("bind")
    };
    assert_eq!(selection.key.socket.as_deref(), Some(SOCKET));
    assert_eq!(selection.key.pane, "%3");
    assert_eq!(selection.expect, DIGEST);
    assert_eq!(selection.selector, AgentSelector::ExistingUuid(UUID.into()));
    assert!(selection.save);
    assert!(c.confirm().is_none());
    assert!(c.scan(scope()).is_none());
    c.close();
    c.open();
    assert_eq!(c.pending.as_ref(), Some(&task));
    assert!(c.confirm().is_none());
}
#[test]
fn resize_or_selection_change_revokes_confirmation() {
    let mut c = controller();
    assert!(c.review(AgentSelector::NewName("worker".into()), false));
    c.readable.set(true);
    c.invalidate();
    assert!(c.confirm().is_none());
    assert!(c.review(AgentSelector::NewName("worker".into()), false));
    c.readable.set(true);
    let key = c.selected.clone().unwrap();
    c.select(&key);
    assert!(c.confirm().is_none());
}
#[test]
fn clipped_confirmation_is_effect_free() {
    let mut c = controller();
    assert!(c.review(AgentSelector::NewName("worker".into()), false));
    assert!(c.confirm().is_none());
    assert!(c.pending.is_none());
}
#[test]
fn scan_coalesces_obsolete_generations_without_changing_authority() {
    let mut c = Controller::new(1, scope());
    let active = c.scan(scope()).unwrap();
    for _ in 0..100 {
        assert!(c.scan(scope()).is_none());
        assert_eq!(c.pending.as_ref(), Some(&active));
    }
    let next = finished(
        &mut c,
        active,
        Ok(Outcome::Scan(ScanObservation::decode(&document()).unwrap())),
    )
    .task
    .unwrap();
    assert!(c.observation.is_none());
    assert!(c.confirmation.is_none());
    finished(
        &mut c,
        next,
        Ok(Outcome::Scan(ScanObservation::decode(&document()).unwrap())),
    );
    assert_eq!(c.serial, 2);
    assert!(c.observation.is_some());
}
#[test]
fn close_reopen_retains_one_active_scan_until_completion() {
    let mut c = Controller::new(1, scope());
    let active = c.scan(scope()).unwrap();
    for _ in 0..100 {
        c.close();
        c.open();
        assert_eq!(c.pending.as_ref(), Some(&active));
        assert!(c.scan(scope()).is_none());
    }
    c.close();
    finished(
        &mut c,
        active,
        Ok(Outcome::Scan(ScanObservation::decode(&document()).unwrap())),
    );
    assert!(c.observation.is_none());
    assert!(c.pending.is_none());
    assert_eq!(c.serial, 1);
}
#[test]
fn missing_anchor_after_rescan_does_not_select_a_neighbor() {
    let mut c = controller();
    let task = c.scan(scope()).unwrap();
    let mut v = document();
    v["candidates"][0]["pane"] = "%8".into();
    finished(
        &mut c,
        task,
        Ok(Outcome::Scan(ScanObservation::decode(&v).unwrap())),
    );
    assert!(c.selected.is_none());
    assert!(!c.review(AgentSelector::NewName("worker".into()), false));
}
#[test]
fn partial_and_unknown_reports_survive_close_rescan_without_replay() {
    let mut c = controller();
    let task = bind(&mut c);
    c.close();
    finished(
        &mut c,
        task,
        Ok(Outcome::Bind(BindReport::Partial {
            code: "RECONCILIATION_FAILED".into(),
            message: "Publication uncertain".into(),
        })),
    );
    c.open();
    assert_eq!(c.attempts.len(), 1);
    assert!(c.reload_blocked());
    assert!(c.pending.is_none());
    let task = c.scan(scope()).unwrap();
    finished(
        &mut c,
        task,
        Ok(Outcome::Scan(ScanObservation::decode(&document()).unwrap())),
    );
    let task = bind(&mut c);
    c.close();
    finished(
        &mut c,
        task,
        Err(SquadError::new("SQUAD_CORE_UNAVAILABLE", "Lost completion")),
    );
    c.open();
    assert_eq!(c.attempts.len(), 2);
    assert!(c.attempts[1].result.is_err());
    assert!(c.reload_blocked());
    assert!(c.pending.is_none());
    assert_eq!(c.attempts[0].selection.key.pane, "%3");
}
#[test]
fn successful_bound_uuid_returns_for_public_refresh_only_once() {
    let mut c = controller();
    let task = bind(&mut c);
    let completed = finished(
        &mut c,
        task.clone(),
        Ok(Outcome::Bind(BindReport::Bound {
            id: UUID.into(),
            name: "renamed".into(),
            pane: "%3".into(),
            lifetime: "temporary".into(),
            already_bound: false,
        })),
    );
    assert_eq!(completed.bound_uuid.as_deref(), Some(UUID));
    assert!(c.observation.is_none());
    assert!(
        finished(&mut c, task, Err(model::invalid()))
            .bound_uuid
            .is_none()
    );
    assert_eq!(c.attempts.len(), 1);
}
#[test]
fn ordinary_unknown_runtime_is_not_a_refusal_or_an_auto_adoption() {
    let c = controller();
    assert_eq!(c.selected_pane().unwrap().runtime_state, "unknown");
    assert!(c.pending.is_none());
    assert!(c.confirmation.is_none());
}
#[test]
fn literal_option_like_name_and_uuid_use_separate_operands() {
    let mut c = controller();
    assert!(c.review(AgentSelector::NewName("--identity=$(never)".into()), false));
    c.readable.set(true);
    let job = c.confirm().unwrap().job;
    let (args, operands) = job.arguments();
    assert_eq!(operands, vec!["%3", "--identity=$(never)"]);
    assert!(!args.iter().any(|s| s.contains("never")));
    let mut c = controller();
    assert!(c.review(AgentSelector::ExistingUuid(UUID.into()), true));
    c.readable.set(true);
    let (args, operands) = c.confirm().unwrap().job.arguments();
    assert_eq!(operands, vec!["%3"]);
    assert!(args.windows(2).any(|s| s == ["--identity", UUID]));
    assert!(args.contains(&"--save".into()));
}
#[test]
fn public_nonzero_partial_refused_and_idempotent_receipts_keep_their_meaning() {
    let mut c = controller();
    let Job::Bind(selection) = bind(&mut c).job else {
        panic!("bind")
    };
    for status in ["partial", "refused"] {
        let v = json!({"status":status,"pane":"%3","error":{"code":"PANE_CHANGED","message":"Native target changed"}});
        let r = BindReport::decode(&v, &selection, false).unwrap();
        assert_eq!(matches!(r, BindReport::Partial { .. }), status == "partial");
        assert!(BindReport::decode(&v, &selection, true).is_err());
    }
    let v = json!({"version":1,"status":"already_bound","bound":true,"id":UUID,"name":"worker","pane":"%3","lifetime":"temporary"});
    assert!(matches!(
        BindReport::decode(&v, &selection, true).unwrap(),
        BindReport::Bound {
            already_bound: true,
            ..
        }
    ));
    let mut v = v;
    v["pane"] = "%4".into();
    assert!(BindReport::decode(&v, &selection, true).is_err());
}
#[test]
fn existing_uuid_is_never_accepted_from_a_name_or_case_alias() {
    let mut c = controller();
    for input in ["@worker", "worker", "AAAAAAAA-1111-4111-8111-111111111111"] {
        assert!(!c.review(AgentSelector::ExistingUuid(input.into()), false));
        assert!(c.confirm().is_none());
    }
}

fn paint(c: &Controller, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| c.render(frame, picker_surface::evidence::looks()[0], frame.area()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
fn click(c: &mut Controller, id: &str) -> Option<Task> {
    use ratatui::crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let rect =
        c.ui.borrow()
            .frame
            .as_ref()
            .unwrap()
            .list
            .as_ref()
            .unwrap()
            .geometry
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .visible;
    assert!(rect.width > 0 && rect.height > 0, "visible {id}");
    c.input(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    }))
}
#[test]
fn real_surface_confirmation_requires_all_target_notes_and_choices_visible() {
    let mut c = controller();
    assert!(c.review(AgentSelector::NewName("worker".into()), false));
    assert!(paint(&c, 160, 36).contains("Bind agent"));
    assert!(c.readable.get());
    c.ui.borrow_mut().invalidate();
    c.readable.set(false);
    paint(&c, 60, 18);
    assert!(!c.readable.get());
    assert!(c.confirm().is_none());
    assert!(c.pending.is_none());
    paint(&c, 160, 36);
    assert!(c.readable.get());
    assert!(click(&mut c, "confirm").is_some());
    assert!(c.confirm().is_none());
}
#[test]
fn binding_review_fits_the_target_without_changing_inventory_height() {
    let mut c = controller();
    paint(&c, 160, 80);
    assert_eq!(c.ui.borrow().frame.as_ref().unwrap().areas.outer.height, 38);
    assert!(c.review(AgentSelector::NewName("worker".into()), false));
    let text = paint(&c, 160, 80);
    assert!(text.contains("Bind agent"));
    assert_eq!(
        c.rows()
            .iter()
            .find(|row| row.id == "confirm")
            .unwrap()
            .label,
        "Bind agent"
    );
    assert!(c.readable.get());
    let outer = c.ui.borrow().frame.as_ref().unwrap().areas.outer;
    assert!(
        outer.height < 38,
        "short binding review must not demand 38 rows"
    );
    assert!(outer.width <= 120);
    c.readable.set(false);
    c.ui.borrow_mut().invalidate();
    assert!(
        c.confirm().is_none(),
        "a shorter label does not bypass current paint"
    );
}
#[test]
fn discovery_confirmation_keeps_escape_hint_whole_at_low_height() {
    for width in [60, 100, 160] {
        for height in [8, 24, 36] {
            let mut c = controller();
            assert!(c.review(AgentSelector::NewName("worker".into()), false));
            let mut app = crate::board::app::workspace::tests::board();
            app.discovery = Some(c);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let c = app.discovery.as_ref().unwrap();
            let footer = c.ui.borrow().frame.as_ref().unwrap().areas.footer;
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
#[test]
fn short_binding_review_keeps_its_subject_and_cancel_visible() {
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut c = controller();
    assert!(c.review(AgentSelector::NewName("worker".into()), false));
    let mut app = crate::board::app::workspace::tests::board();
    app.discovery = Some(c);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 8)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let c = app.discovery.as_ref().unwrap();
    let state = c.ui.borrow();
    let map = state.frame.as_ref().unwrap();
    assert_eq!(map.areas.outer, ratatui::layout::Rect::new(0, 0, 60, 8));
    assert!(map.areas.content.height > 0);
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Bind %3"), "{text}");
    assert!(text.contains("worker"), "{text}");
    assert!(
        map.hits
            .iter()
            .any(|hit| hit.row_id.as_deref() == Some("cancel") && hit.rect.height > 0)
    );
    assert!(
        !c.readable.get(),
        "clipped target notes cannot authorize binding"
    );
    drop(state);
    assert!(
        app.discovery
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
    assert!(text.contains("Bind agent"));
    assert!(text.contains("Bind %3"));
    assert!(!app.discovery.as_ref().unwrap().readable.get());
}
#[test]
fn actual_mouse_scan_select_review_and_cancel_emit_no_binding() {
    let mut c = Controller::new(1, scope());
    paint(&c, 160, 36);
    let task = click(&mut c, "scan").unwrap();
    assert!(matches!(task.job, Job::Scan(_)));
    finished(
        &mut c,
        task,
        Ok(Outcome::Scan(ScanObservation::decode(&document()).unwrap())),
    );
    paint(&c, 160, 36);
    assert!(click(&mut c, "candidate:0").is_none());
    assert_eq!(c.screen, Screen::Details);
    paint(&c, 160, 36);
    assert!(click(&mut c, "bind").is_none());
    assert_eq!(c.screen, Screen::Form);
    assert!(
        c.input(&ratatui::crossterm::event::Event::Paste("worker".into()))
            .is_none()
    );
    paint(&c, 160, 36);
    assert!(click(&mut c, "review").is_none());
    assert_eq!(c.screen, Screen::Confirm);
    paint(&c, 160, 36);
    assert!(click(&mut c, "cancel").is_none());
    assert!(c.pending.is_none());
    assert_eq!(c.serial, 1);
}
#[test]
fn untrusted_candidate_text_is_inert_on_the_shared_surface() {
    let c = controller();
    let rendered = paint(&c, 160, 36);
    assert!(rendered.contains("#{run-shell:never} $(never)\\nnew row\\tstill one title"));
    assert!(!rendered.contains('\u{1b}'));
    assert!(c.pending.is_none());
}

#[test]
fn candidate_preview_is_bounded_while_details_keep_complete_literal_evidence() {
    use unicode_width::UnicodeWidthStr;
    let mut value = document();
    let title = "#{literal} ".repeat(93);
    let session = "untrusted-session-".repeat(50);
    value["candidates"][0]["display"]["title"] = title.clone().into();
    value["candidates"][0]["session"] = session.clone().into();
    let mut c = controller();
    c.observation = Some(ScanObservation::decode(&value).unwrap());
    c.choose("back");
    let candidate = &c.rows()[0];
    assert!(candidate.label.width() <= 116);
    assert!(candidate.label.contains('…'));
    assert!(candidate.label.ends_with(" · review"));
    c.choose("candidate:0");
    let details = c.notes();
    assert!(details.contains(&format!("Title: {title}")));
    assert!(details.iter().any(|line| line.contains(&session)));
}

#[test]
fn name_field_uses_shared_editor_geometry_and_refuses_stale_pointer() {
    use ratatui::crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let mut c = controller();
    c.choose("bind");
    c.input(&Event::Paste("abcdef".into()));
    paint(&c, 160, 36);
    let area = c.editor_area.get().unwrap();
    let pointer = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x + 2,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    });
    assert!(c.input(&pointer).is_none());
    assert_eq!(c.editor.cursor(), 2);
    c.input(&Event::Paste("Z".into()));
    assert_eq!(c.editor.as_str(), "abZcdef");
    c.input(&Event::Resize(60, 18));
    let cursor = c.editor.cursor();
    c.input(&pointer);
    assert_eq!(c.editor.cursor(), cursor);
    assert!(c.pending.is_none());
}
#[test]
fn agents_default_and_modal_open_close_use_the_effective_table_without_scan() {
    use crate::{
        action::{OpsUiAction, Verb},
        board::{app::Effect, shell::OpsTab},
    };
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::board::app::workspace::tests::board();
    for tab in [OpsTab::Overview, OpsTab::Digest] {
        app.shell.tab = tab;
        assert!(
            !app.bindings()
                .values()
                .any(|a| a.verb == Verb::Ui(OpsUiAction::DiscoverAgents))
        );
    }
    app.shell.tab = OpsTab::Agents;
    app.focus = 0;
    assert_eq!(
        app.bindings()["s"].verb,
        Verb::Ui(OpsUiAction::DiscoverAgents)
    );
    app.focus = 1;
    assert!(
        !app.bindings()
            .values()
            .any(|a| a.verb == Verb::Ui(OpsUiAction::DiscoverAgents))
    );
    app.focus = 0;
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
        Effect::None
    );
    assert!(app.discovery_shown());
    assert!(app.modal_open());
    assert!(app.reload_blocked());
    assert!(app.discovery.as_ref().unwrap().pending.is_none());
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Effect::None
    );
    assert!(!app.discovery_shown());
    assert!(!app.modal_open());
    assert!(!app.reload_blocked());
}
#[test]
fn help_has_one_effective_discovery_key_only_in_agents() {
    use crate::board::shell::OpsTab;
    let mut app = crate::board::app::workspace::tests::board();
    for (tab, expected) in [
        (OpsTab::Agents, 1),
        (OpsTab::Overview, 0),
        (OpsTab::Digest, 0),
    ] {
        app.shell.tab = tab;
        app.focus = 0;
        let value = crate::board::help::model(&app).value().to_string();
        assert_eq!(value.matches("\"keys\":\"s\"").count(), expected);
    }
}

struct PublicCommandFixture(std::path::PathBuf);
impl PublicCommandFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ops-discovery-command-{}",
            crate::id::new_v4().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        crate::test_support::write_ready_executable(
            &root.join("core"),
            r#"#!/bin/sh
cd "$(dirname "$0")" || exit 1
printf '%s\000' "$@" >> calls
cat response
exit "$(cat exit)"
"#,
        );
        Self(root)
    }
    fn response(&self, value: &Value, exit: u8) {
        std::fs::write(self.0.join("response"), value.to_string()).unwrap();
        std::fs::write(self.0.join("exit"), exit.to_string()).unwrap();
    }
    fn core(&self) -> crate::core::Core {
        crate::core::Core::at(self.0.join("core"))
    }
    fn arguments(&self) -> Vec<String> {
        std::fs::read(self.0.join("calls"))
            .unwrap()
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8(s.to_vec()).unwrap())
            .collect()
    }
}
impl Drop for PublicCommandFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn public_scan_and_guarded_add_execute_literal_argv_and_preserve_nonzero_receipts() {
    let fixture = PublicCommandFixture::new();
    fixture.response(&document(), 0);
    let mut c = Controller::new(1, scope());
    let task = c.scan(scope()).unwrap();
    c.loaded(load::complete(&fixture.core(), task));
    assert_eq!(
        fixture.arguments(),
        ["discover", "--host", "tmux", "--socket", SOCKET, "--json"]
    );
    assert!(c.observation.is_some());
    assert!(c.attempts.is_empty());
    std::fs::remove_file(fixture.0.join("calls")).unwrap();
    let key = c.observation.as_ref().unwrap().panes[0].key.clone();
    c.select(&key);
    assert!(c.review(AgentSelector::NewName("--identity=$(never)".into()), false));
    c.readable.set(true);
    let task = c.confirm().unwrap();
    fixture.response(&json!({"status":"partial","pane":"%3","error":{"code":"STORAGE_ERROR","message":"Publication uncertain"}}), 1);
    c.loaded(load::complete(&fixture.core(), task));
    assert_eq!(
        fixture.arguments(),
        [
            "add",
            "--expect",
            DIGEST,
            "--socket",
            SOCKET,
            "--json",
            "--",
            "%3",
            "--identity=$(never)"
        ]
    );
    assert!(matches!(
        c.attempts[0].result,
        Ok(BindReport::Partial { .. })
    ));
    assert!(c.confirm().is_none());
    assert!(c.reload_blocked());
    // The ordinary JSON adapter keeps its existing error semantics.
    assert_eq!(
        fixture.core().json(&["irrelevant"]).unwrap_err().code,
        "STORAGE_ERROR"
    );
}

#[test]
fn scoped_public_scan_rejects_another_socket_without_adoption_authority() {
    let fixture = PublicCommandFixture::new();
    fixture.response(&document(), 0);
    let mut c = Controller::new(
        1,
        ScanScope {
            host: Some("tmux".into()),
            socket: Some("/tmp/other-server.sock".into()),
        },
    );
    let task = c.scan(c.scope.clone()).unwrap();
    c.loaded(load::complete(&fixture.core(), task));
    assert!(c.failure.is_some());
    assert!(c.observation.is_none());
    assert!(c.attempts.is_empty());
    assert!(c.confirm().is_none());
}

#[test]
fn successful_public_refresh_reveals_the_exact_uuid_and_existing_membership() {
    use crate::board::app::workspace::tests::{MEMBER, ROOM};
    use crate::board::{app::Effect, workspace::OpsTarget};
    let mut app = crate::board::app::workspace::tests::board();
    app.workspace.expanded.clear();
    app.search = "does not match".into();
    app.discovery = Some(controller());
    let task = bind(app.discovery.as_mut().unwrap());
    let selected = app.workspace.selected.clone();
    assert_eq!(
        app.finished_discovery(load::Completed {
            task,
            result: Ok(Outcome::Bind(BindReport::Bound {
                id: MEMBER.into(),
                name: "worker".into(),
                pane: "%3".into(),
                lifetime: "temporary".into(),
                already_bound: true,
            })),
        }),
        Effect::Refresh
    );
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.discovered_uuid.as_deref(), Some(MEMBER));
    let mut loaded = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    loaded.view = Ok(crate::board::app::workspace::tests::board()
        .view
        .take()
        .unwrap());
    app.apply(loaded);
    assert!(app.discovered_uuid.is_none());
    assert!(app.search.is_empty());
    assert!(app.workspace.expanded.contains(ROOM));
    assert_eq!(
        app.workspace.selected,
        Some(OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        })
    );
    assert!(app.workspace_selection_visible());
}

#[test]
fn unknown_pool_membership_retains_refresh_selection_until_it_is_observed() {
    use crate::board::app::workspace::tests::MEMBER;
    use crate::board::workspace::OpsTarget;
    let mut app = crate::board::app::workspace::tests::board();
    let roster = &mut app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster;
    roster.squads.clear();
    roster.membership_complete = false;
    app.discovered_uuid = Some(MEMBER.into());
    let selected = app.workspace.selected.clone();
    app.discovery_refreshed();
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.discovered_uuid.as_deref(), Some(MEMBER));
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .membership_complete = true;
    app.discovery_refreshed();
    assert!(app.discovered_uuid.is_none());
    assert!(app.workspace.pool_expanded);
    assert_eq!(
        app.workspace.selected,
        Some(OpsTarget::Member {
            room_id: None,
            identity_id: MEMBER.into(),
        })
    );
    assert!(app.workspace_selection_visible());
}

#[test]
#[ignore = "explicit fitted review matrix; requires TMT_OVERLAY_CUE_OUTPUT"]
fn capture_fitted_discovery_review() {
    std::env::var("TMT_OVERLAY_CUE_OUTPUT").expect("task-owned output directory");
    crate::status::with_now_ms(1_791_600_000_000, || {
        for (theme, look) in picker_surface::evidence::looks().into_iter().enumerate() {
            for width in [160, 100, 60] {
                for height in [36, 24, 8] {
                    let mut c = controller();
                    assert!(c.review(AgentSelector::NewName("worker".into()), false));
                    let mut app = crate::board::app::workspace::tests::board();
                    app.view.as_mut().unwrap().look = look;
                    app.discovery = Some(c);
                    let mut terminal =
                        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                            .unwrap();
                    terminal
                        .draw(|frame| crate::board::view::render(frame, &app))
                        .unwrap();
                    let c = app.discovery.as_ref().unwrap();
                    picker_surface::evidence::capture(
                        &format!("discovery-bind-{theme}-{width}x{height}"),
                        terminal.backend().buffer(),
                        &c.ui.borrow(),
                    );
                }
            }
        }
    });
}
